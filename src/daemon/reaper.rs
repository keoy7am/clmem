use std::sync::Arc;

use crate::models::{Config, Event, EventKind, ProcessInfo, ProcessState};
use crate::platform::Platform;

/// Orphan process reaper implementing the safety-first cleanup protocol.
///
/// Safety rules:
/// - ACTIVE processes: NEVER touched
/// - IDLE processes: Only with `force` flag
/// - STALE processes: Only with `force` flag
/// - ORPHAN processes: Cleaned automatically (graceful terminate -> wait -> kill)
///
/// Termination flow: SIGTERM -> wait grace period -> SIGKILL.
/// On Windows: kill process tree + EmptyWorkingSet.
pub struct Reaper {
    platform: Arc<dyn Platform>,
    config: Config,
}

impl Reaper {
    pub fn new(platform: Arc<dyn Platform>, config: Config) -> Self {
        Self { platform, config }
    }

    /// Reap orphan processes from the provided list.
    /// Only processes in ORPHAN state are eligible.
    /// Returns events describing what happened.
    pub async fn reap_orphans(&self, processes: &[ProcessInfo]) -> Vec<Event> {
        let mut events = Vec::new();

        let orphans: Vec<&ProcessInfo> = processes
            .iter()
            .filter(|p| p.state == ProcessState::Orphan)
            .collect();

        if orphans.is_empty() {
            return events;
        }

        tracing::info!(count = orphans.len(), "Reaping orphan processes");

        for proc in orphans {
            events.push(Event::new(EventKind::CleanupStarted { pid: proc.pid }));

            let success = self.terminate_gracefully(proc.pid).await;

            tracing::info!(pid = proc.pid, success, "Orphan cleanup completed");
            events.push(Event::new(EventKind::CleanupCompleted {
                pid: proc.pid,
                success,
            }));
        }

        events
    }

    /// Force cleanup of specific PIDs. The `force` flag controls whether non-orphan
    /// processes can be terminated.
    ///
    /// Returns `(cleaned_count, failed_count)`.
    pub async fn cleanup_pids(
        &self,
        pids: &[u32],
        force: bool,
        processes: &[ProcessInfo],
    ) -> (u32, u32) {
        let mut cleaned = 0u32;
        let mut failed = 0u32;

        for &pid in pids {
            // Find the process info
            let proc_info = processes.iter().find(|p| p.pid == pid);

            // Safety check: refuse to touch ACTIVE processes without force
            if let Some(info) = proc_info {
                match info.state {
                    ProcessState::Active => {
                        if !force {
                            tracing::warn!(pid, "Refusing to clean ACTIVE process without --force");
                            failed += 1;
                            continue;
                        }
                        tracing::warn!(pid, "Force-cleaning ACTIVE process");
                    }
                    ProcessState::Idle | ProcessState::Stale => {
                        if !force {
                            tracing::warn!(
                                pid,
                                state = %info.state,
                                "Refusing to clean non-orphan process without --force"
                            );
                            failed += 1;
                            continue;
                        }
                        tracing::info!(pid, state = %info.state, "Force-cleaning process");
                    }
                    ProcessState::Orphan => {
                        tracing::info!(pid, "Cleaning orphan process");
                    }
                }
            } else {
                // Process not tracked, check if it is still alive
                if !self.platform.is_process_alive(pid) {
                    tracing::info!(pid, "Process already dead, skipping");
                    failed += 1;
                    continue;
                }
                if !force {
                    tracing::warn!(pid, "Unknown process, refusing without --force");
                    failed += 1;
                    continue;
                }
                tracing::warn!(pid, "Force-cleaning unknown process");
            }

            if self.terminate_gracefully(pid).await {
                cleaned += 1;
            } else {
                failed += 1;
            }
        }

        (cleaned, failed)
    }

    /// Attempt graceful termination, falling back to force kill.
    ///
    /// Flow: terminate (SIGTERM) -> wait orphan_grace_period -> kill (SIGKILL) + tree kill.
    /// On Windows: additionally calls release_memory (EmptyWorkingSet).
    async fn terminate_gracefully(&self, pid: u32) -> bool {
        tracing::debug!(pid, "Attempting graceful termination");

        // Step 1: Send SIGTERM / TerminateProcess
        if let Err(e) = self.platform.terminate_process(pid) {
            tracing::warn!(pid, error = %e, "Terminate signal failed");
            // Process might already be dead, which is fine
            if !self.platform.is_process_alive(pid) {
                return true;
            }
        }

        // Step 2: Wait grace period for process to exit
        let grace_ms = self.config.orphan_grace_period_secs * 1000;
        let check_interval = std::time::Duration::from_millis(500);
        let mut elapsed = 0u64;

        while elapsed < grace_ms {
            tokio::time::sleep(check_interval).await;
            elapsed += 500;

            if !self.platform.is_process_alive(pid) {
                tracing::debug!(pid, elapsed_ms = elapsed, "Process exited gracefully");
                // Release memory on Windows
                let _ = self.platform.release_memory(pid);
                return true;
            }
        }

        // Step 3: Force kill the process tree
        tracing::info!(pid, "Grace period expired, force killing process tree");
        if let Err(e) = self.platform.kill_process_tree(pid) {
            tracing::warn!(pid, error = %e, "Process tree kill failed");
        }

        // Step 4: Final kill attempt on the root process
        if self.platform.is_process_alive(pid) {
            if let Err(e) = self.platform.kill_process(pid) {
                tracing::error!(pid, error = %e, "Force kill failed");
                return false;
            }
        }

        // Release memory after cleanup (Windows: EmptyWorkingSet)
        let _ = self.platform.release_memory(pid);

        !self.platform.is_process_alive(pid)
    }
}

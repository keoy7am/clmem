use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::models::{Config, Event, EventKind, ProcessInfo, ProcessState};
use crate::platform::Platform;

/// A process being tracked by the scanner with metadata for state classification.
struct TrackedProcess {
    info: ProcessInfo,
    /// When this process was first discovered (retained for diagnostics).
    #[allow(dead_code)]
    first_seen: DateTime<Utc>,
    /// When the process last changed state (retained for diagnostics).
    #[allow(dead_code)]
    last_state_change: DateTime<Utc>,
    stale_since: Option<DateTime<Utc>>,
}

/// Polls for Claude Code processes and classifies their state according to safety rules.
///
/// Safety classification:
/// - ACTIVE: has TTY/stdin -> NEVER touch
/// - IDLE: no activity < idle_threshold -> monitor only, soft alert
/// - STALE: no activity, parent alive -> wait stale_grace_period before downgrade
/// - ORPHAN: parent dead, no IPC -> safe to auto-clean
pub struct Scanner {
    platform: Arc<dyn Platform>,
    config: Config,
    known_processes: HashMap<u32, TrackedProcess>,
}

impl Scanner {
    pub fn new(platform: Arc<dyn Platform>, config: Config) -> Self {
        Self {
            platform,
            config,
            known_processes: HashMap::new(),
        }
    }

    /// Perform one scan cycle: discover processes, classify states, emit events.
    pub fn scan(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        let now = Utc::now();

        let processes = match self.platform.list_claude_processes() {
            Ok(procs) => procs,
            Err(e) => {
                tracing::error!(error = %e, "Failed to list Claude processes");
                return events;
            }
        };

        // Track which PIDs are still alive this cycle
        let mut seen_pids: Vec<u32> = Vec::with_capacity(processes.len());

        for mut proc_info in processes {
            let pid = proc_info.pid;
            seen_pids.push(pid);

            // Classify the process state
            let new_state = self.classify(&proc_info, now);
            proc_info.state = new_state;

            if let Some(tracked) = self.known_processes.get_mut(&pid) {
                let old_state = tracked.info.state;

                // Update stale_since tracking
                if new_state == ProcessState::Stale && tracked.stale_since.is_none() {
                    tracked.stale_since = Some(now);
                } else if new_state != ProcessState::Stale {
                    tracked.stale_since = None;
                }

                // Emit state change event if state differs
                if old_state != new_state {
                    tracing::info!(pid, %old_state, %new_state, "Process state changed");
                    events.push(Event::new(EventKind::StateChange {
                        pid,
                        from: old_state,
                        to: new_state,
                    }));
                    tracked.last_state_change = now;
                }

                tracked.info = proc_info;
            } else {
                // New process discovered
                tracing::info!(pid, name = %proc_info.name, "New Claude process discovered");
                events.push(Event::new(EventKind::ProcessDiscovered {
                    pid,
                    name: proc_info.name.clone(),
                }));

                let stale_since = if new_state == ProcessState::Stale {
                    Some(now)
                } else {
                    None
                };

                self.known_processes.insert(
                    pid,
                    TrackedProcess {
                        info: proc_info,
                        first_seen: now,
                        last_state_change: now,
                        stale_since,
                    },
                );
            }
        }

        // Remove processes that are no longer present
        self.known_processes
            .retain(|pid, _| seen_pids.contains(pid));

        events
    }

    /// Return the current list of tracked processes with their classified states.
    pub fn get_processes(&self) -> Vec<ProcessInfo> {
        self.known_processes
            .values()
            .map(|t| t.info.clone())
            .collect()
    }

    /// Classify a process according to the safety rules.
    fn classify(&self, proc_info: &ProcessInfo, now: DateTime<Utc>) -> ProcessState {
        // Rule 1: Has active TTY/stdin -> ACTIVE (never touch)
        if proc_info.has_tty {
            return ProcessState::Active;
        }

        // Check TTY via platform for extra safety
        match self.platform.has_active_tty(proc_info.pid) {
            Ok(true) => return ProcessState::Active,
            Ok(false) => {}
            Err(e) => {
                tracing::debug!(pid = proc_info.pid, error = %e, "TTY check failed, continuing");
            }
        }

        // Rule 4: Parent dead and no IPC -> ORPHAN (safe to auto-clean)
        let parent_alive = proc_info
            .parent_pid
            .map(|ppid| self.platform.is_process_alive(ppid))
            .unwrap_or(false);

        if !parent_alive && !proc_info.has_ipc {
            // Check IPC via platform for extra safety
            match self.platform.has_active_ipc(proc_info.pid) {
                Ok(true) => {} // Has IPC, not orphan - fall through
                Ok(false) => return ProcessState::Orphan,
                Err(e) => {
                    tracing::debug!(
                        pid = proc_info.pid,
                        error = %e,
                        "IPC check failed, treating as orphan"
                    );
                    return ProcessState::Orphan;
                }
            }
        }

        // Rule 3: No activity, parent alive -> STALE (wait grace period before downgrade)
        let idle_duration = now
            .signed_duration_since(proc_info.last_activity)
            .num_seconds() as u64;

        if idle_duration > self.config.idle_threshold_secs {
            // Check if it has been stale long enough to be considered for downgrade
            if let Some(tracked) = self.known_processes.get(&proc_info.pid) {
                if let Some(stale_since) = tracked.stale_since {
                    let stale_duration =
                        now.signed_duration_since(stale_since).num_seconds() as u64;
                    if stale_duration >= self.config.stale_grace_period_secs && !parent_alive {
                        return ProcessState::Orphan;
                    }
                }
            }
            return ProcessState::Stale;
        }

        // Rule 2: No activity < idle_threshold -> IDLE (monitor only)
        // If we get here, the process has some recent activity but no TTY
        ProcessState::Idle
    }
}

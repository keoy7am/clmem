use super::Platform;
use crate::models::{MemorySnapshot, MemoryUsage, ProcessInfo, ProcessState};
use anyhow::Result;
use chrono::Utc;
use sysinfo::{Pid, MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

pub struct WindowsPlatform {
    system: std::sync::Mutex<System>,
}

impl WindowsPlatform {
    pub fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::new()
                .with_processes(ProcessRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        Self {
            system: std::sync::Mutex::new(system),
        }
    }

    /// Heuristic: is this process related to Claude Code?
    fn is_claude_process(name: &str, cmdline: &str) -> bool {
        let name_lower = name.to_lowercase();
        let cmd_lower = cmdline.to_lowercase();
        name_lower.contains("claude")
            || (name_lower.contains("node") && cmd_lower.contains("claude"))
            || cmd_lower.contains("claude-code")
            || cmd_lower.contains("@anthropic")
    }

    /// Join OsString slices into a single String for matching.
    fn cmd_to_string(cmd: &[std::ffi::OsString]) -> String {
        cmd.iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl Platform for WindowsPlatform {
    fn list_claude_processes(&self) -> Result<Vec<ProcessInfo>> {
        let mut sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        sys.refresh_processes(ProcessesToUpdate::All, true);

        let mut result = Vec::new();
        for (pid, proc) in sys.processes() {
            let name = proc.name().to_string_lossy().to_string();
            let cmdline = Self::cmd_to_string(proc.cmd());
            if !Self::is_claude_process(&name, &cmdline) {
                continue;
            }

            let memory = MemoryUsage {
                rss_bytes: proc.memory(),
                vms_bytes: proc.virtual_memory(),
                swap_bytes: 0,
                committed_bytes: proc.memory(), // Approximation; refined later with Win32 API
            };

            // Initial classification -- daemon scanner will refine with TTY/IPC checks
            let has_tty = false;
            let has_ipc = false;
            let state = if proc.parent().is_none() && !has_ipc {
                ProcessState::Orphan
            } else if has_tty {
                ProcessState::Active
            } else {
                ProcessState::Idle
            };

            result.push(ProcessInfo {
                pid: pid.as_u32(),
                parent_pid: proc.parent().map(|p| p.as_u32()),
                name,
                cmdline,
                state,
                memory,
                started_at: Utc::now(), // sysinfo doesn't expose start time reliably
                last_activity: Utc::now(),
                has_tty,
                has_ipc,
            });
        }
        Ok(result)
    }

    fn take_snapshot(&self) -> Result<MemorySnapshot> {
        let processes = self.list_claude_processes()?;
        let mut sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        sys.refresh_memory();

        let total_rss: u64 = processes.iter().map(|p| p.memory.rss_bytes).sum();
        let total_vms: u64 = processes.iter().map(|p| p.memory.vms_bytes).sum();
        let total_swap: u64 = processes.iter().map(|p| p.memory.swap_bytes).sum();
        let total_committed: u64 = processes.iter().map(|p| p.memory.committed_bytes).sum();
        let orphan_count = processes
            .iter()
            .filter(|p| p.state == ProcessState::Orphan)
            .count() as u32;
        let claude_count = processes.len() as u32;

        Ok(MemorySnapshot {
            timestamp: Utc::now(),
            processes,
            system_total_memory: sys.total_memory(),
            system_used_memory: sys.used_memory(),
            system_available_memory: sys.available_memory(),
            total_rss,
            total_vms,
            total_swap,
            total_committed,
            claude_process_count: claude_count,
            orphan_count,
        })
    }

    fn is_process_alive(&self, pid: u32) -> bool {
        let sys = self.system.lock().ok();
        sys.map(|s| s.process(Pid::from_u32(pid)).is_some())
            .unwrap_or(false)
    }

    fn has_active_tty(&self, _pid: u32) -> Result<bool> {
        // Windows: check if process has a console window attached.
        // Full implementation will use Win32 GetConsoleWindow / AttachConsole.
        // Stub returns false -- daemon scanner will refine this.
        Ok(false)
    }

    fn has_active_ipc(&self, _pid: u32) -> Result<bool> {
        // Check if process has open Named Pipe handles related to clmem.
        // Full implementation will use NtQuerySystemInformation.
        // Stub returns false -- daemon scanner will refine this.
        Ok(false)
    }

    fn terminate_process(&self, pid: u32) -> Result<()> {
        let sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            proc.kill();
        }
        Ok(())
    }

    fn kill_process(&self, pid: u32) -> Result<()> {
        // On Windows, terminate and kill are effectively the same (TerminateProcess).
        self.terminate_process(pid)
    }

    fn kill_process_tree(&self, pid: u32) -> Result<()> {
        let sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        // BFS to find all descendants
        let mut to_kill = vec![pid];
        let mut i = 0;
        while i < to_kill.len() {
            let parent = Pid::from_u32(to_kill[i]);
            for (child_pid, proc) in sys.processes() {
                if proc.parent() == Some(parent) {
                    to_kill.push(child_pid.as_u32());
                }
            }
            i += 1;
        }

        // Kill in reverse order (children first, then parent)
        for &p in to_kill.iter().rev() {
            if let Some(proc) = sys.process(Pid::from_u32(p)) {
                proc.kill();
            }
        }
        Ok(())
    }

    fn system_total_memory(&self) -> u64 {
        let mut sys = self.system.lock().ok();
        sys.as_deref_mut().map(|s| { s.refresh_memory(); s.total_memory() }).unwrap_or(0)
    }

    fn system_available_memory(&self) -> u64 {
        let mut sys = self.system.lock().ok();
        sys.as_deref_mut().map(|s| { s.refresh_memory(); s.available_memory() }).unwrap_or(0)
    }

    fn name(&self) -> &'static str {
        "windows"
    }

    fn release_memory(&self, pid: u32) -> Result<()> {
        // Windows: EmptyWorkingSet via the windows crate.
        // Full implementation deferred -- will use OpenProcess + EmptyWorkingSet.
        tracing::debug!(pid, "EmptyWorkingSet requested (stub)");
        Ok(())
    }
}

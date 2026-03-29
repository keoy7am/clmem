use super::{cmd_to_string, collect_process_tree, is_claude_process, Platform};
use crate::models::{MemorySnapshot, MemoryUsage, ProcessInfo, ProcessState};
use anyhow::Result;
use chrono::Utc;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, Signal, System};

pub struct MacosPlatform {
    system: std::sync::Mutex<System>,
}

impl MacosPlatform {
    pub fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
        );
        Self {
            system: std::sync::Mutex::new(system),
        }
    }
}

impl Platform for MacosPlatform {
    fn list_claude_processes(&self) -> Result<Vec<ProcessInfo>> {
        let mut sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        sys.refresh_processes(ProcessesToUpdate::All, true);

        let mut result = Vec::new();
        for (pid, proc) in sys.processes() {
            let name = proc.name().to_string_lossy().to_string();
            let raw_cmdline = cmd_to_string(proc.cmd());
            if !is_claude_process(&name, &raw_cmdline) {
                continue;
            }
            let cmdline = super::redact_sensitive_args(&raw_cmdline);

            let memory = MemoryUsage {
                rss_bytes: proc.memory(),
                vms_bytes: proc.virtual_memory(),
                swap_bytes: 0,
                committed_bytes: 0,
            };

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
                started_at: Utc::now(),
                last_activity: Utc::now(),
                has_tty,
                has_ipc,
            });
        }
        Ok(result)
    }

    fn take_snapshot(&self) -> Result<MemorySnapshot> {
        let processes = self.list_claude_processes()?;
        let sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        let total_rss: u64 = processes.iter().map(|p| p.memory.rss_bytes).sum();
        let total_vms: u64 = processes.iter().map(|p| p.memory.vms_bytes).sum();
        let total_swap: u64 = processes.iter().map(|p| p.memory.swap_bytes).sum();
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
            total_committed: 0,
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
        // macOS: check via /dev/ttys* -- simplified stub for now.
        Ok(false)
    }

    fn has_active_ipc(&self, _pid: u32) -> Result<bool> {
        // macOS: check for unix socket connections related to clmem.
        Ok(false)
    }

    fn terminate_process(&self, pid: u32) -> Result<()> {
        let sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            proc.kill_with(Signal::Term);
        }
        Ok(())
    }

    fn kill_process(&self, pid: u32) -> Result<()> {
        let sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            proc.kill();
        }
        Ok(())
    }

    fn kill_process_tree(&self, pid: u32) -> Result<()> {
        let sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        let to_kill = collect_process_tree(&sys, pid);
        for p in &to_kill {
            if let Some(proc) = sys.process(*p) {
                proc.kill();
            }
        }
        Ok(())
    }

    fn system_total_memory(&self) -> u64 {
        let sys = self.system.lock().ok();
        sys.map(|s| s.total_memory()).unwrap_or(0)
    }

    fn system_available_memory(&self) -> u64 {
        let sys = self.system.lock().ok();
        sys.map(|s| s.available_memory()).unwrap_or(0)
    }

    fn name(&self) -> &'static str {
        "macos"
    }

    fn release_memory(&self, _pid: u32) -> Result<()> {
        // macOS: no direct equivalent to EmptyWorkingSet.
        Ok(())
    }
}

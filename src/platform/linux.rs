use super::{cmd_to_string, collect_process_tree, is_claude_process, Platform};
use crate::models::{MemorySnapshot, MemoryUsage, ProcessInfo, ProcessState};
use anyhow::Result;
use chrono::Utc;
use sysinfo::{Pid, MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, Signal, System};

pub struct LinuxPlatform {
    system: std::sync::Mutex<System>,
}

impl LinuxPlatform {
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
}

impl Platform for LinuxPlatform {
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

            // Use sysinfo start_time (seconds since UNIX epoch)
            let started_at = {
                let epoch_secs = proc.start_time() as i64;
                chrono::DateTime::from_timestamp(epoch_secs, 0).unwrap_or_else(Utc::now)
            };

            // Estimate last_activity from CPU usage: if cpu > 0, active now
            // Scanner will refine this by tracking CPU time changes
            let last_activity = if proc.cpu_usage() > 0.0 {
                Utc::now()
            } else {
                started_at
            };

            let has_tty = self.has_active_tty(pid.as_u32()).unwrap_or(false);
            let has_ipc = self.has_active_ipc(pid.as_u32()).unwrap_or(false);

            // ACTIVE checked FIRST (safety rule: ACTIVE → NEVER touch)
            let state = if has_tty {
                ProcessState::Active
            } else if proc.parent().is_none() && !has_ipc {
                ProcessState::Orphan
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
                started_at,
                last_activity,
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
        if let Ok(mut sys) = self.system.lock() {
            sys.refresh_processes(
                ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
                false,
            );
            sys.process(Pid::from_u32(pid)).is_some()
        } else {
            false
        }
    }

    fn has_active_tty(&self, pid: u32) -> Result<bool> {
        // Check /proc/{pid}/fd/0 symlink target
        let path = format!("/proc/{}/fd/0", pid);
        match std::fs::read_link(&path) {
            Ok(target) => {
                let target_str = target.to_string_lossy();
                Ok(target_str.starts_with("/dev/pts/") || target_str.starts_with("/dev/tty"))
            }
            Err(_) => Ok(false),
        }
    }

    fn has_active_ipc(&self, pid: u32) -> Result<bool> {
        // Check /proc/{pid}/fd for unix socket connections related to clmem
        let fd_dir = format!("/proc/{}/fd", pid);
        if let Ok(entries) = std::fs::read_dir(&fd_dir) {
            for entry in entries.flatten() {
                if let Ok(target) = std::fs::read_link(entry.path()) {
                    let s = target.to_string_lossy();
                    if s.contains("clmem") {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn terminate_process(&self, pid: u32) -> Result<()> {
        let mut sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        sys.refresh_processes(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            false,
        );
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            match proc.kill_with(Signal::Term) {
                Some(true) => Ok(()),
                Some(false) => {
                    tracing::warn!(pid, "SIGTERM delivery failed");
                    Err(anyhow::anyhow!("Failed to send SIGTERM to PID {pid}"))
                }
                None => {
                    tracing::warn!(pid, "SIGTERM not supported, falling back to SIGKILL");
                    proc.kill();
                    Ok(())
                }
            }
        } else {
            Err(anyhow::anyhow!("Process {pid} not found"))
        }
    }

    fn kill_process(&self, pid: u32) -> Result<()> {
        let mut sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        sys.refresh_processes(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            false,
        );
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            proc.kill();
            Ok(())
        } else {
            Err(anyhow::anyhow!("Process {pid} not found"))
        }
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
        let mut sys = self.system.lock().ok();
        sys.as_deref_mut().map(|s| { s.refresh_memory(); s.total_memory() }).unwrap_or(0)
    }

    fn system_available_memory(&self) -> u64 {
        let mut sys = self.system.lock().ok();
        sys.as_deref_mut().map(|s| { s.refresh_memory(); s.available_memory() }).unwrap_or(0)
    }

    fn name(&self) -> &'static str {
        "linux"
    }

    fn release_memory(&self, _pid: u32) -> Result<()> {
        // Linux: no equivalent to EmptyWorkingSet; kernel manages memory reclaim.
        Ok(())
    }
}

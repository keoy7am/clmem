use super::{build_process_info, cmd_to_string, collect_process_tree, is_claude_process, Platform};
use crate::models::{MemorySnapshot, MemoryUsage, ProcessInfo, ProcessState};
use anyhow::Result;
use chrono::Utc;
use sysinfo::{Pid, MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, Signal, System};

pub struct MacosPlatform {
    system: std::sync::Mutex<System>,
}

impl MacosPlatform {
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

/// Check if a process has an active TTY by inspecting parent process name.
/// Takes &System directly to avoid re-locking (fixes potential deadlock).
fn check_active_tty(sys: &System, pid: u32) -> bool {
    let pid_sysinfo = Pid::from_u32(pid);
    if let Some(proc) = sys.process(pid_sysinfo) {
        if let Some(parent_pid) = proc.parent() {
            if let Some(parent) = sys.process(parent_pid) {
                let parent_name = parent.name().to_string_lossy().to_ascii_lowercase();
                return parent_name.contains("terminal")
                    || parent_name.contains("iterm")
                    || parent_name.contains("alacritty")
                    || parent_name.contains("kitty")
                    || parent_name.contains("warp")
                    || parent_name.contains("hyper")
                    || parent_name.contains("bash")
                    || parent_name.contains("zsh")
                    || parent_name.contains("fish")
                    || parent_name.contains("tmux")
                    || parent_name.contains("screen");
            }
        }
    }
    false
}

/// Enumerate Claude processes from an already-locked System reference.
/// Shared by list_claude_processes and take_snapshot to avoid double-locking.
fn enumerate_claude_processes(sys: &System) -> Vec<ProcessInfo> {
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

        let has_tty = check_active_tty(sys, pid.as_u32());
        let has_ipc = false; // macOS lacks /proc; TODO: implement via proc_pidinfo

        result.push(build_process_info(
            pid.as_u32(),
            proc.parent().map(|p| p.as_u32()),
            name,
            cmdline,
            memory,
            proc.start_time(),
            proc.cpu_usage(),
            has_tty,
            has_ipc,
            proc.parent().is_some(),
        ));
    }
    result
}

impl Platform for MacosPlatform {
    fn list_claude_processes(&self) -> Result<Vec<ProcessInfo>> {
        let mut sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        sys.refresh_processes(ProcessesToUpdate::All, true);
        Ok(enumerate_claude_processes(&sys))
    }

    fn take_snapshot(&self) -> Result<MemorySnapshot> {
        let mut sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        sys.refresh_processes(ProcessesToUpdate::All, true);
        sys.refresh_memory();

        let processes = enumerate_claude_processes(&sys);
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
        let sys = self.system.lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire system lock"))?;
        Ok(check_active_tty(&sys, pid))
    }

    fn has_active_ipc(&self, _pid: u32) -> Result<bool> {
        // macOS lacks /proc/{pid}/fd; implementing requires libproc FFI.
        // Returns false conservatively — the scanner's secondary checks provide safety.
        // TODO: Implement via proc_pidinfo(PROC_PIDLISTFDS) for accurate detection.
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
        "macos"
    }

    fn release_memory(&self, _pid: u32) -> Result<()> {
        // macOS: no direct equivalent to EmptyWorkingSet.
        Ok(())
    }

    fn runtime_dir(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string()),
        )
    }
}

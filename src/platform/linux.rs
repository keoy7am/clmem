use super::{build_process_info, cmd_to_string, collect_process_tree, is_claude_process, Platform};
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

/// Check if a process has an active TTY via /proc filesystem (no lock needed).
fn check_active_tty(pid: u32) -> bool {
    let path = format!("/proc/{}/fd/0", pid);
    match std::fs::read_link(&path) {
        Ok(target) => {
            let target_str = target.to_string_lossy();
            target_str.starts_with("/dev/pts/") || target_str.starts_with("/dev/tty")
        }
        Err(_) => false,
    }
}

/// Check if a process has an active IPC connection via /proc filesystem (no lock needed).
fn check_active_ipc(pid: u32) -> bool {
    let fd_dir = format!("/proc/{}/fd", pid);
    if let Ok(entries) = std::fs::read_dir(&fd_dir) {
        for entry in entries.flatten() {
            if let Ok(target) = std::fs::read_link(entry.path()) {
                let s = target.to_string_lossy();
                if s.contains("clmem") {
                    return true;
                }
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

        let has_tty = check_active_tty(pid.as_u32());
        let has_ipc = check_active_ipc(pid.as_u32());

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

/// Build ProcessInfo for a single sysinfo::Process entry (Linux-specific TTY/IPC checks).
fn build_info_for_process(pid: &Pid, proc: &sysinfo::Process) -> ProcessInfo {
    let name = proc.name().to_string_lossy().to_string();
    let raw_cmdline = cmd_to_string(proc.cmd());
    let cmdline = super::redact_sensitive_args(&raw_cmdline);

    let memory = MemoryUsage {
        rss_bytes: proc.memory(),
        vms_bytes: proc.virtual_memory(),
        swap_bytes: 0,
        committed_bytes: 0,
    };

    let has_tty = check_active_tty(pid.as_u32());
    let has_ipc = check_active_ipc(pid.as_u32());

    build_process_info(
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
    )
}

impl Platform for LinuxPlatform {
    fn list_claude_processes(&self) -> Result<Vec<ProcessInfo>> {
        let mut sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        sys.refresh_processes(ProcessesToUpdate::All, true);
        Ok(enumerate_claude_processes(&sys))
    }

    fn refresh_known_processes(&self, pids: &[u32]) -> Result<Vec<ProcessInfo>> {
        let mut sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;
        let sysinfo_pids: Vec<Pid> = pids.iter().map(|&p| Pid::from_u32(p)).collect();
        sys.refresh_processes(ProcessesToUpdate::Some(&sysinfo_pids), true);

        let mut result = Vec::with_capacity(pids.len());
        for &pid in pids {
            let sysinfo_pid = Pid::from_u32(pid);
            if let Some(proc) = sys.process(sysinfo_pid) {
                result.push(build_info_for_process(&sysinfo_pid, proc));
            }
        }
        Ok(result)
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
        Ok(check_active_tty(pid))
    }

    fn has_active_ipc(&self, pid: u32) -> Result<bool> {
        Ok(check_active_ipc(pid))
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

    fn runtime_dir(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string()),
        )
    }
}

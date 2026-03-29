use super::{build_process_info, cmd_to_string, collect_process_tree, is_claude_process, Platform};
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
}

/// Check if parent is a known terminal process (cmd, powershell, etc.)
fn check_parent_terminal(sys: &System, proc: &sysinfo::Process) -> bool {
    proc.parent().map(|ppid| {
        sys.process(ppid)
            .map(|parent| {
                let pname = parent.name().to_string_lossy().to_lowercase();
                pname.contains("cmd.exe")
                    || pname.contains("powershell")
                    || pname.contains("pwsh")
                    || pname.contains("windowsterminal")
                    || pname.contains("conhost")
                    || pname.contains("wt.exe")
            })
            .unwrap_or(false)
    }).unwrap_or(false)
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
            committed_bytes: proc.memory(), // Approximation; refined later with Win32 API
        };

        let has_tty = check_parent_terminal(sys, proc);
        let has_ipc = false;

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

/// Build ProcessInfo for a single sysinfo::Process entry (Windows-specific TTY check).
fn build_info_for_process(sys: &System, pid: &Pid, proc: &sysinfo::Process) -> ProcessInfo {
    let name = proc.name().to_string_lossy().to_string();
    let raw_cmdline = cmd_to_string(proc.cmd());
    let cmdline = super::redact_sensitive_args(&raw_cmdline);

    let memory = MemoryUsage {
        rss_bytes: proc.memory(),
        vms_bytes: proc.virtual_memory(),
        swap_bytes: 0,
        committed_bytes: proc.memory(),
    };

    let has_tty = check_parent_terminal(sys, proc);
    let has_ipc = false;

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

impl Platform for WindowsPlatform {
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
                result.push(build_info_for_process(&sys, &sysinfo_pid, proc));
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
        use windows::Win32::System::Console::{AttachConsole, FreeConsole};

        // AttachConsole modifies global per-process state, so we serialize it
        // via the system mutex. The daemon is typically a background process
        // without its own console, so temporarily changing console attachment
        // is safe.
        let sys = self
            .system
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {}", e))?;

        // Safety: AttachConsole/FreeConsole are Win32 APIs with no memory
        // unsafety. The global console state change is serialized by the mutex.
        unsafe {
            // Detach from our current console (no-op if none attached)
            let _ = FreeConsole();

            // Try to attach to the target process's console
            if AttachConsole(pid).is_ok() {
                let _ = FreeConsole();
                return Ok(true);
            }
        }

        // AttachConsole failed: target has no console, or access denied.
        // Fall back to parent-terminal heuristic.
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            Ok(check_parent_terminal(&sys, proc))
        } else {
            Ok(false)
        }
    }

    fn has_active_ipc(&self, _pid: u32) -> Result<bool> {
        // Per-PID named pipe handle detection requires NtQuerySystemInformation
        // (undocumented). Instead, check if the daemon pipe exists by briefly
        // connecting as a client. If the daemon is running, conservatively
        // return true to prevent premature orphan classification — the process
        // might have an active IPC connection we cannot verify from outside.
        let pipe_exists = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(r"\\.\pipe\clmem")
            .is_ok();
        Ok(pipe_exists)
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
        "windows"
    }

    fn release_memory(&self, pid: u32) -> Result<()> {
        // Windows: EmptyWorkingSet via the windows crate.
        // Full implementation deferred -- will use OpenProcess + EmptyWorkingSet.
        tracing::debug!(pid, "EmptyWorkingSet requested (stub)");
        Ok(())
    }

    fn runtime_dir(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(
            std::env::var("TEMP").unwrap_or_else(|_| r"C:\Temp".to_string()),
        )
    }
}

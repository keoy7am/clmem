use crate::models::{MemorySnapshot, ProcessInfo};
use anyhow::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Cross-platform abstraction for OS-level process and memory operations.
///
/// All platform-specific code lives behind this trait. Callers in daemon/cli/tui
/// must never call OS APIs directly -- always go through `Platform`.
pub trait Platform: Send + Sync {
    /// List all Claude Code related processes currently running.
    fn list_claude_processes(&self) -> Result<Vec<ProcessInfo>>;

    /// Take a point-in-time memory snapshot of all Claude processes + system memory.
    fn take_snapshot(&self) -> Result<MemorySnapshot>;

    /// Check if a process with the given PID is still alive.
    fn is_process_alive(&self, pid: u32) -> bool;

    /// Check if a process has an active TTY/stdin (indicates interactive session).
    fn has_active_tty(&self, pid: u32) -> Result<bool>;

    /// Check if a process has an active IPC connection to the daemon.
    fn has_active_ipc(&self, pid: u32) -> Result<bool>;

    /// Gracefully terminate a process (SIGTERM on Unix, TerminateProcess on Windows).
    fn terminate_process(&self, pid: u32) -> Result<()>;

    /// Force kill a process (SIGKILL on Unix, TerminateProcess on Windows).
    fn kill_process(&self, pid: u32) -> Result<()>;

    /// Kill an entire process tree rooted at the given PID (children first).
    fn kill_process_tree(&self, pid: u32) -> Result<()>;

    /// Get total physical memory on the system in bytes.
    fn system_total_memory(&self) -> u64;

    /// Get available physical memory on the system in bytes.
    fn system_available_memory(&self) -> u64;

    /// Platform name string ("windows", "linux", or "macos").
    fn name(&self) -> &'static str;

    /// Return the platform-specific runtime directory for ephemeral files
    /// (PID files, sockets). Unix: $XDG_RUNTIME_DIR or /tmp. Windows: %TEMP%.
    fn runtime_dir(&self) -> std::path::PathBuf;

    /// Release memory back to the OS after cleanup.
    /// On Windows this calls EmptyWorkingSet; on other platforms it is a no-op.
    fn release_memory(&self, pid: u32) -> Result<()>;
}

/// Heuristic: is this process related to Claude Code?
pub(crate) fn is_claude_process(name: &str, cmdline: &str) -> bool {
    let name_lower = name.to_ascii_lowercase();
    let cmd_lower = cmdline.to_ascii_lowercase();
    name_lower.contains("claude")
        || (name_lower.contains("node") && cmd_lower.contains("claude"))
        || cmd_lower.contains("claude-code")
        || cmd_lower.contains("@anthropic")
}

/// Join OsString slices into a single String for matching.
pub(crate) fn cmd_to_string(cmd: &[std::ffi::OsString]) -> String {
    let mut result = String::new();
    for (i, s) in cmd.iter().enumerate() {
        if i > 0 {
            result.push(' ');
        }
        result.push_str(&s.to_string_lossy());
    }
    result
}

/// Collect all descendant PIDs of a root process using BFS traversal.
/// Returns PIDs in reverse order (children before parents) for safe termination.
pub(crate) fn collect_process_tree(sys: &sysinfo::System, root_pid: u32) -> Vec<sysinfo::Pid> {
    use std::collections::HashSet;
    use sysinfo::Pid;
    let root = Pid::from_u32(root_pid);
    let mut to_kill = vec![root];
    let mut visited: HashSet<Pid> = HashSet::new();
    visited.insert(root);
    let mut queue = vec![root];
    while let Some(parent) = queue.pop() {
        for (child_pid, proc_info) in sys.processes() {
            if proc_info.parent() == Some(parent) && !visited.contains(child_pid) {
                visited.insert(*child_pid);
                to_kill.push(*child_pid);
                queue.push(*child_pid);
            }
        }
    }
    to_kill.reverse();
    to_kill
}

/// Redact sensitive arguments from command line strings.
pub(crate) fn redact_sensitive_args(cmdline: &str) -> String {
    let sensitive_flags = ["--api-key", "--token", "--password", "--secret", "--auth"];
    let mut result = String::new();
    let mut redact_next = false;
    for part in cmdline.split_whitespace() {
        if !result.is_empty() {
            result.push(' ');
        }
        if redact_next {
            result.push_str("[REDACTED]");
            redact_next = false;
        } else if sensitive_flags.iter().any(|f| part.starts_with(f)) {
            if part.contains('=') {
                if let Some(flag) = part.split('=').next() {
                    result.push_str(flag);
                    result.push_str("=[REDACTED]");
                }
            } else {
                result.push_str(part);
                redact_next = true;
            }
        } else {
            result.push_str(part);
        }
    }
    result
}

/// Build a `ProcessInfo` from OS-specific inputs.
///
/// Centralises the common logic shared by all platform `enumerate_claude_processes`
/// implementations: timestamp conversion, activity estimation, state classification,
/// and struct assembly.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_process_info(
    pid: u32,
    parent_pid: Option<u32>,
    name: String,
    cmdline: String,
    memory: crate::models::MemoryUsage,
    start_time_epoch: u64,
    cpu_usage: f32,
    has_tty: bool,
    has_ipc: bool,
    parent_exists: bool,
) -> ProcessInfo {
    let started_at = chrono::DateTime::from_timestamp(start_time_epoch as i64, 0)
        .unwrap_or_else(chrono::Utc::now);
    let last_activity = if cpu_usage > 0.0 {
        chrono::Utc::now()
    } else {
        started_at
    };

    let state = if has_tty {
        crate::models::ProcessState::Active
    } else if !parent_exists && !has_ipc {
        crate::models::ProcessState::Orphan
    } else {
        crate::models::ProcessState::Idle
    };

    ProcessInfo {
        pid,
        parent_pid,
        name,
        cmdline,
        state,
        memory,
        started_at,
        last_activity,
        has_tty,
        has_ipc,
    }
}

/// Create the platform implementation for the current OS.
pub fn create_platform() -> Box<dyn Platform> {
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsPlatform::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxPlatform::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacosPlatform::new())
    }
}

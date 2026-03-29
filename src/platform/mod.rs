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
#[allow(dead_code)]
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

    /// Release memory back to the OS after cleanup.
    /// On Windows this calls EmptyWorkingSet; on other platforms it is a no-op.
    fn release_memory(&self, pid: u32) -> Result<()>;
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

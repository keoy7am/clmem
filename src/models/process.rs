use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Classification of a Claude Code process based on its activity and connectivity.
///
/// Safety rules:
/// - ACTIVE: Has TTY/stdin -> NEVER touch
/// - IDLE: Activity < threshold -> Monitor only, soft alert
/// - STALE: No activity, parent alive -> Wait grace period before downgrade
/// - ORPHAN: Parent dead, no IPC -> Safe to auto-clean
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessState {
    Active,
    Idle,
    Stale,
    Orphan,
}

impl std::fmt::Display for ProcessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "ACTIVE"),
            Self::Idle => write!(f, "IDLE"),
            Self::Stale => write!(f, "STALE"),
            Self::Orphan => write!(f, "ORPHAN"),
        }
    }
}

/// Per-process memory usage metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryUsage {
    /// Resident Set Size in bytes
    pub rss_bytes: u64,
    /// Virtual Memory Size in bytes
    pub vms_bytes: u64,
    /// Swap usage in bytes
    pub swap_bytes: u64,
    /// Windows-specific committed memory in bytes (0 on other platforms)
    pub committed_bytes: u64,
}

/// Information about a single Claude Code related process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub cmdline: String,
    pub state: ProcessState,
    pub memory: MemoryUsage,
    pub started_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub has_tty: bool,
    pub has_ipc: bool,
}

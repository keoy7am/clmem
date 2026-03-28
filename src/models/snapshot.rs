use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::process::ProcessInfo;

/// A point-in-time snapshot of all Claude Code processes and system memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    pub timestamp: DateTime<Utc>,
    pub processes: Vec<ProcessInfo>,
    /// Total physical memory on the system in bytes
    pub system_total_memory: u64,
    /// Used physical memory on the system in bytes
    pub system_used_memory: u64,
    /// Available physical memory on the system in bytes
    pub system_available_memory: u64,
    /// Sum of RSS across all tracked Claude processes
    pub total_rss: u64,
    /// Sum of VMS across all tracked Claude processes
    pub total_vms: u64,
    /// Sum of swap across all tracked Claude processes
    pub total_swap: u64,
    /// Sum of committed memory (Windows-specific, 0 on other platforms)
    pub total_committed: u64,
    /// Number of Claude Code related processes found
    pub claude_process_count: u32,
    /// Number of orphan processes detected
    pub orphan_count: u32,
}

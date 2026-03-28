use serde::{Deserialize, Serialize};

use crate::models::{Config, Event, MemorySnapshot, ProcessInfo};

/// Messages that can be sent from CLI/TUI to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcMessage {
    /// Health check
    Ping,
    /// Request daemon status summary
    GetStatus,
    /// Request a fresh memory snapshot
    GetSnapshot,
    /// Request list of tracked processes
    GetProcessList,
    /// Request the last N snapshots from the ring buffer
    GetHistory { last_n: usize },
    /// Request the last N events
    GetEvents { last_n: usize },
    /// Request cleanup of specific PIDs
    Cleanup { pids: Vec<u32>, force: bool },
    /// Request current config
    GetConfig,
    /// Subscribe to real-time event stream (for TUI)
    Subscribe,
    /// Request daemon shutdown
    Shutdown,
}

/// Responses from the daemon back to CLI/TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    /// Response to Ping
    Pong,
    /// Daemon status summary
    Status {
        uptime_secs: u64,
        monitoring_count: u32,
        orphan_count: u32,
        total_rss: u64,
    },
    /// Full memory snapshot
    Snapshot(Box<MemorySnapshot>),
    /// List of tracked processes
    ProcessList(Vec<ProcessInfo>),
    /// Historical snapshots from ring buffer
    History(Vec<MemorySnapshot>),
    /// Recent events
    Events(Vec<Event>),
    /// Result of a cleanup operation
    CleanupResult { cleaned: u32, failed: u32 },
    /// Current daemon configuration
    Config(Config),
    /// Subscription confirmed
    Subscribed,
    /// Generic success
    Ok,
    /// Error with message
    Error(String),
}

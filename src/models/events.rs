use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::process::ProcessState;

/// Severity level for alerts emitted by the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
}

impl std::fmt::Display for AlertLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "INFO"),
            Self::Warning => write!(f, "WARN"),
            Self::Critical => write!(f, "CRIT"),
        }
    }
}

/// The kind of event that occurred in the monitoring system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventKind {
    ProcessDiscovered {
        pid: u32,
        name: String,
    },
    StateChange {
        pid: u32,
        from: ProcessState,
        to: ProcessState,
    },
    MemoryLeak {
        pid: u32,
        growth_rate_bytes_per_sec: f64,
    },
    CleanupStarted {
        pid: u32,
    },
    CleanupCompleted {
        pid: u32,
        success: bool,
    },
    Alert {
        level: AlertLevel,
        message: String,
    },
    DaemonStarted,
    DaemonStopped,
}

/// A timestamped event from the monitoring system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub timestamp: DateTime<Utc>,
    pub kind: EventKind,
}

impl Event {
    #[allow(dead_code)]
    pub fn new(kind: EventKind) -> Self {
        Self {
            timestamp: Utc::now(),
            kind,
        }
    }
}

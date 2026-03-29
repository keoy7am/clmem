use std::collections::VecDeque;
use std::sync::Arc;

use crate::models::{Config, MemorySnapshot};
use crate::platform::Platform;

/// Memory profiler that maintains a ring buffer of point-in-time snapshots.
///
/// Records at the configured scan interval and retains up to `max_entries`
/// snapshots (default: 1 hour at 1-second intervals = 3600 entries).
pub struct Profiler {
    platform: Arc<dyn Platform>,
    ring_buffer: VecDeque<Arc<MemorySnapshot>>,
    max_entries: usize,
}

impl Profiler {
    pub fn new(platform: Arc<dyn Platform>, config: &Config) -> Self {
        // Calculate max entries from retention period and scan interval
        let interval_secs = config.scan_interval_ms.max(1) as f64 / 1000.0;
        let max_entries = (config.history_retention_secs as f64 / interval_secs).ceil() as usize;
        let max_entries = max_entries.max(1); // At least 1 entry

        Self {
            platform,
            ring_buffer: VecDeque::with_capacity(max_entries),
            max_entries,
        }
    }

    /// Take a new memory snapshot and store it in the ring buffer.
    /// Returns an Arc-wrapped snapshot (cheap clone).
    pub fn record(&mut self) -> anyhow::Result<Arc<MemorySnapshot>> {
        let snapshot = Arc::new(self.platform.take_snapshot()?);

        if self.ring_buffer.len() >= self.max_entries {
            self.ring_buffer.pop_front();
        }
        self.ring_buffer.push_back(Arc::clone(&snapshot));

        tracing::debug!(
            process_count = snapshot.claude_process_count,
            total_rss = snapshot.total_rss,
            buffer_size = self.ring_buffer.len(),
            "Memory snapshot recorded"
        );

        Ok(snapshot)
    }

    /// Return the most recent snapshot, if any.
    pub fn get_latest(&self) -> Option<Arc<MemorySnapshot>> {
        self.ring_buffer.back().cloned()
    }

    /// Return the last `last_n` snapshots (or all if fewer exist).
    /// Clones are deep copies for serde serialization over IPC.
    pub fn get_history(&self, last_n: usize) -> Vec<MemorySnapshot> {
        let len = self.ring_buffer.len();
        let start = len.saturating_sub(last_n);
        self.ring_buffer
            .iter()
            .skip(start)
            .map(|arc| MemorySnapshot::clone(arc))
            .collect()
    }
}

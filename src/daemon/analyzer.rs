use std::collections::HashMap;

use crate::models::{Config, Event, EventKind, MemorySnapshot};

/// Analyzes memory trends from profiler history to detect leaks and anomalies.
///
/// Calculates VMS growth rate over a sliding window and emits `MemoryLeak`
/// events when sustained growth exceeds the configured threshold.
pub struct Analyzer {
    config: Config,
}

impl Analyzer {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Analyze a slice of historical snapshots for memory anomalies.
    /// Returns events for any detected issues.
    pub fn analyze(&self, history: &[MemorySnapshot]) -> Vec<Event> {
        let mut events = Vec::new();

        if history.len() < 2 {
            return events;
        }

        // Pre-index snapshots by PID: pid -> Vec<(time_offset_secs, vms_bytes)>
        let first_ts = match history.first() {
            Some(s) => s.timestamp,
            None => return events,
        };
        let mut pid_series: HashMap<u32, Vec<(f64, f64)>> = HashMap::new();
        for snapshot in history {
            let time_offset = snapshot
                .timestamp
                .signed_duration_since(first_ts)
                .num_milliseconds() as f64
                / 1000.0;
            for proc in &snapshot.processes {
                pid_series
                    .entry(proc.pid)
                    .or_default()
                    .push((time_offset, proc.memory.vms_bytes as f64));
            }
        }

        // Check each PID for sustained memory growth
        for (&pid, data_points) in &pid_series {
            if let Some(growth_rate) = self.calculate_growth_rate(data_points) {
                if growth_rate > self.config.leak_growth_threshold_bytes_per_sec {
                    tracing::warn!(
                        pid,
                        growth_rate_bytes_per_sec = growth_rate,
                        threshold = self.config.leak_growth_threshold_bytes_per_sec,
                        "Potential memory leak detected"
                    );
                    events.push(Event::new(EventKind::MemoryLeak {
                        pid,
                        growth_rate_bytes_per_sec: growth_rate,
                    }));
                }
            }
        }

        events
    }

    /// Calculate the VMS growth rate (bytes/sec) from pre-indexed data points.
    ///
    /// Uses linear regression over the (time_offset_secs, vms_bytes) pairs.
    /// Returns `None` if there are fewer than 10 data points or insufficient time span.
    fn calculate_growth_rate(&self, data_points: &[(f64, f64)]) -> Option<f64> {
        if data_points.len() < 10 {
            return None;
        }

        // Require at least 30 seconds of data to avoid false positives on startup
        let time_span = data_points.last().unwrap().0 - data_points.first().unwrap().0;
        if time_span < 30.0 {
            return None;
        }

        // Simple linear regression: slope = Σ((x-x̄)(y-ȳ)) / Σ((x-x̄)²)
        let n = data_points.len() as f64;
        let sum_x: f64 = data_points.iter().map(|(x, _)| x).sum();
        let sum_y: f64 = data_points.iter().map(|(_, y)| y).sum();
        let mean_x = sum_x / n;
        let mean_y = sum_y / n;

        let mut numerator = 0.0;
        let mut denominator = 0.0;

        for &(x, y) in data_points {
            let dx = x - mean_x;
            let dy = y - mean_y;
            numerator += dx * dy;
            denominator += dx * dx;
        }

        if denominator.abs() < f64::EPSILON {
            return None;
        }

        let slope = numerator / denominator;

        // Only report positive growth rates (memory growing, not shrinking)
        if slope > 0.0 {
            Some(slope)
        } else {
            None
        }
    }
}

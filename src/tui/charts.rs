use ratatui::{
    layout::Rect,
    style::{Color, Style},
    symbols,
    text::Span,
    widgets::{Axis, Block, Borders, Chart, Dataset},
    Frame,
};

use crate::models::MemorySnapshot;

const MB: f64 = 1024.0 * 1024.0;

/// Memory trend chart showing RSS, VMS, and available memory over time.
pub struct ChartPanel {
    /// (timestamp_secs_offset, rss_mb)
    rss_data: Vec<(f64, f64)>,
    /// (timestamp_secs_offset, vms_mb)
    vms_data: Vec<(f64, f64)>,
    /// (timestamp_secs_offset, available_mb)
    available_data: Vec<(f64, f64)>,
    /// Maximum Y value seen (for axis scaling)
    max_y: f64,
    /// Display window in seconds
    pub timespan_secs: u64,
}

impl ChartPanel {
    pub fn new() -> Self {
        Self {
            rss_data: Vec::new(),
            vms_data: Vec::new(),
            available_data: Vec::new(),
            max_y: 100.0,
            timespan_secs: 300, // 5 minutes
        }
    }

    /// Update chart data from a sequence of historical snapshots.
    pub fn update(&mut self, history: &[MemorySnapshot]) {
        if history.is_empty() {
            return;
        }

        self.rss_data.clear();
        self.vms_data.clear();
        self.available_data.clear();
        self.max_y = 100.0; // minimum axis height

        // Use the latest snapshot timestamp as the reference point (t=0 at latest)
        let latest_ts = history
            .last()
            .map(|s| s.timestamp.timestamp() as f64)
            .unwrap_or(0.0);

        for snap in history {
            let t = snap.timestamp.timestamp() as f64 - latest_ts;
            // Only include data within the timespan window
            if t < -(self.timespan_secs as f64) {
                continue;
            }

            let rss_mb = snap.total_rss as f64 / MB;
            let vms_mb = snap.total_vms as f64 / MB;
            let avail_mb = snap.system_available_memory as f64 / MB;

            self.rss_data.push((t, rss_mb));
            self.vms_data.push((t, vms_mb));
            self.available_data.push((t, avail_mb));

            let local_max = rss_mb.max(vms_mb).max(avail_mb);
            if local_max > self.max_y {
                self.max_y = local_max;
            }
        }

        // Add 10% headroom to Y axis
        self.max_y *= 1.1;
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let x_min = -(self.timespan_secs as f64);
        let x_max = 0.0;

        let datasets = vec![
            Dataset::default()
                .name("RSS")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::Blue))
                .data(&self.rss_data),
            Dataset::default()
                .name("VMS")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::Yellow))
                .data(&self.vms_data),
            Dataset::default()
                .name("Available")
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(Color::Green))
                .data(&self.available_data),
        ];

        let x_labels: Vec<Span> = vec![
            format!("-{}s", self.timespan_secs).into(),
            format!("-{}s", self.timespan_secs / 2).into(),
            "now".into(),
        ];

        let y_labels: Vec<Span> = vec![
            "0 MB".into(),
            format!("{:.0} MB", self.max_y / 2.0).into(),
            format!("{:.0} MB", self.max_y).into(),
        ];

        let chart = Chart::new(datasets)
            .block(
                Block::default()
                    .title(" Memory Trend ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .x_axis(
                Axis::default()
                    .title("Time")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([x_min, x_max])
                    .labels(x_labels),
            )
            .y_axis(
                Axis::default()
                    .title("MB")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([0.0, self.max_y])
                    .labels(y_labels),
            );

        frame.render_widget(chart, area);
    }
}

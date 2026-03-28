use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::models::{AlertLevel, Event, EventKind};

/// A single alert entry in the alerts panel.
struct AlertEntry {
    timestamp: String,
    level: AlertLevel,
    message: String,
}

/// Scrollable alerts panel showing recent events with severity coloring.
pub struct AlertsPanel {
    alerts: Vec<AlertEntry>,
    max_alerts: usize,
    scroll_offset: usize,
}

impl AlertsPanel {
    pub fn new(max_alerts: usize) -> Self {
        Self {
            alerts: Vec::new(),
            max_alerts,
            scroll_offset: 0,
        }
    }

    /// Ingest events from the daemon and convert them to alert entries.
    pub fn update(&mut self, events: &[Event]) {
        for event in events {
            let (level, message) = match &event.kind {
                EventKind::ProcessDiscovered { pid, name } => (
                    AlertLevel::Info,
                    format!("Process discovered: {name} (PID {pid})"),
                ),
                EventKind::StateChange { pid, from, to } => {
                    let level = match to {
                        crate::models::ProcessState::Orphan => AlertLevel::Warning,
                        crate::models::ProcessState::Stale => AlertLevel::Warning,
                        _ => AlertLevel::Info,
                    };
                    (level, format!("PID {pid}: {from} -> {to}"))
                }
                EventKind::MemoryLeak {
                    pid,
                    growth_rate_bytes_per_sec,
                } => {
                    let rate_mb = growth_rate_bytes_per_sec / (1024.0 * 1024.0);
                    (
                        AlertLevel::Critical,
                        format!("Memory leak detected: PID {pid} ({rate_mb:.2} MB/s)"),
                    )
                }
                EventKind::CleanupStarted { pid } => {
                    (AlertLevel::Info, format!("Cleanup started: PID {pid}"))
                }
                EventKind::CleanupCompleted { pid, success } => {
                    if *success {
                        (AlertLevel::Info, format!("Cleanup completed: PID {pid}"))
                    } else {
                        (AlertLevel::Warning, format!("Cleanup failed: PID {pid}"))
                    }
                }
                EventKind::Alert { level, message } => (*level, message.clone()),
                EventKind::DaemonStarted => (AlertLevel::Info, "Daemon started".to_string()),
                EventKind::DaemonStopped => (AlertLevel::Warning, "Daemon stopped".to_string()),
            };

            let timestamp = event.timestamp.format("%H:%M:%S").to_string();
            self.alerts.push(AlertEntry {
                timestamp,
                level,
                message,
            });
        }

        // Trim to max alerts
        while self.alerts.len() > self.max_alerts {
            self.alerts.remove(0);
        }
    }

    /// Add a local alert (not from daemon events).
    pub fn add_alert(&mut self, level: AlertLevel, message: String) {
        let timestamp = chrono::Utc::now().format("%H:%M:%S").to_string();
        self.alerts.push(AlertEntry {
            timestamp,
            level,
            message,
        });
        while self.alerts.len() > self.max_alerts {
            self.alerts.remove(0);
        }
    }

    pub fn alert_count(&self) -> usize {
        self.alerts.len()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, is_focused: bool) {
        let border_color = if is_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let block = Block::default()
            .title(format!(" Alerts ({}) ", self.alerts.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        // Calculate visible range
        let inner_height = block.inner(area).height as usize;
        let total = self.alerts.len();

        // Show the latest alerts, adjusted by scroll offset
        let end = total.saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(inner_height);

        let items: Vec<ListItem> = self.alerts[start..end]
            .iter()
            .map(|entry| {
                let (level_color, level_modifier) = level_style(entry.level);
                let line = Line::from(vec![
                    Span::styled(
                        format!("[{}] ", entry.timestamp),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("[{:>4}] ", entry.level),
                        Style::default()
                            .fg(level_color)
                            .add_modifier(level_modifier),
                    ),
                    Span::styled(&entry.message, Style::default().fg(Color::White)),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(block);
        frame.render_widget(list, area);
    }

    pub fn scroll_down(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    pub fn scroll_up(&mut self) {
        let max_scroll = self.alerts.len();
        if self.scroll_offset < max_scroll {
            self.scroll_offset += 1;
        }
    }
}

/// Return a color and modifier for an alert level.
fn level_style(level: AlertLevel) -> (Color, Modifier) {
    match level {
        AlertLevel::Info => (Color::Blue, Modifier::empty()),
        AlertLevel::Warning => (Color::Yellow, Modifier::empty()),
        AlertLevel::Critical => (Color::Red, Modifier::BOLD),
    }
}

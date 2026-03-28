use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use crate::models::MemorySnapshot;

use super::format_bytes;

/// Dashboard overview panel showing system memory gauges and summary stats.
pub struct DashboardPanel {
    system_used: u64,
    system_total: u64,
    claude_rss: u64,
    claude_vms: u64,
    swap_used: u64,
    process_count: u32,
    orphan_count: u32,
    alert_count: usize,
    daemon_uptime_secs: Option<u64>,
}

impl DashboardPanel {
    pub fn new() -> Self {
        Self {
            system_used: 0,
            system_total: 1, // avoid division by zero
            claude_rss: 0,
            claude_vms: 0,
            swap_used: 0,
            process_count: 0,
            orphan_count: 0,
            alert_count: 0,
            daemon_uptime_secs: None,
        }
    }

    pub fn update(&mut self, snapshot: &MemorySnapshot) {
        self.system_used = snapshot.system_used_memory;
        self.system_total = snapshot.system_total_memory.max(1);
        self.claude_rss = snapshot.total_rss;
        self.claude_vms = snapshot.total_vms;
        self.swap_used = snapshot.total_swap;
        self.process_count = snapshot.claude_process_count;
        self.orphan_count = snapshot.orphan_count;
    }

    pub fn set_uptime(&mut self, secs: u64) {
        self.daemon_uptime_secs = Some(secs);
    }

    pub fn set_alert_count(&mut self, count: usize) {
        self.alert_count = count;
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" Dashboard ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Split inner area: top for gauges, bottom for summary stats
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(4), Constraint::Length(2)])
            .split(inner);

        self.render_gauges(frame, chunks[0]);
        self.render_summary(frame, chunks[1]);
    }

    fn render_gauges(&self, frame: &mut Frame, area: Rect) {
        // Decide how many gauges to show (skip swap if zero)
        let show_swap = self.swap_used > 0;
        let constraints = if show_swap {
            vec![
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
            ]
        } else {
            vec![
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ]
        };

        let gauge_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let mut idx = 0;

        // System Memory gauge
        let sys_ratio = self.ratio(self.system_used, self.system_total);
        let sys_label = format!(
            "System Memory: {} / {} ({:.0}%)",
            format_bytes(self.system_used),
            format_bytes(self.system_total),
            sys_ratio * 100.0,
        );
        frame.render_widget(self.make_gauge(&sys_label, sys_ratio), gauge_areas[idx]);
        idx += 1;

        // Claude RSS gauge (relative to system total)
        let rss_ratio = self.ratio(self.claude_rss, self.system_total);
        let rss_label = format!(
            "Claude RSS: {} ({:.1}% of system)",
            format_bytes(self.claude_rss),
            rss_ratio * 100.0,
        );
        frame.render_widget(self.make_gauge(&rss_label, rss_ratio), gauge_areas[idx]);
        idx += 1;

        // Claude VMS gauge (relative to system total)
        let vms_ratio = self.ratio(self.claude_vms, self.system_total);
        let vms_label = format!(
            "Claude VMS: {} ({:.1}% of system)",
            format_bytes(self.claude_vms),
            vms_ratio * 100.0,
        );
        frame.render_widget(self.make_gauge(&vms_label, vms_ratio), gauge_areas[idx]);

        // Swap gauge (only if non-zero)
        if show_swap {
            idx += 1;
            let swap_ratio = self.ratio(self.swap_used, self.system_total);
            let swap_label = format!(
                "Swap: {} ({:.1}% of system)",
                format_bytes(self.swap_used),
                swap_ratio * 100.0,
            );
            frame.render_widget(self.make_gauge(&swap_label, swap_ratio), gauge_areas[idx]);
        }
    }

    fn render_summary(&self, frame: &mut Frame, area: Rect) {
        let uptime_str = match self.daemon_uptime_secs {
            Some(secs) => {
                let h = secs / 3600;
                let m = (secs % 3600) / 60;
                let s = secs % 60;
                format!("{h}h {m}m {s}s")
            }
            None => "N/A".to_string(),
        };

        let line = Line::from(vec![
            Span::styled("  Processes: ", Style::default().fg(Color::Gray)),
            Span::styled(
                self.process_count.to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled("Orphans: ", Style::default().fg(Color::Gray)),
            Span::styled(
                self.orphan_count.to_string(),
                Style::default().fg(if self.orphan_count > 0 {
                    Color::Red
                } else {
                    Color::Green
                }),
            ),
            Span::raw("    "),
            Span::styled("Alerts: ", Style::default().fg(Color::Gray)),
            Span::styled(
                self.alert_count.to_string(),
                Style::default().fg(if self.alert_count > 0 {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
            Span::raw("    "),
            Span::styled("Uptime: ", Style::default().fg(Color::Gray)),
            Span::styled(uptime_str, Style::default().fg(Color::White)),
        ]);

        frame.render_widget(Paragraph::new(line), area);
    }

    fn make_gauge(&self, label: &str, ratio: f64) -> Gauge<'_> {
        let color = percent_color(ratio);
        Gauge::default()
            .gauge_style(Style::default().fg(color).bg(Color::DarkGray))
            .ratio(ratio)
            .label(Span::styled(
                label.to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ))
    }

    fn ratio(&self, numerator: u64, denominator: u64) -> f64 {
        if denominator == 0 {
            0.0
        } else {
            (numerator as f64 / denominator as f64).clamp(0.0, 1.0)
        }
    }
}

/// Choose a color based on percentage: green (<60%), yellow (60-80%), red (>80%).
fn percent_color(ratio: f64) -> Color {
    if ratio < 0.6 {
        Color::Green
    } else if ratio < 0.8 {
        Color::Yellow
    } else {
        Color::Red
    }
}

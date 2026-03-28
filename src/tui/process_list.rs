use chrono::Utc;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Row, Table, TableState},
    Frame,
};

use crate::models::{ProcessInfo, ProcessState};

use super::format_bytes;

/// Sortable, scrollable process list panel.
pub struct ProcessListPanel {
    processes: Vec<ProcessInfo>,
    pub state: TableState,
    sort_column: SortColumn,
    sort_ascending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Pid,
    Name,
    Rss,
    Vms,
    State,
}

impl ProcessListPanel {
    pub fn new() -> Self {
        let mut state = TableState::default();
        state.select(Some(0));
        Self {
            processes: Vec::new(),
            state,
            sort_column: SortColumn::Rss,
            sort_ascending: false,
        }
    }

    pub fn update(&mut self, mut processes: Vec<ProcessInfo>) {
        // Remember which PID was selected so we can restore it after re-sort
        let selected_pid = self.selected_process().map(|p| p.pid);

        self.sort_processes(&mut processes);
        self.processes = processes;

        if self.processes.is_empty() {
            self.state.select(None);
        } else if let Some(pid) = selected_pid {
            // Find the same PID in the new list
            let new_idx = self
                .processes
                .iter()
                .position(|p| p.pid == pid)
                .unwrap_or(0);
            self.state.select(Some(new_idx));
        } else {
            self.state.select(Some(0));
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, is_focused: bool) {
        let border_color = if is_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let header_cells = ["PID", "Name", "RSS", "VMS", "State", "Uptime"]
            .into_iter()
            .map(|h| {
                Cell::from(Span::styled(
                    h,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
            });
        let header = Row::new(header_cells).height(1);

        let now = Utc::now();
        let rows = self.processes.iter().map(|p| {
            let state_color = state_color(p.state);
            let uptime = format_duration((now - p.started_at).num_seconds().max(0) as u64);

            Row::new(vec![
                Cell::from(p.pid.to_string()),
                Cell::from(truncate_name(&p.name, 20)),
                Cell::from(format_bytes(p.memory.rss_bytes)),
                Cell::from(format_bytes(p.memory.vms_bytes)),
                Cell::from(Span::styled(
                    p.state.to_string(),
                    Style::default()
                        .fg(state_color)
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(uptime),
            ])
        });

        let widths = [
            ratatui::layout::Constraint::Length(8),
            ratatui::layout::Constraint::Min(15),
            ratatui::layout::Constraint::Length(10),
            ratatui::layout::Constraint::Length(10),
            ratatui::layout::Constraint::Length(8),
            ratatui::layout::Constraint::Length(10),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .title(format!(
                        " Processes ({}) [sort: {:?} {}] ",
                        self.processes.len(),
                        self.sort_column,
                        if self.sort_ascending { "▲" } else { "▼" }
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
            )
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        let mut table_state = self.state.clone();
        frame.render_stateful_widget(table, area, &mut table_state);
    }

    pub fn select_next(&mut self) {
        if self.processes.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.processes.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn select_prev(&mut self) {
        if self.processes.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.processes.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        self.state.selected().and_then(|i| self.processes.get(i))
    }

    pub fn sort_by(&mut self, col: SortColumn) {
        if self.sort_column == col {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_column = col;
            self.sort_ascending = true;
        }
        let mut procs = std::mem::take(&mut self.processes);
        self.sort_processes(&mut procs);
        self.processes = procs;
    }

    fn sort_processes(&self, processes: &mut [ProcessInfo]) {
        let asc = self.sort_ascending;
        processes.sort_by(|a, b| {
            let ord = match self.sort_column {
                SortColumn::Pid => a.pid.cmp(&b.pid),
                SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortColumn::Rss => a.memory.rss_bytes.cmp(&b.memory.rss_bytes),
                SortColumn::Vms => a.memory.vms_bytes.cmp(&b.memory.vms_bytes),
                SortColumn::State => state_order(a.state).cmp(&state_order(b.state)),
            };
            if asc {
                ord
            } else {
                ord.reverse()
            }
        });
    }
}

/// Map process state to color.
fn state_color(state: ProcessState) -> Color {
    match state {
        ProcessState::Active => Color::Green,
        ProcessState::Idle => Color::Yellow,
        ProcessState::Stale => Color::Rgb(204, 153, 0), // dark yellow / orange
        ProcessState::Orphan => Color::Red,
    }
}

/// Map process state to a sort order number.
fn state_order(state: ProcessState) -> u8 {
    match state {
        ProcessState::Active => 0,
        ProcessState::Idle => 1,
        ProcessState::Stale => 2,
        ProcessState::Orphan => 3,
    }
}

/// Format a duration in seconds into a human-readable string.
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h {m}m")
    }
}

/// Truncate a name to fit within a column width.
fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}...", &name[..max_len.saturating_sub(3)])
    }
}

impl std::fmt::Debug for SortColumn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pid => write!(f, "PID"),
            Self::Name => write!(f, "Name"),
            Self::Rss => write!(f, "RSS"),
            Self::Vms => write!(f, "VMS"),
            Self::State => write!(f, "State"),
        }
    }
}

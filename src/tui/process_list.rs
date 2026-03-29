use std::borrow::Cow;
use std::collections::HashMap;

use chrono::Utc;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Row, Table, TableState},
    Frame,
};

use crate::models::{ProcessInfo, ProcessState};

use crate::util::format_bytes;

/// A process with tree display metadata.
struct DisplayProcess {
    info: ProcessInfo,
    /// 0 = root, 1 = child, 2 = grandchild, ...
    depth: u16,
    /// True when this node is the last sibling at its depth (renders └─ vs ├─).
    is_last: bool,
}

/// Sortable, scrollable process list panel with optional tree view.
pub struct ProcessListPanel {
    /// Flat display list (in tree-order when tree_mode is on).
    display_list: Vec<DisplayProcess>,
    /// Raw processes kept for rebuild on sort/toggle.
    raw_processes: Vec<ProcessInfo>,
    pub state: TableState,
    sort_column: SortColumn,
    sort_ascending: bool,
    tree_mode: bool,
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
            display_list: Vec::new(),
            raw_processes: Vec::new(),
            state,
            sort_column: SortColumn::Rss,
            sort_ascending: false,
            tree_mode: true,
        }
    }

    pub fn update(&mut self, processes: Vec<ProcessInfo>) {
        let selected_pid = self.selected_process().map(|p| p.pid);

        self.raw_processes = processes;
        self.rebuild_display_list();

        if self.display_list.is_empty() {
            self.state.select(None);
        } else if let Some(pid) = selected_pid {
            let new_idx = self
                .display_list
                .iter()
                .position(|d| d.info.pid == pid)
                .unwrap_or(0);
            self.state.select(Some(new_idx));
        } else {
            self.state.select(Some(0));
        }
    }

    pub fn toggle_tree_mode(&mut self) {
        let selected_pid = self.selected_process().map(|p| p.pid);
        self.tree_mode = !self.tree_mode;
        self.rebuild_display_list();
        if let Some(pid) = selected_pid {
            let new_idx = self
                .display_list
                .iter()
                .position(|d| d.info.pid == pid)
                .unwrap_or(0);
            self.state.select(Some(new_idx));
        }
    }

    /// Rebuild the display list from raw_processes using current tree_mode and sort settings.
    fn rebuild_display_list(&mut self) {
        if self.tree_mode {
            self.display_list = build_tree_list(
                &self.raw_processes,
                self.sort_column,
                self.sort_ascending,
            );
        } else {
            let mut flat = self.raw_processes.clone();
            sort_processes_flat(&mut flat, self.sort_column, self.sort_ascending);
            self.display_list = flat
                .into_iter()
                .map(|info| DisplayProcess {
                    info,
                    depth: 0,
                    is_last: false,
                })
                .collect();
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
        let rows = self.display_list.iter().map(|d| {
            let p = &d.info;
            let state_color = state_color(p.state);
            let uptime = format_duration((now - p.started_at).num_seconds().max(0) as u64);

            let name_display = if self.tree_mode && d.depth > 0 {
                let indent = "  ".repeat((d.depth - 1) as usize);
                let branch = if d.is_last { "└─ " } else { "├─ " };
                let prefix = format!("{indent}{branch}");
                // Leave more room for the actual name
                let max_name = 22_usize.saturating_sub(prefix.len());
                format!("{prefix}{}", truncate_name(&p.name, max_name))
            } else {
                truncate_name(&p.name, 22).into_owned()
            };

            Row::new(vec![
                Cell::from(p.pid.to_string()),
                Cell::from(name_display),
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
            ratatui::layout::Constraint::Min(25),
            ratatui::layout::Constraint::Length(10),
            ratatui::layout::Constraint::Length(10),
            ratatui::layout::Constraint::Length(8),
            ratatui::layout::Constraint::Length(10),
        ];

        let mode_tag = if self.tree_mode { "tree" } else { "flat" };
        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .title(format!(
                        " Processes ({}) [{}] [sort: {:?} {}] ",
                        self.display_list.len(),
                        mode_tag,
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
        if self.display_list.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.display_list.len() - 1 {
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
        if self.display_list.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.display_list.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        self.state
            .selected()
            .and_then(|i| self.display_list.get(i))
            .map(|d| &d.info)
    }

    pub fn sort_by(&mut self, col: SortColumn) {
        if self.sort_column == col {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_column = col;
            self.sort_ascending = true;
        }
        self.rebuild_display_list();
    }
}

// ---------------------------------------------------------------------------
// Tree building
// ---------------------------------------------------------------------------

/// Build a tree-ordered display list from a flat set of processes.
///
/// Roots are processes whose `parent_pid` is `None` or whose parent is not
/// in the provided list.  Roots are sorted by the chosen column; children
/// are sorted by PID for stability.
fn build_tree_list(
    processes: &[ProcessInfo],
    sort_col: SortColumn,
    sort_asc: bool,
) -> Vec<DisplayProcess> {
    if processes.is_empty() {
        return Vec::new();
    }

    // Index processes by PID
    let pid_set: std::collections::HashSet<u32> = processes.iter().map(|p| p.pid).collect();

    // Build parent → children map
    let mut children_of: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut root_indices: Vec<usize> = Vec::new();

    for (idx, p) in processes.iter().enumerate() {
        let is_root = match p.parent_pid {
            None => true,
            Some(ppid) => !pid_set.contains(&ppid),
        };
        if is_root {
            root_indices.push(idx);
        } else {
            children_of
                .entry(p.parent_pid.unwrap())
                .or_default()
                .push(idx);
        }
    }

    // Sort roots by the active sort column
    sort_indices(&mut root_indices, processes, sort_col, sort_asc);

    // Sort children groups by PID for stability
    for children in children_of.values_mut() {
        children.sort_by_key(|&idx| processes[idx].pid);
    }

    // DFS traversal
    let mut result = Vec::with_capacity(processes.len());
    for &root_idx in &root_indices {
        dfs_collect(
            root_idx,
            0,
            true, // roots are always "last" (no sibling context at level 0)
            processes,
            &children_of,
            &mut result,
        );
    }

    result
}

fn dfs_collect(
    idx: usize,
    depth: u16,
    is_last: bool,
    processes: &[ProcessInfo],
    children_of: &HashMap<u32, Vec<usize>>,
    result: &mut Vec<DisplayProcess>,
) {
    result.push(DisplayProcess {
        info: processes[idx].clone(),
        depth,
        is_last,
    });

    let pid = processes[idx].pid;
    if let Some(children) = children_of.get(&pid) {
        let last_i = children.len().saturating_sub(1);
        for (i, &child_idx) in children.iter().enumerate() {
            dfs_collect(child_idx, depth + 1, i == last_i, processes, children_of, result);
        }
    }
}

fn sort_indices(
    indices: &mut [usize],
    processes: &[ProcessInfo],
    col: SortColumn,
    asc: bool,
) {
    indices.sort_by(|&a, &b| {
        let pa = &processes[a];
        let pb = &processes[b];
        let ord = match col {
            SortColumn::Pid => pa.pid.cmp(&pb.pid),
            SortColumn::Name => pa.name.to_ascii_lowercase().cmp(&pb.name.to_ascii_lowercase()),
            SortColumn::Rss => pa.memory.rss_bytes.cmp(&pb.memory.rss_bytes),
            SortColumn::Vms => pa.memory.vms_bytes.cmp(&pb.memory.vms_bytes),
            SortColumn::State => state_order(pa.state).cmp(&state_order(pb.state)),
        };
        if asc { ord } else { ord.reverse() }
    });
}

// ---------------------------------------------------------------------------
// Flat sort (existing logic, extracted)
// ---------------------------------------------------------------------------

fn sort_processes_flat(processes: &mut [ProcessInfo], col: SortColumn, asc: bool) {
    if matches!(col, SortColumn::Name) {
        processes.sort_by_cached_key(|p| p.name.to_ascii_lowercase());
        if !asc {
            processes.reverse();
        }
    } else {
        processes.sort_by(|a, b| {
            let ord = match col {
                SortColumn::Pid => a.pid.cmp(&b.pid),
                SortColumn::Name => unreachable!(),
                SortColumn::Rss => a.memory.rss_bytes.cmp(&b.memory.rss_bytes),
                SortColumn::Vms => a.memory.vms_bytes.cmp(&b.memory.vms_bytes),
                SortColumn::State => state_order(a.state).cmp(&state_order(b.state)),
            };
            if asc { ord } else { ord.reverse() }
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
fn truncate_name(name: &str, max_len: usize) -> Cow<'_, str> {
    if name.chars().count() <= max_len {
        Cow::Borrowed(name)
    } else {
        let truncated: String = name.chars().take(max_len.saturating_sub(3)).collect();
        Cow::Owned(format!("{truncated}..."))
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

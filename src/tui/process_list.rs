use std::collections::{HashMap, HashSet, VecDeque};

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
    /// True when this node has children (used for expand/collapse indicator).
    has_children: bool,
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
    /// PIDs whose children are collapsed (hidden) in tree view.
    collapsed: HashSet<u32>,
    /// When true, show full cmdline; when false, show process name only.
    show_cmdline: bool,
    /// Active filter string (empty = no filter). Matches against name and cmdline.
    filter: String,
    /// True when the user is typing into the filter input.
    pub filter_active: bool,
    /// Previous RSS values per PID for delta computation.
    previous_rss: HashMap<u32, u64>,
    /// RSS deltas (current - previous) per PID, computed each update cycle.
    rss_deltas: HashMap<u32, i64>,
    /// RSS history for sparkline rendering (last 20 samples per PID).
    rss_history: HashMap<u32, VecDeque<u64>>,
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
            collapsed: HashSet::new(),
            show_cmdline: true,
            filter: String::new(),
            filter_active: false,
            previous_rss: HashMap::new(),
            rss_deltas: HashMap::new(),
            rss_history: HashMap::new(),
        }
    }

    pub fn update(&mut self, processes: Vec<ProcessInfo>) {
        let selected_pid = self.selected_process().map(|p| p.pid);

        // Compute RSS deltas before replacing raw_processes
        let current_pids: HashSet<u32> = processes.iter().map(|p| p.pid).collect();
        self.rss_deltas.clear();
        for p in &processes {
            if let Some(&prev) = self.previous_rss.get(&p.pid) {
                self.rss_deltas
                    .insert(p.pid, p.memory.rss_bytes as i64 - prev as i64);
            }
            // else: first observation, no delta entry → will show "--"
        }
        // Store current RSS for next cycle
        self.previous_rss.clear();
        for p in &processes {
            self.previous_rss.insert(p.pid, p.memory.rss_bytes);
        }
        self.raw_processes = processes;

        // Update RSS history for sparkline (clean up departed PIDs)
        self.rss_history
            .retain(|pid, _| current_pids.contains(pid));
        for p in &self.raw_processes {
            let history = self
                .rss_history
                .entry(p.pid)
                .or_insert_with(|| VecDeque::with_capacity(20));
            history.push_back(p.memory.rss_bytes);
            if history.len() > 20 {
                history.pop_front();
            }
        }

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

    /// Toggle expand/collapse for the currently selected process in tree mode.
    pub fn toggle_collapse(&mut self) {
        if !self.tree_mode {
            return;
        }
        if let Some(pid) = self.selected_process().map(|p| p.pid) {
            if self.collapsed.contains(&pid) {
                self.collapsed.remove(&pid);
            } else {
                self.collapsed.insert(pid);
            }
            self.rebuild_display_list();
            // Restore selection to the same PID
            if let Some(new_idx) = self
                .display_list
                .iter()
                .position(|d| d.info.pid == pid)
            {
                self.state.select(Some(new_idx));
            }
        }
    }

    /// Rebuild the display list from raw_processes using current tree_mode,
    /// sort settings, and filter.
    fn rebuild_display_list(&mut self) {
        // Apply filter first
        let source: Vec<ProcessInfo> = if self.filter.is_empty() {
            self.raw_processes.clone()
        } else {
            let needle = self.filter.to_ascii_lowercase();
            self.raw_processes
                .iter()
                .filter(|p| {
                    p.name.to_ascii_lowercase().contains(&needle)
                        || p.cmdline.to_ascii_lowercase().contains(&needle)
                        || p.pid.to_string().contains(&needle)
                })
                .cloned()
                .collect()
        };

        if self.tree_mode && self.filter.is_empty() {
            // Tree mode only when not filtering (filter flattens to show matches)
            self.display_list = build_tree_list(
                &source,
                self.sort_column,
                self.sort_ascending,
                &self.collapsed,
            );
        } else {
            let mut flat = source;
            sort_processes_flat(&mut flat, self.sort_column, self.sort_ascending);
            self.display_list = flat
                .into_iter()
                .map(|info| DisplayProcess {
                    info,
                    depth: 0,
                    is_last: false,
                    has_children: false,
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

        let cmd_header = if self.show_cmdline { "Command" } else { "Name" };
        let header_cells = ["PID", cmd_header, "RSS", "Delta", "VMS", "Trend", "State", "Uptime"]
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

            // Build the command display string
            let cmd_text = if self.show_cmdline {
                format_command(&p.name, &p.cmdline)
            } else {
                p.name.clone()
            };

            let cmd_display = if self.tree_mode && d.depth > 0 {
                let indent = "  ".repeat((d.depth - 1) as usize);
                let branch = if d.is_last { "└─ " } else { "├─ " };
                format!("{indent}{branch}{cmd_text}")
            } else if self.tree_mode && d.has_children {
                // Root with children: show collapse indicator
                let indicator = if self.collapsed.contains(&p.pid) {
                    "[+] "
                } else {
                    "[-] "
                };
                format!("{indicator}{cmd_text}")
            } else {
                cmd_text
            };

            // RSS delta
            let delta = self.rss_deltas.get(&p.pid).copied();
            let (delta_text, delta_color) = match delta {
                Some(d) => format_delta(d),
                None => ("--".to_string(), Color::DarkGray),
            };

            // Get sparkline for this process
            let sparkline_text = self
                .rss_history
                .get(&p.pid)
                .map(render_sparkline)
                .unwrap_or_default();

            Row::new(vec![
                Cell::from(p.pid.to_string()),
                Cell::from(cmd_display),
                Cell::from(Span::styled(
                    format_bytes(p.memory.rss_bytes),
                    Style::default().fg(rss_color(p.memory.rss_bytes)),
                )),
                Cell::from(Span::styled(
                    delta_text,
                    Style::default().fg(delta_color),
                )),
                Cell::from(Span::styled(
                    format_bytes(p.memory.vms_bytes),
                    Style::default().fg(vms_color(p.memory.vms_bytes)),
                )),
                Cell::from(Span::styled(
                    sparkline_text,
                    Style::default().fg(Color::Cyan),
                )),
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
            ratatui::layout::Constraint::Length(8),  // PID
            ratatui::layout::Constraint::Min(40),    // Command/Name
            ratatui::layout::Constraint::Length(10), // RSS
            ratatui::layout::Constraint::Length(10), // Delta
            ratatui::layout::Constraint::Length(10), // VMS
            ratatui::layout::Constraint::Length(12), // Trend
            ratatui::layout::Constraint::Length(8),  // State
            ratatui::layout::Constraint::Length(10), // Uptime
        ];

        let mode_tag = if self.tree_mode && self.filter.is_empty() {
            "tree"
        } else {
            "flat"
        };
        let filter_tag = if self.filter.is_empty() {
            String::new()
        } else {
            format!(" [filter: {}]", self.filter)
        };
        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .title(format!(
                        " Processes ({}{}) [{}] [sort: {:?} {}]{} ",
                        self.display_list.len(),
                        if self.display_list.len() != self.raw_processes.len() {
                            format!("/{}", self.raw_processes.len())
                        } else {
                            String::new()
                        },
                        mode_tag,
                        self.sort_column,
                        if self.sort_ascending { "▲" } else { "▼" },
                        filter_tag,
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

    /// Move selection up by one page.
    pub fn select_page_up(&mut self, page_size: usize) {
        if self.display_list.is_empty() {
            return;
        }
        let current = self.state.selected().unwrap_or(0);
        let new_idx = current.saturating_sub(page_size);
        self.state.select(Some(new_idx));
    }

    /// Move selection down by one page.
    pub fn select_page_down(&mut self, page_size: usize) {
        if self.display_list.is_empty() {
            return;
        }
        let current = self.state.selected().unwrap_or(0);
        let new_idx = (current + page_size).min(self.display_list.len() - 1);
        self.state.select(Some(new_idx));
    }

    /// Jump to the first item.
    pub fn select_first(&mut self) {
        if !self.display_list.is_empty() {
            self.state.select(Some(0));
        }
    }

    /// Jump to the last item.
    pub fn select_last(&mut self) {
        if !self.display_list.is_empty() {
            self.state.select(Some(self.display_list.len() - 1));
        }
    }

    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        self.state
            .selected()
            .and_then(|i| self.display_list.get(i))
            .map(|d| &d.info)
    }

    /// Toggle between showing full cmdline and name-only.
    pub fn toggle_cmdline(&mut self) {
        self.show_cmdline = !self.show_cmdline;
    }

    // -- Filter API ----------------------------------------------------------

    /// Start filter input mode.
    pub fn start_filter(&mut self) {
        self.filter_active = true;
    }

    /// Cancel filter input and clear the filter.
    pub fn cancel_filter(&mut self) {
        self.filter_active = false;
        if !self.filter.is_empty() {
            self.filter.clear();
            self.rebuild_display_list();
        }
    }

    /// Append a character to the filter string.
    pub fn filter_push(&mut self, ch: char) {
        self.filter.push(ch);
        self.rebuild_display_list();
    }

    /// Remove the last character from the filter string.
    pub fn filter_pop(&mut self) {
        self.filter.pop();
        self.rebuild_display_list();
    }

    /// Return the current filter string (for status bar display).
    pub fn filter_text(&self) -> &str {
        &self.filter
    }

    pub fn has_active_filter(&self) -> bool {
        !self.filter.is_empty()
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
// Command display (htop-style)
// ---------------------------------------------------------------------------

/// Format a process command for display, similar to htop.
///
/// If `cmdline` is non-empty and differs from just the bare name, show the
/// full command line. Otherwise fall back to the process name.
fn format_command(name: &str, cmdline: &str) -> String {
    if cmdline.is_empty() || cmdline == name {
        return name.to_string();
    }
    // cmdline already contains the full invocation (e.g. "node /path/to/script.js --flag")
    // Show it as-is; the table's Min constraint + terminal width will clip naturally.
    cmdline.to_string()
}

// ---------------------------------------------------------------------------
// Tree building
// ---------------------------------------------------------------------------

/// Build a tree-ordered display list from a flat set of processes.
///
/// Roots are processes whose `parent_pid` is `None` or whose parent is not
/// in the provided list.  Roots are sorted by the chosen column; children
/// are sorted by PID for stability.  Collapsed nodes' children are hidden.
fn build_tree_list(
    processes: &[ProcessInfo],
    sort_col: SortColumn,
    sort_asc: bool,
    collapsed: &HashSet<u32>,
) -> Vec<DisplayProcess> {
    if processes.is_empty() {
        return Vec::new();
    }

    // Index processes by PID
    let pid_set: HashSet<u32> = processes.iter().map(|p| p.pid).collect();

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
            true,
            processes,
            &children_of,
            collapsed,
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
    collapsed: &HashSet<u32>,
    result: &mut Vec<DisplayProcess>,
) {
    let pid = processes[idx].pid;
    let has_children = children_of.contains_key(&pid);
    let is_collapsed = collapsed.contains(&pid);

    result.push(DisplayProcess {
        info: processes[idx].clone(),
        depth,
        is_last,
        has_children,
    });

    // Skip children if this node is collapsed
    if is_collapsed {
        return;
    }

    if let Some(children) = children_of.get(&pid) {
        let last_i = children.len().saturating_sub(1);
        for (i, &child_idx) in children.iter().enumerate() {
            dfs_collect(
                child_idx,
                depth + 1,
                i == last_i,
                processes,
                children_of,
                collapsed,
                result,
            );
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

/// Render a mini sparkline from RSS history using block characters.
fn render_sparkline(history: &VecDeque<u64>) -> String {
    if history.is_empty() {
        return String::new();
    }
    let bars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let min_val = history.iter().copied().min().unwrap_or(0);
    let max_val = history.iter().copied().max().unwrap_or(1);
    let range = max_val.saturating_sub(min_val).max(1);
    history
        .iter()
        .map(|&v| {
            let normalized = v.saturating_sub(min_val);
            let idx = ((normalized as f64 / range as f64) * 7.0) as usize;
            bars[idx.min(7)]
        })
        .collect()
}

/// Format an RSS delta value into a human-readable string with color.
fn format_delta(delta: i64) -> (String, Color) {
    if delta == 0 {
        return ("--".to_string(), Color::DarkGray);
    }
    let abs_bytes = delta.unsigned_abs();
    let (value, unit) = if abs_bytes >= 1024 * 1024 {
        (abs_bytes as f64 / (1024.0 * 1024.0), "MB")
    } else if abs_bytes >= 1024 {
        (abs_bytes as f64 / 1024.0, "KB")
    } else {
        (abs_bytes as f64, "B")
    };
    if delta > 0 {
        (format!("+{:.1} {}", value, unit), Color::Red)
    } else {
        (format!("-{:.1} {}", value, unit), Color::Green)
    }
}

/// Color for RSS cell based on memory usage.
fn rss_color(bytes: u64) -> Color {
    const MB50: u64 = 50 * 1024 * 1024;
    const MB200: u64 = 200 * 1024 * 1024;
    if bytes < MB50 {
        Color::Green
    } else if bytes <= MB200 {
        Color::Yellow
    } else {
        Color::Red
    }
}

/// Color for VMS cell based on virtual memory usage.
fn vms_color(bytes: u64) -> Color {
    const GB1: u64 = 1024 * 1024 * 1024;
    const GB5: u64 = 5 * GB1;
    if bytes < GB1 {
        Color::Green
    } else if bytes <= GB5 {
        Color::Yellow
    } else {
        Color::Red
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

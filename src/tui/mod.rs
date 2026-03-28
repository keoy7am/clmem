mod alerts;
mod charts;
mod dashboard;
mod process_list;

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::ipc::{self, IpcMessage, IpcResponse};
use crate::models::{AlertLevel, MemorySnapshot};

use alerts::AlertsPanel;
use charts::ChartPanel;
use dashboard::DashboardPanel;
use process_list::ProcessListPanel;

/// Format bytes into a human-readable string (B, KB, MB, GB).
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Which panel is currently focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Dashboard,
    ProcessList,
    Alerts,
}

impl Panel {
    fn next(self) -> Self {
        match self {
            Panel::Dashboard => Panel::ProcessList,
            Panel::ProcessList => Panel::Alerts,
            Panel::Alerts => Panel::Dashboard,
        }
    }
}

/// Confirmation dialog state for kill operations.
struct ConfirmDialog {
    pid: u32,
    name: String,
}

/// The interactive TUI application.
///
/// Built by the tui-dashboard team using ratatui + crossterm.
pub struct App {
    running: bool,
    active_panel: Panel,
    dashboard: DashboardPanel,
    chart: ChartPanel,
    process_list: ProcessListPanel,
    alerts: AlertsPanel,
    ipc_path: PathBuf,
    last_snapshot: Option<MemorySnapshot>,
    show_help: bool,
    confirm_kill: Option<ConfirmDialog>,
    status_message: Option<(String, Instant)>,
    daemon_connected: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            running: true,
            active_panel: Panel::ProcessList,
            dashboard: DashboardPanel::new(),
            chart: ChartPanel::new(),
            process_list: ProcessListPanel::new(),
            alerts: AlertsPanel::new(200),
            ipc_path: ipc::default_ipc_path(),
            last_snapshot: None,
            show_help: false,
            confirm_kill: None,
            status_message: None,
            daemon_connected: false,
        }
    }

    /// Run the TUI main loop (blocking, sync).
    pub fn run(&mut self) -> Result<()> {
        // Terminal setup
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let tick_rate = Duration::from_millis(250);
        let mut last_tick = Instant::now();

        // Initial data fetch
        self.update();

        let result = self.main_loop(&mut terminal, tick_rate, &mut last_tick);

        // Terminal cleanup (always runs)
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    fn main_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        tick_rate: Duration,
        last_tick: &mut Instant,
    ) -> Result<()> {
        while self.running {
            terminal.draw(|frame| self.render(frame))?;

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    // Windows sends both Press and Release events.
                    // Only handle Press to avoid double-triggering.
                    if key.kind == crossterm::event::KeyEventKind::Press {
                        self.handle_key(key);
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                self.update();
                *last_tick = Instant::now();
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        // Help overlay intercepts all keys except ? and Esc
        if self.show_help {
            match key.code {
                KeyCode::Char('?') | KeyCode::Esc => self.show_help = false,
                _ => {}
            }
            return;
        }

        // Kill confirmation dialog intercepts keys
        if self.confirm_kill.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(dialog) = self.confirm_kill.take() {
                        self.do_kill(dialog.pid);
                    }
                }
                _ => {
                    self.confirm_kill = None;
                    self.set_status("Kill cancelled");
                }
            }
            return;
        }

        match key.code {
            // Quit
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.running = false;
            }

            // Panel switching
            KeyCode::Tab => self.active_panel = self.active_panel.next(),

            // Navigation
            KeyCode::Up | KeyCode::Char('k') => match self.active_panel {
                Panel::ProcessList => self.process_list.select_prev(),
                Panel::Alerts => self.alerts.scroll_up(),
                Panel::Dashboard => {}
            },
            KeyCode::Down | KeyCode::Char('j') => match self.active_panel {
                Panel::ProcessList => self.process_list.select_next(),
                Panel::Alerts => self.alerts.scroll_down(),
                Panel::Dashboard => {}
            },

            // Kill selected process (shift-K)
            KeyCode::Char('K') => {
                if let Some(proc_info) = self.process_list.selected_process() {
                    self.confirm_kill = Some(ConfirmDialog {
                        pid: proc_info.pid,
                        name: proc_info.name.clone(),
                    });
                }
            }

            // Refresh
            KeyCode::Char('r') => {
                self.update();
                self.set_status("Refreshed");
            }

            // Sort columns (number keys)
            KeyCode::Char('1') => self.process_list.sort_by(process_list::SortColumn::Pid),
            KeyCode::Char('2') => self.process_list.sort_by(process_list::SortColumn::Name),
            KeyCode::Char('3') => self.process_list.sort_by(process_list::SortColumn::Rss),
            KeyCode::Char('4') => self.process_list.sort_by(process_list::SortColumn::Vms),
            KeyCode::Char('5') => self.process_list.sort_by(process_list::SortColumn::State),

            // Help
            KeyCode::Char('?') => self.show_help = true,

            _ => {}
        }
    }

    /// Poll the daemon for fresh data.
    fn update(&mut self) {
        // Try to get a snapshot
        match ipc::send_request(&self.ipc_path, &IpcMessage::GetSnapshot) {
            Ok(IpcResponse::Snapshot(snapshot)) => {
                self.daemon_connected = true;
                self.dashboard.update(&snapshot);
                self.process_list.update(snapshot.processes.clone());
                self.last_snapshot = Some(*snapshot);
            }
            Ok(_) => {
                self.daemon_connected = true;
            }
            Err(_) => {
                self.daemon_connected = false;
            }
        }

        // Try to get daemon status for uptime
        if self.daemon_connected {
            if let Ok(IpcResponse::Status { uptime_secs, .. }) =
                ipc::send_request(&self.ipc_path, &IpcMessage::GetStatus)
            {
                self.dashboard.set_uptime(uptime_secs);
            }
        }

        // Try to get events
        if self.daemon_connected {
            if let Ok(IpcResponse::Events(events)) =
                ipc::send_request(&self.ipc_path, &IpcMessage::GetEvents { last_n: 50 })
            {
                self.alerts.update(&events);
            }
        }

        // Try to get history for chart
        if self.daemon_connected {
            if let Ok(IpcResponse::History(history)) =
                ipc::send_request(&self.ipc_path, &IpcMessage::GetHistory { last_n: 300 })
            {
                self.chart.update(&history);
            }
        }

        self.dashboard.set_alert_count(self.alerts.alert_count());
    }

    fn render(&self, frame: &mut Frame) {
        let size = frame.area();

        // Main layout: top (dashboard + chart), middle (process list), bottom (alerts), status bar
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(9),
                Constraint::Min(10),
                Constraint::Length(8),
                Constraint::Length(1),
            ])
            .split(size);

        // Top row: dashboard (left) + chart (right)
        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(main_chunks[0]);

        self.dashboard.render(frame, top_chunks[0]);
        self.chart.render(frame, top_chunks[1]);

        // Process list
        self.process_list.render(
            frame,
            main_chunks[1],
            self.active_panel == Panel::ProcessList,
        );

        // Alerts
        self.alerts
            .render(frame, main_chunks[2], self.active_panel == Panel::Alerts);

        // Status bar
        self.render_status_bar(frame, main_chunks[3]);

        // Overlays (rendered last, on top)
        if !self.daemon_connected {
            self.render_disconnected_banner(frame, size);
        }

        if self.show_help {
            self.render_help_overlay(frame, size);
        }

        if let Some(ref dialog) = self.confirm_kill {
            self.render_confirm_dialog(frame, size, dialog);
        }
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        let conn_status = if self.daemon_connected {
            Span::styled(
                " CONNECTED ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                " DISCONNECTED ",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            )
        };

        let panel_name = match self.active_panel {
            Panel::Dashboard => "Dashboard",
            Panel::ProcessList => "Processes",
            Panel::Alerts => "Alerts",
        };

        let status_msg = self
            .status_message
            .as_ref()
            .filter(|(_, t)| t.elapsed() < Duration::from_secs(3))
            .map(|(msg, _)| msg.as_str())
            .unwrap_or("");

        let line = Line::from(vec![
            conn_status,
            Span::raw(" "),
            Span::styled(format!("[{panel_name}]"), Style::default().fg(Color::Cyan)),
            Span::raw("  "),
            Span::styled(status_msg, Style::default().fg(Color::Yellow)),
            Span::raw("  "),
            Span::styled(
                "?: help  q: quit  Tab: switch panel",
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_disconnected_banner(&self, frame: &mut Frame, size: Rect) {
        let width = 40_u16.min(size.width);
        let x = (size.width.saturating_sub(width)) / 2;
        let area = Rect::new(x, 0, width, 3);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let text = Paragraph::new(Line::from(vec![Span::styled(
            " Daemon not connected ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]))
        .block(block);

        frame.render_widget(Clear, area);
        frame.render_widget(text, area);
    }

    fn render_help_overlay(&self, frame: &mut Frame, size: Rect) {
        let width = 50_u16.min(size.width.saturating_sub(4));
        let height = 18_u16.min(size.height.saturating_sub(4));
        let x = (size.width.saturating_sub(width)) / 2;
        let y = (size.height.saturating_sub(height)) / 2;
        let area = Rect::new(x, y, width, height);

        let help_text = vec![
            Line::from(Span::styled(
                "Keyboard Shortcuts",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("  q / Esc     Quit"),
            Line::from("  Tab         Cycle panels"),
            Line::from("  Up / k      Navigate up"),
            Line::from("  Down / j    Navigate down"),
            Line::from("  K           Kill selected process"),
            Line::from("  r           Refresh data"),
            Line::from("  1-5         Sort by column"),
            Line::from("  ?           Toggle this help"),
            Line::from(""),
            Line::from(Span::styled(
                "Sort Columns:",
                Style::default().fg(Color::Yellow),
            )),
            Line::from("  1=PID  2=Name  3=RSS  4=VMS  5=State"),
            Line::from(""),
            Line::from(Span::styled(
                "Press ? or Esc to close",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let block = Block::default()
            .title(" Help ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let paragraph = Paragraph::new(help_text)
            .block(block)
            .wrap(Wrap { trim: false });

        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }

    fn render_confirm_dialog(&self, frame: &mut Frame, size: Rect, dialog: &ConfirmDialog) {
        let width = 46_u16.min(size.width.saturating_sub(4));
        let height = 5_u16;
        let x = (size.width.saturating_sub(width)) / 2;
        let y = (size.height.saturating_sub(height)) / 2;
        let area = Rect::new(x, y, width, height);

        let text = vec![
            Line::from(Span::styled(
                format!("Kill {} (PID {})?", dialog.name, dialog.pid),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("  y = confirm, any other key = cancel"),
        ];

        let block = Block::default()
            .title(" Confirm Kill ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let paragraph = Paragraph::new(text).block(block);

        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }

    fn do_kill(&mut self, pid: u32) {
        match ipc::send_request(
            &self.ipc_path,
            &IpcMessage::Cleanup {
                pids: vec![pid],
                force: false,
            },
        ) {
            Ok(IpcResponse::CleanupResult { cleaned, failed }) => {
                if cleaned > 0 {
                    self.set_status(&format!("Killed PID {pid}"));
                    self.alerts
                        .add_alert(AlertLevel::Info, format!("Process {pid} killed"));
                } else if failed > 0 {
                    self.set_status(&format!("Failed to kill PID {pid}"));
                    self.alerts
                        .add_alert(AlertLevel::Warning, format!("Failed to kill PID {pid}"));
                }
            }
            Ok(IpcResponse::Error(e)) => {
                self.set_status(&format!("Error: {e}"));
                self.alerts.add_alert(AlertLevel::Warning, e);
            }
            Err(e) => {
                self.set_status(&format!("IPC error: {e}"));
            }
            _ => {}
        }
    }

    fn set_status(&mut self, msg: &str) {
        self.status_message = Some((msg.to_string(), Instant::now()));
    }
}

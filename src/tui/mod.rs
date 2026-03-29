mod alerts;
mod charts;
mod dashboard;
mod process_list;

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
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

/// Data received from daemon via background IPC poller.
struct IpcData {
    snapshot: Option<Box<MemorySnapshot>>,
    uptime_secs: Option<u64>,
    events: Vec<crate::models::Event>,
    history: Vec<MemorySnapshot>,
    connected: bool,
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
    ipc_rx: Option<mpsc::Receiver<IpcData>>,
    poller_stop: Arc<AtomicBool>,
}

/// Fallback: 4 individual IPC round-trips for older daemons without GetAll.
fn poll_individual(ipc_path: &std::path::Path) -> IpcData {
    let mut data = IpcData {
        snapshot: None,
        uptime_secs: None,
        events: Vec::new(),
        history: Vec::new(),
        connected: false,
    };

    match ipc::send_request(ipc_path, &IpcMessage::GetSnapshot) {
        Ok(IpcResponse::Snapshot(snapshot)) => {
            data.connected = true;
            data.snapshot = Some(snapshot);
        }
        Ok(_) => {
            data.connected = true;
        }
        Err(_) => return data,
    }

    if let Ok(IpcResponse::Status { uptime_secs, .. }) =
        ipc::send_request(ipc_path, &IpcMessage::GetStatus)
    {
        data.uptime_secs = Some(uptime_secs);
    }

    if let Ok(IpcResponse::Events(events)) =
        ipc::send_request(ipc_path, &IpcMessage::GetEvents { last_n: 50 })
    {
        data.events = events;
    }

    if let Ok(IpcResponse::History(history)) =
        ipc::send_request(ipc_path, &IpcMessage::GetHistory { last_n: 300 })
    {
        data.history = history;
    }

    data
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
            ipc_rx: None,
            poller_stop: Arc::new(AtomicBool::new(false)),
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

        // Start background IPC poller (non-blocking data fetch)
        self.start_poller();

        let result = self.main_loop(&mut terminal, tick_rate, &mut last_tick);

        // Signal background poller to stop
        self.poller_stop.store(true, Ordering::Relaxed);

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

            // Expand/collapse tree node
            KeyCode::Enter => {
                if self.active_panel == Panel::ProcessList {
                    self.process_list.toggle_collapse();
                }
            }

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

            // Toggle tree/flat view
            KeyCode::Char('t') => self.process_list.toggle_tree_mode(),

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

    /// Start background thread that polls daemon for data via IPC.
    fn start_poller(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.ipc_rx = Some(rx);
        let ipc_path = self.ipc_path.clone();
        let stop = self.poller_stop.clone();

        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let data = match ipc::send_request(&ipc_path, &IpcMessage::GetAll) {
                    Ok(IpcResponse::All {
                        snapshot,
                        uptime_secs,
                        events,
                        history,
                    }) => IpcData {
                        snapshot,
                        uptime_secs: Some(uptime_secs),
                        events,
                        history,
                        connected: true,
                    },
                    Ok(IpcResponse::Error(_)) => {
                        // Fallback: older daemon without GetAll support
                        poll_individual(&ipc_path)
                    }
                    Ok(_) => IpcData {
                        snapshot: None,
                        uptime_secs: None,
                        events: Vec::new(),
                        history: Vec::new(),
                        connected: true,
                    },
                    Err(_) => IpcData {
                        snapshot: None,
                        uptime_secs: None,
                        events: Vec::new(),
                        history: Vec::new(),
                        connected: false,
                    },
                };

                let _ = tx.send(data);
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }

    /// Consume latest IPC data from background poller (non-blocking).
    fn update(&mut self) {
        let mut latest: Option<IpcData> = None;
        if let Some(ref rx) = self.ipc_rx {
            while let Ok(data) = rx.try_recv() {
                latest = Some(data);
            }
        }

        if let Some(data) = latest {
            self.daemon_connected = data.connected;

            if let Some(snapshot) = data.snapshot {
                let mut snap = *snapshot;
                self.dashboard.update(&snap);
                self.process_list.update(std::mem::take(&mut snap.processes));
                self.last_snapshot = Some(snap);
            }

            if let Some(uptime) = data.uptime_secs {
                self.dashboard.set_uptime(uptime);
            }

            if !data.events.is_empty() {
                self.alerts.update(&data.events);
            }

            if !data.history.is_empty() {
                self.chart.update(&data.history);
            }

            self.dashboard.set_alert_count(self.alerts.alert_count());
        }
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

        let version = env!("CARGO_PKG_VERSION");

        let line = Line::from(vec![
            Span::styled(
                format!(" clmem v{version} "),
                Style::default().fg(Color::DarkGray).bg(Color::Black),
            ),
            Span::raw(" "),
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
            Line::from("  Enter       Expand/collapse node"),
            Line::from("  K           Kill selected process"),
            Line::from("  r           Refresh data"),
            Line::from("  t           Toggle tree/flat view"),
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
                force: true,
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

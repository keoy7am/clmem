mod analyzer;
mod event;
mod profiler;
mod reaper;
mod scanner;

pub use analyzer::Analyzer;
pub use event::EventBus;
pub use profiler::Profiler;
pub use reaper::Reaper;
pub use scanner::Scanner;

use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::Utc;
use tokio::sync::{watch, Mutex as TokioMutex};

use crate::ipc::{default_ipc_path, IpcMessage, IpcResponse};
use crate::models::{Config, Event, EventKind, MemorySnapshot};
use crate::platform::{create_platform, Platform};

/// The main daemon orchestrating all monitoring subsystems.
pub struct Daemon {
    config: Config,
    platform: Arc<dyn Platform>,
    scanner: Arc<Mutex<Scanner>>,
    profiler: Arc<Mutex<Profiler>>,
    analyzer: Analyzer,
    reaper: Reaper,
    event_bus: TokioMutex<EventBus>,
    start_time: chrono::DateTime<Utc>,
    shutdown_tx: watch::Sender<bool>,
}

impl Daemon {
    /// Create a new daemon with the given configuration.
    pub fn new(config: Config) -> Result<Self> {
        let platform: Arc<dyn Platform> = Arc::from(create_platform());

        let scanner = Scanner::new(Arc::clone(&platform), config.clone());
        let profiler = Profiler::new(Arc::clone(&platform), &config);
        let analyzer = Analyzer::new(config.clone());
        let reaper = Reaper::new(Arc::clone(&platform), config.clone());
        let event_bus = EventBus::new(1000);
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);

        Ok(Self {
            config,
            platform,
            scanner: Arc::new(Mutex::new(scanner)),
            profiler: Arc::new(Mutex::new(profiler)),
            analyzer,
            reaper,
            event_bus: TokioMutex::new(event_bus),
            start_time: Utc::now(),
            shutdown_tx,
        })
    }

    /// Execute one scan + profile + optional reap cycle.
    async fn run_scan_cycle(&self) {
        // Run blocking platform calls (list_claude_processes, take_snapshot) off
        // the async runtime to avoid starving the tokio worker thread.
        let scanner = Arc::clone(&self.scanner);
        let profiler = Arc::clone(&self.profiler);
        let (scan_events, record_err) = tokio::task::spawn_blocking(move || {
            let scan_events = {
                let mut s = scanner.lock().expect("scanner mutex poisoned");
                s.scan()
            };
            let record_err = {
                let mut p = profiler.lock().expect("profiler mutex poisoned");
                p.record().err()
            };
            (scan_events, record_err)
        })
        .await
        .unwrap_or_else(|_| (Vec::new(), None));

        if let Some(e) = record_err {
            tracing::error!(error = %e, "Failed to record memory snapshot");
        }
        if !scan_events.is_empty() {
            let mut bus = self.event_bus.lock().await;
            bus.publish_many(scan_events);
        }

        if self.config.auto_cleanup {
            let processes = {
                let scanner = self.scanner.lock().expect("scanner mutex poisoned");
                scanner.get_processes()
            };
            let reap_events = self.reaper.reap_orphans(&processes).await;
            if !reap_events.is_empty() {
                let mut bus = self.event_bus.lock().await;
                bus.publish_many(reap_events);
            }
        }
    }

    /// Run the leak detection analyzer against the profiler history.
    async fn run_leak_analysis(&self) {
        let history = {
            let profiler = self.profiler.lock().expect("profiler mutex poisoned");
            profiler.get_history(60)
        };

        // Move CPU-bound linear regression analysis off the async runtime.
        let analyzer = self.analyzer.clone();
        let leak_events = tokio::task::spawn_blocking(move || {
            analyzer.analyze(&history)
        })
        .await
        .unwrap_or_default();

        if !leak_events.is_empty() {
            let mut bus = self.event_bus.lock().await;
            bus.publish_many(leak_events);
        }
    }

    /// Handle a single IPC message from a client.
    pub(crate) async fn handle_message(&self, msg: IpcMessage) -> IpcResponse {
        match msg {
            IpcMessage::Ping => IpcResponse::Pong,

            IpcMessage::GetStatus => {
                let scanner = self.scanner.lock().expect("scanner mutex poisoned");
                let processes = scanner.get_processes();
                let monitoring_count = processes.len() as u32;
                let orphan_count = processes
                    .iter()
                    .filter(|p| p.state == crate::models::ProcessState::Orphan)
                    .count() as u32;
                let total_rss: u64 = processes.iter().map(|p| p.memory.rss_bytes).sum();
                let uptime_secs = Utc::now()
                    .signed_duration_since(self.start_time)
                    .num_seconds() as u64;

                IpcResponse::Status {
                    uptime_secs,
                    monitoring_count,
                    orphan_count,
                    total_rss,
                }
            }

            IpcMessage::GetSnapshot => {
                let profiler = self.profiler.lock().expect("profiler mutex poisoned");
                match profiler.get_latest() {
                    Some(arc) => IpcResponse::Snapshot(Box::new(MemorySnapshot::clone(&arc))),
                    None => IpcResponse::Error("No snapshots recorded yet".to_string()),
                }
            }

            IpcMessage::GetProcessList => {
                let scanner = self.scanner.lock().expect("scanner mutex poisoned");
                IpcResponse::ProcessList(scanner.get_processes())
            }

            IpcMessage::GetHistory { last_n } => {
                let last_n = last_n.min(3600);
                let profiler = self.profiler.lock().expect("profiler mutex poisoned");
                IpcResponse::History(profiler.get_history(last_n))
            }

            IpcMessage::GetEvents { last_n } => {
                let last_n = last_n.min(1000);
                let bus = self.event_bus.lock().await;
                IpcResponse::Events(bus.get_recent(last_n))
            }

            IpcMessage::Cleanup { pids, force } => {
                let processes = {
                    let scanner = self.scanner.lock().expect("scanner mutex poisoned");
                    scanner.get_processes()
                };
                let (cleaned, failed) = self.reaper.cleanup_pids(&pids, force, &processes).await;

                {
                    let mut bus = self.event_bus.lock().await;
                    for &pid in &pids {
                        bus.publish(Event::new(EventKind::CleanupStarted { pid }));
                    }
                }

                IpcResponse::CleanupResult { cleaned, failed }
            }

            IpcMessage::GetConfig => IpcResponse::Config(self.config.clone()),

            IpcMessage::GetAll => {
                let uptime_secs = Utc::now()
                    .signed_duration_since(self.start_time)
                    .num_seconds() as u64;

                let (snapshot, history) = {
                    let profiler = self.profiler.lock().expect("profiler mutex poisoned");
                    let snapshot = profiler
                        .get_latest()
                        .map(|arc| Box::new(MemorySnapshot::clone(&arc)));
                    let history = profiler.get_history(300);
                    (snapshot, history)
                };

                let events = {
                    let bus = self.event_bus.lock().await;
                    bus.get_recent(50)
                };

                IpcResponse::All {
                    snapshot,
                    uptime_secs,
                    events,
                    history,
                }
            }

            IpcMessage::Shutdown => {
                tracing::info!("Shutdown requested via IPC");
                let _ = self.shutdown_tx.send(true);
                IpcResponse::Ok
            }
        }
    }

    fn pid_file_path(&self) -> std::path::PathBuf {
        self.platform.runtime_dir().join("clmem.pid")
    }

    fn write_pid_file(&self) -> Result<()> {
        let path = self.pid_file_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Try atomic creation first (create_new fails if file exists)
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                writeln!(file, "{}", std::process::id())?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Check if existing PID is alive via Platform trait
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if let Ok(pid) = contents.trim().parse::<u32>() {
                        if self.platform.is_process_alive(pid) {
                            anyhow::bail!("Another daemon is already running (PID {pid})");
                        }
                    }
                }
                // Stale PID file -- remove and retry
                std::fs::remove_file(&path)?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                use std::io::Write;
                writeln!(file, "{}", std::process::id())?;
            }
            Err(e) => return Err(e.into()),
        }

        tracing::debug!(path = %path.display(), "PID file written");
        Ok(())
    }

    fn remove_pid_file(&self) {
        let path = self.pid_file_path();
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(error = %e, "Failed to remove PID file");
            }
        }
    }
}

/// Run the daemon with proper Arc-based ownership, enabling the IPC server
/// to dispatch messages back to the daemon.
///
/// This is the recommended entry point for starting the daemon.
pub async fn run_daemon(config: Config) -> Result<()> {
    let daemon = Arc::new(Daemon::new(config)?);
    run_daemon_arc(daemon).await
}

/// Internal: run the daemon loop with Arc ownership.
async fn run_daemon_arc(daemon: Arc<Daemon>) -> Result<()> {
    tracing::info!(
        platform = std::env::consts::OS,
        scan_interval_ms = daemon.config.scan_interval_ms,
        "Daemon starting"
    );

    daemon.write_pid_file()?;

    {
        let mut bus = daemon.event_bus.lock().await;
        bus.publish(Event::new(EventKind::DaemonStarted));
    }

    let ipc_path = daemon
        .config
        .ipc_path
        .clone()
        .unwrap_or_else(default_ipc_path);

    let ipc_handle = crate::ipc::server::run_ipc_server(Arc::clone(&daemon), &ipc_path).await?;

    let scan_interval = tokio::time::Duration::from_millis(daemon.config.scan_interval_ms.max(100));
    let leak_check_interval =
        tokio::time::Duration::from_secs(daemon.config.leak_check_interval_secs.max(1));

    let mut scan_ticker = tokio::time::interval(scan_interval);
    let mut leak_ticker = tokio::time::interval(leak_check_interval);

    leak_ticker.tick().await;

    let mut shutdown_rx = daemon.shutdown_tx.subscribe();

    loop {
        tokio::select! {
            _ = scan_ticker.tick() => {
                daemon.run_scan_cycle().await;
            }
            _ = leak_ticker.tick() => {
                daemon.run_leak_analysis().await;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Ctrl+C received, shutting down");
                break;
            }
            _ = shutdown_rx.changed() => {
                tracing::info!("Shutdown requested via IPC, shutting down");
                break;
            }
        }
    }

    tracing::info!("Daemon shutting down");
    {
        let mut bus = daemon.event_bus.lock().await;
        bus.publish(Event::new(EventKind::DaemonStopped));
    }
    ipc_handle.abort();
    daemon.remove_pid_file();
    crate::ipc::remove_ipc_socket(&ipc_path);

    Ok(())
}


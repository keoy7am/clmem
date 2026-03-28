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

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::Mutex;

use crate::ipc::{default_ipc_path, IpcMessage, IpcResponse};
use crate::models::{Config, Event, EventKind};
use crate::platform::{create_platform, Platform};

/// PID file path for the daemon process.
fn pid_file_path() -> std::path::PathBuf {
    #[cfg(unix)]
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    #[cfg(windows)]
    let runtime_dir = std::env::var("TEMP").unwrap_or_else(|_| "C:\\Temp".to_string());

    std::path::PathBuf::from(runtime_dir).join("clmem.pid")
}

/// The main daemon orchestrating all monitoring subsystems.
pub struct Daemon {
    config: Config,
    #[allow(dead_code)]
    platform: Arc<dyn Platform>,
    scanner: Mutex<Scanner>,
    profiler: Mutex<Profiler>,
    analyzer: Analyzer,
    reaper: Reaper,
    event_bus: Mutex<EventBus>,
    start_time: chrono::DateTime<Utc>,
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

        Ok(Self {
            config,
            platform,
            scanner: Mutex::new(scanner),
            profiler: Mutex::new(profiler),
            analyzer,
            reaper,
            event_bus: Mutex::new(event_bus),
            start_time: Utc::now(),
        })
    }

    /// Execute one scan + profile + optional reap cycle.
    async fn run_scan_cycle(&self) {
        let scan_events = {
            let mut scanner = self.scanner.lock().await;
            scanner.scan()
        };
        if !scan_events.is_empty() {
            let mut bus = self.event_bus.lock().await;
            bus.publish_many(scan_events);
        }

        {
            let mut profiler = self.profiler.lock().await;
            if let Err(e) = profiler.record() {
                tracing::error!(error = %e, "Failed to record memory snapshot");
            }
        }

        if self.config.auto_cleanup {
            let processes = {
                let scanner = self.scanner.lock().await;
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
            let profiler = self.profiler.lock().await;
            profiler.get_history(60)
        };

        let leak_events = self.analyzer.analyze(&history);
        if !leak_events.is_empty() {
            let mut bus = self.event_bus.lock().await;
            bus.publish_many(leak_events);
        }
    }

    /// Handle a single IPC message from a client.
    async fn handle_message(&self, msg: IpcMessage) -> IpcResponse {
        match msg {
            IpcMessage::Ping => IpcResponse::Pong,

            IpcMessage::GetStatus => {
                let scanner = self.scanner.lock().await;
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
                let profiler = self.profiler.lock().await;
                match profiler.get_latest() {
                    Some(snapshot) => IpcResponse::Snapshot(Box::new(snapshot.clone())),
                    None => IpcResponse::Error("No snapshots recorded yet".to_string()),
                }
            }

            IpcMessage::GetProcessList => {
                let scanner = self.scanner.lock().await;
                IpcResponse::ProcessList(scanner.get_processes())
            }

            IpcMessage::GetHistory { last_n } => {
                let profiler = self.profiler.lock().await;
                IpcResponse::History(profiler.get_history(last_n))
            }

            IpcMessage::GetEvents { last_n } => {
                let bus = self.event_bus.lock().await;
                IpcResponse::Events(bus.get_recent(last_n))
            }

            IpcMessage::Cleanup { pids, force } => {
                let processes = {
                    let scanner = self.scanner.lock().await;
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

            IpcMessage::Subscribe => IpcResponse::Subscribed,

            IpcMessage::Shutdown => {
                tracing::info!("Shutdown requested via IPC");
                IpcResponse::Ok
            }
        }
    }

    /// Run the daemon main loop with graceful shutdown support.
    ///
    /// Note: this consumes self by wrapping it in an Arc for IPC dispatch.
    /// The `&mut self` signature is kept for CLI compatibility; the mutable
    /// borrow is released immediately.
    pub async fn run(&mut self) -> Result<()> {
        // We need Arc ownership for the IPC server to dispatch back to the daemon.
        // Since we can't move out of &mut self, we swap fields into a new Daemon
        // and wrap it. The caller should not use self after calling run().
        //
        // Alternative: The CLI could call run_daemon() directly. For now, we
        // reconstruct an Arc-wrapped daemon from self's config.
        let config = self.config.clone();
        run_daemon(config).await
    }

    fn write_pid_file(&self) -> Result<()> {
        let path = pid_file_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, std::process::id().to_string())?;
        tracing::debug!(path = %path.display(), "PID file written");
        Ok(())
    }

    fn remove_pid_file(&self) {
        let path = pid_file_path();
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(error = %e, "Failed to remove PID file");
            }
        }
    }

    fn remove_ipc_socket(&self) {
        let ipc_path = self
            .config
            .ipc_path
            .clone()
            .unwrap_or_else(default_ipc_path);

        #[cfg(unix)]
        if ipc_path.exists() {
            if let Err(e) = std::fs::remove_file(&ipc_path) {
                tracing::warn!(error = %e, "Failed to remove IPC socket");
            }
        }

        #[cfg(windows)]
        let _ = &ipc_path;
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
        platform = daemon.platform.name(),
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

    let ipc_handle = start_ipc_listener(Arc::clone(&daemon), &ipc_path).await?;

    let scan_interval = tokio::time::Duration::from_millis(daemon.config.scan_interval_ms);
    let leak_check_interval =
        tokio::time::Duration::from_secs(daemon.config.leak_check_interval_secs);

    let mut scan_ticker = tokio::time::interval(scan_interval);
    let mut leak_ticker = tokio::time::interval(leak_check_interval);

    leak_ticker.tick().await;

    loop {
        tokio::select! {
            _ = scan_ticker.tick() => {
                daemon.run_scan_cycle().await;
            }
            _ = leak_ticker.tick() => {
                daemon.run_leak_analysis().await;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
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
    daemon.remove_ipc_socket();

    Ok(())
}

/// Start the platform-appropriate IPC listener.
async fn start_ipc_listener(
    daemon: Arc<Daemon>,
    ipc_path: &std::path::Path,
) -> Result<tokio::task::JoinHandle<()>> {
    #[cfg(unix)]
    if ipc_path.exists() {
        let _ = std::fs::remove_file(ipc_path);
    }

    tracing::info!(path = %ipc_path.display(), "Starting IPC server");

    start_ipc_listener_platform(daemon, ipc_path).await
}

#[cfg(unix)]
async fn start_ipc_listener_platform(
    daemon: Arc<Daemon>,
    ipc_path: &std::path::Path,
) -> Result<tokio::task::JoinHandle<()>> {
    let listener = tokio::net::UnixListener::bind(ipc_path)?;
    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let daemon = Arc::clone(&daemon);
                    tokio::spawn(async move {
                        if let Err(e) = handle_unix_connection(daemon, stream).await {
                            tracing::debug!(error = %e, "IPC connection error");
                        }
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "IPC accept error");
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        }
    });
    Ok(handle)
}

#[cfg(windows)]
async fn start_ipc_listener_platform(
    daemon: Arc<Daemon>,
    ipc_path: &std::path::Path,
) -> Result<tokio::task::JoinHandle<()>> {
    let pipe_path = ipc_path.to_path_buf();
    let handle = tokio::spawn(async move {
        loop {
            let daemon = Arc::clone(&daemon);
            let path = pipe_path.clone();
            let result =
                tokio::task::spawn_blocking(move || handle_windows_pipe(daemon, &path)).await;

            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::debug!(error = %e, "Named pipe connection error");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Named pipe task panicked");
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    });
    Ok(handle)
}

#[cfg(unix)]
async fn handle_unix_connection(daemon: Arc<Daemon>, stream: tokio::net::UnixStream) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut reader, mut writer) = stream.into_split();

    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let msg_len = u32::from_le_bytes(len_buf) as usize;

    if msg_len > 16 * 1024 * 1024 {
        anyhow::bail!("IPC message too large: {msg_len} bytes");
    }

    let mut msg_buf = vec![0u8; msg_len];
    reader.read_exact(&mut msg_buf).await?;
    let msg: IpcMessage = serde_json::from_slice(&msg_buf)?;

    tracing::debug!(?msg, "IPC message received");

    let response = daemon.handle_message(msg).await;

    let resp_bytes = serde_json::to_vec(&response)?;
    let resp_len = (resp_bytes.len() as u32).to_le_bytes();
    writer.write_all(&resp_len).await?;
    writer.write_all(&resp_bytes).await?;
    writer.flush().await?;

    Ok(())
}

#[cfg(windows)]
fn handle_windows_pipe(daemon: Arc<Daemon>, path: &std::path::Path) -> Result<()> {
    use std::fs::OpenOptions;
    use std::io::{Read, Write};

    let pipe_path_str = path.to_string_lossy();
    let expected_prefix = "\\\\.\\pipe\\";
    if !pipe_path_str.starts_with(expected_prefix) {
        anyhow::bail!("Invalid named pipe path: {}", pipe_path_str);
    }

    let mut pipe = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(p) => p,
        Err(_) => {
            std::thread::sleep(std::time::Duration::from_millis(100));
            return Ok(());
        }
    };

    let mut len_buf = [0u8; 4];
    if pipe.read_exact(&mut len_buf).is_err() {
        return Ok(());
    }
    let msg_len = u32::from_le_bytes(len_buf) as usize;

    if msg_len > 16 * 1024 * 1024 {
        anyhow::bail!("IPC message too large: {} bytes", msg_len);
    }

    let mut msg_buf = vec![0u8; msg_len];
    pipe.read_exact(&mut msg_buf)?;
    let msg: IpcMessage = serde_json::from_slice(&msg_buf)?;

    tracing::debug!(?msg, "IPC message received (Windows pipe)");

    let rt = tokio::runtime::Handle::current();
    let response = rt.block_on(daemon.handle_message(msg));

    let resp_bytes = serde_json::to_vec(&response)?;
    let resp_len = (resp_bytes.len() as u32).to_le_bytes();
    pipe.write_all(&resp_len)?;
    pipe.write_all(&resp_bytes)?;
    pipe.flush()?;

    Ok(())
}

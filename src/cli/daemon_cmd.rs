use anyhow::Result;

use crate::ipc;
use crate::models::Config;

/// Run the `clmem daemon` command.
///
/// Starts the background monitoring daemon. If `foreground` is false,
/// logs a note that background daemonization is not yet implemented
/// and runs in the foreground anyway.
pub fn run(foreground: bool) -> Result<()> {
    // Check if daemon is already running
    let ipc_path = ipc::default_ipc_path();
    if ipc::is_daemon_running(&ipc_path) {
        anyhow::bail!(
            "Daemon is already running. Use `clmem status` to check or `clmem cleanup` to manage processes."
        );
    }

    if !foreground {
        tracing::info!("Note: Background daemonization not yet implemented, running in foreground");
    }

    let config = Config::load()?;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        scan_interval_ms = config.scan_interval_ms,
        auto_cleanup = config.auto_cleanup,
        "Starting daemon"
    );

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut daemon = crate::daemon::Daemon::new(config)?;
        daemon.run().await
    })
}

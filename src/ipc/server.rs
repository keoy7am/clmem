use std::sync::Arc;

use anyhow::Result;

use crate::daemon::Daemon;
use crate::ipc::IpcMessage;

/// Start the IPC listener, cleaning up stale sockets on Unix.
///
/// Returns a `JoinHandle` for the listener task which should be aborted
/// on daemon shutdown.
pub async fn run_ipc_server(
    daemon: Arc<Daemon>,
    ipc_path: &std::path::Path,
) -> Result<tokio::task::JoinHandle<()>> {
    #[cfg(unix)]
    if ipc_path.exists() {
        if let Err(e) = std::fs::remove_file(ipc_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %e, path = %ipc_path.display(), "Failed to remove stale IPC socket");
            }
        }
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
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(ipc_path, std::fs::Permissions::from_mode(0o600))?;
        tracing::info!("IPC socket permissions set to 0o600");
    }
    let max_connections = Arc::new(tokio::sync::Semaphore::new(32));
    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let permit = match max_connections.clone().try_acquire_owned() {
                        Ok(p) => p,
                        Err(_) => {
                            tracing::warn!("Max IPC connections reached, rejecting");
                            continue;
                        }
                    };
                    let daemon = Arc::clone(&daemon);
                    tokio::spawn(async move {
                        let _permit = permit;
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
    use tokio::net::windows::named_pipe::ServerOptions;

    let pipe_name = ipc_path.to_string_lossy().to_string();

    // Check if another daemon is already running by trying to connect as a client
    if tokio::net::windows::named_pipe::ClientOptions::new()
        .open(&pipe_name)
        .is_ok()
    {
        anyhow::bail!("Another daemon instance is already running");
    }

    // Create the first pipe server instance (reject_remote_clients is default-true,
    // but set explicitly for defense-in-depth)
    let mut server = ServerOptions::new()
        .reject_remote_clients(true)
        .create(&pipe_name)?;

    tracing::info!(pipe = %pipe_name, "Windows named pipe server created");

    let max_connections = Arc::new(tokio::sync::Semaphore::new(32));
    let handle = tokio::spawn(async move {
        loop {
            // Wait for a client to connect
            if let Err(e) = server.connect().await {
                tracing::error!(error = %e, "Named pipe connect error");
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                continue;
            }

            tracing::debug!("Named pipe client connected");

            // Create a new server instance for the next client BEFORE handling this one
            let new_server = match ServerOptions::new()
                .reject_remote_clients(true)
                .create(&pipe_name)
            {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to create next pipe instance");
                    // Disconnect current and retry
                    server.disconnect().ok();
                    continue;
                }
            };

            // Hand off current connection, swap in new server for next iteration
            let connected_pipe = server;
            server = new_server;

            let permit = match max_connections.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    tracing::warn!("Max IPC connections reached, rejecting");
                    connected_pipe.disconnect().ok();
                    continue;
                }
            };
            let daemon = Arc::clone(&daemon);
            tokio::spawn(async move {
                let _permit = permit;
                if let Err(e) = handle_windows_pipe_async(daemon, connected_pipe).await {
                    tracing::debug!(error = %e, "Named pipe connection error");
                }
            });
        }
    });
    Ok(handle)
}

#[cfg(unix)]
async fn handle_unix_connection(
    daemon: Arc<Daemon>,
    stream: tokio::net::UnixStream,
) -> Result<()> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        handle_unix_connection_inner(daemon, stream),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!("IPC connection timed out");
            Ok(())
        }
    }
}

#[cfg(unix)]
async fn handle_unix_connection_inner(
    daemon: Arc<Daemon>,
    stream: tokio::net::UnixStream,
) -> Result<()> {
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

    tracing::debug!(msg_type = ?std::mem::discriminant(&msg), "IPC message received");

    let response = daemon.handle_message(msg).await;

    let resp_bytes = serde_json::to_vec(&response)?;
    let resp_len = (resp_bytes.len() as u32).to_le_bytes();
    writer.write_all(&resp_len).await?;
    writer.write_all(&resp_bytes).await?;
    writer.flush().await?;

    Ok(())
}

#[cfg(windows)]
async fn handle_windows_pipe_async(
    daemon: Arc<Daemon>,
    pipe: tokio::net::windows::named_pipe::NamedPipeServer,
) -> Result<()> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        handle_windows_pipe_async_inner(daemon, pipe),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!("IPC connection timed out (Windows pipe)");
            Ok(())
        }
    }
}

#[cfg(windows)]
async fn handle_windows_pipe_async_inner(
    daemon: Arc<Daemon>,
    mut pipe: tokio::net::windows::named_pipe::NamedPipeServer,
) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut len_buf = [0u8; 4];
    pipe.read_exact(&mut len_buf).await?;
    let msg_len = u32::from_le_bytes(len_buf) as usize;

    if msg_len > 16 * 1024 * 1024 {
        anyhow::bail!("IPC message too large: {} bytes", msg_len);
    }

    let mut msg_buf = vec![0u8; msg_len];
    pipe.read_exact(&mut msg_buf).await?;
    let msg: IpcMessage = serde_json::from_slice(&msg_buf)?;

    tracing::debug!(msg_type = ?std::mem::discriminant(&msg), "IPC message received (Windows pipe)");

    let response = daemon.handle_message(msg).await;

    let resp_bytes = serde_json::to_vec(&response)?;
    let resp_len = (resp_bytes.len() as u32).to_le_bytes();
    pipe.write_all(&resp_len).await?;
    pipe.write_all(&resp_bytes).await?;
    pipe.flush().await?;

    // Disconnect so the pipe instance is properly cleaned up
    pipe.disconnect()?;

    Ok(())
}

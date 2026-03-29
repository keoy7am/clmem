mod protocol;
pub mod server;
pub use protocol::{IpcMessage, IpcResponse};

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Get the default IPC path for this platform.
///
/// On Unix this resolves to `<runtime_dir>/clmem.sock` via the Platform
/// trait, then validates that the directory is owned by the current user
/// and is not world-writable.  On Windows the path is the fixed named
/// pipe `\\.\pipe\clmem`, which is not subject to filesystem hijacking.
pub fn default_ipc_path() -> PathBuf {
    #[cfg(unix)]
    {
        let dir = crate::platform::create_platform().runtime_dir();
        validate_runtime_dir_unix(&dir);
        dir.join("clmem.sock")
    }
    #[cfg(windows)]
    {
        PathBuf::from(r"\\.\pipe\clmem")
    }
}

/// On Unix, warn if the runtime directory is not owned by us or is
/// world-writable.  We log warnings rather than hard-failing so the
/// daemon can still start on unusual setups.
#[cfg(unix)]
fn validate_runtime_dir_unix(dir: &Path) {
    use std::os::unix::fs::MetadataExt;

    let meta = match std::fs::metadata(dir) {
        Ok(m) => m,
        Err(_) => return, // directory may not exist yet; daemon will create it
    };

    let uid = unsafe { libc::getuid() };
    if meta.uid() != uid {
        tracing::warn!(
            dir = %dir.display(),
            owner = meta.uid(),
            expected = uid,
            "IPC runtime directory is not owned by current user"
        );
    }

    // Check world-writable bit (o+w = 0o002)
    if meta.mode() & 0o002 != 0 {
        tracing::warn!(
            dir = %dir.display(),
            mode = format!("{:o}", meta.mode()),
            "IPC runtime directory is world-writable"
        );
    }
}

/// Send a request to the daemon and receive a response (blocking).
///
/// Uses Unix Domain Sockets on Linux/macOS and Named Pipes on Windows.
/// The wire protocol is length-prefixed JSON:
///   [4 bytes LE u32 length][JSON payload]
pub fn send_request(path: &Path, msg: &IpcMessage) -> Result<IpcResponse> {
    let serialized = serde_json::to_vec(msg)?;

    #[cfg(unix)]
    {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;

        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
        let len = (serialized.len() as u32).to_le_bytes();
        stream.write_all(&len)?;
        stream.write_all(&serialized)?;
        stream.flush()?;

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let resp_len = u32::from_le_bytes(len_buf) as usize;
        if resp_len > 16 * 1024 * 1024 {
            anyhow::bail!("IPC response too large: {} bytes", resp_len);
        }
        let mut resp_buf = vec![0u8; resp_len];
        stream.read_exact(&mut resp_buf)?;
        Ok(serde_json::from_slice(&resp_buf)?)
    }

    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::io::{Read, Write};
        use std::sync::mpsc;
        use std::time::Duration;

        let path = path.to_owned();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> Result<IpcResponse> {
                let mut pipe = OpenOptions::new().read(true).write(true).open(&path)?;
                let len = (serialized.len() as u32).to_le_bytes();
                pipe.write_all(&len)?;
                pipe.write_all(&serialized)?;
                pipe.flush()?;

                let mut len_buf = [0u8; 4];
                pipe.read_exact(&mut len_buf)?;
                let resp_len = u32::from_le_bytes(len_buf) as usize;
                if resp_len > 16 * 1024 * 1024 {
                    anyhow::bail!("IPC response too large: {} bytes", resp_len);
                }
                let mut resp_buf = vec![0u8; resp_len];
                pipe.read_exact(&mut resp_buf)?;
                Ok(serde_json::from_slice(&resp_buf)?)
            })();
            let _ = tx.send(result);
        });
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => result,
            Err(_) => anyhow::bail!("IPC request timed out"),
        }
    }
}

/// Check if the daemon is running by attempting a Ping.
pub fn is_daemon_running(path: &Path) -> bool {
    send_request(path, &IpcMessage::Ping).is_ok()
}

/// Remove the IPC socket file on shutdown (Unix only; no-op on Windows).
pub fn remove_ipc_socket(ipc_path: &Path) {
    #[cfg(unix)]
    if ipc_path.exists() {
        if let Err(e) = std::fs::remove_file(ipc_path) {
            tracing::warn!(error = %e, "Failed to remove IPC socket");
        }
    }

    #[cfg(windows)]
    let _ = ipc_path;
}

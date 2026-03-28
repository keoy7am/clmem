mod protocol;
pub use protocol::{IpcMessage, IpcResponse};

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Get the default IPC path for this platform.
pub fn default_ipc_path() -> PathBuf {
    #[cfg(unix)]
    {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(runtime_dir).join("clmem.sock")
    }
    #[cfg(windows)]
    {
        PathBuf::from(r"\\.\pipe\clmem")
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

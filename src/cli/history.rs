use anyhow::Result;

use crate::ipc::{self, IpcMessage, IpcResponse};

use super::format_bytes;

/// Run the `clmem history` command.
///
/// Requires a running daemon (history is stored in the daemon's ring buffer).
/// Displays the last N memory snapshots as a table or CSV.
pub fn run(count: usize, csv: bool) -> Result<()> {
    let ipc_path = ipc::default_ipc_path();

    if !ipc::is_daemon_running(&ipc_path) {
        anyhow::bail!("Daemon is not running. Start it with `clmem daemon` to collect history.");
    }

    let msg = IpcMessage::GetHistory { last_n: count };
    let history = match ipc::send_request(&ipc_path, &msg)? {
        IpcResponse::History(snapshots) => snapshots,
        IpcResponse::Error(e) => anyhow::bail!("Daemon error: {}", e),
        _ => anyhow::bail!("Unexpected response from daemon"),
    };

    if history.is_empty() {
        println!("No history entries yet.");
        return Ok(());
    }

    if csv {
        print_csv(&history);
    } else {
        print_table(&history);
    }

    Ok(())
}

/// Print history as a formatted table.
fn print_table(snapshots: &[crate::models::MemorySnapshot]) {
    println!(
        "{:<24} {:<8} {:<12} {:<12} {:<8}",
        "TIMESTAMP", "PROCS", "TOTAL RSS", "TOTAL VMS", "ORPHANS"
    );
    println!("{}", "-".repeat(66));

    for snap in snapshots {
        let ts = snap.timestamp.format("%Y-%m-%d %H:%M:%S");
        println!(
            "{:<24} {:<8} {:<12} {:<12} {:<8}",
            ts,
            snap.claude_process_count,
            format_bytes(snap.total_rss),
            format_bytes(snap.total_vms),
            snap.orphan_count,
        );
    }

    println!("\n{} entries shown.", snapshots.len());
}

/// Print history as CSV to stdout.
fn print_csv(snapshots: &[crate::models::MemorySnapshot]) {
    println!(
        "timestamp,process_count,total_rss_bytes,total_vms_bytes,total_swap_bytes,orphan_count"
    );
    for snap in snapshots {
        println!(
            "{},{},{},{},{},{}",
            snap.timestamp.to_rfc3339(),
            snap.claude_process_count,
            snap.total_rss,
            snap.total_vms,
            snap.total_swap,
            snap.orphan_count,
        );
    }
}

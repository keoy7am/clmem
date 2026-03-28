use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;

use crate::ipc::{self, IpcMessage, IpcResponse};
use crate::models::MemorySnapshot;
use crate::platform::create_platform;

use super::format_bytes;

/// Run the `clmem report` command.
///
/// Generates a Markdown diagnostic report including system info,
/// current snapshot, process details, and (if daemon running)
/// history summary and recent events.
pub fn run(output: Option<PathBuf>) -> Result<()> {
    let report = generate_report()?;

    match output {
        Some(path) => {
            std::fs::write(&path, &report)?;
            println!("Report written to {}", path.display());
        }
        None => {
            print!("{}", report);
        }
    }

    Ok(())
}

/// Build the complete diagnostic report as a Markdown string.
fn generate_report() -> Result<String> {
    let mut out = String::with_capacity(4096);
    let platform = create_platform();
    let snapshot = platform.take_snapshot()?;
    let now = Utc::now();

    // Title
    out.push_str("# clmem Diagnostic Report\n\n");
    out.push_str(&format!(
        "Generated: {}\n\n",
        now.format("%Y-%m-%d %H:%M:%S UTC")
    ));

    // System info
    out.push_str("## System Information\n\n");
    out.push_str(&format!("- **Platform**: {}\n", platform.name()));
    out.push_str(&format!(
        "- **Total Memory**: {}\n",
        format_bytes(platform.system_total_memory())
    ));
    out.push_str(&format!(
        "- **Available Memory**: {}\n",
        format_bytes(platform.system_available_memory())
    ));
    out.push('\n');

    // Current snapshot summary
    out.push_str("## Current Snapshot\n\n");
    out.push_str(&format!(
        "- **Claude Processes**: {}\n",
        snapshot.claude_process_count
    ));
    out.push_str(&format!(
        "- **Total RSS**: {}\n",
        format_bytes(snapshot.total_rss)
    ));
    out.push_str(&format!(
        "- **Total VMS**: {}\n",
        format_bytes(snapshot.total_vms)
    ));
    if snapshot.total_swap > 0 {
        out.push_str(&format!(
            "- **Total Swap**: {}\n",
            format_bytes(snapshot.total_swap)
        ));
    }
    if snapshot.total_committed > 0 {
        out.push_str(&format!(
            "- **Total Committed**: {}\n",
            format_bytes(snapshot.total_committed)
        ));
    }
    out.push_str(&format!("- **Orphans**: {}\n", snapshot.orphan_count));
    out.push('\n');

    // Process detail table
    report_process_table(&mut out, &snapshot);

    // Daemon info (if running)
    report_daemon_info(&mut out);

    Ok(out)
}

/// Append the per-process table to the report.
fn report_process_table(out: &mut String, snapshot: &MemorySnapshot) {
    out.push_str("## Process Details\n\n");

    if snapshot.processes.is_empty() {
        out.push_str("No Claude Code processes found.\n\n");
        return;
    }

    out.push_str("| PID | PPID | Name | State | RSS | VMS | TTY | IPC |\n");
    out.push_str("|-----|------|------|-------|-----|-----|-----|-----|\n");

    for proc in &snapshot.processes {
        let ppid = proc
            .parent_pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            proc.pid,
            ppid,
            proc.name,
            proc.state,
            format_bytes(proc.memory.rss_bytes),
            format_bytes(proc.memory.vms_bytes),
            if proc.has_tty { "yes" } else { "no" },
            if proc.has_ipc { "yes" } else { "no" },
        ));
    }
    out.push('\n');
}

/// Append daemon status and history summary if the daemon is running.
fn report_daemon_info(out: &mut String) {
    let ipc_path = ipc::default_ipc_path();

    if !ipc::is_daemon_running(&ipc_path) {
        out.push_str("## Daemon\n\nDaemon is not running.\n\n");
        return;
    }

    out.push_str("## Daemon Status\n\n");

    // Status
    if let Ok(IpcResponse::Status {
        uptime_secs,
        monitoring_count,
        orphan_count,
        total_rss,
    }) = ipc::send_request(&ipc_path, &IpcMessage::GetStatus)
    {
        let h = uptime_secs / 3600;
        let m = (uptime_secs % 3600) / 60;
        let s = uptime_secs % 60;
        out.push_str(&format!("- **Uptime**: {h}h {m}m {s}s\n"));
        out.push_str(&format!(
            "- **Monitoring**: {} processes\n",
            monitoring_count
        ));
        out.push_str(&format!("- **Orphans**: {}\n", orphan_count));
        out.push_str(&format!("- **Total RSS**: {}\n", format_bytes(total_rss)));
        out.push('\n');
    }

    // History summary (last 10 entries)
    if let Ok(IpcResponse::History(history)) =
        ipc::send_request(&ipc_path, &IpcMessage::GetHistory { last_n: 10 })
    {
        if !history.is_empty() {
            out.push_str("### Recent History (last 10 snapshots)\n\n");
            out.push_str("| Timestamp | Processes | RSS | VMS | Orphans |\n");
            out.push_str("|-----------|-----------|-----|-----|--------|\n");
            for snap in &history {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    snap.timestamp.format("%H:%M:%S"),
                    snap.claude_process_count,
                    format_bytes(snap.total_rss),
                    format_bytes(snap.total_vms),
                    snap.orphan_count,
                ));
            }
            out.push('\n');
        }
    }

    // Recent events (last 20)
    if let Ok(IpcResponse::Events(events)) =
        ipc::send_request(&ipc_path, &IpcMessage::GetEvents { last_n: 20 })
    {
        if !events.is_empty() {
            out.push_str("### Recent Events\n\n");
            for event in &events {
                let ts = event.timestamp.format("%H:%M:%S");
                out.push_str(&format!("- `[{ts}]` {:?}\n", event.kind));
            }
            out.push('\n');
        }
    }
}

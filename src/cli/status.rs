use anyhow::Result;

use crate::ipc::{self, IpcMessage, IpcResponse};
use crate::platform::create_platform;

use super::format_bytes;

/// Run the `clmem status` command.
///
/// Takes a one-shot snapshot using the platform directly (no daemon required).
/// If the daemon is running, also displays daemon status info.
pub fn run(json: bool) -> Result<()> {
    let platform = create_platform();
    let snapshot = platform.take_snapshot()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }

    // Header
    println!("clmem - Claude Code Memory Monitor");
    println!("{}", "=".repeat(60));
    println!();

    // System memory
    println!("System Memory:");
    println!(
        "  Total:     {}",
        format_bytes(snapshot.system_total_memory)
    );
    println!("  Used:      {}", format_bytes(snapshot.system_used_memory));
    println!(
        "  Available: {}",
        format_bytes(snapshot.system_available_memory)
    );
    println!();

    // Claude summary
    println!("Claude Code Processes: {}", snapshot.claude_process_count);
    println!("  Total RSS:       {}", format_bytes(snapshot.total_rss));
    println!("  Total VMS:       {}", format_bytes(snapshot.total_vms));
    if snapshot.total_swap > 0 {
        println!("  Total Swap:      {}", format_bytes(snapshot.total_swap));
    }
    if snapshot.total_committed > 0 {
        println!(
            "  Total Committed: {}",
            format_bytes(snapshot.total_committed)
        );
    }
    println!("  Orphans:         {}", snapshot.orphan_count);
    println!();

    // Per-process table
    if !snapshot.processes.is_empty() {
        println!(
            "{:<8} {:<8} {:<20} {:<10} {:<10} {:<10}",
            "PID", "PPID", "NAME", "STATE", "RSS", "VMS"
        );
        println!("{}", "-".repeat(66));
        for proc in &snapshot.processes {
            let ppid = proc
                .parent_pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_string());
            let name = if proc.name.len() > 18 {
                format!("{}...", &proc.name[..17])
            } else {
                proc.name.clone()
            };
            println!(
                "{:<8} {:<8} {:<20} {:<10} {:<10} {:<10}",
                proc.pid,
                ppid,
                name,
                proc.state,
                format_bytes(proc.memory.rss_bytes),
                format_bytes(proc.memory.vms_bytes),
            );
        }
    } else {
        println!("No Claude Code processes found.");
    }

    // Daemon status (best-effort)
    let ipc_path = ipc::default_ipc_path();
    if ipc::is_daemon_running(&ipc_path) {
        if let Ok(IpcResponse::Status {
            uptime_secs,
            monitoring_count,
            orphan_count,
            total_rss,
        }) = ipc::send_request(&ipc_path, &IpcMessage::GetStatus)
        {
            println!();
            println!("Daemon Status:");
            let h = uptime_secs / 3600;
            let m = (uptime_secs % 3600) / 60;
            let s = uptime_secs % 60;
            println!("  Uptime:     {h}h {m}m {s}s");
            println!("  Monitoring: {} processes", monitoring_count);
            println!("  Orphans:    {}", orphan_count);
            println!("  Total RSS:  {}", format_bytes(total_rss));
        }
    } else {
        println!();
        println!("Daemon: not running (start with `clmem daemon`)");
    }

    Ok(())
}

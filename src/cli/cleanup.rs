use anyhow::Result;

use crate::ipc::{self, IpcMessage, IpcResponse};
use crate::models::ProcessState;
use crate::platform::create_platform;

use crate::util::format_bytes;

/// Run the `clmem cleanup` command.
///
/// Safety rules:
/// - Default: only clean ORPHAN processes
/// - --force: also clean IDLE processes
/// - --all: clean ALL Claude processes (requires "yes" confirmation)
/// - --pids: clean specific PIDs regardless of state
/// - --dry-run: show what would be cleaned without doing it
///
/// If the daemon is running, delegates via IPC. Otherwise uses platform directly.
pub fn run(dry_run: bool, force: bool, all: bool, pids: Option<Vec<u32>>) -> Result<()> {
    // --all requires explicit confirmation
    if all && !dry_run {
        println!("WARNING: This will terminate ALL Claude Code processes.");
        println!("Type 'yes' to confirm: ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim() != "yes" {
            println!("Aborted.");
            return Ok(());
        }
    }

    let ipc_path = ipc::default_ipc_path();

    // If specific PIDs are given, handle that path
    if let Some(ref target_pids) = pids {
        return cleanup_specific_pids(target_pids, dry_run, &ipc_path);
    }

    // Try daemon first, fall back to direct platform access
    if ipc::is_daemon_running(&ipc_path) {
        return cleanup_via_daemon(dry_run, force, all, &ipc_path);
    }

    // No daemon -- use platform directly
    cleanup_direct(dry_run, force, all)
}

/// Clean up specific PIDs via daemon or platform.
fn cleanup_specific_pids(pids: &[u32], dry_run: bool, ipc_path: &std::path::Path) -> Result<()> {
    if dry_run {
        println!("Dry run -- would clean PIDs: {:?}", pids);
        return Ok(());
    }

    if ipc::is_daemon_running(ipc_path) {
        let msg = IpcMessage::Cleanup {
            pids: pids.to_vec(),
            force: true,
        };
        match ipc::send_request(ipc_path, &msg)? {
            IpcResponse::CleanupResult { cleaned, failed } => {
                println!("Cleanup complete: {} cleaned, {} failed", cleaned, failed);
            }
            IpcResponse::Error(e) => {
                anyhow::bail!("Daemon cleanup error: {}", e);
            }
            other => {
                tracing::warn!(?other, "Unexpected daemon response");
            }
        }
        return Ok(());
    }

    // Direct cleanup
    let platform = create_platform();
    let mut cleaned: u32 = 0;
    let mut failed: u32 = 0;
    for &pid in pids {
        if !platform.is_process_alive(pid) {
            println!("  PID {} -- already dead, skipping", pid);
            continue;
        }
        println!("  Terminating PID {}...", pid);
        match platform.kill_process_tree(pid) {
            Ok(()) => {
                let _ = platform.release_memory(pid);
                cleaned += 1;
            }
            Err(e) => {
                println!("  PID {} -- failed: {}", pid, e);
                failed += 1;
            }
        }
    }
    println!("Cleanup complete: {} cleaned, {} failed", cleaned, failed);
    Ok(())
}

/// Delegate cleanup to the running daemon via IPC.
fn cleanup_via_daemon(
    dry_run: bool,
    force: bool,
    all: bool,
    ipc_path: &std::path::Path,
) -> Result<()> {
    // Get current process list from daemon to figure out which PIDs to clean
    let process_list = match ipc::send_request(ipc_path, &IpcMessage::GetProcessList)? {
        IpcResponse::ProcessList(procs) => procs,
        IpcResponse::Error(e) => anyhow::bail!("Daemon error: {}", e),
        _ => anyhow::bail!("Unexpected response from daemon"),
    };

    let targets = select_targets(&process_list, force, all);

    if targets.is_empty() {
        println!("No processes eligible for cleanup.");
        return Ok(());
    }

    print_cleanup_plan(&process_list, &targets);

    if dry_run {
        println!("\nDry run -- no processes were terminated.");
        return Ok(());
    }

    let msg = IpcMessage::Cleanup {
        pids: targets,
        force,
    };
    match ipc::send_request(ipc_path, &msg)? {
        IpcResponse::CleanupResult { cleaned, failed } => {
            println!("\nCleanup complete: {} cleaned, {} failed", cleaned, failed);
        }
        IpcResponse::Error(e) => {
            anyhow::bail!("Daemon cleanup error: {}", e);
        }
        other => {
            tracing::warn!(?other, "Unexpected daemon response");
        }
    }

    Ok(())
}

/// Perform cleanup directly using the platform (no daemon).
fn cleanup_direct(dry_run: bool, force: bool, all: bool) -> Result<()> {
    println!("Daemon not running -- performing direct cleanup.\n");

    let platform = create_platform();
    let processes = platform.list_claude_processes()?;

    let targets = select_targets(&processes, force, all);

    if targets.is_empty() {
        println!("No processes eligible for cleanup.");
        return Ok(());
    }

    print_cleanup_plan(&processes, &targets);

    if dry_run {
        println!("\nDry run -- no processes were terminated.");
        return Ok(());
    }

    let mut cleaned: u32 = 0;
    let mut failed: u32 = 0;

    for pid in &targets {
        print!("  Terminating PID {}...", pid);
        match platform.kill_process_tree(*pid) {
            Ok(()) => {
                let _ = platform.release_memory(*pid);
                println!(" OK");
                cleaned += 1;
            }
            Err(e) => {
                println!(" FAILED: {}", e);
                failed += 1;
            }
        }
    }

    println!("\nCleanup complete: {} cleaned, {} failed", cleaned, failed);
    Ok(())
}

/// Select which PIDs to target based on flags and process states.
fn select_targets(processes: &[crate::models::ProcessInfo], force: bool, all: bool) -> Vec<u32> {
    processes
        .iter()
        .filter(|p| {
            if all {
                true
            } else if force {
                matches!(p.state, ProcessState::Orphan | ProcessState::Idle)
            } else {
                p.state == ProcessState::Orphan
            }
        })
        .map(|p| p.pid)
        .collect()
}

/// Print a table of processes that will be cleaned.
fn print_cleanup_plan(processes: &[crate::models::ProcessInfo], targets: &[u32]) {
    println!("Processes to clean ({}):\n", targets.len());
    println!("{:<8} {:<10} {:<20} {:<10}", "PID", "STATE", "NAME", "RSS");
    println!("{}", "-".repeat(50));
    for proc in processes {
        if targets.contains(&proc.pid) {
            let name = if proc.name.len() > 18 {
                format!("{}...", &proc.name[..17])
            } else {
                proc.name.clone()
            };
            println!(
                "{:<8} {:<10} {:<20} {:<10}",
                proc.pid,
                proc.state,
                name,
                format_bytes(proc.memory.rss_bytes),
            );
        }
    }
}

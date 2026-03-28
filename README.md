# clmem

A cross-platform memory monitoring and management tool for Claude Code CLI.

Claude Code CLI (Node.js-based) suffers from memory leaks, orphaned processes, and unreleased virtual/committed memory — especially on Windows. `clmem` watches, detects, and cleans up these issues automatically.

## Features

- **Real-time monitoring** — Track RSS, virtual memory, swap, and committed memory for all Claude Code processes
- **Leak detection** — Identify abnormal memory growth patterns and alert before they become critical
- **Orphan cleanup** — Detect and safely terminate processes that survive after Claude Code exits
- **Windows committed memory** — Handle V8 engine memory retention, orphaned handles, and named pipe leaks
- **TUI dashboard** — htop-style terminal interface with live charts and process management
- **Diagnostic reports** — Export memory history and generate diagnostic reports for debugging

## Installation

```bash
# From source
cargo install --path .

# Or build manually
cargo build --release
# Binary at target/release/clmem
```

## Quick Start

```bash
# One-shot status check (no daemon needed)
clmem status

# Start background monitoring
clmem daemon start

# Open real-time dashboard
clmem tui

# Clean up orphaned processes
clmem cleanup --dry-run    # Preview first
clmem cleanup              # Execute cleanup
```

## Usage

### Daemon

```bash
clmem daemon start         # Start background monitor
clmem daemon stop          # Stop daemon
clmem daemon status        # Check daemon status
```

### TUI Dashboard

```bash
clmem tui                  # Open terminal dashboard
```

| Key | Action |
|-----|--------|
| `↑↓` | Navigate process list |
| `Enter` | Process detail view |
| `K` | Kill selected process |
| `C` | Cleanup all orphans |
| `R` | Refresh |
| `Q` | Quit |

### Commands

```bash
clmem status               # Quick snapshot
clmem status --json        # JSON output for scripting

clmem cleanup              # Clean orphaned processes
clmem cleanup --dry-run    # Preview without executing
clmem cleanup --force      # Include IDLE processes
clmem cleanup --all        # All Claude processes (requires confirmation)

clmem history              # Memory usage history
clmem history --last 1h    # Last hour
clmem history --export     # Export to CSV

clmem report               # Diagnostic report to stdout
clmem report --output report.md

clmem config --show        # Show current config
clmem config --edit        # Open config in editor
clmem config --reset       # Reset to defaults
```

## Configuration

Default config location:
- **Windows**: `%APPDATA%\clmem\clmem.toml`
- **macOS**: `~/Library/Application Support/clmem/clmem.toml`
- **Linux**: `~/.config/clmem/clmem.toml`

See [`clmem.toml.example`](clmem.toml.example) for all available options.

### Key Settings

```toml
[thresholds]
vms_leak_rate = "500MB/10min"   # Alert when VMS grows faster than this
rss_max = "2GB"                  # Alert when RSS exceeds this
orphan_timeout = "15min"         # Time before STALE -> ORPHAN

[cleanup]
grace_period = "30s"             # Wait after main process exits
auto_cleanup_orphans = true      # Auto-clean orphaned processes
strategy = "terminate_tree"      # Kill entire process tree
```

## Process Safety

`clmem` classifies every Claude Code process into safety zones:

| State | Indicator | Auto-cleanup | Description |
|-------|-----------|-------------|-------------|
| ACTIVE | `🟢` | Never | Has active TTY/stdin connection |
| IDLE | `🟡` | Never | No activity for <5min, still connected |
| STALE | `🟠` | After timeout | No activity, parent alive, waiting |
| ORPHAN | `🔴` | Yes | Parent dead, no IPC connection |

**Rule**: Active and idle processes are never automatically cleaned. Only `--force` allows manual intervention on idle processes.

## How It Works

1. **Scanner** polls the OS process table every second, filtering for Claude Code related processes (`node`, `claude`, MCP servers)
2. **Profiler** records memory snapshots (RSS, VMS, Swap) into a ring buffer with 1-hour retention
3. **Analyzer** evaluates memory trends every 10 seconds, detecting leaks by comparing VMS growth rate against thresholds
4. **Reaper** safely terminates orphaned processes — `SIGTERM` first, then `SIGKILL` after timeout (Windows: `TerminateProcess` on entire process tree followed by `EmptyWorkingSet`)

## Platform Support

| Feature | Windows | Linux | macOS |
|---------|---------|-------|-------|
| Process monitoring | Win32 API | `/proc` | `libproc` |
| Memory profiling | `GetProcessMemoryInfo` | `/proc/[pid]/smaps` | `mach_vm_region` |
| Committed memory tracking | Native | N/A | N/A |
| Handle leak detection | `NtQuerySystemInformation` | `/proc/[pid]/fd` | `proc_pidinfo` |
| IPC | Named Pipes | Unix Socket | Unix Socket |

## Tech Stack

- **Language**: Rust
- **Async Runtime**: tokio
- **TUI**: ratatui + crossterm
- **CLI**: clap (derive)
- **System Info**: sysinfo

## License

MIT

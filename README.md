# clmem

A cross-platform memory monitoring and management tool for Claude Code CLI.

Claude Code CLI (Node.js-based) suffers from memory leaks, orphaned processes, and unreleased virtual/committed memory — especially on Windows. `clmem` watches, detects, and cleans up these issues automatically.

## Features

- **Real-time monitoring** — Track RSS, virtual memory, swap, and committed memory for all Claude Code processes
- **Leak detection** — Identify abnormal memory growth patterns and alert before they become critical
- **Orphan cleanup** — Detect and safely terminate processes that survive after Claude Code exits
- **Windows committed memory** — Handle V8 engine memory retention and orphaned named pipe handles
- **TUI dashboard** — htop-style terminal interface with live charts and process management
- **Diagnostic reports** — Export memory history and generate diagnostic reports for debugging

## Installation

```bash
# Build from source (requires Rust toolchain)
cargo build --release

# Binary at target/release/clmem (or clmem.exe on Windows)

# Or install directly
cargo install --path .
```

## Quick Start

```bash
# One-shot status check (no daemon needed)
clmem status

# Start background monitoring daemon
clmem daemon

# Open real-time TUI dashboard
clmem tui

# Clean up orphaned processes
clmem cleanup --dry-run    # Preview first
clmem cleanup              # Execute cleanup
```

## Commands

### `clmem status` — Quick Snapshot

Works standalone, no daemon required. Scans the process table and shows all Claude Code processes.

```bash
clmem status               # Human-readable table
clmem status --json        # JSON output for scripting
```

### `clmem daemon` — Background Monitor

Starts the monitoring daemon with continuous scanning, leak detection, and optional auto-cleanup.

```bash
clmem daemon               # Run in background
clmem daemon --foreground  # Run in foreground (see logs)
```

### `clmem tui` — Interactive Dashboard

Terminal UI with live memory charts, process list, and alerts. Connects to the daemon via IPC.

```bash
clmem tui
```

| Key | Action |
|-----|--------|
| `Tab` | Cycle between panels |
| `j/k` or `↑/↓` | Navigate process list |
| `K` | Kill selected process |
| `r` | Refresh |
| `1`-`5` | Sort by column |
| `?` | Help |
| `q` / `Esc` | Quit |

### `clmem cleanup` — Process Cleanup

Safety-first cleanup with multiple modes.

```bash
clmem cleanup              # Clean ORPHAN processes only
clmem cleanup --dry-run    # Preview without executing
clmem cleanup --force      # Also clean IDLE processes
clmem cleanup --all        # All Claude processes (requires typing "yes")
clmem cleanup --pids 1234,5678  # Specific PIDs
```

### `clmem history` — Memory History

Requires daemon running. Shows memory snapshots from the ring buffer.

```bash
clmem history              # Last 60 snapshots
clmem history -n 300       # Last 300 snapshots
clmem history --csv        # Export as CSV
```

### `clmem report` — Diagnostic Report

Generates a Markdown diagnostic report with system info, process details, and (if daemon running) history and events.

```bash
clmem report               # Output to stdout
clmem report -o report.md  # Save to file
```

### `clmem config` — Configuration

```bash
clmem config show          # Display current config as TOML
clmem config path          # Show config file location
clmem config edit          # Open in $EDITOR (or notepad on Windows)
clmem config reset         # Reset to defaults
```

## Configuration

Default config location:
- **Windows**: `%APPDATA%\clmem\clmem\clmem.toml`
- **macOS**: `~/Library/Application Support/dev.clmem.clmem/clmem.toml`
- **Linux**: `~/.config/clmem/clmem.toml`

See [`clmem.toml.example`](clmem.toml.example) for all options.

### Key Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `scan_interval_ms` | `1000` | Process scan interval (ms) |
| `history_retention_secs` | `3600` | Ring buffer retention (1 hour) |
| `idle_threshold_secs` | `300` | Idle classification threshold (5 min) |
| `stale_grace_period_secs` | `900` | Wait before STALE downgrade (15 min) |
| `orphan_grace_period_secs` | `30` | Grace period after parent exits |
| `leak_check_interval_secs` | `10` | Leak analysis interval |
| `leak_growth_threshold_bytes_per_sec` | `1048576` | Leak alert threshold (1 MB/s) |
| `auto_cleanup` | `false` | Auto-clean orphans when daemon running |
| `log_level` | `"info"` | Logging verbosity |

## Process Safety Classification

`clmem` classifies every Claude Code process before taking any action:

| State | Condition | Auto-cleanup | `--force` | `--all` |
|-------|-----------|:------------:|:---------:|:-------:|
| **ACTIVE** | Has TTY/stdin | Never | Never | Yes |
| **IDLE** | Inactive < threshold | Never | Yes | Yes |
| **STALE** | Inactive, parent alive | After grace period | Yes | Yes |
| **ORPHAN** | Parent dead, no IPC | Yes | Yes | Yes |

**Rule**: Active processes are never touched without `--all` + confirmation.

## Architecture

```
clmem (single binary)
├── daemon    — Background engine (tokio async)
│   ├── scanner    — Process table polling (1s interval)
│   ├── profiler   — Ring buffer memory snapshots
│   ├── analyzer   — Leak detection via linear regression
│   ├── reaper     — Safe orphan termination
│   └── event bus  — Publish/subscribe for alerts
├── tui       — Terminal dashboard (ratatui + crossterm)
│   ├── dashboard  — Memory gauges and summary stats
│   ├── charts     — Real-time memory trend lines
│   ├── process list — Sortable, color-coded table
│   └── alerts     — Event history with severity colors
├── cli       — Command interface (clap derive)
│   ├── status, cleanup, history, report, config
│   └── daemon/tui launchers
├── platform  — OS abstraction (Platform trait)
│   ├── windows    — sysinfo + Win32 (EmptyWorkingSet)
│   ├── linux      — sysinfo + /proc filesystem
│   └── macos      — sysinfo + libproc
├── ipc       — Daemon <-> CLI/TUI communication
│   └── Length-prefixed JSON over Unix socket / Named pipe
└── models    — Shared data types
    └── ProcessInfo, MemorySnapshot, Event, Config
```

## Platform Support

| Feature | Windows | Linux | macOS |
|---------|---------|-------|-------|
| Process monitoring | sysinfo + Win32 | sysinfo + /proc | sysinfo + libproc |
| Memory profiling | RSS/VMS/committed | RSS/VMS/swap | RSS/VMS |
| TTY detection | Console check | /proc/[pid]/fd/0 | Stub |
| IPC detection | Named pipe scan | /proc/[pid]/fd socket | Stub |
| Process cleanup | Kill tree + EmptyWorkingSet | SIGTERM/SIGKILL | SIGTERM/SIGKILL |
| IPC transport | Named Pipe | Unix Socket | Unix Socket |

## Tech Stack

| Crate | Purpose |
|-------|---------|
| `sysinfo` | Cross-platform process/memory info |
| `ratatui` + `crossterm` | TUI rendering |
| `clap` (derive) | CLI argument parsing |
| `tokio` | Async runtime (daemon) |
| `serde` + `toml` + `serde_json` | Config and IPC serialization |
| `tracing` | Structured logging |
| `anyhow` + `thiserror` | Error handling |
| `chrono` | Timestamps |
| `directories` | Platform config paths |

## License

MIT

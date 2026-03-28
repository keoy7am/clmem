# clmem - Claude Code Memory Monitor & Manager

**Date**: 2026-03-29
**Status**: Approved
**Author**: YAP

---

## 1. Overview

`clmem` is a cross-platform memory monitoring and management tool for Claude Code CLI, built in Rust. It addresses memory leaks, orphaned processes, and Windows committed memory issues that persist after Claude Code sessions end.

### Problem Statement

Claude Code CLI (Node.js-based) exhibits the following memory issues:
- **Post-exit memory retention**: Virtual memory and committed memory remain occupied after CLI closes
- **Long session memory bloat**: RSS grows continuously during extended sessions
- **Orphaned processes**: MCP servers, subagents, and child processes survive parent exit
- **Windows committed memory**: V8 engine pre-commits large virtual memory blocks that are not released
- **Handle leaks**: Named pipes, memory-mapped files, and other handles remain open

### Solution

A single Rust binary (`clmem`) providing three operational modes:
1. **Daemon** - Background monitoring engine with leak detection and automatic cleanup
2. **TUI** - Real-time terminal dashboard (htop-style)
3. **CLI** - Quick commands for status checks, cleanup, and diagnostics

## 2. Architecture

### 2.1 Component Overview

```
clmem (single binary)
├── daemon       - Background monitoring engine
│   ├── process_scanner    - Detect Claude Code processes (node, claude, mcp servers)
│   ├── memory_profiler    - Track RSS / VMS / Heap trends in ring buffer
│   ├── leak_detector      - Compare memory growth patterns to detect leaks
│   ├── orphan_reaper      - Detect and cleanup orphaned processes after CLI exit
│   └── event_bus          - Internal event system (alerts, state changes)
│
├── tui          - Real-time terminal dashboard
│   ├── dashboard          - Memory overview (live charts, process list)
│   ├── process_detail     - Single process deep inspection
│   ├── alerts_panel       - Alert history and real-time notifications
│   └── controls           - Manual cleanup triggers, threshold settings
│
├── cli          - Command operations
│   ├── status             - Quick snapshot view
│   ├── cleanup            - Manual cleanup of orphaned processes
│   ├── history            - Memory usage history
│   ├── config             - Configuration management
│   └── report             - Diagnostic report generation
│
└── ipc          - Inter-process communication layer
    ├── unix_socket        - Linux/macOS
    └── named_pipe         - Windows
```

### 2.2 Daemon Core Flow

```
                    ┌─────────────────────────────────┐
                    │         clmem daemon             │
                    │                                  │
  ┌──────────┐     │  ┌──────────┐    ┌────────────┐  │
  │ OS API   │────>│  │ Scanner  │───>│ Profiler   │  │
  │ sysinfo  │     │  │ (1s)     │    │ (snapshots)│  │
  └──────────┘     │  └──────────┘    └─────┬──────┘  │
                   │                        │         │
                   │                  ┌─────▼──────┐  │
                   │                  │ Analyzer   │  │
                   │                  │ - leak det │  │
                   │                  │ - trends   │  │
                   │                  │ - anomaly  │  │
                   │                  └─────┬──────┘  │
                   │                        │         │
                   │  ┌──────────┐    ┌─────▼──────┐  │
                   │  │ Reaper   │<───│ Event Bus  │  │
                   │  │ (cleanup)│    │ (dispatch) │  │
                   │  └──────────┘    └─────┬──────┘  │
                   │                        │         │
                   └────────────────────────┼─────────┘
                                            │ IPC
                              ┌─────────────┼──────────────┐
                              │             │              │
                        ┌─────▼───┐   ┌─────▼───┐   ┌─────▼───┐
                        │  TUI    │   │  CLI    │   │  Log    │
                        └─────────┘   └─────────┘   └─────────┘
```

**Workflow**:
1. **Scanner** polls system process table every 1s, filtering Claude Code processes by name/cmdline
2. **Profiler** records memory snapshots (RSS, VMS, Swap) into ring buffer (1h retention)
3. **Analyzer** evaluates trends every 10s - flags VMS growth >500MB/10min as potential leak
4. **Event Bus** dispatches alerts to TUI/CLI/Log subscribers
5. **Reaper** safely terminates orphaned processes (SIGTERM first, SIGKILL on timeout; TerminateProcess on Windows)

### 2.3 Process Safety Classification

```
Process detected
     │
     ▼
Has active TTY/stdin? ──yes──> ACTIVE (green)  -> Monitor only, never intervene
     │ no
     ▼
Last API activity < 5min? ──yes──> IDLE (yellow) -> Monitor + soft alert
     │ no
     ▼
Parent dead? No IPC? ──yes──> ORPHAN (red) -> Safe to cleanup
     │ no
     ▼
STALE (orange) -> Wait + observe, downgrade to ORPHAN after 15min timeout
```

**Core Safety Rule**: ACTIVE and IDLE processes are NEVER auto-cleaned. Only `clmem cleanup --force` allows manual intervention on IDLE processes.

### 2.4 Windows Committed Memory Handling

Windows-specific issue: Node.js V8 engine pre-commits large virtual memory blocks. After CLI exit, residual:
- Child processes (MCP servers, spawned agents)
- Unreleased V8 isolate heaps
- Unclosed named pipe handles
- Residual memory-mapped files

**Release flow**:
1. Detect Claude Code main process exit
2. Wait `grace_period` (default 30s) for children to exit naturally
3. Scan residual process tree + orphaned handles
4. `TerminateProcess` entire process tree
5. Call `EmptyWorkingSet` API to release working set
6. Log committed memory delta to history

## 3. Cross-Platform Abstraction

### 3.1 Platform Trait

```rust
pub trait Platform {
    fn list_processes(&self) -> Vec<ProcessInfo>;
    fn get_memory_info(&self, pid: u32) -> MemorySnapshot;
    fn is_process_active(&self, pid: u32) -> bool;
    fn terminate_process_tree(&self, pid: u32) -> Result<()>;
    fn release_working_set(&self, pid: u32) -> Result<()>;
    fn scan_orphaned_handles(&self, pid: u32) -> Vec<HandleInfo>;
}
```

### 3.2 Platform Implementations

| Capability | Windows | Linux | macOS |
|---|---|---|---|
| Process listing | Win32 `CreateToolhelp32Snapshot` | `/proc` filesystem | `libproc` |
| Memory info | `GetProcessMemoryInfo` | `/proc/[pid]/smaps` | `mach_vm_region` |
| Handle scanning | `NtQuerySystemInformation` | `/proc/[pid]/fd` | `proc_pidinfo` |
| Process termination | `TerminateProcess` | `kill()` | `kill()` |
| Working set release | `EmptyWorkingSet` | `madvise(DONTNEED)` | `madvise(FREE)` |
| IPC | Named Pipes | Unix Domain Socket | Unix Domain Socket |

## 4. Configuration

### 4.1 Config File (`clmem.toml`)

```toml
[daemon]
scan_interval = "1s"
analysis_interval = "10s"
history_retention = "1h"
log_level = "info"

[thresholds]
vms_leak_rate = "500MB/10min"
rss_max = "2GB"
orphan_timeout = "15min"
idle_timeout = "5min"

[cleanup]
grace_period = "30s"
strategy = "terminate_tree"
auto_cleanup_orphans = true
confirm_before_kill = true

[tui]
refresh_rate = "500ms"
chart_timespan = "30min"
theme = "auto"

[windows]
committed_memory_threshold = "512MB"
scan_handles = true
force_release_strategy = "terminate_tree"

[notifications]
enable = true
method = "terminal"
```

## 5. CLI Commands

```
clmem <command>

Commands:
  daemon start|stop|status    Background monitor management
  tui                         Real-time terminal dashboard
  status [--json]             Quick snapshot (no daemon required)
  cleanup [--dry-run|--force|--all]  Clean orphaned processes
  history [--last <duration>] [--export]  Memory usage history
  report [--output <path>]    Diagnostic report
  config [--edit|--reset|--show]  Configuration management
```

## 6. Key Dependencies

| Crate | Purpose |
|---|---|
| `sysinfo` | Cross-platform process/memory information |
| `ratatui` | TUI rendering |
| `clap` | CLI argument parsing (derive) |
| `tokio` | Async runtime (daemon main loop) |
| `serde` + `toml` | Config serialization |
| `crossterm` | Terminal backend for ratatui |
| `tracing` | Structured logging |
| `directories` | Cross-platform config/data paths |

## 7. Project Structure

```
clmem/
├── Cargo.toml
├── CLAUDE.md
├── README.md
├── LICENSE
├── .gitignore
├── clmem.toml.example
├── docs/plans/
├── src/
│   ├── main.rs
│   ├── cli/
│   │   ├── mod.rs
│   │   ├── status.rs
│   │   ├── cleanup.rs
│   │   ├── history.rs
│   │   ├── report.rs
│   │   └── config.rs
│   ├── daemon/
│   │   ├── mod.rs
│   │   ├── scanner.rs
│   │   ├── profiler.rs
│   │   ├── analyzer.rs
│   │   ├── reaper.rs
│   │   └── event.rs
│   ├── tui/
│   │   ├── mod.rs
│   │   ├── dashboard.rs
│   │   ├── process_list.rs
│   │   ├── charts.rs
│   │   └── alerts.rs
│   ├── platform/
│   │   ├── mod.rs
│   │   ├── windows.rs
│   │   ├── linux.rs
│   │   └── macos.rs
│   ├── ipc/
│   │   ├── mod.rs
│   │   ├── server.rs
│   │   └── client.rs
│   └── models/
│       ├── mod.rs
│       ├── process.rs
│       ├── memory.rs
│       ├── event.rs
│       └── config.rs
└── tests/
    ├── integration/
    │   ├── daemon_test.rs
    │   ├── cleanup_test.rs
    │   └── platform_test.rs
    └── mock/
        └── fake_process.rs
```

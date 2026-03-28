# clmem Multi-Team Parallel Development Plan

**Date**: 2026-03-29
**Orchestration**: `/auto-review:multi-team`
**Total Teams**: 4
**Estimated Phases**: 3

---

## Team Structure

### Team 1: `core-platform`
**Scope**: Platform abstraction layer + data models
**Files**: `src/platform/`, `src/models/`, `src/ipc/`

**Tasks**:
1. Define `Platform` trait in `src/platform/mod.rs`
2. Implement shared data models (`ProcessInfo`, `MemorySnapshot`, `ProcessState`, `Event`, `Config`)
3. Implement `WindowsPlatform` (`src/platform/windows.rs`)
   - Process listing via `CreateToolhelp32Snapshot`
   - `GetProcessMemoryInfo` for memory snapshots
   - `NtQuerySystemInformation` for handle scanning
   - `TerminateProcess` + `EmptyWorkingSet`
4. Implement `LinuxPlatform` (`src/platform/linux.rs`)
   - `/proc` filesystem parsing
   - `/proc/[pid]/smaps` for detailed memory maps
5. Implement `MacosPlatform` (`src/platform/macos.rs`)
   - `libproc` + `mach_vm_region`
6. Implement IPC abstraction (`src/ipc/`)
   - Server/client pattern
   - Unix socket (Linux/macOS) + Named pipe (Windows)
7. Setup `Cargo.toml` with all dependencies

**Deliverables**: Compilable platform layer with unit tests on all 3 OS targets

**Dependencies**: None (this is the foundation)

---

### Team 2: `daemon-engine`
**Scope**: Background monitoring daemon
**Files**: `src/daemon/`

**Tasks**:
1. Daemon main loop (`src/daemon/mod.rs`)
   - tokio runtime setup
   - Signal handling (graceful shutdown)
   - PID file management
2. Process scanner (`src/daemon/scanner.rs`)
   - 1s interval process table polling
   - Claude Code process identification (name + cmdline matching)
   - Process state classification (ACTIVE/IDLE/STALE/ORPHAN)
3. Memory profiler (`src/daemon/profiler.rs`)
   - Ring buffer implementation (1h retention)
   - Memory snapshot recording (RSS, VMS, Swap, committed)
4. Leak analyzer (`src/daemon/analyzer.rs`)
   - Trend analysis (10s interval)
   - VMS growth rate detection
   - Anomaly scoring algorithm
5. Orphan reaper (`src/daemon/reaper.rs`)
   - Safety classification enforcement
   - Graceful termination flow (SIGTERM -> timeout -> SIGKILL)
   - Windows: process tree termination + EmptyWorkingSet
   - Post-exit grace period handling
6. Event bus (`src/daemon/event.rs`)
   - Publish/subscribe pattern
   - Event types: Alert, StateChange, CleanupResult
   - IPC event broadcasting

**Deliverables**: Running daemon that monitors, detects leaks, and safely cleans orphans

**Dependencies**: Blocked by Team 1 (Platform trait + models)

---

### Team 3: `tui-dashboard`
**Scope**: Terminal user interface
**Files**: `src/tui/`

**Tasks**:
1. TUI main loop (`src/tui/mod.rs`)
   - ratatui + crossterm setup
   - Layout management (3-pane: overview, process list, alerts)
   - Keyboard event handling
2. Dashboard panel (`src/tui/dashboard.rs`)
   - Memory overview bars (RSS, VMS, Swap)
   - Summary stats (process count, orphan count, alert count)
3. Memory charts (`src/tui/charts.rs`)
   - Real-time line chart (memory trend over time)
   - Configurable timespan
4. Process list (`src/tui/process_list.rs`)
   - Sortable table (PID, name, RSS, VMS, status, uptime)
   - Color-coded status indicators
   - Process detail view (Enter key)
   - Kill action (K key)
5. Alerts panel (`src/tui/alerts.rs`)
   - Alert history with timestamps
   - Severity levels and color coding

**Deliverables**: Functional TUI dashboard connected to daemon via IPC

**Dependencies**: Blocked by Team 1 (models), partially by Team 2 (event types)

---

### Team 4: `cli-commands`
**Scope**: CLI interface + entry point
**Files**: `src/cli/`, `src/main.rs`

**Tasks**:
1. Entry point (`src/main.rs`)
   - clap top-level command routing
   - Platform detection and initialization
2. CLI command definitions (`src/cli/mod.rs`)
   - clap derive structs for all subcommands
3. Status command (`src/cli/status.rs`)
   - One-shot snapshot (no daemon required)
   - JSON output mode
4. Cleanup command (`src/cli/cleanup.rs`)
   - Dry-run mode
   - Force/all modes with confirmation prompts
5. History command (`src/cli/history.rs`)
   - Time range filtering
   - CSV export
6. Report command (`src/cli/report.rs`)
   - Diagnostic report generation (Markdown format)
7. Config command (`src/cli/config.rs`)
   - Show/edit/reset operations
   - Config file path resolution per platform

**Deliverables**: All CLI commands functional, connected to daemon or standalone

**Dependencies**: Blocked by Team 1 (models + platform), partially by Team 2 (daemon IPC)

---

## Execution Phases

### Phase 1: Foundation (Team 1 solo)
```
Team 1: core-platform
├── Cargo.toml + project skeleton
├── Platform trait definition
├── All data models
├── IPC abstraction
└── Platform stubs (compilable, not yet functional)
```
**Gate**: `cargo build` passes on all targets, models and traits compile

### Phase 2: Parallel Development (Teams 2, 3, 4 in parallel)
```
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│ Team 2: daemon   │  │ Team 3: tui     │  │ Team 4: cli     │
│                  │  │                  │  │                  │
│ scanner          │  │ dashboard        │  │ main.rs          │
│ profiler         │  │ charts           │  │ status cmd       │
│ analyzer         │  │ process list     │  │ cleanup cmd      │
│ reaper           │  │ alerts           │  │ history cmd      │
│ event bus        │  │                  │  │ report cmd       │
└────────┬─────────┘  └────────┬─────────┘  └────────┬─────────┘
         │                     │                      │
         └─────────────────────┼──────────────────────┘
                               │
                        Phase 3: Integration
```
**Gate**: Each team's module compiles independently, unit tests pass

### Phase 3: Integration & Testing
```
All Teams:
├── Wire daemon <-> TUI via IPC
├── Wire daemon <-> CLI via IPC
├── CLI standalone mode (status without daemon)
├── Cross-platform integration tests
├── Windows committed memory E2E test
└── Performance benchmarks (clmem itself must be <10MB RSS)
```
**Gate**: Full integration tests pass, `clmem` binary <5MB, RSS <10MB

---

## Multi-Team Command Reference

### Launch Command
```
/auto-review:multi-team
```

### Team Definitions for Orchestrator

```yaml
teams:
  - name: core-platform
    scope: "src/platform/, src/models/, src/ipc/, Cargo.toml"
    tasks:
      - "Create Cargo.toml with all dependencies"
      - "Define Platform trait in src/platform/mod.rs"
      - "Implement data models in src/models/ (ProcessInfo, MemorySnapshot, ProcessState, Event, Config)"
      - "Implement IPC abstraction in src/ipc/ (server, client, unix socket, named pipe)"
      - "Implement WindowsPlatform in src/platform/windows.rs"
      - "Implement LinuxPlatform in src/platform/linux.rs"
      - "Implement MacosPlatform in src/platform/macos.rs"
    priority: 1
    blocking: [daemon-engine, tui-dashboard, cli-commands]

  - name: daemon-engine
    scope: "src/daemon/"
    tasks:
      - "Implement daemon main loop with tokio in src/daemon/mod.rs"
      - "Implement process scanner in src/daemon/scanner.rs"
      - "Implement memory profiler with ring buffer in src/daemon/profiler.rs"
      - "Implement leak analyzer in src/daemon/analyzer.rs"
      - "Implement orphan reaper with safety classification in src/daemon/reaper.rs"
      - "Implement event bus in src/daemon/event.rs"
    priority: 2
    blocked_by: [core-platform]

  - name: tui-dashboard
    scope: "src/tui/"
    tasks:
      - "Implement TUI main loop with ratatui in src/tui/mod.rs"
      - "Implement dashboard overview panel in src/tui/dashboard.rs"
      - "Implement memory trend charts in src/tui/charts.rs"
      - "Implement process list with status indicators in src/tui/process_list.rs"
      - "Implement alerts panel in src/tui/alerts.rs"
    priority: 2
    blocked_by: [core-platform]

  - name: cli-commands
    scope: "src/cli/, src/main.rs"
    tasks:
      - "Implement entry point and clap routing in src/main.rs"
      - "Define CLI command structs in src/cli/mod.rs"
      - "Implement status command in src/cli/status.rs"
      - "Implement cleanup command in src/cli/cleanup.rs"
      - "Implement history command in src/cli/history.rs"
      - "Implement report command in src/cli/report.rs"
      - "Implement config command in src/cli/config.rs"
    priority: 2
    blocked_by: [core-platform]
```

### Merge Order
1. `core-platform` -> main
2. `daemon-engine` + `tui-dashboard` + `cli-commands` -> main (parallel merge)
3. Integration branch -> final validation -> main

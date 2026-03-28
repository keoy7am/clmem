# CLAUDE.md - clmem Development Guide

## Project Overview

**clmem** is a cross-platform memory monitoring and management tool for Claude Code CLI, built in Rust. Single binary with three modes: daemon, TUI, CLI.

## Architecture

```
src/
├── main.rs          # Entry: clap routing to subcommands
├── cli/             # CLI commands (status, cleanup, history, report, config)
├── daemon/          # Background engine (scanner, profiler, analyzer, reaper, event bus)
├── tui/             # Terminal dashboard (ratatui-based)
├── platform/        # OS abstraction (Platform trait + windows/linux/macos impls)
├── ipc/             # IPC layer (unix socket / named pipe)
└── models/          # Shared data types (ProcessInfo, MemorySnapshot, Config, Event)
```

## Key Design Decisions

- **Single binary**: All modes via subcommand (`clmem daemon`, `clmem tui`, `clmem status`, etc.)
- **Platform trait**: `src/platform/mod.rs` defines cross-platform interface; each OS has its own impl
- **IPC**: Unix Domain Socket on Linux/macOS, Named Pipes on Windows
- **Ring buffer**: Memory history stored in fixed-size ring buffer (default 1h retention)
- **Safety-first cleanup**: Processes classified as ACTIVE/IDLE/STALE/ORPHAN; only ORPHAN auto-cleaned

## Development Rules

### Build & Test
```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo test                     # Run all tests
cargo test --lib               # Unit tests only
cargo test --test '*'          # Integration tests only
cargo clippy -- -D warnings    # Lint (must pass with zero warnings)
cargo fmt --check              # Format check
```

### Cross-Platform
- Always use `Platform` trait, never call OS APIs directly outside `src/platform/`
- Use `cfg(target_os = "...")` only in platform impl files
- Test on Windows first (primary pain point), then Linux/macOS
- IPC abstraction in `src/ipc/mod.rs` - never use socket/pipe directly in daemon/cli/tui

### Code Conventions
- **Error handling**: Use `anyhow::Result` for application code, `thiserror` for library errors
- **Async**: `tokio` for daemon; TUI and CLI are sync (TUI uses crossterm event loop)
- **Logging**: `tracing` crate with structured fields; never use `println!` for logging
- **Config**: `serde` + `toml`; config struct in `src/models/config.rs`
- **CLI parsing**: `clap` derive API; command definitions in `src/cli/mod.rs`

### Safety Rules for Reaper
1. ACTIVE processes (has TTY/stdin) -> NEVER touch
2. IDLE processes (activity < 5min) -> Monitor only, soft alert
3. STALE processes (no activity, parent alive) -> Wait 15min before downgrade
4. ORPHAN processes (parent dead, no IPC) -> Safe to auto-clean
5. `--force` flag required to touch IDLE processes
6. `--all` flag required to touch ALL processes (double confirmation)

### Windows-Specific
- `EmptyWorkingSet` after process tree termination
- Scan for orphaned Named Pipe handles via `NtQuerySystemInformation`
- Grace period (30s default) after main process exit before cleanup
- Track committed memory separately from RSS

### Dependencies (Cargo.toml)
| Crate | Purpose |
|---|---|
| `sysinfo` | Cross-platform process/memory info |
| `ratatui` + `crossterm` | TUI rendering |
| `clap` (derive) | CLI parsing |
| `tokio` | Async runtime |
| `serde` + `toml` | Config |
| `tracing` + `tracing-subscriber` | Logging |
| `anyhow` + `thiserror` | Error handling |
| `directories` | XDG/platform config paths |
| `chrono` | Timestamps |

### Git Workflow
- Branch naming: `feat/<name>`, `fix/<name>`, `refactor/<name>`
- Commit messages: imperative mood, concise (`Add process scanner`, `Fix Windows handle leak`)
- PR required for `main` branch
- CI must pass: `cargo clippy`, `cargo fmt --check`, `cargo test`

## File References

- Design doc: `docs/plans/2026-03-29-clmem-design.md`
- Config example: `clmem.toml.example`

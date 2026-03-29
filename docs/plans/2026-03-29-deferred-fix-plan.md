# clmem Deferred Fix Plan

**Date**: 2026-03-29
**Base Version**: v0.4.0 (tag pushed)
**Prior Work**: 5 rounds of auto-review fixed 72/84 issues (v0.1.0 → v0.4.0)
**Remaining**: 12 deferred items across 5 phases

---

## Status of Prior Plan

The original `2026-03-29-review-fix-plan.md` (Phase 1–5) is **complete**:
- Phase 1 (IPC Reliability): All 5 issues fixed
- Phase 2 (Security): 13/14 fixed, 1 deferred (S-12)
- Phase 3 (Platform Correctness): 15/17 fixed, 2 deferred (P-6, P-15)
- Phase 4 (Performance): 11/14 fixed, 3 deferred (PF-1, PF-8, PF-13)
- Phase 5 (Architecture): 10/16 fixed, 6 deferred (A-1~A-4, A-14, A-16)

---

## Phase A: IPC Server Extraction (5 items, 1 session) ✅ DONE (v0.5.0)

**Goal**: Move IPC server code from daemon/ to ipc/, eliminate all cfg in daemon/.

### Items

| ID | Severity | Issue |
|----|----------|-------|
| A-1 | P0 | IPC server (~225 lines) in `daemon/mod.rs` instead of `ipc/` |
| A-2 | P0 | 9x `#[cfg(unix)]`/`#[cfg(windows)]` in `daemon/mod.rs` |
| A-3 | P1 | Direct `sysinfo::System` usage in `write_pid_file()` |
| A-14 | P2 | `pid_file_path()` uses cfg in daemon/ |
| A-16 | P2 | `from_timestamp` fallback duplicated (centralized in `build_process_info` but fallback semantics unclear) |

### Execution Plan

#### Step 1: Create `src/ipc/server.rs`
- Move `start_ipc_listener`, `start_ipc_listener_platform` from `daemon/mod.rs`
- Move `handle_unix_connection`, `handle_unix_connection_inner` (Unix)
- Move `handle_windows_pipe_async`, `handle_windows_pipe_async_inner` (Windows)
- Move the `Semaphore` connection limiter
- Export a single entry point: `pub async fn run_ipc_server(path, daemon, semaphore)`
- All `#[cfg]` attributes stay in `ipc/server.rs` (allowed by architecture rules)

#### Step 2: Add `pid_file_path()` and `remove_ipc_socket()` to Platform trait
- Add `fn runtime_dir(&self) -> PathBuf` to Platform trait
- Implement per-OS in windows.rs, linux.rs, macos.rs
- Derive `pid_file_path()` and IPC socket path from `runtime_dir()`
- Replace `write_pid_file()` sysinfo usage with `platform.is_process_alive()`

#### Step 3: Fix `from_timestamp` fallback
- In `build_process_info()` (platform/mod.rs), change fallback from `Utc::now` to log warning + use epoch 0

#### Validation
```bash
cargo clippy -- -D warnings && cargo test
```

---

## Phase B: IPC Protocol Consolidation (1 item, 1 session) ✅ DONE (v0.5.1)

**Goal**: Reduce TUI→daemon IPC from 4 round-trips to 1.

### Items

| ID | Severity | Issue |
|----|----------|-------|
| PF-13 | P2 | 4 sequential IPC round-trips per 500ms TUI poll |

### Execution Plan

#### Step 1: Add GetAll protocol variant
- `src/ipc/protocol.rs`: Add `IpcMessage::GetAll` and `IpcResponse::All { snapshot, uptime_secs, events, history }`
- `src/daemon/mod.rs`: Handle `GetAll` in `handle_message` — lock each mutex once, build compound response

#### Step 2: Update TUI poller
- `src/tui/mod.rs`: Change `start_poller` to send single `GetAll` request
- Parse `IpcResponse::All` into the existing `IpcData` struct
- Fallback: if daemon is older version (returns Error), fall back to 4 individual calls

#### Validation
```bash
cargo clippy -- -D warnings && cargo test
# Manual: clmem daemon + clmem tui, verify CONNECTED stable
```

---

## Phase C: Incremental Process Refresh (1 item, 1 session) ✅ DONE (v0.5.2)

**Goal**: Reduce daemon CPU usage by scanning only known PIDs on most ticks.

### Items

| ID | Severity | Issue |
|----|----------|-------|
| PF-1 | P1 | Full system process refresh every second |

### Execution Plan

#### Step 1: Add `refresh_known_processes` to Platform trait
- `src/platform/mod.rs`: Add `fn refresh_known_processes(&self, pids: &[u32]) -> Result<Vec<ProcessInfo>>`
- Each platform impl: use `ProcessesToUpdate::Some(&pids)` instead of `All`

#### Step 2: Scanner cadence control
- `src/daemon/scanner.rs`: Add `full_scan_counter: u32`
- Every 5th scan: `platform.list_claude_processes()` (full scan, discovers new processes)
- Other scans: `platform.refresh_known_processes(&known_pids)` (fast update)

#### Validation
```bash
cargo clippy -- -D warnings && cargo test
# Benchmark: compare CPU usage before/after with `clmem status`
```

---

## Phase D: Windows FFI (1 item, 1 session) ✅ DONE (v0.5.3)

**Goal**: Implement real TTY/IPC detection on Windows.

### Items

| ID | Severity | Issue |
|----|----------|-------|
| P-6 | P0 | Windows `has_active_tty` and `has_active_ipc` are stubs |

### Execution Plan

#### Step 1: Implement `has_active_tty` on Windows
- Use `windows` crate: `GetConsoleProcessList` to check if PID is attached to a console
- Or check parent process chain for known terminal emulators (already done partially)

#### Step 2: Implement `has_active_ipc` on Windows
- Use `NtQuerySystemInformation` or iterate handle table for Named Pipe matching `clmem`
- Alternative: check if process has open handle to the clmem pipe name

#### Validation
```bash
cargo clippy -- -D warnings && cargo test
# Manual: verify ACTIVE state detected for interactive Claude sessions
```

---

## Phase E: Low Priority Cleanup (4 items, optional) ✅ DONE (v0.6.0)

### Items

| ID | Severity | Issue |
|----|----------|-------|
| A-4 | P1 | `std::process::Command` in `cli/config.rs` should be in Platform trait |
| PF-8 | P2 | `GetSnapshot` clone → `Arc<MemorySnapshot>` |
| P-15 | P2 | `swap_bytes` always 0 on all platforms |
| S-12 | P2 | Socket path hijacking via env var |

These can be addressed individually as time permits. None affect correctness or safety at current risk levels.

---

## Execution Priority

```
Phase A (IPC重構)  ──→ Phase B (GetAll) ──→ Phase C (增量掃描)
     ↓                                          ↓
Phase D (Windows FFI)                    Phase E (可選清理)
```

Phase A is the foundation — it unblocks cleaner architecture for B and C.
Phase D is independent and can run in parallel.
Phase E items are individually addressable at any time.

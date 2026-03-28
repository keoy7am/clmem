# clmem Auto-Review Fix Plan

**Date**: 2026-03-29
**Version**: 0.1.0
**Total Issues**: 41 (3 P0, 15 P1, 23 P2)
**Estimated Phases**: 5 (each phase = one conversation session)

---

## Phase Overview

| Phase | Focus | Issues | Priority |
|-------|-------|--------|----------|
| Phase 1 | IPC Reliability (daemon not connected) | #1, #4, #5, #6, #15 | **Highest** — user-facing bug |
| Phase 2 | Security Hardening | #2, #3, #11, #12, #13 | High — P0 + P1 security |
| Phase 3 | Platform Correctness | #14, P2-linux/macos, P2-truncate | High — safety classification |
| Phase 4 | Performance Optimization | #7, #8, #9, #10, P2-perf | Medium — daemon efficiency |
| Phase 5 | Architecture & Cleanup | #3-arch, #16, #17, #18, P2-convention | Low — code quality |

---

## Phase 1: IPC Reliability

**Goal**: Fix the intermittent "daemon not connected" in TUI.

### Issues

1. **IPC client has no timeout** (`ipc/mod.rs:25-65`)
   - `UnixStream::connect` and `read_exact` block indefinitely
   - Fix: Add 2s read/write timeout on all IPC operations

2. **TUI makes 4 blocking IPC calls per 250ms tick** (`tui/mod.rs:248-292`)
   - GetSnapshot + GetStatus + GetEvents + GetHistory = 4 round-trips
   - Fix: Consolidate into single IPC call or move to background thread

3. **Windows `first_pipe_instance(true)` blocks restart after crash** (`daemon/mod.rs:363-367`)
   - Stale pipe object persists after crash, daemon can't restart
   - Fix: Remove `first_pipe_instance(true)` or add fallback retry

4. **IPC server has no per-connection timeout** (`daemon/mod.rs:408-472`)
   - Stale client holds connection forever, leaks tokio task
   - Fix: Wrap connection handler in `tokio::time::timeout(5s)`

5. **IPC response size unbounded (client side)** (`ipc/mod.rs:41-62`)
   - Client allocates up to 4GB on malicious response
   - Fix: Add 16MB cap matching server-side limit

### Prompt for New Session

```
為 clmem 專案修復 IPC 可靠性問題（Phase 1）。

## 背景
clmem 是 Rust 跨平台 Claude Code 記憶體監控工具。TUI 不定時顯示 "daemon not connected"。
專案根目錄: A:\991.Funny\1.Claude-Better
開發規範見 CLAUDE.md

## 修復清單（按順序執行）

### Fix 1: IPC client 加 timeout
File: src/ipc/mod.rs
- Unix: connect 後加 stream.set_read_timeout(Some(Duration::from_secs(2)))
  和 stream.set_write_timeout(Some(Duration::from_secs(2)))
- Windows: pipe open 後加相同的 timeout
- 加 response size 上限 16MB (與 server 端一致)

### Fix 2: TUI IPC 移到背景線程
File: src/tui/mod.rs
- 將 update() 的 4 次 IPC 呼叫移到 std::thread::spawn 背景線程
- 用 std::sync::mpsc channel 傳回結果
- main loop 用 try_recv() 非阻塞接收
- 若無新資料則保留上次快照

### Fix 3: Windows Named Pipe 重啟容錯
File: src/daemon/mod.rs
- 移除 first_pipe_instance(true)
- 啟動時先嘗試連接既有 pipe，若成功表示 daemon 已在跑，bail
- 若連接失敗則建立新 pipe server

### Fix 4: IPC server per-connection timeout
File: src/daemon/mod.rs
- Unix: tokio::spawn 內包 tokio::time::timeout(Duration::from_secs(5), handler)
- Windows: 同理包 timeout

## 驗證
每個 fix 後執行:
1. cargo clippy -- -D warnings
2. cargo test
3. 手動測試: 啟動 daemon + TUI，確認 CONNECTED 穩定不閃爍

## 完成後
git commit 每個 fix 為獨立 commit，push 到 origin/master
```

---

## Phase 2: Security Hardening

**Goal**: Fix P0/P1 security vulnerabilities.

### Issues

1. **`GetHistory/GetEvents` last_n 無上限** — daemon OOM
2. **Windows Named Pipe 無存取控制** — 任何本機使用者可連接
3. **TOCTOU: state 判斷與 kill 非原子** — PID 回收風險
4. **`$EDITOR` 命令注入** — `config edit` 執行任意程式
5. **PID file 無 exclusive lock** — 雙 daemon race condition
6. **Config `scan_interval_ms=0` 造成 busy-loop**

### Prompt for New Session

```
為 clmem 專案修復安全漏洞（Phase 2）。

## 背景
clmem 是 Rust 跨平台 Claude Code 記憶體監控工具。
專案根目錄: A:\991.Funny\1.Claude-Better
開發規範見 CLAUDE.md

## 修復清單（按順序執行）

### Fix 1: Cap IPC last_n parameters
File: src/daemon/mod.rs (handle_message)
- GetHistory { last_n } → last_n.min(3600)
- GetEvents { last_n } → last_n.min(1000)

### Fix 2: Config validation
File: src/models/config.rs
- Config::load() 後 clamp scan_interval_ms 最小 100
- clamp orphan_grace_period_secs 最大 3600
- clamp leak_check_interval_secs 最小 5

### Fix 3: PID file exclusive lock
File: src/daemon/mod.rs (write_pid_file)
- 用 OpenOptions::new().write(true).create_new(true).open() 原子建立
- 若已存在，讀取內容檢查 PID 是否活著
- 若 PID 已死則移除舊檔重建
- daemon 退出時清理

### Fix 4: Reaper TOCTOU mitigation
File: src/daemon/reaper.rs (terminate_gracefully)
- 執行 terminate_process 前重新呼叫 platform.is_process_alive(pid)
- 執行前重新呼叫 platform.has_active_tty(pid)，若 true 則 abort

### Fix 5: $EDITOR 安全處理
File: src/cli/config.rs
- split editor 字串為 argv（用空格分割）
- Command::new(parts[0]).args(&parts[1..]).arg(&path)
- 或限制為已知 editor 白名單

### Fix 6: Unix socket permissions
File: src/daemon/mod.rs (start_ipc_listener, #[cfg(unix)])
- bind 後加 std::fs::set_permissions(path, Permissions::from_mode(0o600))

## 驗證
cargo clippy -- -D warnings && cargo test
每個 fix 獨立 commit，push 到 origin/master
```

---

## Phase 3: Platform Correctness

**Goal**: Fix Linux/macOS state classification and platform stubs.

### Issues

1. **Linux/macOS `started_at` 和 `last_activity` 永遠 `Utc::now()`** — IDLE/STALE 永遠判不到
2. **macOS `has_active_tty` 是 stub** — ACTIVE 永遠判不到
3. **Linux `list_claude_processes` 不呼叫 `has_active_tty`** — has_tty 永遠 false
4. **`truncate_name` byte index panic** — 非 ASCII 程序名會 crash TUI
5. **`is_claude_process` 三平台重複** — 應提取到 common module

### Prompt for New Session

```
為 clmem 專案修復跨平台正確性問題（Phase 3）。

## 背景
clmem 是 Rust 跨平台 Claude Code 記憶體監控工具。
專案根目錄: A:\991.Funny\1.Claude-Better
開發規範見 CLAUDE.md
目前 Windows 平台已正確實作 started_at/has_tty，但 Linux/macOS 仍有問題。

## 修復清單

### Fix 1: Linux/macOS started_at 和 last_activity
Files: src/platform/linux.rs, src/platform/macos.rs
- 參照 src/platform/windows.rs 的實作方式
- started_at: 用 proc.start_time() 轉 DateTime<Utc>
- last_activity: 用 cpu_usage() > 0 判斷，否則設為 started_at
- has_tty: Linux 已有 has_active_tty() 實作，在 list_claude_processes 內呼叫它設定 has_tty 欄位
- macOS: 實作基本的 has_tty（可暫時用 parent process name 檢查，同 Windows 方式）

### Fix 2: 提取共用邏輯
- 建立 src/platform/common.rs
- 將 is_claude_process() 和 cmd_to_string() 移入
- 三個平台 impl 改為 use super::common::*

### Fix 3: truncate_name UTF-8 安全
File: src/tui/process_list.rs
- 改用 name.chars().take(max_len - 3).collect::<String>() + "..."
- 或用 char_indices 找安全邊界

## 驗證
cargo clippy -- -D warnings && cargo test
每個 fix 獨立 commit，push 到 origin/master
```

---

## Phase 4: Performance Optimization

**Goal**: Reduce daemon CPU usage and TUI render overhead.

### Issues

1. **每秒全系統 process refresh** — 改為增量掃描
2. **take_snapshot 重複 refresh** — 合併為單次 lock
3. **O(N²) seen_pids.contains** — 改用 HashSet
4. **Vec::remove(0) in alerts** — 改用 VecDeque
5. **std::sync::Mutex in async context** — spawn_blocking 包裝
6. **O(P²×S) analyzer** — 預索引 HashMap
7. **sort 每次 to_lowercase 分配** — 預計算
8. **clone MemorySnapshot per IPC tick** — Arc 共享

### Prompt for New Session

```
為 clmem 專案優化效能（Phase 4）。

## 背景
clmem 是 Rust 跨平台 Claude Code 記憶體監控工具。
daemon 每秒掃描一次，TUI 每 250ms 更新一次，效能很重要。
專案根目錄: A:\991.Funny\1.Claude-Better
開發規範見 CLAUDE.md

## 修復清單（按影響大小排序）

### Fix 1: 增量 process refresh
Files: src/platform/windows.rs, linux.rs, macos.rs
- 新增方法 refresh_known_processes(pids: &[u32]) 使用 ProcessesToUpdate::Some
- Scanner 維護已知 PID 集合，平時只 refresh 已知的
- 每 10 秒做一次 All refresh 發現新程序

### Fix 2: take_snapshot 合併 refresh
File: src/platform/windows.rs
- 將 list_claude_processes + refresh_memory 合併在同一次 lock 內
- 避免 double lock 和 double refresh

### Fix 3: Scanner retain 改用 HashSet
File: src/daemon/scanner.rs
- seen_pids: Vec<u32> → HashSet<u32>
- retain 查找從 O(N) 降為 O(1)

### Fix 4: Alerts 改用 VecDeque
File: src/tui/alerts.rs
- alerts: Vec<AlertEntry> → VecDeque<AlertEntry>
- 溢出時 pop_front() 取代 remove(0)

### Fix 5: Analyzer 預索引
File: src/daemon/analyzer.rs
- HashMap<u32, bool> → HashSet<u32>
- 預建 HashMap<u32, Vec<(f64, f64)>> 避免 O(P²×S) 內層掃描

### Fix 6: async context 中的 blocking 呼叫
File: src/daemon/mod.rs
- run_scan_cycle 中的 scanner.scan() 包 tokio::task::spawn_blocking
- 或將 Platform 的 std::sync::Mutex 替換

### Fix 7: TUI 小優化
Files: src/tui/process_list.rs
- sort 前預計算 lowercase name
- render 改 &mut self 避免 clone TableState

## 驗證
cargo clippy -- -D warnings && cargo test
cargo build --release 後用 clmem status 確認功能正常
每個 fix 獨立 commit，push 到 origin/master
```

---

## Phase 5: Architecture & Cleanup

**Goal**: Align codebase with CLAUDE.md layer boundaries, remove dead code.

### Issues

1. **IPC server 在 daemon/ 而非 ipc/** — 違反架構規則
2. **pid_file_path 用 cfg 在 daemon/** — 應在 platform trait
3. **remove_ipc_socket 用 cfg 在 daemon/** — 應在 ipc/
4. **format_bytes 重複** — 提取到共用模組
5. **std::process::Command 在 cli/config.rs** — 應在 platform/
6. **stale #[allow(dead_code)]** — 清理
7. **Daemon 持有未使用的 platform Arc** — 移除
8. **debug log 含 cmdline** — 可能洩漏 API key

### Prompt for New Session

```
為 clmem 專案修復架構違規和清理（Phase 5）。

## 背景
clmem 是 Rust 跨平台 Claude Code 記憶體監控工具。
專案根目錄: A:\991.Funny\1.Claude-Better
開發規範見 CLAUDE.md

架構規則:
- cfg(target_os) 只能在 src/platform/ 和 src/ipc/mod.rs
- OS API 只能在 src/platform/
- IPC socket/pipe 操作只能在 src/ipc/
- 使用 tracing 記錄 log，不要 println!
- 使用 anyhow::Result，不要 unwrap()

## 修復清單

### Fix 1: 提取 format_bytes 到共用模組
- 建立 src/util.rs，移入 format_bytes()
- src/cli/mod.rs 和 src/tui/mod.rs 改為 use crate::util::format_bytes

### Fix 2: 移除 stale #[allow(dead_code)]
Files: src/models/events.rs, src/models/mod.rs, src/daemon/mod.rs
- 移除 Event::new 上的 #[allow(dead_code)]
- 移除 models/mod.rs 的 #[allow(unused_imports)]
- 移除 Daemon.platform 欄位（未使用）

### Fix 3: unwrap → expect 或 if-let
Files: src/daemon/scanner.rs, src/daemon/analyzer.rs
- scanner.rs:68,93 的 .unwrap() → .expect("invariant: ...")
- analyzer.rs:85 的 .unwrap() → ? operator

### Fix 4: debug log 不記錄完整 IPC message
File: src/daemon/mod.rs
- tracing::debug!(?msg, ...) → tracing::debug!(msg_type = ?std::mem::discriminant(&msg), ...)
- 只記錄 message 類型，不記錄完整內容

### Fix 5: pid_file_path 移到 Platform trait（可選）
如果時間允許，將 pid_file_path() 的 cfg 邏輯移到 Platform trait
新增 fn runtime_dir(&self) -> PathBuf 方法

### Fix 6: IPC server 提取到 ipc/ （可選，大重構）
此項涉及大量搬移，建議作為獨立 PR

## 驗證
cargo clippy -- -D warnings && cargo test
每個 fix 獨立 commit，push 到 origin/master
```

---

## Issue Cross-Reference

| Issue ID | Phase | Status |
|----------|-------|--------|
| #1 IPC response unbounded | Phase 1 | Pending |
| #2 last_n uncapped | Phase 2 | Pending |
| #3 IPC server in daemon | Phase 5 | Pending (optional) |
| #4 IPC no timeout | Phase 1 | Pending |
| #5 TUI 4x blocking IPC | Phase 1 | Pending |
| #6 first_pipe_instance | Phase 1 | Pending |
| #7 All process refresh | Phase 4 | Pending |
| #8 Double refresh | Phase 4 | Pending |
| #9 O(N²) retain | Phase 4 | Pending |
| #10 Vec::remove(0) | Phase 4 | Pending |
| #11 TOCTOU reaper | Phase 2 | Pending |
| #12 $EDITOR injection | Phase 2 | Pending |
| #13 PID file race | Phase 2 | Pending |
| #14 TTY/IPC stubs | Phase 3 | Pending |
| #15 No connection timeout | Phase 1 | Pending |
| #16 cfg in daemon | Phase 5 | Pending |
| #17 unwrap in prod | Phase 5 | Pending |
| #18 format_bytes dup | Phase 5 | Pending |
| P2 (23 items) | Phase 3-5 | Pending |

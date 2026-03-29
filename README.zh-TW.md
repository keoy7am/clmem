# clmem

[English](README.md) | [繁體中文](README.zh-TW.md) | [简体中文](README.zh-CN.md)

跨平台的 Claude Code CLI 記憶體監控與管理工具。

Claude Code CLI（基於 Node.js）常出現記憶體洩漏、孤兒程序、虛擬/已提交記憶體未釋放等問題——在 Windows 上尤其嚴重。`clmem` 自動監控、偵測並清理這些問題。

## 功能特色

- **即時監控** — 追蹤所有 Claude Code 程序的 RSS、虛擬記憶體、Swap 和已提交記憶體
- **洩漏偵測** — 識別異常記憶體增長模式，在問題惡化前發出警報
- **孤兒清理** — 偵測並安全終止 Claude Code 退出後殘留的程序
- **Windows 已提交記憶體** — 處理 V8 引擎記憶體殘留和孤兒 Named Pipe 句柄
- **TUI 儀表板** — htop 風格的終端介面，含即時圖表和程序管理
- **記憶體智慧分析** — RSS 變化量追蹤、色彩分級顯示、行內趨勢迷你圖
- **診斷報告** — 匯出記憶體歷史並生成診斷報告

## 安裝

```bash
# 從原始碼編譯（需要 Rust 工具鏈）
cargo build --release

# 執行檔位於 target/release/clmem（Windows 為 clmem.exe）

# 或直接安裝
cargo install --path .
```

## 快速開始

```bash
# 一次性狀態檢查（不需要 daemon）
clmem status

# 啟動背景監控 daemon
clmem daemon

# 開啟即時 TUI 儀表板
clmem tui

# 清理孤兒程序
clmem cleanup --dry-run    # 先預覽
clmem cleanup              # 執行清理
```

## 指令

### `clmem status` — 快速快照

獨立運作，不需要 daemon。掃描程序表並顯示所有 Claude Code 程序。

```bash
clmem status               # 人類可讀的表格
clmem status --json        # JSON 輸出供腳本使用
```

### `clmem daemon` — 背景監控

啟動持續掃描、洩漏偵測和可選自動清理的監控 daemon。

```bash
clmem daemon               # 背景執行
clmem daemon --foreground  # 前景執行（查看日誌）
```

### `clmem tui` — 互動式儀表板

帶有即時記憶體圖表、程序列表和警報的終端 UI。透過 IPC 連接 daemon。

```bash
clmem tui
```

**鍵盤快捷鍵：**

| 按鍵 | 功能 |
|------|------|
| `Tab` | 切換面板 |
| `j/k` 或 `↑/↓` | 上下瀏覽程序 |
| `PgUp/PgDn` | 上/下翻頁 |
| `Home/End` | 跳至最前/最後 |
| `Enter` | 展開/摺疊樹節點 |
| `K` | 終止選中的程序 |
| `d` | 開關程序詳細資訊彈窗 |
| `t` | 切換樹狀/平面視圖 |
| `c` | 切換名稱/命令列顯示 |
| `/` | 篩選程序 |
| `r` | 重新整理 |
| `1`–`5` | 依欄位排序（PID、名稱、RSS、VMS、狀態） |
| `?` | 說明 |
| `q` / `Esc` | 退出（或清除篩選） |

**功能鍵：**

| 按鍵 | 功能 |
|------|------|
| `F1` | 說明 |
| `F3` | 篩選 |
| `F5` | 切換樹狀視圖 |
| `F9` | 終止程序 |
| `F10` | 退出 |

**表格欄位：**

| 欄位 | 說明 |
|------|------|
| PID | 程序 ID |
| Command / Name | 完整命令列或程序名稱（按 `c` 切換） |
| RSS | 常駐記憶體大小，色彩分級：綠色 (<50 MB)、黃色 (50–200 MB)、紅色 (>200 MB) |
| Delta | 上次更新以來的 RSS 變化：紅色（+增長）、綠色（−縮減）、灰色（無變化） |
| VMS / Commit | 虛擬記憶體（Linux/macOS）或已提交記憶體（Windows），色彩分級 |
| Trend | 行內迷你圖，顯示 RSS 歷史趨勢（最近 20 個樣本） |
| State | 程序狀態：ACTIVE、IDLE、STALE、ORPHAN |
| Uptime | 程序啟動後的運行時間 |

### `clmem cleanup` — 程序清理

安全優先的清理機制，支援多種模式。

```bash
clmem cleanup              # 僅清理 ORPHAN 程序
clmem cleanup --dry-run    # 預覽但不執行
clmem cleanup --force      # 同時清理 IDLE 程序
clmem cleanup --all        # 所有 Claude 程序（需輸入 "yes" 確認）
clmem cleanup --pids 1234,5678  # 指定 PID
```

### `clmem history` — 記憶體歷史

需要 daemon 執行中。顯示環形緩衝區的記憶體快照。

```bash
clmem history              # 最近 60 筆快照
clmem history -n 300       # 最近 300 筆快照
clmem history --csv        # 匯出為 CSV
```

### `clmem report` — 診斷報告

生成包含系統資訊、程序詳情的 Markdown 診斷報告，若 daemon 執行中還包含歷史和事件。

```bash
clmem report               # 輸出至 stdout
clmem report -o report.md  # 儲存至檔案
```

### `clmem config` — 設定

```bash
clmem config show          # 以 TOML 格式顯示目前設定
clmem config path          # 顯示設定檔位置
clmem config edit          # 以 $EDITOR 開啟（Windows 為 notepad）
clmem config reset         # 重設為預設值
```

## 設定

預設設定檔位置：
- **Windows**：`%APPDATA%\clmem\clmem\clmem.toml`
- **macOS**：`~/Library/Application Support/dev.clmem.clmem/clmem.toml`
- **Linux**：`~/.config/clmem/clmem.toml`

所有選項請參閱 [`clmem.toml.example`](clmem.toml.example)。

### 主要設定

| 設定項 | 預設值 | 說明 |
|--------|--------|------|
| `scan_interval_ms` | `1000` | 程序掃描間隔（毫秒） |
| `history_retention_secs` | `3600` | 環形緩衝區保留時間（1 小時） |
| `idle_threshold_secs` | `300` | 閒置分類閾值（5 分鐘） |
| `stale_grace_period_secs` | `900` | STALE 降級等待時間（15 分鐘） |
| `orphan_grace_period_secs` | `30` | 父程序退出後的寬限期 |
| `leak_check_interval_secs` | `10` | 洩漏分析間隔 |
| `leak_growth_threshold_bytes_per_sec` | `1048576` | 洩漏警報閾值（1 MB/s） |
| `auto_cleanup` | `false` | daemon 執行時自動清理孤兒 |
| `log_level` | `"info"` | 日誌詳細程度 |

## 程序安全分類

`clmem` 在採取任何動作前，會先對每個 Claude Code 程序進行分類：

| 狀態 | 條件 | 自動清理 | `--force` | `--all` |
|------|------|:--------:|:---------:|:-------:|
| **ACTIVE** | 有 TTY/stdin | 永不 | 永不 | 是 |
| **IDLE** | 未活動 < 閾值 | 永不 | 是 | 是 |
| **STALE** | 未活動，父程序存在 | 寬限期後 | 是 | 是 |
| **ORPHAN** | 父程序已終止，無 IPC | 是 | 是 | 是 |

**規則**：不使用 `--all` 加確認，永遠不會觸碰 ACTIVE 程序。

## 架構

```
clmem（單一執行檔）
├── daemon    — 背景引擎（tokio 非同步）
│   ├── scanner    — 程序表輪詢（1 秒間隔）
│   ├── profiler   — 環形緩衝區記憶體快照
│   ├── analyzer   — 線性迴歸洩漏偵測
│   ├── reaper     — 安全孤兒終止
│   └── event bus  — 警報發布/訂閱
├── tui       — 終端儀表板（ratatui + crossterm）
│   ├── dashboard  — 記憶體量表和摘要統計
│   ├── charts     — 即時記憶體趨勢線
│   ├── process list — 可排序表格，含樹狀視圖、變化量、趨勢圖
│   └── alerts     — 事件歷史，含嚴重性色彩
├── cli       — 命令介面（clap derive）
│   ├── status, cleanup, history, report, config
│   └── daemon/tui 啟動器
├── platform  — 作業系統抽象層（Platform trait）
│   ├── windows    — sysinfo + Win32 (EmptyWorkingSet)
│   ├── linux      — sysinfo + /proc 檔案系統
│   └── macos      — sysinfo + libproc
├── ipc       — Daemon <-> CLI/TUI 通訊
│   └── 長度前綴 JSON，透過 Unix socket / Named pipe
└── models    — 共用資料類型
    └── ProcessInfo, MemorySnapshot, Event, Config
```

## 平台支援

| 功能 | Windows | Linux | macOS |
|------|---------|-------|-------|
| 程序監控 | sysinfo + Win32 | sysinfo + /proc | sysinfo + libproc |
| 記憶體分析 | RSS/VMS/已提交 | RSS/VMS/Swap | RSS/VMS |
| TTY 偵測 | Console 檢查 | /proc/[pid]/fd/0 | Stub |
| IPC 偵測 | Named pipe 掃描 | /proc/[pid]/fd socket | Stub |
| 程序清理 | Kill tree + EmptyWorkingSet | SIGTERM/SIGKILL | SIGTERM/SIGKILL |
| IPC 傳輸 | Named Pipe | Unix Socket | Unix Socket |

## 技術棧

| Crate | 用途 |
|-------|------|
| `sysinfo` | 跨平台程序/記憶體資訊 |
| `ratatui` + `crossterm` | TUI 渲染 |
| `clap` (derive) | CLI 參數解析 |
| `tokio` | 非同步執行時（daemon） |
| `serde` + `toml` + `serde_json` | 設定和 IPC 序列化 |
| `tracing` | 結構化日誌 |
| `anyhow` + `thiserror` | 錯誤處理 |
| `chrono` | 時間戳 |
| `directories` | 平台設定路徑 |

## 授權

MIT

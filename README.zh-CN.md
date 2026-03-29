# clmem

[English](README.md) | [繁體中文](README.zh-TW.md) | [简体中文](README.zh-CN.md)

跨平台的 Claude Code CLI 内存监控与管理工具。

Claude Code CLI（基于 Node.js）常出现内存泄漏、孤儿进程、虚拟/已提交内存未释放等问题——在 Windows 上尤为严重。`clmem` 自动监控、检测并清理这些问题。

## 功能特色

- **实时监控** — 跟踪所有 Claude Code 进程的 RSS、虚拟内存、Swap 和已提交内存
- **泄漏检测** — 识别异常内存增长模式，在问题恶化前发出警报
- **孤儿清理** — 检测并安全终止 Claude Code 退出后残留的进程
- **Windows 已提交内存** — 处理 V8 引擎内存残留和孤儿 Named Pipe 句柄
- **TUI 仪表盘** — htop 风格的终端界面，含实时图表和进程管理
- **内存智能分析** — RSS 变化量跟踪、色彩分级显示、行内趋势迷你图
- **诊断报告** — 导出内存历史并生成诊断报告

## 安装

```bash
# 从源码编译（需要 Rust 工具链）
cargo build --release

# 可执行文件位于 target/release/clmem（Windows 为 clmem.exe）

# 或直接安装
cargo install --path .
```

## 快速开始

```bash
# 一次性状态检查（不需要 daemon）
clmem status

# 启动后台监控 daemon
clmem daemon

# 打开实时 TUI 仪表盘
clmem tui

# 清理孤儿进程
clmem cleanup --dry-run    # 先预览
clmem cleanup              # 执行清理
```

## 命令

### `clmem status` — 快速快照

独立运行，不需要 daemon。扫描进程表并显示所有 Claude Code 进程。

```bash
clmem status               # 人类可读的表格
clmem status --json        # JSON 输出供脚本使用
```

### `clmem daemon` — 后台监控

启动持续扫描、泄漏检测和可选自动清理的监控 daemon。

```bash
clmem daemon               # 后台运行
clmem daemon --foreground  # 前台运行（查看日志）
```

### `clmem tui` — 交互式仪表盘

带有实时内存图表、进程列表和警报的终端 UI。通过 IPC 连接 daemon。

```bash
clmem tui
```

**键盘快捷键：**

| 按键 | 功能 |
|------|------|
| `Tab` | 切换面板 |
| `j/k` 或 `↑/↓` | 上下浏览进程 |
| `PgUp/PgDn` | 上/下翻页 |
| `Home/End` | 跳至最前/最后 |
| `Enter` | 展开/折叠树节点 |
| `K` | 终止选中的进程 |
| `d` | 开关进程详细信息弹窗 |
| `t` | 切换树状/平面视图 |
| `c` | 切换名称/命令行显示 |
| `/` | 筛选进程 |
| `r` | 刷新 |
| `1`–`5` | 按列排序（PID、名称、RSS、VMS、状态） |
| `?` | 帮助 |
| `q` / `Esc` | 退出（或清除筛选） |

**功能键：**

| 按键 | 功能 |
|------|------|
| `F1` | 帮助 |
| `F3` | 筛选 |
| `F5` | 切换树状视图 |
| `F9` | 终止进程 |
| `F10` | 退出 |

**表格列：**

| 列 | 说明 |
|----|------|
| PID | 进程 ID |
| Command / Name | 完整命令行或进程名称（按 `c` 切换） |
| RSS | 常驻内存大小，色彩分级：绿色 (<50 MB)、黄色 (50–200 MB)、红色 (>200 MB) |
| Delta | 上次更新以来的 RSS 变化：红色（+增长）、绿色（−缩减）、灰色（无变化） |
| VMS / Commit | 虚拟内存（Linux/macOS）或已提交内存（Windows），色彩分级 |
| Trend | 行内迷你图，显示 RSS 历史趋势（最近 20 个样本） |
| State | 进程状态：ACTIVE、IDLE、STALE、ORPHAN |
| Uptime | 进程启动后的运行时间 |

### `clmem cleanup` — 进程清理

安全优先的清理机制，支持多种模式。

```bash
clmem cleanup              # 仅清理 ORPHAN 进程
clmem cleanup --dry-run    # 预览但不执行
clmem cleanup --force      # 同时清理 IDLE 进程
clmem cleanup --all        # 所有 Claude 进程（需输入 "yes" 确认）
clmem cleanup --pids 1234,5678  # 指定 PID
```

### `clmem history` — 内存历史

需要 daemon 运行中。显示环形缓冲区的内存快照。

```bash
clmem history              # 最近 60 条快照
clmem history -n 300       # 最近 300 条快照
clmem history --csv        # 导出为 CSV
```

### `clmem report` — 诊断报告

生成包含系统信息、进程详情的 Markdown 诊断报告，若 daemon 运行中还包含历史和事件。

```bash
clmem report               # 输出至 stdout
clmem report -o report.md  # 保存至文件
```

### `clmem config` — 配置

```bash
clmem config show          # 以 TOML 格式显示当前配置
clmem config path          # 显示配置文件位置
clmem config edit          # 以 $EDITOR 打开（Windows 为 notepad）
clmem config reset         # 重置为默认值
```

## 配置

默认配置文件位置：
- **Windows**：`%APPDATA%\clmem\clmem\clmem.toml`
- **macOS**：`~/Library/Application Support/dev.clmem.clmem/clmem.toml`
- **Linux**：`~/.config/clmem/clmem.toml`

所有选项请参阅 [`clmem.toml.example`](clmem.toml.example)。

### 主要配置

| 配置项 | 默认值 | 说明 |
|--------|--------|------|
| `scan_interval_ms` | `1000` | 进程扫描间隔（毫秒） |
| `history_retention_secs` | `3600` | 环形缓冲区保留时间（1 小时） |
| `idle_threshold_secs` | `300` | 空闲分类阈值（5 分钟） |
| `stale_grace_period_secs` | `900` | STALE 降级等待时间（15 分钟） |
| `orphan_grace_period_secs` | `30` | 父进程退出后的宽限期 |
| `leak_check_interval_secs` | `10` | 泄漏分析间隔 |
| `leak_growth_threshold_bytes_per_sec` | `1048576` | 泄漏警报阈值（1 MB/s） |
| `auto_cleanup` | `false` | daemon 运行时自动清理孤儿 |
| `log_level` | `"info"` | 日志详细程度 |

## 进程安全分类

`clmem` 在采取任何操作前，会先对每个 Claude Code 进程进行分类：

| 状态 | 条件 | 自动清理 | `--force` | `--all` |
|------|------|:--------:|:---------:|:-------:|
| **ACTIVE** | 有 TTY/stdin | 永不 | 永不 | 是 |
| **IDLE** | 未活动 < 阈值 | 永不 | 是 | 是 |
| **STALE** | 未活动，父进程存在 | 宽限期后 | 是 | 是 |
| **ORPHAN** | 父进程已终止，无 IPC | 是 | 是 | 是 |

**规则**：不使用 `--all` 加确认，永远不会触碰 ACTIVE 进程。

## 架构

```
clmem（单一可执行文件）
├── daemon    — 后台引擎（tokio 异步）
│   ├── scanner    — 进程表轮询（1 秒间隔）
│   ├── profiler   — 环形缓冲区内存快照
│   ├── analyzer   — 线性回归泄漏检测
│   ├── reaper     — 安全孤儿终止
│   └── event bus  — 警报发布/订阅
├── tui       — 终端仪表盘（ratatui + crossterm）
│   ├── dashboard  — 内存仪表和摘要统计
│   ├── charts     — 实时内存趋势线
│   ├── process list — 可排序表格，含树状视图、变化量、趋势图
│   └── alerts     — 事件历史，含严重性色彩
├── cli       — 命令接口（clap derive）
│   ├── status, cleanup, history, report, config
│   └── daemon/tui 启动器
├── platform  — 操作系统抽象层（Platform trait）
│   ├── windows    — sysinfo + Win32 (EmptyWorkingSet)
│   ├── linux      — sysinfo + /proc 文件系统
│   └── macos      — sysinfo + libproc
├── ipc       — Daemon <-> CLI/TUI 通信
│   └── 长度前缀 JSON，通过 Unix socket / Named pipe
└── models    — 共用数据类型
    └── ProcessInfo, MemorySnapshot, Event, Config
```

## 平台支持

| 功能 | Windows | Linux | macOS |
|------|---------|-------|-------|
| 进程监控 | sysinfo + Win32 | sysinfo + /proc | sysinfo + libproc |
| 内存分析 | RSS/VMS/已提交 | RSS/VMS/Swap | RSS/VMS |
| TTY 检测 | Console 检查 | /proc/[pid]/fd/0 | Stub |
| IPC 检测 | Named pipe 扫描 | /proc/[pid]/fd socket | Stub |
| 进程清理 | Kill tree + EmptyWorkingSet | SIGTERM/SIGKILL | SIGTERM/SIGKILL |
| IPC 传输 | Named Pipe | Unix Socket | Unix Socket |

## 技术栈

| Crate | 用途 |
|-------|------|
| `sysinfo` | 跨平台进程/内存信息 |
| `ratatui` + `crossterm` | TUI 渲染 |
| `clap` (derive) | CLI 参数解析 |
| `tokio` | 异步运行时（daemon） |
| `serde` + `toml` + `serde_json` | 配置和 IPC 序列化 |
| `tracing` | 结构化日志 |
| `anyhow` + `thiserror` | 错误处理 |
| `chrono` | 时间戳 |
| `directories` | 平台配置路径 |

## 许可

MIT

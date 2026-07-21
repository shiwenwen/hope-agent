# 内嵌终端

底部终端是一个进程级、内存态的交互式 PTY 控制面。它服务桌面 GUI 与 HTTP/Web GUI，业务实现位于 `ha-core::terminal`，Tauri 与 axum 只提供薄适配。

## 数据流

```text
xterm.js ── terminal_write / terminal_resize ──> TerminalManager ──> PTY shell
    ^                                                │
    ├──── terminal:output ─────── Terminal Output Bus┤
    └──── created / exit / closed ──── App EventBus ┘
```

- 桌面使用 Tauri invoke + Tauri EventBus 桥；Web 使用 Bearer 保护的 REST + `/ws/events`，并要求显式开启 `filesystem.allowRemoteWrites`。
- `TerminalManager` 由 `init_runtime()` 创建，Tauri `AppState` 与内嵌/独立 `AppContext` 共享同一个 `Arc`，避免同一进程出现两套终端注册表。
- 会话只在内存中存在。隐藏底部面板不终止 shell；关闭标签或进程退出会终止 shell。
- UI 重新挂载、WebSocket 重连或事件序列出现缺口时，通过 `terminal_snapshot` 重放有界输出并按 `seq` 去重。

## 生命周期与边界

- 每个进程最多 12 个终端；单次输入上限 64 KiB；每个终端保留最近 2 MiB 原始 PTY 字节。
- 输出使用 base64 传输原始字节，避免 UTF-8 字符或 ANSI 序列跨读取块时损坏。
- 高频 `terminal:output` 使用独立广播通道，不能占用 App EventBus 的聊天、审批与会话事件容量；通道 lag 后前端必须通过 snapshot 恢复。
- 初始目录必须 canonicalize 成现存目录；未指定时使用用户主目录，主目录不可用才回退进程目录。
- Unix 使用 `$SHELL`（缺失时 `/bin/sh`），Windows 使用 `%COMSPEC%`（缺失时 `cmd.exe`）；统一设置 `TERM=xterm-256color`、`COLORTERM=truecolor`。
- HTTP 路由属于 owner 写平面，必须经过 API Key 中间件且服从默认关闭的 `filesystem.allowRemoteWrites`。它等价于交互式本机 shell，禁止移动到公开或只读 token 路由，也不得绕过远程写入门。
- `allowRemoteWrites` 是实时能力而非仅创建门：关闭后立即终止并移除所有 HTTP 创建的 shell；HTTP WebSocket 在关闭期间不得转发任何 `terminal:*` 事件。桌面创建的 shell 不受撤权影响。
- `terminal_list` 只返回会话元数据；原始输出只由单会话 `terminal_snapshot` 返回，避免多标签挂载时重复编码和传输全部回滚缓存。

## 前端交互

- 标题栏按钮或 `⌘/Ctrl+J` 显示/隐藏面板。
- 支持多标签、新建/关闭、拖拽高度、最大化；新终端继承创建瞬间的有效工作目录。
- xterm `FitAddon` 将容器尺寸换算为行列，并把 resize 回传 PTY；输入按 16 ms 合并，避免 HTTP 模式逐按键发请求。

## 接口

完整 Tauri ↔ HTTP 对照见 [api-reference.md](api-reference.md) 的 Terminal 小节。新增生命周期操作时必须同时更新核心管理器、两套适配、`COMMAND_MAP` 和本文件。

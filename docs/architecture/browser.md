# 浏览器自动化子系统

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/browser/`](../../crates/ha-core/src/browser/)、[`crates/ha-core/src/tools/browser/mod.rs`](../../crates/ha-core/src/tools/browser/mod.rs)、[`src/components/chat/BrowserPanel.tsx`](../../src/components/chat/BrowserPanel.tsx)、[`skills/ha-browser/SKILL.md`](../../skills/ha-browser/SKILL.md)

LLM 看到一个 `browser` 工具，**8 个高层 action**。底层可以是直连 CDP（`chromiumoxide`，零运行时依赖）或 Google 官方 `chrome-devtools-mcp`（stdio MCP 子进程，要求 Node.js ≥ 18）。Backend 在 Chrome 会话首次启动时**按 Node.js 可用性自动选择**，整个生命周期不切换。**ref id、action 名、错误语义在两种 backend 上完全一致**——只有 BrowserPanel 角标会显示当前 backend。

## 8-action 表面

```
status                                           # 当前 backend / 连接 / 活动 tab
profile { op: list|launch|connect|disconnect }   # Chrome 会话生命周期
tabs    { op: list|new|select|close }            # 标签页
navigate { url?, op: go|back|forward|reload }
snapshot { format: role|screenshot|pdf }
act     { kind: click|type|hover|drag|select|fill|press|upload }
observe { kind: console|network|page_errors, since? }
control { op: resize|scroll|wait_for|handle_dialog|evaluate }
```

完整 schema 在 [`tools/definitions/core_tools.rs`](../../crates/ha-core/src/tools/definitions/core_tools.rs)（`TOOL_BROWSER` 段）。工具标记 `default_deferred: true`，常态不进 system prompt，通过 `tool_search` 按需暴露。配套 [`skills/ha-browser/SKILL.md`](../../skills/ha-browser/SKILL.md) 教 agent 标准 loop：`status → tabs → snapshot → act → 必要时 resnapshot`，含登录 / 2FA / captcha / camera prompt / 文件下载等阻塞情形清单（一律 `ask_user_question`）。

## 双 backend 架构

```
┌─────────────────────────────────────────────────────────┐
│ tools/browser/mod.rs  ←─ 8-action dispatch, SSRF guard ─┐│
└────────────────┬───────────────────────────────────────┘│
                 ▼                                         │
         browser::acquire_backend()  ← AppConfig.browser  │
                 │     ── BackendPreference::{Auto|Cdp|Mcp}│
                 ▼                                         │
       ┌─────────┴──────────┐                              │
       ▼                    ▼                              │
   CdpBackend          ChromeMcpBackend                    │
   chromiumoxide       npx chrome-devtools-mcp@latest      │
       │                    │                              │
       ▼                    ▼                              │
    Chrome via CDP      Chrome via CDP (through MCP)       │
                                                           │
   observe_buffer ─── ring buffer: console / network / err │
   frame.rs ─────── BROWSER_FRAME event + capture API ─────┘
```

设计要点：hope-agent **自己**用 chromiumoxide launch / connect Chrome（复用 `browser_state` 全局单例），随后 `ChromeMcpBackend::try_new()` 通过 `--browserUrl http://127.0.0.1:9222` 把同一 Chrome 交给 chrome-devtools-mcp 主控；hope-agent 持有的 chromiumoxide 连接继续做 observe 旁路（Console/Network/Exception → `observe_buffer`）。两个 CDP 客户端共存一个 Chrome 是 CDP 协议支持的多会话场景，互不干扰。

### `BrowserBackend` trait（[`backend.rs`](../../crates/ha-core/src/browser/backend.rs)）

20 个 async method 覆盖 8-action 全部底层操作。共享类型 `ElementRef` / `Snapshot` / `ActKind` / `ActParams` / `ObserveEntry` / `ScreenshotParams` / `PdfParams` 等不依赖具体 backend。`ElementRef.locator` 是 backend 私有字段（CDP 是 CSS selector；MCP 是 chrome-devtools-mcp 的 `uid` 字符串）——8-action 层从不读它，只透传 `ref_id`。

### `CdpBackend`（[`cdp_backend.rs`](../../crates/ha-core/src/browser/cdp_backend.rs)）

包装现有 [`browser_state`](../../crates/ha-core/src/browser_state.rs) 全局单例。`browser_state` 维护 chromiumoxide `Browser` handle、`Page` 池、`active_page_id`、`ElementRef` 表、CDP event handler 任务。`CdpBackend` 是 trait 适配薄壳，不持状态。

**Stale-ref 一次自恢复**：`act` 失败且错误匹配 `is_stale_ref_error`（`not found` / `no such element` / `stale` / `detached`）时，内部触发：

1. 取出当前 `ref_id` 对应的 `role` + `text`
2. 重新 `take_snapshot_inner()` 刷新所有 ref
3. 按 `(role, text)` 精确或模糊匹配找新 ref
4. 用新 ref 重试一次 `act_inner`

成功返回字符串末尾追加 `(ref auto-recovered: old → new)` 让 LLM 知道发生过。**只重试一次**，避免死循环。`navigate` / `tabs.*` / `control.*` 不走 recovery。

### `ChromeMcpBackend`（[`mcp_backend.rs`](../../crates/ha-core/src/browser/mcp_backend.rs)）

设计上 spawn `npx -y chrome-devtools-mcp@latest --experimentalStructuredContent --experimental-page-id-routing`，复用 [`crates/ha-core/src/mcp/`](../../crates/ha-core/src/mcp/) stdio transport 代码（**不**走用户 `mcp_servers` 配置，独立内部 client 实例）。8-action → chrome-devtools-mcp 工具 1:1 映射：

| 8-action | chrome-devtools-mcp 工具 |
| --- | --- |
| `tabs.list/new/select/close` | `list_pages` / `new_page` / `select_page` / `close_page` |
| `navigate.{go,back,forward,reload}` | `navigate_page`（带 type 参数）|
| `snapshot.role` | `take_snapshot` |
| `snapshot.screenshot` | `take_screenshot` |
| `act.{click,fill,hover,drag,upload,press}` | `click` / `fill` / `hover` / `drag` / `upload_file` / `press_key` |
| `act.fill`（多字段）| `fill_form` |
| `control.{evaluate,wait_for,handle_dialog,resize}` | `evaluate_script` / `wait_for` / `handle_dialog` / `resize_page` |

`observe.{console,network,page_errors}` chrome-devtools-mcp **不暴露**——hope-agent 保留对同一 Chrome 的 chromiumoxide 控制平面（`browser_state` 全局单例），`ChromeMcpBackend::try_new` 在握手成功后立刻调用 [`cdp_backend::activate_observe_subscribers_for_all_pages`](../../crates/ha-core/src/browser/cdp_backend.rs) 显式拉起 `Console.messageAdded` / `Network.responseReceived` / `Runtime.exceptionThrown` 三个事件流，喂同一个 [`observe_buffer.rs`](../../crates/ha-core/src/browser/observe_buffer.rs) 单例 ring（500 条上限）。`ChromeMcpBackend::observe` 因此就是一个 buffer 读，不依赖 chrome-devtools-mcp。

**实际 wire-up**：

- spawn 入口 [`mcp_client::spawn(browser_url)`](../../crates/ha-core/src/browser/mcp_client.rs)：复用 [`mcp::transport::build_stdio_client`](../../crates/ha-core/src/mcp/transport.rs) 构造一个**不**进入 `mcp_servers` 配置的内部 rmcp client；外层包 `tokio::time::timeout(60s)` —— 首次 `npx -y` 拉包超时直接返回 `Err`，`backend_select` 回退 CDP；stderr 行流后台 drain → `app_warn!` 避免管道堵塞。
- 子进程 reaper：`ConnectedClient.running: RunningService` 装在 `Arc<dyn BrowserBackend>` 里；`reset_backend()` 清空 `ACTIVE_BACKEND` 触发 Arc Drop，rmcp 内部关闭 stdio，`TokioChildProcess` 的 `kill_on_drop=true` 把 npx/node 子进程回收。
- `uid ↔ ref_id` 映射：`take_snapshot` 重置 `BTreeMap<u32, String>` 并按 chrome-devtools-mcp 返回的 `uid` 树递增分配 LLM 可见的 `ref_id`；`act` 通过反查表把 `ref_id` 转回 `uid`，找不到时直接 `Err("stale ref ... — call snapshot first")`。
- PDF：chrome-devtools-mcp 不支持，`save_pdf` 显式 `bail!`，UI 提示「切换到 Force CDP」。`act.select` 也无原生工具，回退用 `evaluate_script` 触发 input/change 事件。

`reset_backend()` 时机：`profile.launch` / `profile.connect` / 用户在 settings 切换 Backend Preference 都会触发，重新选 backend 时 chrome-devtools-mcp 子进程会被回收并按新配置重启。

### Backend selection（[`backend_select.rs`](../../crates/ha-core/src/browser/backend_select.rs)）

```
BackendPreference  Node available?  → resulting backend
Cdp                     -            → CdpBackend
Mcp                     yes          → ChromeMcpBackend (or error)
Mcp                     no           → error (don't silently downgrade)
Auto (默认)             yes + try_new ok  → ChromeMcpBackend
Auto                    yes + try_new err → CdpBackend (warn-log)
Auto                    no           → CdpBackend
```

`detect_node_available()` 缓存到 `OnceCell`：`which node && node --version >= v18 && which npx`。整个进程只探一次。`ACTIVE_BACKEND` `Mutex<Option<Arc<dyn BrowserBackend>>>` 持有当前 backend；`profile.launch` / `profile.connect` 调 `reset_backend()` 清空 + 重建（同时 `observe_buffer::clear_all`）。

## 实时 BrowserPanel

桌面 app 独占优势——chat 右侧固定 panel，实时镜像 agent 控制的 Chrome 窗口。**事件驱动 + 1s 兜底轮询**：

- **后端 emit**：[`browser::frame::emit_frame_async`](../../crates/ha-core/src/browser/frame.rs) 在每次 `act` / `navigate` / `tabs.new|select` 完成后 fire-and-forget 一次截图（JPEG quality=70），通过 EventBus 发 `browser:frame`
- **前端订阅**：[`BrowserPanel.tsx`](../../src/components/chat/BrowserPanel.tsx) `useEffect` 订阅 `browser:frame` 立即替换帧
- **兜底轮询**：panel 打开期 `setInterval(1000, browser_capture_frame)`，关闭即 clear。覆盖用户在 Chrome 里手动操作的场景
- **互斥**：跟 PlanPanel / DiffPanel / CanvasPanel 互斥（ChatScreen.tsx effect），第一次 `browser:frame` 到来自动开 panel，用户手动关闭后保持关闭
- **Backend 角标**：右上角小 chip 显示 `MCP` / `CDP`，让用户知道走的哪条路

`browser_capture_frame` 同时暴露为 Tauri 命令（[`src-tauri/src/commands/browser.rs`](../../src-tauri/src/commands/browser.rs)）和 HTTP `POST /api/browser/capture-frame`（[`crates/ha-server/src/routes/browser.rs`](../../crates/ha-server/src/routes/browser.rs)），保持 Transport 抽象两端对齐。

## SSRF 守卫

8-action 表面是 SSRF 检查的统一入口，**两个 backend 都受益**。check 走 [`security::ssrf::check_url`](../../crates/ha-core/src/security/ssrf.rs) `cfg.ssrf.browser()` policy + `trusted_hosts`：

| 入口 | 检查内容 |
| --- | --- |
| `navigate.go` | `url` |
| `tabs.new` | `url`（`about:blank` 跳过）|
| `profile.connect` | CDP endpoint `url`（防 agent 让我们连任意远程 9222）|
| `control.evaluate` | regex 扫脚本里的 `"http://..."` / `'https://...'` / `\`https://...\`` 字面量；任一被 policy 拒绝整个 evaluate 拒绝 |

`control.evaluate` 的扫描是 **best-effort**：base64 编码 URL、模板字符串动态拼接、`window.location.host` 之类无法防。skill 文档明确告诉 LLM 这条边界。

## 配置

[`AppConfig.browser`](../../crates/ha-core/src/browser/mod.rs) 三字段，全 optional：

```jsonc
{
  "browser": {
    "backend": "auto",          // "auto" (默认) | "cdp" | "mcp"
    "defaultMode": "managed",   // "managed" (默认) | "user_attach"
    "userAttach": {
      "lastSpawnedPort": 9222   // bookkeeping for "Reconnect" UX
    }
  }
}
```

`browser.backend` 风险等级 **LOW**（仅影响实现选择，行为对 LLM 透明），可走 `update_settings` 工具。`browser.defaultMode` 同样 LOW。`browser.userAttach` 由 settings UI 在 spawn user-attach Chrome 后自动写入。

## 双模式 UX（Settings BrowserPanel）

设置面板提供两条互斥的"模式"路径：

- **独立浏览器**（`AppConfig.browser.defaultMode = "managed"`，默认）：hope-agent 用 [`browser-profiles/{name}/`](../../crates/ha-core/src/paths.rs) 维护的隔离 Chrome 实例做自动化。Launch / Profiles section 控制这条路径。
- **接管用户态 Chrome**（`defaultMode = "user_attach"`）：hope-agent 在 [`browser_user_attach_dir()`](../../crates/ha-core/src/paths.rs)（`~/.hope-agent/browser/user-attach/`）下 spawn 一个**独立 user-data-dir 的 Chrome**，让用户日常使用 + agent 自动化共存，但**不动**用户真正的 Chrome 用户数据。Connect section 的 "doctor" banner + 一键启动按钮驱动这条路径。

四个 Tauri 命令支撑这套 UX：

- `browser_probe_user_chrome`：GET `http://127.0.0.1:9222/json/version`（2s 超时），返回 `{ found, version?, browserUrl }`，doctor banner 用
- `browser_check_chrome_running`：跨平台进程探测（macOS/Linux `pgrep -f ...` / Windows `tasklist /FI`），best-effort 返回 bool 给确认 modal 选用文案
- `browser_spawn_user_chrome`：在 9222 端口 spawn detached Chrome，写 `userAttach.lastSpawnedPort`；port 已占时报错让用户先手动关老 Chrome
- `browser_backend_doctor`：聚合 `detect_node_available` / `probe_node_version` / `peek_active`，给 Backend Radio 渲染「Detected Node {{version}} ✓」标签

Backend Radio (`auto` / `cdp` / `mcp`) 在 settings 切换时立刻 toast「下次 launch/connect 生效」+「立即重连」按钮——后者直接调 `browser_disconnect`，下次 `acquire_backend` 按新偏好选 backend。

## 已落地 vs 跟进项

✅ Backend trait + CdpBackend + ChromeMcpBackend（完整 wire-up） + ObserveBuffer + backend_select
✅ 27 → 8 action 收敛 + schema 重写 + ha-browser bundled skill
✅ Stale-ref one-shot 自恢复（CDP backend）+ chrome-devtools-mcp 错误透传
✅ SSRF 守卫覆盖 navigate / tabs.new / profile.connect / control.evaluate
✅ BROWSER_FRAME 事件 + capture_frame Tauri/HTTP + BrowserPanel 前端 + 12 语言 i18n
✅ AppConfig.browser 字段（backend / defaultMode / userAttach）
✅ Settings BrowserPanel：Mode Radio + doctor banner + 一键启动用户态 Chrome + Backend Radio
✅ ChromeMcpBackend wire-up：内部 rmcp stdio client + uid↔ref 映射 + npx 60s timeout + 进程 reaper + observe 旁路（hope-agent 自持 chromiumoxide 控制面）

跟进项见 [`docs/plans/review-followups.md`](../plans/review-followups.md)。

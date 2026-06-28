# 工具系统架构

> 返回 [文档索引](../README.md)

本文档完整涵盖 Hope Agent 工具系统的定义、分层模型、执行流程、结果持久化和权限控制。

---

## 分层模型（4 层 + 2 特殊路径）

工具系统的概念模型沿用户的"控制粒度"切分，**不是**按内部 flag 组合切。每个工具在定义时声明 `ToolTier`，所有注入决策（schema 是否进 LLM 请求、是否在 system prompt 中描述、是否进 tool_search 检索池）由 [`tools::dispatch::resolve_tool_fate`](../../crates/ha-core/src/tools/dispatch.rs) 单入口派生。

### Tier 1: Core（核心基础）

强制注入，UI 不显示开关。包含 5 个子类，子类只决定注入路径分发，不影响"对用户可见性"：

- **Core::FileSystem** — 文件 / shell：`exec`, `process`, `read`, `write`, `edit`, `ls`, `grep`, `find`, `apply_patch`
- **Core::Interaction** — 交互：`ask_user_question`, `send_attachment`, `task_create`, `task_update`, `task_list`
- **Core::SessionAware** — 跨会话（用户决定不可配置）：`sessions_list`, `session_status`, `sessions_search`, `sessions_history`, `sessions_send`, `peek_sessions`, `agents_list`
- **Core::Meta** — 框架元工具：`tool_search`（`deferredTools.enabled=true` 且 `toolNames` 非空，或存在 `McpServerConfig.deferredTools=true` 的 server 时注入）, `job_status`（仅 `asyncTools.enabled` 时注入）, `schedule_wakeup`（agent 自我定时唤醒，一次性 N 秒后注 `<wakeup>`+note 回当前会话续跑，复用注入管线；`internal`=不弹审批，`crate::wakeup` / `wakeups.db`；详见 AGENTS「Subagent / Team / Cron」节）, `runtime_cancel`, `skill`
- **Core::PlanMode** — Plan Mode 触发：`submit_plan`, `update_plan_step`, `amend_plan`（dispatcher 永远返回 Hidden，由 `apply_plan_tools` 按 PlanAgentMode 单独注入）

### Tier 2: Standard（标准工具）

Agent 默认开启、用户可在 Agent 设置里关闭。每个工具在定义时声明 `default_for_main` / `default_for_others` 两个默认值——前者作用于硬编码主 agent（`agent_id == "ha-main"`，即 `agent_loader::DEFAULT_AGENT_ID`），后者作用于其他新建 agent：

| 工具 | main | others | defer_capable |
|---|---|---|---|
| `web_fetch` / `browser` / `manage_cron` | ✓ | ✓ | false |
| `team` / `pdf` / `image` / `get_weather` | ✓ | ✓ | **true** |
| `get_settings` / `update_settings` | ✓ | ✗ | false |
| `mac_control` | ✓ | ✗ | **true** |
| `list_settings_backups` / `restore_settings_backup` | ✓ | ✗ | **true** |

设置类工具是唯一的"主 agent 默认开 / 新 agent 默认关"子类。`defer_capable=true` 表示该工具支持被用户放入 deferred 池；默认仍直接注入。

### Tier 3: Configured（需要全局配置）

Agent 层有开关，但即使开了，全局 provider 没配也不真正注入；此时在系统提示词的 `# Unconfigured Capabilities` 段提示用户去配置：

| 工具 | main | others | defer_capable | config_hint |
|---|---|---|---|---|
| `web_search` | ✓ | ✓ | false | Settings → Tools → Web Search |
| `image_generate` | ✓ | ✓ | false | Settings → Tools → Image Generation |
| `canvas` | ✓ | ✓ | false | Settings → Tools → Canvas |
| `send_notification` | ✓ | ✓ | false | Settings → Tools → Notifications |
| `subagent` | ✓ | ✓ | false | Settings → Agents |
| `acp_spawn` | ✓ | ✗ | **true** | Settings → Agents → ACP |

### 特殊路径 1: Memory

记忆工具（`save_memory`, `recall_memory`, `update_memory`, `delete_memory`, `memory_get`, `update_core_memory`）由 agent 级 `memory.enabled` 控制——开启 → 6 个工具全部注入；关闭 → 全部不注入。UI 不显示这 6 个工具的单独开关，记忆能力作为整体管理。

### 特殊路径 2: MCP

`agent.json` `capabilities.mcpEnabled`（默认 `true`）控制：开启时 MCP 内置元工具（`mcp_resource` / `mcp_prompt`）注入，动态 `mcp__<server>__<tool>` 默认 eager 注入；若单个 MCP server 设置 `deferredTools=true`，该 server 的动态工具改由 `tool_search` 按需发现。关闭时 dispatcher 把这些工具一并 `Hidden`（不注入、不进 `tool_search` 池、不生成 `# Unconfigured Capabilities` 提示）；同时 `agent::build_tool_schemas` / `tool_search` 跳过整个 `mcp_tool_definitions()` 动态目录。

---

## 工具定义

每个工具由 `ToolDefinition` 结构体定义（[`tools/definitions/types.rs`](../../crates/ha-core/src/tools/definitions/types.rs)）：

```rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,    // JSON Schema
    pub tier: ToolTier,       // 单一真相源（Core / Standard / Configured / Memory / Mcp）
    pub internal: bool,       // 与 tier 正交：是否豁免审批
    pub concurrent_safe: bool,// 同轮可并行
    pub async_capable: bool,  // 可被 detach 成后台 job（schema 自动注入 run_in_background）
}
```

`tier` 是注入决策的单一真相源。`internal` 与 tier **正交**——`exec` / `write` 是 Tier 1 Core::FileSystem 但 `internal=false`（修改系统、需用户确认审批），`recall_memory` / `task_list` 也是 Tier 1 但 `internal=true`（自治只读能力）。

### 决策表（dispatch::resolve_tool_fate）

| Tier | 用户开关 | 全局配置 | 派生命运 |
|---|---|---|---|
| Core (FileSystem/Interaction/SessionAware) | — | — | InjectEager |
| Core::Meta `tool_search` | — | (`deferredTools.enabled=true` 且 `toolNames` 非空) 或 deferred MCP server | InjectEager / Hidden |
| Core::Meta `job_status` | — | `asyncTools.enabled` | InjectEager / Hidden |
| Core::Meta `skill` / `runtime_cancel` | — | — | InjectEager |
| Core::PlanMode | — | — | Hidden（由 PlanAgentMode 二次注入）|
| Memory | `agent.memory.enabled` | — | InjectEager / Hidden |
| Mcp | `agent.capabilities.mcpEnabled` | — | InjectEager / Hidden |
| Standard | `tools.allow` 显式开 / `tools.deny` 显式关 | `deferredTools.enabled && toolNames contains name` | InjectEager / InjectDeferred / Hidden |
| Configured | `tools.allow` 显式开 / `tools.deny` 显式关 | provider 是否就绪 + `deferredTools.enabled && toolNames contains name` | InjectEager / InjectDeferred / HintOnly / Hidden |

### 派生方法（不再有独立 bool）

旧的 `deferred` / `always_load` 字段已删除，由 tier + `AppConfig.deferredTools` 派生：

```rust
impl ToolDefinition {
    pub fn is_internal(&self) -> bool { self.internal }
    /// Standard/Configured 中 `default_deferred=true` 的工具；表示该工具
    /// 允许在 `deferredTools.toolNames` 列入后进入 deferred 池。
    pub fn supports_deferred(&self) -> bool { /* tier::Standard/Configured 的 default_deferred 字段 */ }
    pub fn is_always_load(&self) -> bool { !self.supports_deferred() }
    pub fn is_core(&self) -> bool { matches!(self.tier, ToolTier::Core { .. }) }
}
```

注：`to_api_metadata()`（供前端 settings UI 使用）会把 `default_deferred` 渲染为 `defer_capable` 字段。

### 并发安全标记

`concurrent_safe: bool` 决定工具是否可在同一轮次内与其他工具并行执行：

| 并发安全（parallel） | 串行执行（sequential） |
|---------------------|----------------------|
| read, ls, grep, find | exec, write, edit, apply_patch |
| peek_sessions | process, send_attachment |
| recall_memory, memory_get | save_memory, update_memory, delete_memory |
| web_search, web_fetch | browser, subagent, canvas |
| agents_list, sessions_list | image_generate, sessions_send |
| session_status, sessions_history | update_core_memory, manage_cron |
| image, pdf, get_weather | send_notification, acp_spawn, team |
| ask_user_question, task_list | task_create, task_update |
| mcp_resource, mcp_prompt | submit_plan, amend_plan, update_plan_step |
| | tool_search, job_status, skill, runtime_cancel, get_settings, update_settings, list_settings_backups, restore_settings_backup |

> 这个表只是常见示例。每个工具的并发安全性是 `ToolDefinition.concurrent_safe` 字段——`tools::is_concurrent_safe(name)` 是单一查询入口，由 `dispatch::all_dispatchable_tools()` 派生缓存。

---

## 内置工具清单

本节枚举 Hope Agent 当前内置的全部工具（源码：`crates/ha-core/src/tools/definitions/`）。

标记含义：

- **always_load**：一定会加载到 tool schema，不受 `deferredTools.toolNames` 影响（Tier 1 Core 全部、Memory / Mcp 在各自 gate 打开时、以及 `supports_deferred()=false` 的 Tier 2/3 工具）
- **deferred**：`ToolDefinition::supports_deferred()=true`，**允许**被用户放进 deferred 池——只有当 `deferredTools.enabled=true` 且工具名出现在 `deferredTools.toolNames` 时才真延迟，schema 不发送给 LLM，需通过 `tool_search` 元工具按需发现
- **internal**：`is_internal_tool()` 返回 true，**永不弹审批**（条件注入时依然遵守 Agent 权限过滤）
- **concurrent_safe**：同一轮 tool_call 可与其他安全工具并行执行（见上一节表格）
- **async_capable**：支持 `run_in_background: true` 参数把整轮调用 detach 成后台 job，并支持 `job_timeout_secs` 单次收紧后台 job 外层超时；详见 [异步 Tool 执行](#异步-tool-执行async_capable) 小节
- **条件注入**：只有在对应能力开关/全局配置/上下文（Tier 3 / Memory / Mcp / PlanMode 等）满足时才加入 tool schema

### 1. Shell 执行与进程管理

| 工具 | 类别 | 标记 | 说明 |
|------|------|------|------|
| `exec` | Shell | always_load, **async_capable** | 执行 shell 命令，返回 stdout/stderr。参数：`command` (必填)、`cwd`、`timeout`（秒，默认 `0` = 不限制 exec 命令超时，正数上限 7200）、`env`、`run_in_background`（普通长跑命令的首选后台方式，后续用 `job_status` / 完成注入查）、`job_timeout_secs`（单次设置 / 收紧 async job 外层超时）、`background` / `yield_ms`（legacy exec process-session 兼容面，后续用 `process` 查；默认尽量不用）、`pty`、`sandbox`（Docker 沙箱）。有独立的命令级审批流程（见 exec 流程图）。 |
| `process` | Shell | always_load | 管理 `exec` 创建的后台会话。`action`：`list` / `poll`（按 timeout 等待）/ `log`（含 offset/limit 分页）/ `write`（向 stdin 写入）/ `kill` / `clear` / `remove`。除 `list` 外均需 `session_id`。 |

### 2. 文件系统

Path-aware 工具统一使用 `ToolExecContext` 解析默认路径：显式绝对路径保持不变；相对路径先落到当前 session 的 `working_dir`，没有时落到 Agent home，再没有时使用进程当前目录。`exec` 的默认 cwd 同样优先使用 session `working_dir` / Agent home，但最后一层回退是用户 home，保持 shell 命令的历史行为。

| 工具 | 标记 | 说明 |
|------|------|------|
| `read` | always_load, concurrent_safe | 读取文件内容。支持行号分页（`offset` / `limit`），自动识别图片文件并以 base64 返回。兼容 `file_path` 别名。 |
| `write` | always_load | 写入文件（覆盖/创建），自动建父目录。兼容 `file_path` 别名。 |
| `edit` | always_load | 精确字符串替换。`old_text` 必须在文件中唯一匹配。兼容 `file_path` / `oldText` / `old_string` / `newText` / `new_string` 别名。 |
| `ls` | always_load, concurrent_safe | 列目录，返回排序条目（`/` 标记目录、`@` 标记符号链接）。支持 `~` 展开、`limit`（默认 500）。 |
| `grep` | always_load, concurrent_safe | 正则/字面量内容搜索，尊重 `.gitignore`。支持 `glob` 过滤、`ignore_case`、`literal`、`context`（上下文行数）、`limit`（默认 100）。 |
| `find` | always_load, concurrent_safe | 按 glob 模式查找文件，尊重 `.gitignore`。`limit` 默认 1000。 |
| `apply_patch` | always_load | 使用 `*** Begin Patch / *** End Patch` 格式批量创建/修改/删除/移动文件。支持 `Add File` / `Update File`（`@@` 上下文 + `-/+` 行）/ `Delete File` / `Move to` hunk。 |

### 3. Web

| 工具 | 标记 | 说明 |
|------|------|------|
| `web_fetch` | deferred, concurrent_safe | 抓取 URL 并用 Mozilla Readability 提取正文。`extract_mode`：`markdown`（默认，保留链接/标题/列表）或 `text`。`max_chars` 受服务器端上限约束。 |
| `web_search` | 条件注入, concurrent_safe, **async_capable** | 网络搜索（需在设置中启用 Web Search）。参数：`query` (必填)、`count`、`country`（ISO 3166-1 alpha-2）、`language`（ISO 639-1）、`freshness`（`day`/`week`/`month`/`year`）、`run_in_background`、`job_timeout_secs`。不同 provider（Bocha / Brave / SearXNG / Perplexity / Google / Tavily）支持的过滤参数不同。 |

### 4. 记忆系统

均为 internal（永不审批），在 SQLite + FTS5 + 向量检索后端上操作。

| 工具 | 标记 | 说明 |
|------|------|------|
| `save_memory` | deferred, internal | 保存长期记忆。`type`：`user` / `feedback` / `project` / `reference`。`scope`：`global`（默认）或 `agent`。`pinned=true` 时始终进入系统提示不受年龄排序影响。支持 `tags`。 |
| `recall_memory` | deferred, internal, concurrent_safe | 关键词/语义检索。可按 `type` 过滤，`include_history=true` 同时搜索历史对话消息。 |
| `memory_get` | deferred, internal, concurrent_safe | 按 ID 获取单条记忆的完整内容与元数据。 |
| `update_memory` | deferred, internal | 按 ID 更新记忆 `content` 与 `tags`（tags 省略即清空）。 |
| `delete_memory` | deferred, internal | 按 ID 删除记忆。 |
| `update_core_memory` | deferred, internal | 更新常驻系统提示的 core memory 文件（`memory.md`）。`action`：`append` / `replace`；`scope`：`global` / `agent`（默认 `agent`）。 |

### 5. 定时任务

| 工具 | 标记 | 说明 |
|------|------|------|
| `manage_cron` | deferred, internal | 管理 Cron/Scheduled Tasks。`action`：`create` / `list` / `get` / `delete` / `pause` / `resume` / `run_now`。调度类型：`at`（ISO8601 单次）/ `every`（毫秒间隔，最小 60000；可选 `start_at` 指定首个触发时间，省略时后端自动锚定到“当前时刻 + interval”）/ `cron`（cron 表达式 + 可选 `timezone`）。`prompt` 为触发时执行的 agent 指令（隔离会话、无历史）；`agent_id` 默认当前 agent。 |

### 6. 浏览器控制

| 工具 | 标记 | 说明 |
|------|------|------|
| `browser` | deferred | 通过 Chrome DevTools Protocol 驱动浏览器。`action` 覆盖：`connect` / `launch`（可指定 `executable_path` / `headless` / `profile`）/ `disconnect`，页面管理（`list_pages` / `new_page` / `select_page` / `close_page`）、导航（`navigate` / `go_back` / `go_forward`）、快照（`take_snapshot` 返回元素 ref、`take_screenshot` 支持 `full_page`）、交互（`click`/`double_click`/`fill`/`fill_form`/`hover`/`drag`/`press_key`/`upload_file`）、脚本（`evaluate` / `wait_for`）、对话框（`handle_dialog`）、视口（`resize` / `scroll`）、Profile 隔离（`list_profiles`）、`save_pdf`（含 paper_format / landscape / print_background）。`new_page` 现在是默认入口：复用当前显式连接（`connect` / `profile=user_attach`）或自动托管启动 `managed`，不会隐式接管 `127.0.0.1:9222` 上的随机 Chrome。托管启动默认关闭 chromiumoxide 固定 `800x600` viewport 仿真，并以 `1440x960` 大窗口起步，所以首次开页更接近真实浏览器，用户手动拖拽窗口时页面也能自然自适应；`resize` 只在需要固定 viewport 时使用。 |

### 6b. macOS 控制

| 工具 | 标记 | 说明 |
|------|------|------|
| `mac_control` | deferred | 原生 macOS 桌面控制能力。当前支持 `action=status|permissions|diagnostics|snapshot|visual|elements|wait|apps|dock|spaces|windows|act|menu|clipboard|dialog`；`diagnostics.summary/export` 只读返回 readiness、snapshot cache 摘要、recent errors 和 focus anchor，export 写入 `~/.hope-agent/mac-control/diagnostics/`；`snapshot` 返回前台 App / 窗口 / AX 元素摘要，`visual.observe annotate=true` 可生成标注截图和紧凑 `uiMap`，`elements.find` 只读返回排序 AX 候选、score 和 reasons，`wait` 支持 `present|gone` 轮询 AX snapshot。`apps` 支持 list/frontmost/installed/search/activate/launch/quit；`dock` 支持 list/launch/hide/show/menu/select_menu；`spaces` 支持 list/switch/move_window（SkyLight/CGS 私有 API）；`windows` 支持 frontmost/all 窗口发现与 list/focus/move/resize/minimize/close；`act` 支持 dry_run/perform_action/click/click_point/move_cursor/double_click/right_click/type/paste/set_value/hotkey/press/scroll/drag/swipe，其中 dry_run 可用 `dryRunOp` 返回结构化 preview，perform_action 只做 action 名称格式校验，系统不支持时返回 AX error；`menu` 支持 app/system 两种 scope 的 list/click/popover；`clipboard` 支持 get/set/clear UTF-8 文本且需审批；`dialog` 支持 list/inspect/click/input/file/accept/dismiss。桌面 Tauri 注册 bridge，server/headless/HTTP 返回 `supported=false`。 |

`mac_control` 的 schema 是单工具多 `action/op` 形态，因此执行层必须按 op 契约解释参数，而不是让共享字段互相串味：

- 执行层会在权限判断和审批前做 action/op 级 sanitize + preflight；无效参数直接失败，不弹审批；Provider 默认填入的共享字段不能覆盖显式 op 意图。
- `act.click` 只接受 AX `target`，裸坐标必须用 `act.click_point`；`(0, 0)` 是合法坐标，靠 op 区分意图。
- `act.dry_run` 只解析 target，不产生 UI 副作用；`dryRunOp` 指明要预演的真实 act op，返回 `preview.executionPlan/fallbackPlan/verificationPlan/warnings`，用于 mutation 前确认目标元素、fallback 和验证策略。
- `target.elementId` 最好和产生它的 `target.snapshotId` 一起传；mutation 会用旧 snapshot 中的 role/label/value/window/bounds 指纹在当前 AX 树重定位，并在无显式 app filter 时绑定旧前台 App，过期、跨 App 或歧义时拒绝执行。
- `act.type/paste/set_value`、`act.move_cursor/drag/swipe` 和 `windows.focus/move/resize/close` 会尽量返回结构化 `verification`，区分 `verified`、`failed`、`unverified`；模型不能把无业务期望的普通点击自动当作已完成。
- 关键动作有受控 fallback：`act.click` / dialog 按钮优先 `AXPress`，失败且有 bounds 时回退中心点点击；`act.type/set_value` 和 dialog 替换式输入优先 `AXValue`，失败后聚焦、全选并用 pasteboard 替换；`menu.click` 优先 `AXShowMenu -> AXPress -> CGEvent`。
- `act.perform_action` 需要 `target` + `axAction`；常用别名会规范化，其它合法 AX action 名称直接交给 Accessibility 执行，不再依赖目标 `actions[]` 广告。
- `dock.launch/menu/select_menu` 优先用 `dockItemId` 或 `bundleId`；`dock.select_menu` 同时收到 `menuItem` 和 `menuIndex` 时以 `menuItem` 为准，index-only 选择走严格审批；`dock.hide/show` 会写 `com.apple.dock autohide` 并重启 Dock。
- `spaces.switch` 支持 `direction=left|right` 或 1-based `spaceIndex` / `spaceId`，`direction` 和相邻目标优先走 Mission Control `Control+Left/Right`；非相邻精确目标再 fallback 到 Control+数字或 SkyLight/CGS。
- `appNameMatch` 默认 `exact`，`contains` 只用于发现或明确的模糊匹配；有副作用操作优先使用 `bundleId`。
- `target.windowTitleMatch` 默认 `exact`，多个相似窗口时优先使用最新 `windows.list` / `snapshot` 里的 `windowId`。
- `dialog.inspect/list` 返回当前前台 App 的 dialog/sheet 摘要，包含按钮和字段；`dialog.click/accept/dismiss/file` 可用 `buttonText` / `selectButton` 精确按钮，`dialog.input` 用 `field` / `fieldIndex` / `target.elementId` 精确字段。
- 只读 op：`status`、`permissions`、`diagnostics.summary/export`、`snapshot`、`elements.find`、`wait`、`apps.list/frontmost/installed/search`、`dock.list`、`spaces.list`、`windows.list`、`act.dry_run`、`menu.list/popover`、`dialog.inspect/list`。
- 普通突变 op 进入审批：`apps.activate/launch`、`dock.launch/hide/show/menu`、安全 `dock.select_menu menuItem`、`spaces.switch/move_window`、`windows.focus/move/resize/minimize`、除 `dry_run` 外的 `act.*`、普通 `menu.click`、普通 `dialog.click/input/file/dismiss`。
- 高风险突变 op 进入严格审批且禁用 Allow Always：`apps.quit`、`windows.close`、`dialog.accept`、`act.perform_action axAction=AXConfirm`、危险菜单路径或 dialog 按钮（delete / trash / reset / discard 等中英文关键词）、危险或 index-only `dock.select_menu`。
- 审批前会捕获当前 frontmost App 和 focused window；审批通过或超时继续时，执行层会在真正执行 `mac_control` 前 best-effort 恢复该 App，并按 pid-scoped window id / 窗口标题恢复原窗口，避免审批 UI 抢焦点后把 frontmost 依赖动作送到 Hope Agent。

### 7. 多模态（输入/生成）

| 工具 | 标记 | 说明 |
|------|------|------|
| `image` | deferred, internal, concurrent_safe | 视觉输入附件。单图 shorthand：`path` 或 `url`；多图走 `images: [{type, label?, ...}]`（数量受工具限制配置约束，type 可为 `file`/`url`/`clipboard`/`screenshot`，screenshot 可指定 `monitor`）。支持 PNG/JPEG/GIF/WebP/BMP/TIFF，自动缩放过大图片，图片作为下一轮 Provider 视觉输入；用 `task` / `question` 描述检查目标，`prompt` 仅作兼容别名。 |
| `pdf` | deferred, internal, concurrent_safe | PDF 文本提取或视觉解析。`mode`：`auto`（默认，优先文本提取，扫描件自动回退 vision）/ `text` / `vision`。支持 `path`/`url` 单文件或 `pdfs` 数组（默认最多 5，上限 10）。`pages` 支持 `1-5,7,10-12` 语法，`max_chars` 控制文本模式输出长度。 |
| `image_generate` | 条件注入, **async_capable** | 文生图 / 图生图。`action`：`generate`（默认）/ `list`（列出已启用 provider 与能力）。参数（随启用 provider 动态）：`prompt`、`image`/`images`（参考图）、`size`、`aspectRatio`、`resolution`（`1K`/`2K`/`4K`）、`n`、`model`、`run_in_background`、`job_timeout_secs`。默认 `auto`，按优先级顺序失败自动降级。图片落盘并附到消息。 |

### 8. 会话与跨会话通信

| 工具 | 标记 | 说明 |
|------|------|------|
| `agents_list` | always_load, internal, concurrent_safe | 列出全部可用 Agent 及描述/能力。用于选 target agent 下发 subagent。 |
| `sessions_list` | always_load, internal, concurrent_safe | 列出会话（title / agent / model / 消息数）。可按 `agent_id` 过滤，`include_cron=true` 包含 cron 触发会话。默认 limit 20，上限 100。 |
| `session_status` | always_load, internal, concurrent_safe | 查询单个会话的 agent / model / 消息数 / 时间戳。 |
| `sessions_search` | always_load, internal, concurrent_safe | FTS 检索会话消息并返回命中附近上下文窗口。默认当前会话；`scope=all` 只搜全局可见的普通非无痕会话（排除 cron / subagent / channel / knowledge）；`limit` 默认 8（上限 20），`before/after` 控制窗口大小，`include_tools=false` 默认剔除工具细节。压缩后回查具体信息优先用它。 |
| `sessions_history` | always_load, internal, concurrent_safe | 分页读取某会话的历史消息。`limit` 默认 50（上限 200），`before_id` 游标，`include_tools=false` 默认剔除 tool 细节以降噪。 |
| `sessions_send` | always_load, internal | 向其他会话发送 user 消息。`wait=true` 时阻塞直到目标 agent 回复（`timeout_secs` 默认 60，上限 300）。 |
| `peek_sessions` | always_load, internal, concurrent_safe | 跨会话感知窥探。返回其它会话的紧凑 markdown 列表（title / agent / kind / 相对时间 / goal/summary）。参数：`query`（可选子串过滤 title/goal）、`limit`（默认 6，上限 20）。只读。 |

### 9. Agent 调用

| 工具 | 标记 | 说明 |
|------|------|------|
| `subagent` | 条件注入 | 调用并管理子 Agent。`action`：`spawn` / `check`（可 `wait=true` + `wait_timeout` 阻塞）/ `list` / `result` / `kill` / `kill_all` / `steer`（向运行中子 Agent 注入 user 消息纠偏）/ `batch_spawn`（数组 `tasks`）/ `wait_all`（数组 `run_ids`）/ `spawn_and_wait`（`foreground_timeout` 默认 30s，超时自动转后台）。支持 `model` 覆盖、`label` 追踪、`files` 文件附件（UTF-8 / base64）。`timeout_secs` 省略时使用父 Agent 默认（产品默认 `0` = 不超时），显式 `0` 也不超时，正数上限 1800。子 Agent 完成结果自动推送回父会话。 |
| `team` | deferred, internal | Agent Team 多成员协作。`action`：`list_templates`（发现用户预配的模板）/ `create`（支持 `template="<id>"` 一键实例化或 `members=[{name, task, agent_id?, role?, description?}]` 内联）/ `dissolve` / `add_member` / `remove_member` / `send_message` / `create_task` / `update_task` / `list_tasks` / `list_members` / `status` / `pause` / `resume`。成员底层复用 subagent 执行，每个成员可绑定独立 Agent + 模型 + role identity；共享任务板和跨成员消息。 |
| `acp_spawn` | 条件注入 | 调用外部 ACP Agent（Claude Code / Codex CLI / Gemini CLI 等）。`action`：`spawn` / `check` / `list` / `result` / `kill` / `kill_all` / `steer` / `backends`。参数：`backend`（必填）、`task`、`cwd`、`model`、`timeout_secs`（默认 600，上限 3600）、`label`。外部进程有独立工具集与上下文。 |

### 10. Plan Mode

详见 [Plan Mode 文档](plan-mode.md)。这些工具均为 internal（不审批），且根据 Plan 状态条件注入。

| 工具 | 标记 | 注入时机 | 说明 |
|------|------|---------|------|
| `submit_plan` | internal | Planning/Review Agent | 提交最终计划，触发进入 Review 状态。参数：`title`、`content`（markdown：`## Background` + 若干 `### Phase N: <title>` + `- [ ]` 清单）。 |
| `update_plan_step` | internal | Executing/Paused Agent | 执行期更新单步状态。`step_index` 零基 + `status`（`in_progress`/`completed`/`skipped`/`failed`）。 |
| `amend_plan` | internal | Executing/Paused Agent | 执行期修改计划。`action`：`insert`（可指定 `after_index`）/ `delete` / `update`，支持 `title` / `description` / `phase`。 |

### 11. 通用结构化问答

| 工具 | 标记 | 说明 |
|------|------|------|
| `ask_user_question` | always_load, internal, concurrent_safe | 任意对话内向用户发起结构化问答。参数：`questions[]`（建议 1–4 条，每条含 `question_id`、`text`、`header` chip 标签、`options`（2–4 条，每项可选 `recommended`、`description`、`preview` + `previewKind`=`markdown`/`image`/`mermaid`）、`allow_custom`（默认 true，当前运行时强制覆盖为 true）、`multi_select`（默认 false）、`template`（`scope`/`tech_choice`/`priority`）、`timeout_secs`、`default_values`）、`context`。Pending 持久化到 session SQLite，App 重启后重放；IM 渠道按 `supports_buttons` 发送原生按钮或 `1a`/`done`/`cancel` 文本 fallback。 |

### 12. 会话级任务追踪（TODO）

均为 internal（不审批），作用域为当前会话。

| 工具 | 标记 | 说明 |
|------|------|------|
| `task_create` | always_load, internal | 创建可追踪的任务，返回完整任务列表。参数：`content`（祈使句描述）。 |
| `task_update` | always_load, internal | 按 `id` 更新任务。`status`：`pending`/`in_progress`/`completed`；可更新 `content`。返回完整列表。 |
| `task_list` | always_load, internal, concurrent_safe | 返回当前会话所有任务的 JSON。 |

### 13. Canvas 画布

| 工具 | 标记 | 说明 |
|------|------|------|
| `canvas` | 条件注入, internal | 在沙箱预览面板创建/管理可视化项目。`action`：`create` / `update` / `show` / `hide` / `snapshot`（截图当前渲染状态供模型分析）/ `eval_js`（执行 JS）/ `list` / `delete` / `versions` / `restore` / `export`。`content_type`：`html` / `markdown` / `code` / `svg` / `mermaid` / `chart`（Chart.js）/ `slides`。支持 `html` / `css` / `js` / `content` / `language` / `version_id` / `version_message` / 导出 `format`（`html`/`markdown`/`png`）。Plan Mode 默认禁用（在 `PLAN_MODE_DENIED_TOOLS`）。 |

持久化：[`crates/ha-core/src/canvas_db.rs`](../../crates/ha-core/src/canvas_db.rs)（`Versions` 表 + `restore` 走版本历史）。

### 14. 桌面集成

| 工具 | 标记 | 说明 |
|------|------|------|
| `send_notification` | 条件注入, internal | 发送系统原生桌面通知。参数：`title`、`body`（必填）。用于主动提醒任务完成或需要用户注意的事件。 |
| `send_attachment` | always_load, internal | 把生成的文件以可下载卡片形式推送到桌面 UI（PDF / 压缩包 / 日志等二进制）。参数：`path`（必填，绝对路径，上限 20 MB）、`display_name`、`description`。自动复制到 `~/.hope-agent/attachments/{session_id}/`，卡片支持打开 / 文件管理器定位。IM 渠道会话不可用（由渠道插件的原生媒体发送代替）。 |
| `get_weather` | deferred, internal, concurrent_safe | 通过 Open-Meteo 获取天气（免 API key）。`location` 支持城市名或 `latitude,longitude`，省略时使用用户配置位置。`forecast_days` 1–16（默认 1）。 |

### 15. 元工具

| 工具 | 标记 | 说明 |
|------|------|------|
| `tool_search` | always_load, internal | 延迟工具发现（存在内置 deferred 工具或 deferred MCP server 时启用）。`query`：`select:name1,name2` 精确选取或关键词模糊检索。`max_results` 默认 5，上限 20。返回 deferred 工具完整 schema 以便后续直接调用。 |
| `job_status` | always_load, internal | 多作业管理面（R5）：`action ∈ status\|list\|wait\|cancel\|result`，签名 `tool_job_status(args, session_id)`。`status`(默认，单 `job_id`，向后兼容)；`list`(枚举本会话在途 active jobs，`list_active_by_session`，封顶 `MAX_WAIT_TARGETS=32`)；`wait{ids?,mode:all\|any,timeout_ms}`(短便利同步，clamp ≤ `MAX_BLOCK_WAIT_SECS=10s`，超 clamp 返回 `still_running` + 引导走注入路径**绝不长阻塞**，未知 id 记 `settled:unknown` 防永等)；`cancel(id)`(复用 `async_jobs::cancel_job`)；`result`(=status)。**长 fan-out 等齐的正道是注入而非 `wait`**——`batch_spawn` 的 Group（R5）等齐后**合并注入一轮**；`status(job_id=<group>)` 返回 N-of-M 子进度。完成结果主要依赖 `<task-notification>` 自动注入，`job_status` 只用于用户追问或经过一段时间后的非阻塞状态快照，**禁止用"后台化后立即 poll"来重建同步等待**。running/cancelling 响应带 `polling_guidance.should_poll_again_this_turn=false` 与 `next_check_after_secs`，提示模型继续独立工作或停轮等待自动注入。实现仍兼容隐藏 `block=true` / `timeout_ms` 旧参数，但只作为短等待逃生口：默认 5s，最大 10s，且仍受 `AsyncToolsConfig::job_status_ceiling_secs()` 的运行时上限约束。阻塞模式下向 per-job `tokio::sync::Notify` 注册表登记等待者，`tokio::select!` 于 `notified()` 与指数退避轮询（`INITIAL_BACKOFF=100ms` → ×1.5 → `MAX_BACKOFF=2s`）之间择一触发；`finalize_job` 写完 DB 后 `notify_waiters()` 唤醒所有等待者。`register_waiter` 之后强制 recheck DB 关闭"register 之前已 commit"和"重启回放后 in-memory registry 空"两个 race。结果从独立的 `background_jobs.db` 读出预览/磁盘路径/错误。仅当 `asyncTools.enabled = true` 时注入。 |

---

## 延迟工具加载（Deferred Tools）

`deferredTools.enabled` 是 opt-in 总开关（默认 **false**）。打开它只启用 `tool_search` 机制，不会自动延迟任何内置工具；真正进入 deferred 池的内置工具由 `deferredTools.toolNames` 显式列出。因此默认行为始终是：所有可注入工具直接发送给 LLM，用户再逐个把高成本/低频工具改成按需发现。

### Core（always_load，始终发送 schema）

Tier 1 Core 工具不支持 `deferredTools.toolNames` 延迟；它们由 tier 子类直接决定可见性。Memory / MCP 内置元工具是单独的 tier（不是 Core），但只要对应 agent 开关 `memory.enabled` / `capabilities.mcpEnabled` 打开，它们也始终 eager。

| 类别 | 工具 |
|------|------|
| 文件操作 | `read`, `write`, `edit`, `apply_patch`, `ls`, `grep`, `find` |
| Shell 执行 | `exec`, `process` |
| 人机交互 | `ask_user_question`, `send_attachment` |
| 任务跟踪 | `task_create`, `task_update`, `task_list` |
| 跨会话 | `agents_list`, `sessions_list`, `session_status`, `sessions_search`, `sessions_history`, `sessions_send`, `peek_sessions` |
| 技能入口 | `skill`（`Core::Meta`，恒注入） |
| 运行时控制 | `runtime_cancel`（`Core::Meta` + `internal = true`，async_job / subagent / process / cron 统一取消入口） |

此外 `tool_search` 和 `job_status` 两个 `Core::Meta` 元工具按 feature flag 条件注入：前者只在 (`deferredTools.enabled=true` 且 `toolNames` 非空) 或存在 deferred MCP server 时注入，后者只在 `asyncTools.enabled` 打开时注入。

### Deferred（按需通过 `tool_search` 发现）

支持 deferred 的内置工具由 `ToolDefinition::supports_deferred()` 标记，当前主要是低频/高 schema 成本的 Standard / Configured 工具。只有当 `deferredTools.enabled=true` 且工具名出现在 `deferredTools.toolNames` 时，dispatcher 才返回 `InjectDeferred`。

| 子域 | 工具 |
|------|------|
| 设置备份 | `list_settings_backups`, `restore_settings_backup`（`get_settings` / `update_settings` 不在 deferred 候选——为了主 agent 主动改配置时随时可用）|
| 子 Agent / Team | `team`, `acp_spawn` |
| 多模态 | `image`, `pdf` |
| 天气 | `get_weather` |
| MCP 动态工具 | 按 MCP server 的 `deferredTools=true` 设置整台 server 延迟 |

### 发现机制

```mermaid
flowchart LR
    A[模型需要记忆操作] --> B[tool_search<br/>query 'memory recall']
    B --> C[返回 top N 匹配 schema<br/>recall_memory / memory_get...]
    C --> D[模型下一轮直接调用<br/>recall_memory query '...']
    D --> E[execution.rs 正常 dispatch]
```

`query` 支持两种形式：
- `select:name1,name2`：按名字精确挑选（`max_results` 上限 20）
- 关键词：在 name + description 上做模糊检索，返回 top N（默认 5）

### 判定与标记

单一真源是 [`tools::dispatch::resolve_tool_fate`](../../crates/ha-core/src/tools/dispatch.rs)：它同时读取 tier、agent capability、全局 provider 配置、`deferredTools.enabled` 和 `deferredTools.toolNames`，决定 `InjectEager` / `InjectDeferred` / `HintOnly` / `Hidden`。

### 配置

`AppConfig.deferred_tools`（`config.json` → `deferredTools`）：

| 字段 | 默认 | 含义 |
|------|------|------|
| `enabled` | `false` | 总开关。关闭时内置工具不走 deferred |
| `toolNames` | `[]` | 显式延迟的内置工具名列表；默认空，因此即使总开关打开也不会自动延迟内置工具 |

UI 入口：设置 → 工具 → Deferred Tools。`ha-settings` 技能：`update_settings(category="deferred_tools", values={enabled: true, toolNames: ["pdf"]})`。

---

## Schema 组装流程

每轮 LLM 请求前，[`AssistantAgent::build_tool_schemas(provider)`](../../crates/ha-core/src/agent/mod.rs) 重新组装 `tools[]` 数组。结果直接进 Anthropic / OpenAI / Codex 的 API 请求体，**模型只能调用最终留在数组里的工具**。

```mermaid
flowchart TD
    Start([build_tool_schemas provider]) --> Ctx[读取<br/>AppConfig + AgentCaps]
    Ctx --> Loop[遍历 all_dispatchable_tools]
    Loop --> Fate[resolve_tool_fate]
    Fate -- InjectEager --> Push[push schema]
    Fate -- InjectDeferred/HintOnly/Hidden --> Skip[skip schema]
    Push --> Mcp
    Skip --> Mcp
    Mcp[追加非 deferred MCP 动态工具] --> Plan["apply_plan_tools<br/><small>按 PlanAgentMode 分支</small>"]
    Plan --> PlanBranch{PlanAgentMode}
    PlanBranch -- "Off" --> Filter
    PlanBranch -- "PlanAgent" --> PA["push submit_plan<br/>retain 仅 plan allowed_tools"]
    PlanBranch -- "ExecutingAgent" --> EA["按 extra_tools<br/>push update_plan_step / amend_plan"]
    PA --> Filter
    EA --> Filter

    Filter["schemas.retain<br/><small>tool_visible_with_filters 多维过滤</small>"] --> FD[依次 AND:<br/>1. denied_tools 子 Agent 拒绝<br/>2. skill_allowed_tools 技能裁剪<br/>3. plan_allowed_tools Plan 白名单]

    FD --> Done([最终 tool_schemas → API 请求])

```

### 三个易混淆的"开关"对比

| 维度 | 控制谁 | 决策位置 |
|------|--------|----------|
| `supports_deferred()` | 工具是否**允许**被用户放进 deferred 池 | 由 tier `Standard`/`Configured` 的 `default_deferred` 字段派生 |
| `deferredTools.enabled` + `toolNames` | 哪些内置工具**本轮**变成 `InjectDeferred`（两者 AND）| `dispatch::resolve_tool_fate` |
| `tools.allow` / `tools.deny` + provider 配置 | 非 Core 工具是否 eager / hint-only / hidden | `dispatch::resolve_tool_fate` |
| MCP server `deferredTools` | 某台 server 的动态 MCP 工具是否走 `tool_search` | `agent::build_tool_schemas` + `tool_search` |

**规律**：是否"用户能开关启用"决定它走哪条路径——
- Core / Memory / MCP 元工具有自己的注入闸门，不由内置 deferred 总开关自动裁掉
- 支持 deferred 的内置工具默认仍 eager，只有进入 `deferredTools.toolNames` 后才延迟
- MCP 动态工具默认 eager，按 server 设置 `deferredTools=true` 后整台 server 延迟

### 与系统提示词的关系

两条系统提示词路径共享 `dispatch::resolve_tool_fate`：

- [`system_prompt/sections.rs`](../../crates/ha-core/src/system_prompt/sections.rs)：`build_tools_section` 把 `InjectEager` 工具的详细描述写入 `# Available Tools`；`build_deferred_tools_section` 把 `InjectDeferred` 工具 + deferred MCP server 写成 `# Additional Tools (use tool_search to discover)` 的一行目录。
- [`agent/mod.rs::build_full_system_prompt`](../../crates/ha-core/src/agent/mod.rs)：单趟遍历目录，`HintOnly` 累积到 `# Unconfigured Capabilities` 提示段（按 tool 名排序保证 prompt cache 命中），同时把 `send_notification` / `image_generate` / `canvas` 三类工具的额外指引段拼到提示词末尾。

### 与 tool_search 的关系

`tool_search` 的候选池同样由 `dispatch::resolve_tool_fate` 过滤：只包含 `InjectEager` / `InjectDeferred` 的工具，`Hidden` 和 `HintOnly` 不可发现。动态 MCP 工具额外按 server 的 `deferredTools` 设置计入 deferred 发现池。

---

## Tool Loop 执行流程

```mermaid
flowchart TD
    A["模型响应包含 tool_calls[]"] --> B["分组: partition by is_concurrent_safe()"]
    B --> C["Phase 1: 并发安全组 → join_all() 并行执行"]
    C --> D["Phase 2: 串行组 → for loop 逐个执行"]
    D --> E["所有结果合并为 tool_results[] 推入对话历史"]
    E --> F["Tier 1 截断检查"]
    F --> G["下一轮 API 调用（或退出 loop）"]
```

每个工具执行都通过 `tokio::select!` 与 cancel flag 竞争，支持用户随时取消。`async_capable` 工具调用进入 `execute_tool_with_context` 后会先经过下文的“异步决策”三道闸；显式后台或自动后台化时**会立即把 synthetic `{job_id, status: "started"}` 当作合法 tool_result 写回**，对话不阻塞继续推进，真实结果走异步注入回流。

---

## 异步 Tool 执行（async_capable）

长耗时工具（`exec` / `web_search` / `image_generate`）支持把整轮 tool call detach 成后台 job，立即返回 synthetic 结果，让 LLM 可以继续推进对话；真实结果完成后通过会话注入回流，模型靠 `job_id` 关联回去。这条机制完全不改 Anthropic / OpenAI 的 tool_use ↔ tool_result 配对协议，只是把"真实输出"和"配对响应"在时间上解耦。

### 决策三道闸

`tools/execution.rs:decide_async_path()` 在通过可见性 / 审批 / Plan-mode 路径门后立即决策。`bypass_async_dispatch=true` 的 ctx（递归再入路径）整段跳过，保证不会无限套娃。

> **exec 例外（审批前移，仅 Auto-Background 档）**：`exec` 的命令级审批不走外层引擎门（`needs_permission_engine` 排除 `TOOL_EXEC`），其门在 `tool_exec` 内部。**仅对 auto-background 档（Tier 3，`AutoBackgroundEligible`）**，`execute_tool_with_context` 在 detach 前先调 `exec::resolve_exec_command_approval`（命令门单一真相源）跑完审批再 spawn（`should_run_exec_reorder_gate`）。**显式后台 exec（`run_in_background:true` / `always-background`，`ImmediateBackground`）R8 起被刻意排除**——命令门下放到后台 job 线程、命中审批时 park 为 `AwaitingApproval`；详见下「exec 命令审批：两条后台路径」+「后台审批 park」。

```mermaid
flowchart TD
    Start([工具调用通过审批 + 路径门]) --> CheckBypass{ctx.bypass_async_dispatch?}
    CheckBypass -- true --> SyncPath[Sync 同步分发<br/><small>auto-bg 内层 / explicit-bg 内层</small>]
    CheckBypass -- false --> CheckCap{is_async_capable name?}
    CheckCap -- 否 --> SyncPath
    CheckCap -- 是 --> CheckEnabled{config.asyncTools.enabled?}
    CheckEnabled -- 否 --> SyncPath
    CheckEnabled -- 是 --> CheckPolicy{Agent async_tool_policy}
    CheckPolicy -- never-background --> SyncPath
    CheckPolicy -- "其他" --> CheckExplicit{args.run_in_background == true?}
    CheckExplicit -- 是 --> Tier1[Tier 1: ImmediateBackground<br/>JobOrigin::Explicit]
    CheckExplicit -- 否 --> CheckAlways{policy == always-background?}
    CheckAlways -- 是 --> Tier2[Tier 2: ImmediateBackground<br/>JobOrigin::PolicyForced]
    CheckAlways -- 否 --> CheckBudget{autoBackgroundSecs > 0?}
    CheckBudget -- 否 --> SyncPath
    CheckBudget -- 是 --> Tier3[Tier 3: AutoBackgroundEligible]

    Tier1 --> ExplicitSpawn[spawn_explicit_job<br/>立即返回 synthetic]
    Tier2 --> ExplicitSpawn
    Tier3 --> AutoBgRun[dispatch_with_auto_background<br/>同步预算赛跑]

    AutoBgRun --> Race{在预算内完成?}
    Race -- 是 --> InlineResult[把真实结果作为 tool_result 返回]
    Race -- 否 --> AutoBgDetach[原地 detach 成 job<br/>返回 synthetic auto_backgrounded]

```

| Tier | 触发 | 行为 |
|------|------|------|
| **1. Explicit** | `args.run_in_background = true` | 立即 detach，模型主动 opt-in |
| **2. Policy Forced** | `AgentConfig.capabilities.async_tool_policy = "always-background"` | 立即 detach，无视 args；完成仍靠 `<task-notification>` 自动注入，`job_status` 只做偶发状态快照 |
| **3. Auto-Background** | `model-decide` 策略 + `asyncTools.autoBackgroundSecs > 0`（默认 0，关闭） | 先同步跑，超预算再 detach，结果不丢 |

`job_timeout_secs` 是 async-capable 工具 schema 自动注入的可选单次参数，只控制外层 async job 的最长运行时长。`0` 或省略表示沿用用户配置；当 `asyncTools.maxJobSecs = 0` 时，正数 `job_timeout_secs` 可给本次 job 设置外层超时；当 `asyncTools.maxJobSecs > 0` 时，`job_timeout_secs` 只能比用户配置更短，不能放宽它。该字段在递归执行真实工具前会被剥离，不会传给 `exec` / `web_search` / `image_generate` 本体。

### exec 命令审批：两条后台路径（R8）

非 exec 的 async-capable 工具（`web_search` / `image_generate` / …）在到达 detach 分支前已经过外层引擎门审批，所以「先批准、后台化」天然成立。`exec` 不同：它被 `needs_permission_engine` 排除，命令级审批（危险命令 / 编辑命令 / AllowAlways 前缀 / 交互弹窗）历来只在 `tool_exec` 内部跑。**R8 起 exec 的两条后台路径分开处理**：

**① Auto-Background 档（Tier 3，`AutoBackgroundEligible`）——审批前移、detach 前同步跑门**。plain exec 仅在超前台预算时才后台化；`execute_tool_with_context` 在 detach 前先调命令门单一真相源 `exec::resolve_exec_command_approval`，闸为 `should_run_exec_reorder_gate`（`name==exec && AutoBackgroundEligible && !already_approved && should_run_exec_command_gate()`）：

- **Deny** → 直接返回 `ToolRejection`，**不 spawn**，模型得到 STOP，不会看到幽灵 job
- **Allow** → 把 `exec_pre_approved = true` 带入 spawn 的 ctx，后台 re-dispatch 经 `should_run_exec_command_gate()`（`!auto_approve_tools && !exec_pre_approved`）跳过内层门——审批恰好一次。同时把授权来源 `ApprovalOrigin` 写进 ctx，落 job 的 `approval_origin` 审计列
- 审批在 `dispatch_with_auto_background` 之前同步完成，所以审批等待**不**计入 `autoBackgroundSecs` / `maxJobSecs` 预算（消「审批慢→假转后台」，ASYNC-2）

**② 显式后台 exec（`run_in_background:true` / `always-background` 策略，`ImmediateBackground`）——R8 起不再 detach 前审批**（刻意 supersede ASYNC-1 的旧修复）。`should_run_exec_reorder_gate` 明确排除此档（单测 `exec_reorder_gate_excludes_immediate_background_for_r8_parking` 锁死）。模型**立刻拿到 job id**，命令门下放到后台 job 线程内跑（`exec.rs` 的 `should_run_exec_command_gate` 仍守，此处 `exec_pre_approved` 通常为 false）；命中 attended 审批时由 `async_jobs::approval_bridge` 把 job 行 `Running → AwaitingApproval`（见下「后台审批 park」），用户**异步**决定：批准→续跑、拒绝→job 落终态（`DeniedByUser → Failed`，STOP 语义随 `<task-notification>` 注入）。

`exec_pre_approved` 与 `external_pre_approved` 物理分开：后者只压制引擎门、**绝不**压制命令门（async re-entry 安全红线）；前者仅在命令门已对本次调用跑过、用户已批准后才置位（**仅 Auto-Background 档会置**），故可安全压制内层门。

### 后台审批 park（AwaitingApproval，R8 + b8702821）

显式后台 job 在自己的 OS 线程上 dispatch；命中 attended 命令门时 dispatch future 阻塞在审批引擎的 oneshot——job 是**真的在等人**而非在跑。`async_jobs::approval_bridge` 在该 job 线程装一个 thread-local 桥（`on_park` / `on_resume`，桥结构体定义在 `tools::approval` 以保 `tools` 零依赖 `async_jobs`），把行在等待两侧翻转 `Running ⇄ AwaitingApproval` 并记下 pending `request_id`。**scope：只有显式 / policy 的 `ImmediateBackground` exec 路径在此 park**；auto-background 与同步 exec 都已 detach 前审批（不装桥）；后台 subagent 的内层审批走自己的 runtime（桥不在那装，见 R8-followup）。

- **预算排除审批等待（ASYNC-2 机制）**：`run_tool_once` 的预算从一次性 timer 改 deadline-loop，每次到点把 deadline 后移 `parked_budget_extension()`（桥的 thread-local 累计 park 时长，**含在途 park** 故 parked 期间持续增长 → 审批中永不触发 `TimedOut`）；resume 后该值固定，post-approval 执行仍享完整 `max_job_secs`
- **resume 仅 proceed 才回 Running（B 修复，防误发 spurious Running）**：`on_resume` 仅在 proceed 结果（approve / timeout-proceed，`origin=Some`）才 `awaiting_approval → running` 并 emit `job:updated{running}` + F6 用真实决议改正 spawn 期占位 `approval_origin`；deny / timeout-deny / 取消掉 future（`origin=None`）**不 revert、不 emit**，行留 `awaiting_approval` 由终态 settle（`update_terminal` 接受 `awaiting_approval`）直接收——避免对从未续跑的 job 广播假 running
- **取消 parked job 的安全窗口（A 修复）**：`cancel_job` 经 `parked_request_id` 立即 `dismiss_parked_job_approval`——掉 pending sender 使命令门见取消即返回拒绝（永不批准）、所有 surface 弹窗即时消除、parked 的 `rx.await` 被唤醒使 dispatch 在 grace 内收尾；闭合「取消后 ~5s grace 内点 Allow 仍跑已取消命令」的安全窗口，并覆盖**跨进程取消**（仅设 DB flag 的取消也补 dismiss）
- **非终态 + replay**：`awaiting_approval` 不入终态 SQL 列表；replay 把它同 `running` 标 `interrupted`（`list_running` 含 `awaiting_approval`）

R8-followup 把后台 **subagent** 的内层审批也投影为 `AwaitingApproval`（经 `async_jobs::approval_projection_watcher` 订阅 EventBus 的 `approval_required` / `approval:resolved`，不走本桥），详见 [`subagent.md`](subagent.md#background-job-投影r6)。

### Auto-Background 的相位机

Tier 3 是最微妙的一档。`async_jobs::spawn::dispatch_with_auto_background` 用 OS 线程 + `tokio::current_thread` 运行 dispatch（避免对工具 future 的 Send 约束），主线程通过共享 `Arc<Mutex<Phase>>` + `Notify` 等待结果，原子状态转换防止"主线程已超时但 OS 线程刚好完成"的双终结竞态：

```mermaid
stateDiagram-v2
    [*] --> Pending: 主线程开始等待
    Pending --> ResultReady: OS 线程在预算内完成
    Pending --> DetachedRunning: 主线程超时, OS 线程仍在跑
    DetachedRunning --> DetachedDone: OS 线程完成
    ResultReady --> Consumed: 主线程取走结果
    DetachedDone --> [*]: OS 线程自行 finalize_job + 调度注入
    Consumed --> [*]: 主线程把真实 result 作为 tool_result 返回
```

- `Pending → ResultReady → Consumed`：预算内完成，跟同步执行没区别
- `Pending → DetachedRunning → DetachedDone`：主线程预算到，原子转移所有权；OS 线程检测到 `DetachedRunning`，独立写 DB + 触发注入
- 这条相位机是为了避免简单的 `oneshot::timeout` 模式在边界情况下丢结果 —— oneshot 在 timeout 触发瞬间被 drop，OS 线程的 `tx.send` 静默失败，结果消失

### Wait Registry（隐藏短等待唤醒机制）

`async_jobs::wait` 维护一个进程级 `LazyLock<Mutex<HashMap<job_id, Arc<Notify>>>>`，给 `job_status` 的隐藏 `block=true` 兼容路径使用。模型可见 schema 不再暴露阻塞参数；生产者 `finalize_job` 写完 terminal 行后调 `notify_completion`，消费者 `tool_job_status` 走 `tokio::select!` 在 `Notify::notified()` 与指数退避轮询（100ms → ×1.5 → 2s 上限，作为兜底）之间择一触发。

| 函数 | 调用方 | 职责 |
|---|---|---|
| `register_waiter(job_id) -> Arc<Notify>` | `tool_job_status` 入口 | 懒插入或克隆现有 `Arc<Notify>`；多 waiter 共享同一 `Notify`（`Arc::ptr_eq` 验证） |
| `notify_completion(job_id)` | `finalize_job` 写完 DB 之后 | `notify_waiters()` 唤醒所有 parked + `map.remove(job_id)` 在同一临界区内完成；幂等 |
| `cleanup_if_last_waiter(job_id, my_arc)` | `tool_job_status` 返回路径（终态 / 超时 / 错误） | 持锁检查 `Arc::strong_count <= 2`（map + caller）才 `map.remove`；其他 waiter 仍 parked 时不动 |
| `waiter_count(job_id)`（test-only） | 单元测试 | 返回 `Arc::strong_count` |

**关键不变量**：

1. **Lazy insertion**：从不在 job 创建时预插，避免无人 poll 的 job 留 registry slot
2. **Producer 一次性 remove**：`notify_completion` 在临界区内 `notify_waiters` + `remove`，保证后到 waiter 不会拿到一个已经被 fire 过的 stale `Notify`（`Notify::notify_waiters` 不留 permit）
3. **Late waiter 自愈**：`notify_completion` 之后才到的 waiter 会拿到一个**全新**的 `Notify`；`tool_job_status` 强制在 register 后再读一次 DB，看到 terminal 行直接返回，不会 park——orphan `Notify` 在返回路径上由 `cleanup_if_last_waiter` 清理
4. **Multi-waiter 共生**：同一 job_id 多个 `register_waiter` 调用 `Arc::clone` 同一 `Notify`；其中某个 waiter 超时退出时 `cleanup_if_last_waiter` 因 `strong_count > 2` 不删 entry，不影响其他仍 parked 的 waiter

EventBus `job:completed` 事件（R3 起，旧名 `async_tool_job:completed`）仍由 `finalize_job` emit，`job_status` 阻塞路径不消费它（走进程内 `Notify`）；前端 R4 的 `useBackgroundJobs` 与 `useDesktopAlerts` 消费它驱动面板刷新 + 完成桌面通知。

### Job 持久化

独立 SQLite 文件 `~/.hope-agent/background_jobs.db`（`async_jobs/db.rs`，R1 由 `async_jobs.db` 改名；纯可重建缓存，旧文件启动期 best-effort 丢弃，非迁移），不和 session DB 共享锁，避免热路径阻塞：

```sql
CREATE TABLE background_jobs (              -- R1 由 async_tool_jobs 改名
    job_id          TEXT PRIMARY KEY,        -- "job_<uuid simple>"
    session_id      TEXT,
    agent_id        TEXT,
    tool_name       TEXT NOT NULL,
    tool_call_id    TEXT,
    args_json       TEXT NOT NULL,
    status          TEXT NOT NULL,           -- running / cancelling / completed / failed / interrupted / timed_out / awaiting_approval
    result_preview  TEXT,                    -- inline 预览（head + tail）
    result_path     TEXT,                    -- 大结果 spool 磁盘路径
    error           TEXT,
    created_at      INTEGER NOT NULL,
    completed_at    INTEGER,
    injected        INTEGER NOT NULL DEFAULT 0,
    origin          TEXT NOT NULL DEFAULT 'explicit', -- explicit / policy_forced / auto_backgrounded
    -- 审批/资源治理列骨架（A-7 一次性引入，写入逻辑分散在后续子任务）：
    approval_origin TEXT,                     -- 授权来源 ApprovalOrigin 全 7 值：user / timeout_proceed / unattended_proceed / yolo / auto_approve / external_pre_approved / policy_allow（见 permission-system.md）
    incognito       INTEGER NOT NULL DEFAULT 0, -- 无痕标记（E4）
    pid             INTEGER,                  -- 子进程 pid，重启孤儿探测用（I3）
    cancel_requested INTEGER NOT NULL DEFAULT 0, -- 跨进程取消 flag（I4）
    kind            TEXT NOT NULL DEFAULT 'tool', -- R1：tool / subagent（R6）/ group（R5）
    subagent_run_id TEXT,                         -- R6：kind=subagent 投影的 FK→subagent_runs.run_id
    group_id        TEXT                          -- R5：kind=subagent 子 → 其 group 行 job_id（fan-out join）
);
```

> **R1 统一模型**：表/文件/概念为 **Background Job**（`JobKind = Tool | Subagent | Group`，三类均已落地）；stale-schema 探针改 `SELECT group_id`（最新列；升级即 drop-rebuild，无迁移）。
> **R6 后台 subagent 投影**：用户委派的后台 subagent run 投影为 `kind=subagent` 行（`subagent_run_id` FK，one-way——`subagent_runs` 是执行真相源，投影只承载 status/生命周期、**绝不持有 run 正文也绝不反写**）。`injected=1` 使其**永不进工具注入/replay 路径**（subagent 自有 `inject_and_run_parent`）；同步走 `update_subagent_status` 单一 choke point；取消经 `cancel_job` kind=Subagent 分支路由到 `subagent::request_cancel_run`。详见 [`subagent.md`](subagent.md)。**单一入口 `JobManager`**（`async_jobs::manager`）front 全部 spawn / cancel / list / replay / schedule；`spawn_explicit_job` 等收敛为其 `pub(crate)` 内部（Tool executor）。模块名 `async_jobs/` 与 log category `"async_jobs"` 按 PRD §4.3「沿用血脉演进」保留；`RuntimeTaskKind::AsyncJob` 内部枚举名不变。`progress_json` / `priority` / `attempt` 等列待对应 slice 消费时再加（drop-rebuild 故零成本延后）。
> **R3 统一 `job:*` 事件命名空间**：所有后台任务生命周期事件经 `async_jobs::events` 发 `job:{created,updated,progress,completed}` + 告警 `job:mark_injected_failed`（替代旧 `async_tool_job:*`，破坏性 drop，前端 listener 同步改），kind-tagged（`tool` / `group`）+ `session_id`；`progress` 目前 Group 报 `{current,total}`（N/M 子完成）。**`subagent` kind 沿用 `subagent:*` 流不双发**；R4 面板合并两路 + `job_status list`。**auto-background exec 也接 `output_tail`**（worker 内注册、非 detach 终局 `next.is_none()` 清 / detach 走 finalize 清），与显式 `run_in_background` 对齐。
> **R5 Group fan-out**：`batch_spawn` 建一条 `kind=group` 协调行 + N 个 `kind=subagent` 子（共享 `group_id`=group 的 `job_id`）；子**抑制个体注入**，全部到终态时单赢 CAS（`claim_group_completion`，`Running→Completed`）发**一条**合并注入（join-all-settle）。group 行 `injected=1`（自发合并注入、不进工具 replay），`args_json={"sealed":bool}` 标记「子已全 spawn」。group 行**绝不持有 run 正文**（合并消息构建时才从 `subagent_runs` 读子结果）。详见 [`subagent.md`](subagent.md#group-fan-outr5)。
> **R4 面板 + 完成合并窗口**：owner-plane（host-trusted）`JobManager::list_session_snapshots` / `get_job_snapshot` 出 `BackgroundJobSnapshot`（camelCase 展示向，与 model-facing `job_status` JSON 物理分离；**Group 子投影折叠进 Group 行**，exec 取命令首行为标签，running exec 带 `output_tail` 仅单查）；端点 Tauri `list_background_jobs` / `get_background_job` + HTTP `GET /api/sessions/{id}/background-jobs`、`/api/background-jobs/{id}`（Bearer，owner 平面看全部不经 agent-scope）；`db.list_for_session`（活跃优先 + 最近终态，cap 50）。取消复用 `cancel_runtime_task(kind=async_job)`。**完成注入合并窗口**：`async_tools.completionMergeWindowSecs`（默认 3，`0` 关）—— `finalize_job` 改走 `injection::enqueue_injection` 缓冲，同会话窗口内完成的多 tool job 合并一条 `<task-notification-batch count=… completed=… failed=…>`（内含 N 个标准 `<task-notification>`）一轮注入而非 N 轮计费。首个完成开窗 + 起定时器、窗口内入批、flush 原子取空（后到开新窗）；纯内存 live-path（崩溃则行 terminal-but-uninjected，重启 `replay_pending_jobs` 各自补投，不丢不合并）；Group 是预合并特例绕过；沿用 ghost-turn 闸 + 逐 job claim/release + `on_injected` 逐行恰好一次。前端 `src/types/background-jobs.ts` 镜像 + `useBackgroundJobs` 单订阅喂头部徽标 / 独立面板 / 工作台速览区块；完成桌面通知 `notification.notifyOnBackgroundJobComplete`（默认开，仅 completed/failed/timed_out + 仅后台）。

> `status` 第八态 `awaiting_approval`（A-5）为**非终态**，且是 R8 后**显式后台 exec（`run_in_background` / `always-background`）命中 attended 审批门时的真实、设计内状态**（可长停直到用户答复 / 取消，面板与 `job_status` 据此显示「等待审批」）——非「审批前移落地前的过渡态」。不消耗墙钟预算（park 期间预算 timer 排除审批等待）、不入终态 SQL 列表；replay 把它同 `running` 标 `interrupted`。机制详见上「后台审批 park（AwaitingApproval）」节；后台 subagent 内层审批投影见 R8-followup。

**大结果 spool**：超过 `asyncTools.inlineResultBytes`（默认 4096）的输出写到 `~/.hope-agent/background_jobs/{job_id}.txt`，DB 只存 head/tail 预览 + 路径。后续 `job_status` / 注入消息引用磁盘路径，模型可以用 `read` 工具拉全文。

### Synthetic 响应格式

模型在 tool_result 里看到的（任何 origin 通用）。这条 synthetic 响应刻意不要求 poll：如果没有可并行推进的工作，模型应告知 job 已在后台运行并停轮，等待 `<task-notification>` 自动注入，而不是马上调用 `job_status` 等待。

```json
{
  "job_id": "job_4f9bd1...",
  "status": "started",
  "tool": "exec",
  "origin": "explicit",
  "hint": "The tool is running in the background. Continue with other work if possible; otherwise stop the turn and wait for the auto-injected `<task-notification>`. Do not immediately call `job_status` just to wait. Use `job_status` only for a quick non-blocking snapshot after meaningful elapsed time or when the user asks. Detailed output is saved to the notification's `output-file` when available."
}
```

`origin = "auto_backgrounded"` 的 hint 会换成强调"超过同步预算被自动后台化"的措辞，便于模型追溯发生了什么。

### 结果回流（注入）

job 终态后，`async_jobs::spawn::finalize_job` 经 `async_jobs::injection::dispatch_injection` 把结果注入回父会话。这条路复用 `subagent::injection::inject_and_run_parent`，共享 `ACTIVE_CHAT_SESSIONS` / `SESSION_IDLE_NOTIFY` / `PENDING_INJECTIONS` 的会话空闲检测和重试队列：

```mermaid
sequenceDiagram
    participant LLM as LLM 主对话
    participant Tool as 工具执行
    participant DB as background_jobs.db
    participant Job as Job OS 线程
    participant Inj as injection 派送

    LLM->>Tool: tool_call(exec, run_in_background=true)
    Tool->>DB: INSERT status=running
    Tool->>Job: spawn (tokio current_thread)
    Tool-->>LLM: synthetic {job_id, status: started}
    LLM->>LLM: 继续推进对话 / 调其他工具
    Job->>Job: dispatch + 真实输出
    Job->>DB: UPDATE status=completed + preview / spool path
    Job->>Inj: dispatch_injection
    Inj->>Inj: 等会话空闲（ACTIVE_CHAT_SESSIONS / SESSION_IDLE_NOTIFY）
    Inj->>LLM: 注入 <task-notification> user 消息
    Inj->>DB: UPDATE injected=1
    LLM->>LLM: 模型读到结果, 按 task-id 关联回原 tool_call
```

> 上图为**无需审批 / 已 auto-approve 的 happy path**。R8 后,显式后台 exec 若命中 attended 命令门,会在 `Job OS 线程` 内 `UPDATE status=awaiting_approval`（emit `approval_required`）park 住,待用户决定:批准→续跑回到 `running`、拒绝→落终态（`DeniedByUser→Failed`）经注入回流——详见上「后台审批 park（AwaitingApproval）」节。

注入消息结构（XML 包裹便于模型解析）：

```xml
<task-notification>
<task-id>job_4f9bd1...</task-id>
<tool-use-id>call_xxx</tool-use-id>
<tool>exec</tool>
<status>completed</status>
<output-file>~/.hope-agent/background_jobs/job_4f9bd1....txt</output-file>
<summary>Async tool "exec" completed; full output is saved in output-file.</summary>
</task-notification>
```

当结果文件不可用时，completed 通知可带 `<output-preview>`；媒体结果可带 `<media-items-json>`。失败 / 超时 / 中断走 `<error>` 子标签。注入时若父会话忙，请求进 `PENDING_INJECTIONS` 队列等下次空闲（与子 Agent 注入完全同源）。

**注入终局（I7，MISC-15）**：`inject_and_run_parent` 返回 `InjectionOutcome{Injected, Queued, Abandoned}` 并接收一个 `on_injected` 回调（tool-job 传「标 `injected=1`」闭包，subagent 传 `None`）。回调仅在真正落地（父回合跑完 / 结果已被取走 / 全模型失败终局 = `Injected`）时触发，并随 `PendingInjection` 穿过重排队，使延迟注入最终落地时照样标记来源完成。父会话在 `announce_timeout` 内始终不空闲时返回 **`Abandoned`**——**不**触发回调、**不**重排队、行保持 `injected=0`，留待上面的「重启回放」补投。旧实现无论结果都在 `block_on` 后无条件 `mark_injected`，于是 `Abandoned` 被误标已注入、replay 不再补投、通知永久丢失。

### 终态错误分类（JobError，MISC-7）

job 结算的终态状态由**类型派生**而非字符串再解析。`async_jobs::error::JobError`（替代旧 `e.contains("was cancelled")` / `e.contains("exceeded max_job_secs")` 脆弱匹配）四变体 + `to_status()` 折叠映射：

| `JobError` | `JobStatus` | 注入文案 |
|------------|-------------|----------|
| `Cancelled` | `Cancelled` | "Job was cancelled." |
| `TimedOut { max_secs }` | `TimedOut` | "exceeded max_job_secs (Ns)" |
| `DeniedByUser { rejection }` | `Failed` | `ToolRejection::to_tool_result()`（保「STOP and wait」语义，ASYNC-4） |
| `Failed { message }` | `Failed` | 原始 message |

`DeniedByUser` **刻意折进 `Failed`**——不设独立 `Denied` 终态,免在所有 status match 站点穷举 enum bump。`from_dispatch_error` 用 `downcast::<ToolRejection>()` 保留拒绝的 STOP 语义随 `<task-notification>` 注入;auto-background 内联返回路径用 `into_inline_error()` 折回 `anyhow`（`DeniedByUser` 还原 `ToolRejection` 让流式循环渲染 STOP 模板）。

### 重启回放

`app_init::start_background_tasks` 启动时调用 `async_jobs::replay_pending_jobs()`：

1. 扫描 `status='running'` 行：本地进程已死，无法续跑 → 改为 `interrupted`，附 error 文案后入注入队列。**I3 孤儿清理**：若该行记录了 `pid` 且进程仍存活（崩溃前 detach 的后台 `exec` 子进程组 `process_group(0)` 幸存），先 `platform::terminate_process_tree(pid)` 整组结束孤儿、`app_warn!` 留痕，再标 `interrupted`——避免「DB 称中断、命令实际仍在跑」的状态谎言 + 资源泄漏
2. 扫描 `status in (completed/failed/timed_out/interrupted) AND injected=0`：上次进程崩在注入之前 **或** 上次注入因父会话长期忙碌被放弃（I7 `Abandoned`，下文）→ 重新派送

### 取消传导

后台 job 的取消有三条入口，覆盖「会话删除 / 跨进程 / 回合取消 grace 窗口」三种来源：

- **会话删除（A-8，DELETE-4）**：`session:deleted` → `JobManager::cancel_for_session(session_id)`（R1 单一入口；`cancel_jobs_for_session` 已降为 `pub(crate)` 内部实现）取消该会话全部**活跃** job——R8 后「活跃」含 `awaiting_approval`（park 态）job，关掉「删会话后后台 job 失去取消入口、无限运行」的口子。生产调用方是 `session::cleanup_watcher`（见 [`session.md`](session.md)）
- **跨进程取消（I4，MISC-4）**：`cancel_job` 除了命中本进程内存 cancel token，还写 DB `cancel_requested=1`；`run_job_to_completion` 在运行期每 ~5s `poll` 一次本行的 `cancel_requested`，命中即 `cancel_token.cancel()` 并 abort——这样桌面 + 自托管 server **共用同一 `background_jobs.db`** 时，由另一进程实际执行的 job 也能被中止，而不是只把 DB 状态改成 `cancelled` 却任其在对方进程跑完、结果被 active-status guard 静默丢弃。（auto-background detach 出来的 worker 暂未接 poll 臂——它在 detach 决策前就 spawn，结构上不便旁路，记为已知限制）
- **回合取消 grace 窗口（I5，MISC-2）**：`execute_tool_with_cancel` 的 cancel 臂给在途 dispatch 一个 5s 收尾窗口；若用户恰在窗口内批准了一个可后台化工具，dispatch 会返回合成 `{job_id,status:"started"}` 并已 detach 出带**全新** cancel token 的 runner（回合取消传导不到它）。cancel 臂现在捕获该结果、`extract_started_job_id` 解析出 job_id 后调 `cancel_job` 回收,使「已取消」名实相符。同步内联工具未及时收尾仍照旧 drop,其 `exec` 进程组由 `ProcessGroupGuard::drop` 回收

### 并发上限与排队（max_concurrent_jobs，I2 / MISC-5 / R7.1）

显式后台路径（`run_in_background: true` / `always-background` 策略）每个 job 占一条独立 OS 线程 + current-thread runtime。无上限时模型可跨回合连发 `run_in_background` 线性堆叠耗尽线程 / 内存（YOLO / `auto_approve_tools` 下更无人工闸）。`async_jobs::slots` 的 `SlotManager` 用进程级 per-session 计数 + 有界等待队列封顶：`spawn_explicit_job` 先 `try_reserve(session)`——有空位即起 runner（`SlotReservation` 随 runner 线程生命周期持有，drop 时减计数 + 唤醒调度器，所有退出路径都释放）。达 `asyncTools.maxConcurrentJobs`（默认硬件推导 `clamp(逻辑核数 - 2, 4, 16)`，`0` = 不限，每次实时读配置）时新 job **入队**（status `Queued`），由**每进程调度任务**（`run_scheduler`，tier-agnostic + 幂等：队列是进程本地内存态、只调度本进程队列）在槽位空出时按 **per-session 轮转**（`pick_fair_index`：选当前在跑数最少的会话，平局取最旧）提升——而非拒绝；仅当等待队列本身也满（`asyncTools.maxQueuedJobs`，默认 256、读时 `clamp_queued` 钳到 `[1, 4096]`，R9 配置化；每个排队 job 在内存持有 live ctx）才返回可操作错误结果（提示模型等待 / 查 `job_status` / 改同步执行）。排队 job 的 ctx 不可持久化，故重启不可恢复——与 `running` 一样由 replay 标 `Interrupted`。**范围**：只闸显式后台路径；auto-background detach 的 worker 在 detach 决策前已 spawn、不计入这套配额，改由每回合工具并发 + 同步预算天然约束。

### Retention / Orphan 清扫

长跑实例（数周到数月）会持续累积 terminal job 行 + spool 文件。`async_jobs::retention` 用一个 daily background loop 主动清扫，避免 `~/.hope-agent/background_jobs.db` 和 `~/.hope-agent/background_jobs/` 无界增长。

- **入口**：`app_init::start_background_tasks` 调 `retention::spawn_background_loop()`——内部 `tokio::spawn` 一个 24h ticker，启动时立即跑一次 + 之后每天一次
- **彻底关闭路径**：`retention_secs == 0 && orphan_grace_secs == 0` 时 `spawn_background_loop` 直接 return，不留永久空跑的 ticker
- **Row 清扫**（`retention_secs > 0`）：`db.purge_terminal_older_than(now - retention_secs)` 删 `completed_at` 早于 cutoff 的 terminal 行 + 关联 spool 文件，单事务原子提交
- **Orphan 清扫**（`orphan_grace_secs > 0`）：扫 `~/.hope-agent/background_jobs/*.txt`，跳过任何 DB 行 `result_path` 引用过的文件，剩下的若 mtime 早于 `now - orphan_grace_secs` 就删；`grace` 防误杀刚 spawn 但 DB 行尚未 commit 的 job 写入
- **单次 sweep 上限**：`MAX_ORPHANS_PER_SWEEP = 10_000` 防一个堆积 100k+ 文件的病态目录把 blocking pool 堵死几分钟，超出阈值后 `app_warn!` 退出，剩余下次 daily tick 继续清
- **运行 context**：`run_once()` 是同步函数，loop 用 `tokio::task::spawn_blocking` 派进 blocking pool，避免阻塞主 runtime

每次清到东西都落 `app_info!("async_jobs", "retention", ...)` 日志：`Purged N row(s), M spool file(s), B byte(s) freed (cutoff=Xs ago)`。

### 配置

`AppConfig.async_tools`（`config.json` → `asyncTools`）：

| 字段 | 默认 | 含义 |
|------|------|------|
| `enabled` | `true` | 总开关，关闭后所有 async-capable 工具退化为纯同步执行，`job_status` 工具也不注入 |
| `autoBackgroundSecs` | `0` | Tier 3 同步预算。`0` 关闭自动后台化，仅保留 Tier 1/2 |
| `maxJobSecs` | `0`（不限时） | 后台 job 的用户硬上限；超时 → status=`timed_out` 并注入失败消息。`0` = async job 层默认不限时；具体工具仍可有自己的内部超时（如正数 `exec.timeout`；`exec.timeout=0` 也表示不限）。当全局为 `0` 时，模型单次 `job_timeout_secs > 0` 可为本次 job 设置外层超时；当全局为正数时，`job_timeout_secs` 只能收紧这个上限，不能放宽 |
| `maxConcurrentJobs` | 硬件推导 `clamp(逻辑核数-2,4,16)`（`0` = 不限） | 显式后台路径（`run_in_background` / `always-background`）并发上限，见上「并发上限与排队」节。达上限时新作业**排队**（`Queued`），每进程调度器 per-session 轮转提升；等待队列（`maxQueuedJobs`，默认 256）也满才拒绝。只闸显式路径（per-process cap），auto-background 不计入 |
| `inlineResultBytes` | `4096` | 注入消息内联 preview 上限；超过时 spool 到磁盘并注入路径引用 |
| `retentionSecs` | `30 * SECS_PER_DAY`（30 天） | 终态行 + spool 文件 TTL；超期由 daily background loop 清扫。`0` = 永不清理（长跑实例累积风险，仅极端调试用） |
| `orphanGraceSecs` | `24 * SECS_PER_HOUR`（24h） | 孤儿 spool 文件 TTL：`~/.hope-agent/background_jobs/` 下名字未被任何 DB 行引用、且 mtime 超过这个 grace 的文件被删（grace 防与新写入 race）。`0` 关闭孤儿清扫 |
| `jobStatusMaxWaitSecs` | `7200`（2h） | 隐藏 `job_status(block=true)` 兼容路径的运行时上限。`max_job_secs > 0` 时由 `max_job_secs` 取代（`job_status_ceiling_secs()` 解析）；工具实现还会额外套 10s UI-safety cap，模型可见 schema 不暴露阻塞等待 |
| `outputTailBytes` | `8192`（8KB） | （R9）后台 `exec` **运行时**保留的输出尾环大小（R3 ① tail），供 `job_status(action:status)` 看最新输出判「在跑 / 卡住」、不必等完成。job 启动时快照该值（改值不 resize 已跑 job）；越大越可见、每个在跑 job 占更多 RAM（受并发上限约束）。读时 `configured_bytes()` 钳到 `[256, 1048576]`（256B–1MB）|
| `maxQueuedJobs` | `256` | （R9）后台 job 内存等待队列（R7.1）硬上限；槽位（`maxConcurrentJobs` / per-session）全满时新 `run_in_background` 入队于此，每个排队 job 钉住 live `ToolExecContext` 故必须有界，超过则硬拒（模型等待 / 同步执行）。读时 `clamp_queued` 钳到 `[1, 4096]`——`0` **不**表示无限，是内存护栏 |
| `wakeupMaxDelaySecs` | `86400`（24h） | （R9）`schedule_wakeup` 自调度延迟上限（秒）；请求延迟 clamp 到 `[10, wakeupMaxDelaySecs]`（10s 下限是不可配的忙轮询护栏）。防僵尸定时器无限占用会话，更长节律应走 cron。读时钳到 `[10, 604800]`（10s–7d）|
| `wakeupMaxPendingPerSession` | `5` | （R9）每会话待触发 `schedule_wakeup` 上限；超过是**结构类拒绝**（不排队），防 agent 自调度大量计费回合。读时钳到 `[1, 100]` |

`AgentConfig.capabilities.async_tool_policy`（`agent.json`）：

- `model-decide`（默认）：尊重 `args.run_in_background`，未指定时走 Tier 3 自动后台化
- `always-background`：所有 async-capable 工具一律 detach；适合 IM/GUI 不想被长任务卡住的场景，但不表示模型要主动 poll，完成结果仍靠自动注入
- `never-background`：禁用 async 路径（Tier 1/2/3 全不触发）

**exec 收敛规则**：普通长跑 shell 命令统一交给 async_jobs（`run_in_background` / `job_status` / `<task-notification>`）。`exec(background=true)` 与 `exec(yield_ms=...)` 只保留为 legacy process-session 兼容面（返回 `session_id`，用 `process(action="poll"|"log"|"kill")` 管理）。执行层规则如下：

- async_tools 开启且 agent 不是 `never-background`：`background/yield_ms` 会在执行入口兼容迁移为 `run_in_background=true`，并移除 legacy process flags，避免外层 job 只返回“process session started”。
- async_tools 关闭 / agent `never-background`：保留 legacy process-session 行为，后续通过 `process` 查询/取消。
- 保留下来的 process session 退出时会发 `process:completed` EventBus 信号，并在父会话空闲时注入 `<process-notification>`；显式 `process poll/log/kill/remove` 会标记为已观察，尽量避免重复注入。

### 递归再入与权限

显式后台 + 自动后台 都通过把工具的 `execute_tool_with_context` 在新线程上**递归再入**完成实际工作。再入时必须设置：

- `bypass_async_dispatch = true`：跳过 async 决策，直奔 sync dispatch，避免 `always-background` 策略触发死循环
- `external_pre_approved = true`：外层已经过通用审批门，内层不能重复跑 engine gate；但它**不能**绕过 exec 的命令级危险/编辑审计，exec 只在 `exec_pre_approved = true` 时跳过内层命令 gate

可见性 / Plan-mode 路径检查仍会在内层走一遍，作为 belt-and-suspenders。

### 关键源文件

| 文件 | 职责 |
|------|------|
| `crates/ha-core/src/async_jobs/manager.rs` | **`JobManager`：后台任务操作的单一生产入口**（R1）——spawn_tool / dispatch / get / list / cancel / cancel_for_session / purge_for_session / replay / run_scheduler / retention，薄委托到内部 |
| `crates/ha-core/src/async_jobs/mod.rs` | `JobManager` 再导出 + `(pub(crate))` cancel/cleanup/replay + `get/set_async_jobs_db` 白盒读访问器 |
| `crates/ha-core/src/async_jobs/types.rs` | `BackgroundJob` / `JobStatus` / `JobKind`（Tool/Subagent/Group）/ `JobOrigin` |
| `crates/ha-core/src/async_jobs/db.rs` | `JobsDB`：独立 SQLite `background_jobs` 表 + CRUD |
| `crates/ha-core/src/async_jobs/spawn.rs` | `(pub(crate))` `spawn_explicit_job`、`dispatch_with_auto_background`、相位机、result spool（Tool executor 内部，经 `JobManager` 调用） |
| `crates/ha-core/src/async_jobs/injection.rs` | 注入消息构造 + 复用 `subagent::injection::inject_and_run_parent` |
| `crates/ha-core/src/async_jobs/wait.rs` | per-job `Notify` 注册表：`register_waiter` / `notify_completion` / `cleanup_if_last_waiter`，由 `Arc::strong_count` 管理生命周期 |
| `crates/ha-core/src/async_jobs/retention.rs` | `run_once` 单次清扫 + `spawn_background_loop` daily ticker，删 terminal 行 + 孤儿 spool 文件，`MAX_ORPHANS_PER_SWEEP=10_000` 兜底 |
| `crates/ha-core/src/tools/job_status.rs` | `job_status` 工具实现（模型可见 snapshot；隐藏短 blocking 兼容路径走 `wait::register_waiter`） |
| `crates/ha-core/src/tools/execution.rs` | `decide_async_path` + 三道闸路由 + `bypass_async_dispatch` 递归保护 |
| `crates/ha-core/src/tools/definitions/types.rs` | `ToolDefinition.async_capable` + schema 自动注入 `run_in_background` / `job_timeout_secs` |
| `crates/ha-core/src/system_prompt/sections.rs` | `build_async_tools_section` 教模型何时使用 async tool / 怎么解析 `<task-notification>` |
| `crates/ha-core/src/config/mod.rs` | `AsyncToolsConfig` |
| `crates/ha-core/src/agent_config.rs` | `AsyncToolPolicy` 枚举 + `CapabilitiesConfig.async_tool_policy` |
| `crates/ha-core/src/paths.rs` | `async_jobs_db_path` / `async_jobs_dir` / `async_job_result_path` |

---

## 工具结果磁盘持久化

当工具返回结果超过阈值时，自动写入磁盘：

- **阈值**：默认 50KB，通过 `config.json` → `toolResultDiskThreshold` 配置（0 = 禁用）
- **存储路径**：`~/.hope-agent/tool_results/{session_id}/{tool_name}_{timestamp}.txt`
- **上下文内容**：head 2KB + `[...N bytes omitted...]` + tail 1KB + 路径引用
- **访问方式**：模型可通过 read 工具读取完整文件
- **视觉输出例外**：包含图片 marker 的工具结果不能按普通文本 head/tail 截断；合法图片 marker 会完整保留或物化为受管 `__IMAGE_FILE__` 交给 Provider 视觉输入，非法/损坏 marker 只返回纯文本落盘引用，避免把半截 base64 当图片发送

```mermaid
flowchart TD
    A["工具返回 200KB 结果"] --> B{"result.len() > threshold (50KB)?"}
    B -- 是 --> C["写入磁盘:<br/>~/.hope-agent/tool_results/sess_abc/read_1712345678.txt"]
    C --> D["返回给模型:<br/>[前 2000 字符]<br/>[...197000 bytes omitted...]<br/>[后 1000 字符]<br/>[Full result saved to: ...]<br/>[Use read tool to access full content]"]
    B -- 否 --> E["原文返回给模型"]
```

## 视觉工具输出协议

视觉工具输出分两条通道，职责不能混用：

| 通道 | 协议 | 消费方 | 作用 |
| --- | --- | --- | --- |
| UI / IM 文件资产 | `__MEDIA_ITEMS__[...]` | 前端、HTTP 资源路由、IM channel worker | 展示图片/文件卡片、下载、转发；包含 logical `url`、本地 `localPath`、MIME、大小、kind |
| Provider 视觉输入 | `__IMAGE_BASE64__...` / `__IMAGE_FILE__...` | `agent/events.rs` → 各 Provider adapter | 在发 API 前转换成 Anthropic/OpenAI/Codex 支持的标准图片输入 |

### `__MEDIA_ITEMS__`

工具结果可以用 `__MEDIA_ITEMS__` 前缀携带结构化附件元数据：

```text
__MEDIA_ITEMS__[{"url":"/api/attachments/<session>/<file>","localPath":"/abs/path","name":"...","mimeType":"image/png","sizeBytes":123,"kind":"image"}]
普通 tool_result 文本
```

`agent/events.rs::extract_media_items()` 会把该前缀从 tool_result 文本里剥离，并把 `media_items[]` 挂到 `tool_result` 流式事件上。Tauri 前端可以使用 `localPath`，HTTP/Web 模式的 EventBus 桥会去掉 `localPath` 并给 `/api/attachments/...` 补 token。

`__MEDIA_ITEMS__` 只服务 UI / IM / 文件下载。它不会自动让模型“看见图片”；模型视觉输入必须走下面的图片 marker。

### `__IMAGE_BASE64__`

内联图片协议：

```text
__IMAGE_BASE64__image/png__<base64>__
Screenshot captured (...)
```

工具执行层会优先把普通会话里的内联 marker 物化为 `__IMAGE_FILE__`，避免大 base64 长期留在会话历史；如果 marker 仍以内联形式进入 Provider 请求构造，`agent/events.rs` 会识别并转换为：

- Anthropic：`{ type: "image", source: { type: "base64", media_type, data } }`
- OpenAI Chat：`{ type: "image_url", image_url: { url: "data:image/...;base64,..." } }`
- OpenAI Responses / Codex：追加 `{ type: "input_image", image_url: "data:image/...;base64,..." }`

约束：

- MIME 必须是 `image/*`
- base64 必须完整且可解码
- marker 一旦被截断、混入 `[...bytes omitted...]`、缺少分隔符，必须降级为普通文本，不得生成 Provider 图片输入

### `__IMAGE_FILE__`

新的文件引用图片协议：

```text
__IMAGE_FILE__{"mime":"image/png","path":"/Users/.../.hope-agent/attachments/<session>/browser_screenshot.png"}
Screenshot captured (...)
```

它解决“图片原始文件要保存，但 Provider 不能直接读取本地路径”的问题：工具先把图片 bytes 保存为受管文件，再把路径 marker 写入 tool_result；Provider 发送前由 Hope Agent 读取该路径、校验、编码为 base64，再转换成标准图片输入。

安全边界：

- 只允许 Hope Agent 受管媒体目录下的路径，例如 `~/.hope-agent/attachments/`、`~/.hope-agent/tool_results/` 和 `~/.hope-agent/mac-control/snapshots/`
- 路径必须 canonicalize 后仍在允许目录内，防止 `../` 或 symlink 逃逸
- 文件 MIME 必须由魔数校验为图片，且与 marker 声明 MIME 一致
- 文件大小必须受上限保护，避免把超大本地文件读入 Provider 请求
- 任意工具结果伪造的普通 `/Users/...` 路径不得被自动读取

### 与落盘/压缩的关系

图片 marker 是机器可解析载荷，不是普通文本：

- 大结果落盘不能对 marker 做 head/tail 截断后再保留 marker 前缀
- 合法图片 marker 要么完整保留给 Provider 转换，要么迁移为 `__IMAGE_FILE__` 文件引用；普通非无痕会话会尽早把 `__IMAGE_BASE64__` 物化到 `tool_results/`，即使结果本身未超过大结果阈值
- 非法图片 marker 或包含 marker 的普通落盘预览只允许返回纯文本路径引用，不能再生成 `image_url`
- Tier 1/2 上下文压缩同样不得制造“半截 marker”；如果要裁剪视觉结果，应移除图片载荷并保留文本说明/文件路径

关键实现：

| 文件 | 职责 |
| --- | --- |
| `crates/ha-core/src/tools/image_markers.rs` | 解析/校验 `__IMAGE_BASE64__` 与 `__IMAGE_FILE__`，文件路径安全检查，按需读取并编码图片 |
| `crates/ha-core/src/agent/events.rs` | 把图片 marker 转换为各 Provider 的标准图片输入；解析失败、或编码失败（文件丢失 / 过大 / mime 不符）时降级为文本说明，绝不把内部 marker 回灌模型 |
| `crates/ha-core/src/tools/execution.rs` | 大工具结果落盘；普通会话内联图片 marker 物化；对图片 marker 做完整性保护，避免截断后继续作为图片发送 |
| `crates/ha-core/src/context_compact/truncation.rs` | Tier 1 截断时保护图片 marker，避免压缩阶段制造半截图片载荷 |
| `crates/ha-core/src/tools/browser/snapshot.rs` | browser 截图保存为 session attachment，并用 `__MEDIA_ITEMS__` + `__IMAGE_FILE__` 同时服务 UI 和模型视觉 |
| `crates/ha-core/src/tools/mac_control.rs` | `visual.observe` 把 macOS 受管截图包装为 `__IMAGE_FILE__`，供模型视觉定位 |

### 端到端流程图

```mermaid
flowchart TD
    Start["Tool dispatch<br/>execute_tool_with_context"] --> Run["工具实现返回 raw result 字符串"]

    Run --> HasImageMarker{"包含合法图片 marker?<br/>__IMAGE_BASE64__ / __IMAGE_FILE__"}
    HasImageMarker -->|base64，可物化| Materialize["__IMAGE_BASE64__ 写入<br/>~/.hope-agent/tool_results/<session>/...<br/>替换为 __IMAGE_FILE__"]
    HasImageMarker -->|file marker 或物化失败| PreserveVisual["保留完整 marker<br/>禁止 head/tail 截断"]
    HasImageMarker -->|否| IsLarge{"raw result 超过<br/>toolResultDiskThreshold?"}
    Materialize --> PreserveVisual
    IsLarge -- 否 --> Inline["完整 raw result<br/>返回 streaming_loop"]

    IsLarge -- 是 --> AnyMarker{"包含图片 marker 前缀?"}
    AnyMarker -- 否 --> PersistText["写入 ~/.hope-agent/tool_results/<session>/...txt"]
    PersistText --> TextPreview["返回 head + omitted + tail + 路径引用"]
    AnyMarker -- 是 --> PersistVisualText["写入 tool_results<br/>返回纯文本路径引用<br/>不保留 marker 前缀"]

    Inline --> StripMedia["streaming_loop 调<br/>extract_media_items()<br/>剥离 __MEDIA_ITEMS__ 前缀"]
    TextPreview --> StripMedia
    PersistVisualText --> StripMedia
    PreserveVisual --> StripMedia

    StripMedia --> HasMedia{"结果以<br/>__MEDIA_ITEMS__ 开头?"}
    HasMedia -- 是 --> MediaHeader["结构化附件元数据<br/>url / localPath / mime / size / kind"]
    HasMedia -- 否 --> NoMedia["无 UI 附件元数据"]

    MediaHeader --> EmitEvent["emit tool_result 事件"]
    NoMedia --> EmitEvent

    EmitEvent --> EventPayload["event.result = 文本/marker<br/>event.media_items = UI 附件元数据"]
    EventPayload --> PersistDb["SessionDB 更新 messages.tool_result<br/>附带 duration/is_error/tool_metadata"]
    EventPayload --> Frontend{"前端通道"}
    Frontend -- Tauri --> TauriUi["保留 localPath<br/>convertFileSrc 展示/打开文件"]
    Frontend -- HTTP/Web --> HttpUi["EventBus 桥移除 localPath<br/>/api/attachments/... 补 token"]

    EmitEvent --> History["ExecutedTool.clean_result<br/>原样写入 provider history"]
    History --> ProviderParse{"构造 API request 时<br/>临时解析图片 marker?"}

    ProviderParse -- 无 marker --> PlainToolResult["按普通文本 tool_result<br/>发给模型"]
    ProviderParse -- __IMAGE_BASE64__ --> ValidateB64{"校验 image/* MIME<br/>和完整 base64"}
    ProviderParse -- __IMAGE_FILE__ --> ValidateFile{"canonicalize 路径<br/>限制在受管目录<br/>魔数校验 MIME<br/>大小上限"}

    ValidateB64 -- 失败 --> PlainFallback["降级普通文本<br/>不生成 image_url"]
    ValidateFile -- 失败 --> PlainFallback
    ValidateB64 -- 通过 --> ProviderImage["转换为 Provider 标准图片输入"]
    ValidateFile -- 通过 --> ReadEncode["读取本地图片 bytes<br/>编码 base64"]
    ReadEncode --> ProviderImage

    ProviderImage --> ApiRequest["Anthropic / OpenAI Chat / Responses / Codex API 请求<br/>不把临时 base64 写回 context_json"]
    PlainToolResult --> ApiRequest
    PlainFallback --> ApiRequest

    ApiRequest --> Compact["下一轮前上下文压缩<br/>truncate_tool_results / pruning"]
    Compact --> CompactRule{"遇到图片 marker?"}
    CompactRule -- 是 --> CompactVisual["不得制造半截 marker<br/>移除载荷或保留完整文件引用"]
    CompactRule -- 否 --> CompactText["普通文本按预算截断/清理"]
```

---

## 上下文压缩

工具结果的上下文压缩采用 5 层渐进式策略，完整架构见 [上下文压缩文档](context-compact.md)。

```mermaid
flowchart LR
    T0["Tier 0<br/>微压缩<br/>零成本清除旧临时工具结果"] --> T1["Tier 1<br/>截断<br/>单个过大工具结果 head+tail"]
    T1 --> T2["Tier 2<br/>裁剪<br/>旧工具结果 soft-trim / hard-clear"]
    T2 --> T3["Tier 3<br/>LLM 摘要<br/>调用模型压缩旧消息"]
    T3 --> T4["Tier 4<br/>紧急<br/>清除所有工具结果 + 只保留最近 N 轮"]

```

---

## 权限控制架构

系统中存在 **四个独立的工具控制维度**，按生效层级分为三大类：

| 类别 | 维度 | 作用 | 配置位置 |
|------|------|------|----------|
| **Agent 工具开关** | 非 Core 工具开关（FilterConfig） | 通过 `dispatch::resolve_tool_fate` 统一决定 system prompt、tool schema、`tool_search` 和执行层兜底 | Agent 设置 → 能力 → 工具 → 工具注入 |
| **Schema 可见性** | 子 Agent 工具拒绝（denied_tools） | 从实际发送给 LLM API 的 tool schema 中移除 | Agent 设置 → 子 Agent |
| **执行审批** | 会话权限模式（ToolPermissionMode） | 决定工具执行前**是否弹审批** | 输入框盾牌按钮 |
| **执行审批** | Agent 审批列表（require_approval） | 指定哪些工具需要审批 | Agent 设置 → 能力 → 工具 → 工具审批 |

此外还有 **Plan Mode 路径限制** 和 **exec 命令级 Allowlist** 两个特殊机制。

---

### 1. Agent 工具开关（FilterConfig）

**源码**：`agent_config.rs` → `AgentConfig.capabilities.tools: FilterConfig`
**UI**：Agent 设置面板 → 能力 → 工具子 tab → 工具注入折叠段落
**生效位置**：

- `dispatch::resolve_tool_fate()` — 决定 Standard / Configured 工具的 enabled 状态
- `system_prompt/build.rs:build_tools_section()` — 只描述当前 eager 工具
- `agent/mod.rs:build_tool_schemas()` — 只发送当前 eager schema
- `tools/tool_search.rs` — 只发现当前 eager/deferred 工具
- `tools/execution.rs:execute_tool_with_context()` — 执行层按同一 fate 兜底拒绝

```rust
pub struct FilterConfig {
    pub allow: Vec<String>,  // 非 Core 工具：显式打开
    pub deny: Vec<String>,   // 非 Core 工具：显式关闭
}
```

**判断逻辑**（仅 Standard / Configured 工具）：

```
工具在 deny 中 → 关闭
工具在 allow 中 → 打开
其他 → 使用 ToolTier 的 default_for_main / default_for_others
```

- 默认值：`allow=[]`, `deny=[]`（即不覆盖代码默认值）
- **作用范围**：只控制非 Core 内置工具的开关覆盖。Core 工具不受该字段影响；Memory / MCP 仍走各自 master switch
- **执行层兜底**：执行前重新解析 `resolve_tool_fate()`，避免旧上下文或异常 provider 输出绕过开关

**这样设计的理由**：

- **UI 语义一致**：设置面板的开关只记录用户对默认值的覆盖，不把默认开启工具展开写进 agent.json
- **避免 deferred tools 绕过**：如果只裁剪 prompt 或主 schema，模型仍可能通过 `tool_search` 发现被禁用工具；统一过滤后不会出现这类旁路
- **执行层防绕过**：即使未来某个 Provider 解析异常、历史消息注入异常，执行层仍会按同一规则拒绝被禁用工具
- **保持层次分工**：`FilterConfig` 负责 Agent 级工具开关；`denied_tools`、skill allowlist 和 Plan Mode 负责更强的上下文级收紧

### 2. 子 Agent 工具拒绝（denied_tools）

**源码**：`agent_config.rs` → `SubagentConfig.denied_tools: Vec<String>`
**生效位置**：`agent/mod.rs:build_tool_schemas()` — 在统一 schema 过滤阶段移除

```rust
schemas.retain(|t| {
    let name = extract_tool_name(t);
    tools::tool_visible_with_filters(
        name,
        &agent_tool_filter,
        &self.denied_tools,
        &self.skill_allowed_tools,
        plan_allowed_tools,
    )
});
```

- **作用范围**：从实际发送给 LLM API 的 tool schema 中移除，LLM 完全不知道这些工具的存在
- **使用场景**：子 Agent 深度分层工具策略，防止子 Agent 调用特定危险工具

---

### 3. 会话权限模式（ToolPermissionMode）— 最高优先级

**源码**：`tools/approval.rs` → `ToolPermissionMode` 枚举
**UI**：输入框左侧盾牌按钮（三态切换）
**生效位置**：`tools/execution.rs:execute_tool_with_context()` — 工具执行入口

```rust
pub enum ToolPermissionMode {
    Auto,           // 默认：由 Agent 配置决定
    AskEveryTime,   // 所有工具都弹审批
    FullApprove,    // 全部自动放行
}
```

**存储**：进程级全局单例（`OnceLock<TokioMutex>`），每次发消息时由前端通过 `chat` 命令参数设置。

> ⚠️ **注意**：这是进程级全局状态，多窗口/多会话共享同一个值。

### 4. Agent 审批列表（require_approval）

**源码**：`agent_config.rs` → `CapabilitiesConfig.require_approval: Vec<String>`
**UI**：Agent 设置面板 → 能力 → 工具 → 工具审批（三种模式：全部/无/自定义）
**生效位置**：`tools/execution.rs:tool_needs_approval()`

| 配置值 | 效果 |
|--------|------|
| `["*"]`（默认） | 所有非内部工具需审批 |
| `[]` | 所有工具自动放行 |
| `["exec", "web_fetch"]` | 仅指定工具需审批 |

**仅在 `ToolPermissionMode::Auto` 时生效**。

---

## 完整决策流程

> **说明**：下图描述的是“schema 可见性 + 执行审批”的硬控制链路。非 Core 工具开关先由 `resolve_tool_fate` 决定 schema / `tool_search` 可见性，并在执行层再次兜底校验。

```mermaid
flowchart TD
    Start([工具调用触发]) --> InSchema{工具是否在 Provider<br/>tool_schemas 中？}

    InSchema -- "不在（被 capabilities.tools / denied_tools / skill / Plan 裁剪）" --> Blocked[/LLM 根本不会调用/]
    InSchema -- 在 --> IsInternal{是 internal tool？<br/><small>ask_user_question / submit_plan<br/>update_plan_step / canvas ...</small>}

    IsInternal -- 是 --> DirectExec[✅ 直接执行<br/>永不审批]
    IsInternal -- 否 --> IsSkillRead{是 SKILL.md 读取？<br/><small>read 工具 + 路径以 SKILL.md 结尾</small>}

    IsSkillRead -- 是 --> DirectExec
    IsSkillRead -- 否 --> IsExec{是 exec 工具？}

    IsExec -- 是 --> ExecFlow[走 exec 独立审批流程<br/><small>见下方 exec 流程图</small>]
    IsExec -- 否 --> PermMode{读取 ToolPermissionMode<br/><small>输入框盾牌按钮</small>}

    PermMode -- FullApprove --> DirectExec
    PermMode -- AskEveryTime --> ShowApproval[弹出审批对话框]
    PermMode -- "Auto（默认）" --> AgentConfig{读取 Agent 的<br/>require_approval}

    AgentConfig -- "全部审批（默认）" --> ShowApproval
    AgentConfig -- "空列表" --> DirectExec
    AgentConfig -- "指定工具名" --> MatchTool{工具名在列表中？}

    MatchTool -- 匹配 --> ShowApproval
    MatchTool -- 不匹配 --> DirectExec

    ShowApproval --> UserChoice{用户选择}
    UserChoice -- 允许一次 --> DirectExec
    UserChoice -- 始终允许 --> WriteAllowlist[写入 allowlist<br/><small>仅 Auto 模式生效</small>] --> DirectExec
    UserChoice -- 拒绝 --> Denied[❌ 返回错误<br/>Tool execution denied]
    UserChoice -- "超时（5分钟）" --> Denied

    DirectExec --> PlanCheck{plan_mode_allow_paths<br/>非空？}
    PlanCheck -- 否 --> Execute[🔧 执行工具]
    PlanCheck -- 是 --> IsPathAware{是 write/edit/<br/>apply_patch？}
    IsPathAware -- 否 --> Execute
    IsPathAware -- 是 --> PathAllowed{is_plan_mode_path_allowed?<br/><small>.hope-agent/plans/*.md</small>}
    PathAllowed -- 是 --> Execute
    PathAllowed -- 否 --> PlanDenied[❌ Plan Mode restriction<br/>cannot modify file]

```

### 审批对话框交互

当判定需要审批时，后端发射 `approval_required` 事件，前端 `ApprovalDialog` 显示三个选项：

| 选项 | 行为 |
|------|------|
| **允许一次**（AllowOnce） | 本次放行，下次同样弹出 |
| **始终允许**（AllowAlways） | Auto 模式：写入 `exec-approvals.json` allowlist；AskEveryTime 模式：等同于 AllowOnce（不写 allowlist） |
| **拒绝**（Deny） | 工具返回类型化错误 [`ToolRejection::DeniedByUser`](../../crates/ha-core/src/tools/rejection.rs)，由 [`streaming_loop`](../../crates/ha-core/src/agent/streaming_loop.rs) 出口渲染为 `Tool error: Tool '<name>' execution denied by user. The tool did not execute and no side effects occurred. STOP what you are doing and wait for the user to tell you how to proceed.`；带 `Tool error:` 前缀触发 `is_error` 通道（UI 标红、warn 日志）|

审批等待超时默认 5 分钟，可通过 `config.json` 的 `approvalTimeoutSecs` 配置，`0` 表示不限时。超时后的行为由 `approvalTimeoutAction` 控制：默认 `deny`，阻止工具执行；可选 `proceed`，记录 warning 后继续执行工具。

### IM Channel 审批交互

当工具审批发生在 IM 渠道（Telegram/Discord/Slack 等）对话中时，`channel/worker/approval.rs` 监听 EventBus 的 `approval_required` 事件，通过 `ApprovalRequest.session_id` 反查 `ChannelDB` 关联的渠道信息，将审批提示发送到 IM 渠道本身：

- **支持按钮的渠道**（`ChannelCapabilities.supports_buttons = true`）：Telegram InlineKeyboard / Discord Action Row Button / Slack Block Kit / 飞书 Interactive Card / QQ Bot Keyboard / LINE Buttons Template / Google Chat Card v2
- **不支持按钮的渠道**：发送文本提示，用户回复 "1"（允许一次）/ "2"（始终允许）/ "3"（拒绝）

按钮回调通过各渠道原生机制（callback_query / INTERACTION_CREATE / interactive envelope / card.action.trigger / postback / CARD_CLICKED）路由回 `submit_approval_response()`。

### IM Channel 自动审批

`ChannelAccountConfig.auto_approve_tools: bool`（默认 `false`）可在设置中开启。开启后该渠道的所有工具调用自动审批，通过 `ChatEngineParams.auto_approve_tools` → `AssistantAgent.auto_approve_tools` → `ToolExecContext.auto_approve_tools` 传递到执行层，在审批门控和 exec 命令审批中均直接跳过。

---

## exec 工具的独立审批流程

exec 被排除在通用审批门（`name != TOOL_EXEC`）之外，在 `tools/exec.rs` 内部实现自己的命令级审批逻辑：

```mermaid
flowchart TD
    ExecStart([exec 工具被调用]) --> ExecPerm{读取 ToolPermissionMode<br/><small>输入框盾牌按钮</small>}

    ExecPerm -- FullApprove --> ExecRun[✅ 直接执行<br/><small>跳过一切检查，含 allowlist</small>]
    ExecPerm -- AskEveryTime --> ExecAsk[弹出审批对话框]
    ExecPerm -- "Auto（默认）" --> CheckAllowlist{查 exec-approvals.json<br/>allowlist<br/><small>命令前缀匹配</small>}

    CheckAllowlist -- 命中 --> ExecRun
    CheckAllowlist -- 未命中 --> ExecAskAuto[弹出审批对话框]

    ExecAsk --> ExecChoice1{用户选择}
    ExecChoice1 -- 允许一次 --> ExecRun
    ExecChoice1 -- "始终允许<br/><small>（不写 allowlist）</small>" --> ExecRun
    ExecChoice1 -- 拒绝 --> ExecDenied[❌ 命令被拒绝]

    ExecAskAuto --> ExecChoice2{用户选择}
    ExecChoice2 -- 允许一次 --> ExecRun
    ExecChoice2 -- 始终允许 --> WriteExecAllowlist[写入 exec-approvals.json<br/><small>下次同命令自动放行</small>] --> ExecRun
    ExecChoice2 -- 拒绝 --> ExecDenied

```

**Allowlist 持久化文件**：`~/.hope-agent/exec-approvals.json`
**匹配规则**：`extract_command_prefix()` 提取命令首个空格前的单词作为 pattern，前缀匹配。

---

## Plan Mode 工具限制

Plan Mode 在权限控制层面引入了**两层独立限制**：工具可见性裁剪 + 路径级硬限制。详见 [Plan Mode 文档](plan-mode.md)。

### 常量定义（`plan.rs`）

```rust
pub const PLAN_MODE_DENIED_TOOLS: &[&str] = &["write", "edit", "apply_patch", "canvas"];
pub const PLAN_MODE_ASK_TOOLS: &[&str] = &["exec"];
pub const PLAN_MODE_PATH_AWARE_TOOLS: &[&str] = &["write", "edit"];
```

### 1. 工具可见性裁剪（Planning/Review 阶段）

**源码**：`plan.rs` → `PlanAgentConfig` + `commands/chat.rs`
**生效位置**：chat 入口根据 `get_plan_state()` 动态修改 Agent 的 `denied_tools` 和工具注入

| 配置项 | 值 | 效果 |
|--------|-----|------|
| `PlanAgentConfig.allowed_tools` | `["read", "ls", "grep", "find", "glob", "web_search", "web_fetch", "exec", "ask_user_question", "submit_plan", "write", "edit", "recall_memory", "memory_get", "subagent"]` | Plan Agent 白名单，仅这些工具对 LLM 可见 |
| `PLAN_MODE_DENIED_TOOLS` | `["write", "edit", "apply_patch", "canvas"]` | 追加到 `denied_tools`，从 LLM tool schema 中移除 |
| `PLAN_MODE_ASK_TOOLS` | `["exec"]` | 追加到 `ask_tools`，exec 在 Planning 阶段始终弹审批 |

**双 Agent 模式**（`PlanAgentMode` 枚举）：

| 状态 | Agent 模式 | 工具集 |
|------|-----------|--------|
| Off | 正常 | Agent 配置的完整工具集 |
| Planning / Review | PlanAgent | 白名单工具 + path-restricted `write`/`edit` + 条件注入 `ask_user_question`/`submit_plan` |
| Executing / Paused | ExecutingAgent | 全量工具 + 条件注入 `update_plan_step`/`amend_plan` |
| Completed | ExecutingAgent | 全量工具 + 注入 `PLAN_COMPLETED_SYSTEM_PROMPT` |

### 2. 路径级硬限制（Planning 阶段文件写入）

**源码**：`tools/execution.rs`（执行守卫）+ `plan.rs` → `is_plan_mode_path_allowed()`
**触发条件**：`ToolExecContext.plan_mode_allow_paths` 非空时（Planning 阶段由 `PlanAgentConfig.plan_mode_allow_paths = ["plans"]` 自动设置）

在审批门**之后**、实际执行**之前**做路径检查：

```rust
// tools/execution.rs
if !ctx.plan_mode_allow_paths.is_empty() {
    let is_path_aware = matches!(name, TOOL_WRITE | TOOL_EDIT | TOOL_APPLY_PATCH);
    if is_path_aware {
        let target_path = args.get("file_path")
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str()).unwrap_or("");
        if !target_path.is_empty()
            && !crate::plan::is_plan_mode_path_allowed(target_path) {
            return Err("Plan Mode restriction: cannot modify '{path}'");
        }
    }
}
```

**`is_plan_mode_path_allowed()` 判断逻辑**：

```
文件扩展名不是 .md → 拒绝
路径包含 ".hope-agent/plans/" → 允许
路径以 plans_dir()（解析后的绝对路径）开头 → 允许
其他 → 拒绝
```

允许的路径范围：
- 项目本地：`<project>/.hope-agent/plans/*.md`
- 全局目录：`~/.hope-agent/plans/*.md`
- 自定义：`plansDirectory` 配置覆盖的目录下 `*.md`

这是一个**独立于审批的硬限制**，即使审批通过也会被拦截。

### 3. 子 Agent 安全继承

**源码**：`subagent/spawn.rs`

Planning/Review 状态下 spawn 的子 Agent 自动继承 `PLAN_MODE_DENIED_TOOLS`：

```
子 Agent denied_tools = SubagentConfig.deniedTools ∪ PLAN_MODE_DENIED_TOOLS
```

防止子 Agent 绕过 Plan Mode 的工具限制（如通过子 Agent 修改文件）。

---

## 特殊豁免规则

### Internal Tools（永不审批）

通过 `ToolDefinition.internal = true` 标记，`is_internal_tool()` 检查。包括：

- Plan Mode 工具：`ask_user_question` / `submit_plan` / `update_plan_step` / `amend_plan`
- 记忆 / Cron：`save_memory` / `recall_memory` / `memory_get` / `update_memory` / `delete_memory` / `update_core_memory` / `manage_cron`
- 跨会话通信：`agents_list` / `sessions_list` / `session_status` / `sessions_search` / `sessions_history` / `sessions_send` / `peek_sessions`
- 任务追踪：`task_create` / `task_update` / `task_list`
- 附件：`send_attachment`
- 多 Agent 协作：`team` / `canvas` / `send_notification`
- 技能入口：`skill`
- 元工具 / 设置：`tool_search` / `job_status` / `runtime_cancel` / `get_settings` / `update_settings` / `list_settings_backups` / `restore_settings_backup`
- 多模态分析：`image` / `pdf` / `get_weather`

> 注意：以下工具**不在 internal 列表**，默认会被 `require_approval=["*"]` 拦入审批门——
> - 文件操作：`read` / `write` / `edit` / `apply_patch` / `ls` / `grep` / `find`
> - Shell / 进程：`exec`（命令级独立审批） / `process`
> - 网络：`web_fetch` / `web_search` / `browser`
> - 外部服务 / 调用：`image_generate` / `subagent` / `acp_spawn`
> - MCP 内置元工具：`mcp_resource` / `mcp_prompt`（被 `Tier::Mcp` gate 整体管控；未在 internal 列表，故仍走审批）

### SKILL.md 读取（技能预授权）

`is_skill_read()` 检查 — 当 `read` 工具的路径以 `/SKILL.md` 结尾时，在 `AskEveryTime` 和 `Auto` 模式下均跳过审批。

---

## 优先级总结

```mermaid
block-beta
    columns 1

    block:L1:1
        A["🛡️ ToolPermissionMode（输入框盾牌）— 最高优先级"]
    end

    space

    block:L2:1
        B["📋 Agent require_approval（Agent 设置 → 行为）— 仅 Auto 模式生效"]
    end

    space

    block:L3:1
        C["📝 exec Allowlist（命令级持久化白名单）— 仅 Auto 模式 + exec 工具"]
    end

    space

    block:L4:1
        D["⚡ 特殊豁免 — Internal Tools / SKILL.md 读取 → 永不审批"]
    end

    L1 --> L2
    L2 --> L3
    L3 --> L4

```

> **关键理解**：输入框的盾牌（ToolPermissionMode）是全局最高优先级开关，它能完全覆盖 Agent 设置中的 `require_approval` 配置。Agent 设置中的审批配置只在盾牌为 Auto（默认）时才参与决策。

---

## 飞书业务 toolset

v0.2.0 起把飞书除 IM 之外的核心业务 API（云文档 / 多维表格 / 云盘 / 知识库 / 审批 / 日历 / 联系人 / 招聘）做成 internal tools。设计与 PR 切分见 [`docs/plans/feishu-business-tools.md`](../plans/feishu-business-tools.md)；本节只列**对工具系统的契约**。

**凭据复用**：所有 `feishu_*` tool 共享 [`tools::feishu::resolve_feishu_api`](../../crates/ha-core/src/tools/feishu/mod.rs)，从 [`cached_config().channels.accounts`](../../crates/ha-core/src/config/persistence.rs) 找出已配置的飞书账号，按账号 ID 缓存 [`FeishuAuth`](../../crates/ha-core/src/channel/feishu/auth.rs) —— 与 IM 渠道是否 `start_account` 解耦，**即使没有运行 WS 网关，业务 tool 也能用**（[`feishu-business-tools.md` §6.5`](../plans/feishu-business-tools.md) Option B）。token mutex 共享，7200s 内不会双登。

**多账号路由**：每个 tool schema 都有可选 `account` 参数；零账号报错引导用户去 Settings → Channels；单账号自动选；多账号且未指定 `account` 时报错列出可选 ID。

**Tier 与默认值**：全部 Tier 3 Configured，`default_for_main = false / default_for_others = false`（用户主动开），`default_deferred = true`（飞书工具鼓励放进 deferred 池）。`is_globally_configured` 用 `n.starts_with("feishu_")` 通配——所有飞书 tool 共享一个全局配置门：「至少一个飞书账号已配」。未配但 agent 已开 → `HintOnly`，system prompt `# Unconfigured Capabilities` 段引导。

**SSRF 豁免**：飞书域名（feishu.cn / larksuite.com / 自部署）按既有 `channel/feishu/api.rs::authorized_request` 惯例豁免 `security::ssrf::check_url`。每个 `api_<module>.rs` 顶部 doc 注明此豁免，新增非飞书出站 tool 仍必走 SSRF。

**风险等级**：所有飞书业务 tool 标 **MEDIUM**（影响范围限于飞书租户内，不涉及本机文件 / 全局键位 / 凭据）。例外：
- 审批 `feishu_approval_create` / `feishu_approval_cancel` 标 **HIGH**（创建审批实例影响审批流；C6 PR 落地）
- 联系人 `feishu_contact_*` 仍是 MEDIUM 但 doc 必须警示「读取员工个人信息」（C8 PR 落地）

**当前已实现 tool**：

| PR | tool | 用途 |
|---|---|---|
| C1 | `feishu_docx_create` | 新建空文档，返回 `document_id` |
| C1 | `feishu_docx_get_blocks` | 列文档全部 block（分页） |
| C1 | `feishu_docx_append_block` | 在指定 parent 下追加 block |
| C1 | `feishu_docx_update_block_text` | 覆盖式改 block 文本 |
| C2 | `feishu_bitable_list_records` | 列多维表格记录（view + filter expression + 分页） |
| C2 | `feishu_bitable_search_records` | 结构化查询（field projection + sort + filter object DSL） |
| C2 | `feishu_bitable_create_record` | 单条新增记录 |
| C2 | `feishu_bitable_batch_update_records` | 批量更新记录（≤1000/请求） |
| C3 | `feishu_drive_list_files` | 列云盘文件夹内容（含 doc / sheet / bitable / file / folder） |
| C3 | `feishu_drive_upload_media` | 上传本地文件到云盘（≤20MB；走 protected-path 审批） |
| C3 | `feishu_drive_download_media` | 按 file_token 下载到本地（走 protected-path 审批） |
| C4 | `feishu_wiki_get_node` | 由 wiki token 反查节点元信息（space_id / obj_token / obj_type 等） |
| C5 | `feishu_bitable_list_views` | 列多维表格表的所有视图（grid / kanban / gantt / calendar / gallery / form） |
| C5 | `feishu_bitable_get_view` | 取单个视图完整配置（filter / sort / hidden_fields / 等） |
| C5 | `feishu_bitable_list_dashboards` | 列多维表格 app 下所有看板（dashboard_id + name） |
| C6 | `feishu_approval_create_instance` | **HIGH** 提交新审批实例 |
| C6 | `feishu_approval_get_instance` | 查审批实例状态 / 表单 / 时间线 |
| C6 | `feishu_approval_cancel_instance` | **HIGH** 撤销实例 |
| C6 | `feishu_approval_list_instances` | 按 approval_code + 时间区间列实例码 |
| C6 | `feishu_approval_subscribe` | 启用审批事件推送（v0.2.0 仅 log；行为留 v0.3+ B.2） |
| C7 | `feishu_calendar_list` | 列日历 |
| C7 | `feishu_calendar_create_event` | 建会议 / 事件 |
| C7 | `feishu_calendar_list_events` | 列事件（time range） |
| C7 | `feishu_calendar_update_event` | 改会议（patch） |
| C7 | `feishu_calendar_delete_event` | 删会议 |
| C7 | `feishu_calendar_attendees_create` | 邀人（user / chat / resource / third_party） |
| C8 | `feishu_contact_get_user` | 查用户 profile（敏感数据） |
| C8 | `feishu_contact_batch_get_users` | 批量查用户（≤50；敏感） |
| C8 | `feishu_contact_get_department` | 查部门 info |
| C8 | `feishu_contact_search_users_by_department` | 列部门下用户（敏感） |
| C9 | `feishu_hire_list_jobs` | 列招聘岗位（需 hire 模块开通） |
| C9 | `feishu_hire_get_job` | 查岗位详情 |
| C9 | `feishu_hire_list_talents` | 列人才库（敏感） |
| C9 | `feishu_hire_get_talent` | 查候选人详情（敏感） |
| C9 | `feishu_hire_list_applications` | 列投递记录 |

C2-C9 PR 各自往 [`tools::feishu::get_feishu_tools`](../../crates/ha-core/src/tools/feishu/mod.rs) 追加自己的 tool 定义，本表持续 grow。

**测试基线**：每个 `api_<module>.rs` 用 [`wiremock`](https://crates.io/crates/wiremock) 启动 mock HTTP server，覆盖 happy path + 飞书 envelope 错误码（如 `99991672` 权限不足）+ HTTP 5xx。`tools::feishu::*::execute_*` 单测验证参数缺失/类型错误的早期 `anyhow::Error` 路径。

**配套技能**（v0.2.0 收尾）：[`skills/feishu/SKILL.md`](../../skills/feishu/SKILL.md) wrapper skill，`paths: ["飞书","feishu","lark"]` 条件激活，包含常见工作流剧本（OKR 周报 / 审批 / 会议邀请）+ scope 速查表 + 错误码翻译。

---

## 关键源文件索引

| 文件 | 职责 |
|------|------|
| `crates/ha-core/src/tools/approval.rs` | ToolPermissionMode 定义、审批请求/响应、Allowlist 管理 |
| `crates/ha-core/src/tools/execution.rs` | 统一审批门（`execute_tool_with_context`）、Plan Mode 路径检查 |
| `crates/ha-core/src/tools/exec.rs` | exec 独立命令级审批逻辑 |
| `crates/ha-core/src/tools/dispatch.rs` | **注入决策单一入口**：`resolve_tool_fate()` / `DispatchContext` / `ToolFate`、`all_dispatchable_tools()` LazyLock 静态目录、`is_globally_configured()` Tier 3 配置探针 |
| `crates/ha-core/src/tools/definitions/types.rs` | `ToolDefinition` / `ToolTier` / `CoreSubclass` 定义；`to_api_metadata()` 渲染前端 settings UI 元数据 |
| `crates/ha-core/src/tools/definitions/registry.rs` | `is_internal_tool()` / `is_async_capable()` / `is_concurrent_safe()` —— 由 `dispatch::all_dispatchable_tools()` 派生的 LazyLock 缓存 |
| `crates/ha-core/src/async_jobs/` | 异步 Tool 执行（types/db/spawn/injection），独立 `~/.hope-agent/background_jobs.db` |
| `crates/ha-core/src/tools/job_status.rs` | `job_status` 工具：snapshot / 阻塞等待 per-job `Notify` + 100ms→×1.5→2s 退避轮询兜底 |
| `crates/ha-core/src/agent_config.rs` | `FilterConfig`（非 Core 工具 allow/deny 开关覆盖）、`CapabilitiesConfig.require_approval` / `mcp_enabled`、`SubagentConfig.denied_tools` |
| `crates/ha-core/src/agent/mod.rs` | `build_tool_schemas()` / `build_full_system_prompt()` 共享 `dispatch::resolve_tool_fate` 单一注入决策；`tool_context()` 构建 ToolExecContext |
| `crates/ha-core/src/agent/providers/*.rs` | 消费已过滤后的 `tool_schemas` 并发送 API 请求 |
| `crates/ha-core/src/system_prompt/sections.rs` | `build_tools_section()` / `build_deferred_tools_section()` 由 `dispatch::resolve_tool_fate` 驱动，分别渲染 eager 描述段落 / deferred 一行索引 |
| `crates/ha-core/src/tools/tool_search.rs` | `tool_search` 按当前 Agent/Skill/Plan 限制过滤可发现工具 |
| `crates/ha-core/src/tools/execution.rs` | 工具执行前按当前限制做 defense-in-depth 校验 |
| `src-tauri/src/commands/chat.rs` | Tauri 命令层：解析前端 tool_permission_mode 参数并设置全局模式 |
| `crates/ha-server/src/routes/chat.rs` | HTTP 路由层：REST API + WebSocket 流式推送 |
| `src/components/chat/ChatInput.tsx` | 盾牌按钮 UI（三态切换） |
| `src/components/chat/ApprovalDialog.tsx` | 审批弹窗 UI |
| `src/components/settings/agent-panel/tabs/CapabilitiesTab.tsx` | Agent 能力配置 UI（工具注入 / 审批 / 技能） |
| `crates/ha-core/src/channel/worker/approval.rs` | IM Channel 审批交互（EventBus 监听、按钮/文本发送、回调处理） |
| `src/components/settings/channel-panel/EditAccountDialog.tsx` | Channel 设置中的 auto_approve_tools 开关 |

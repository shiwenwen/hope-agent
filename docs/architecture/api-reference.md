# API 参考：Tauri ↔ HTTP/WebSocket 对照

> 返回 [文档索引](../README.md) | 关联源码：[`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs) · [`crates/ha-server/src/lib.rs`](../../crates/ha-server/src/lib.rs) · [`src/lib/transport-http.ts`](../../src/lib/transport-http.ts)

## 概述

Hope Agent 前端通过 `Transport` 抽象层和后端通信，内部根据运行环境自动在 Tauri IPC 和 HTTP/WebSocket 之间切换。本文档把两条通道上的**每一条接口**列成一一对应的表格，并标记对齐状态。

## 数据来源（截至 2026-07-03）

| 源 | 位置 | 数量 |
|---|---|---|
| Tauri 命令 | `src-tauri/src/lib.rs` 的 `tauri::generate_handler!` | **813** |
| HTTP 路由 | `crates/ha-server/src/lib.rs` 的 `.route(...)` | **766** |
| 前端 COMMAND_MAP | `src/lib/transport-http.ts::COMMAND_MAP` | **799** |
| WebSocket 端点 | `crates/ha-server/src/ws/` | **1** |
| EventBus 事件 | 全代码 `emit_event` 调用 | **59+** |

## 对齐情况摘要

| 分类 | 数量 | 说明 |
|---|---|---|
| ✅ 两端完全对齐（在 COMMAND_MAP 中） | 799 | 常规请求/响应命令（所有 COMMAND_MAP 条目都有 Tauri 命令对应） |
| 🔧 特殊处理（不在 COMMAND_MAP 但 HTTP 已实现，走专用 Transport 方法） | 6 | `save_avatar` multipart、`fs_list_dir` / `fs_search_files` query-string GET（`listServerDirectory` / `searchFiles`）、`fs_create_dir`（`createDirectory`）、`project_fs_upload`（`projectFsUpload`）、`export_session_cmd`（`exportSession`，两端形态不对称） |
| 🖥️ Desktop-only / Tauri-only（HTTP 无对应） | 8 | macOS / legacy 系统权限探测（5 条）+ `project_fs_resolve` / `kb_file_resolve_cmd`（`convertFileSrc`）+ `set_dock_badge_cmd` |
| ❌ HTTP 路由存在但 COMMAND_MAP 漏写 | 0 | — |
| ❌ HTTP 路由完全缺失 | 0 | — |

Tauri ↔ COMMAND_MAP 差集为 14 条合法非 REST 命令（5 条 Desktop-only 系统权限命令 + `save_avatar` multipart + `fs_list_dir` / `fs_search_files` query-string GET + `fs_create_dir` / `project_fs_upload` / `export_session_cmd` 走专用 Transport 方法 + `project_fs_resolve` / `kb_file_resolve_cmd` / `set_dock_badge_cmd` Tauri-only）；HTTP 路由侧的非 REST endpoint（`/api/health` / `/api/server/status` / `/api/filesystem/list-dir` / `/api/filesystem/search-files` / `/api/filesystem/create-dir` / `/api/avatars/{...}` multipart / `/api/chat` 流式等）本身不映射到单条 Tauri 命令，它们的对齐状态在各自功能域章节单独说明。新增 Tauri 命令时须同步补 HTTP 路由 + COMMAND_MAP，保持差集不变——详见下文"新增接口 checklist"与"验证脚本"两节。

## 运行模式与 Transport 切换

| 模式 | 前端通信 | 选择逻辑 |
|---|---|---|
| 桌面（Tauri GUI） | Tauri IPC + `@tauri-apps/api/event` | `window.__TAURI_INTERNALS__` 存在 → `TauriTransport` |
| Web / 远程 | HTTP REST + WebSocket | 默认 → `HttpTransport` |

前端业务代码仅调 `getTransport().call(cmd, args)` / `startChat(args, onEvent)` / `listen(event, handler)`，具体如何落地由 Transport 实现决定。

## 鉴权

| 模式 | 机制 |
|---|---|
| Tauri | 无鉴权（本地 IPC） |
| HTTP REST | `Authorization: Bearer <api_key>` header |
| WebSocket | `?token=<api_key>` 查询参数（浏览器 WS 不支持自定义 header） |
| Knowledge Agent 只读 token | `server.knowledgeAgentReadToken` 或 `HA_KNOWLEDGE_AGENT_READ_TOKEN`；仅在 owner API key 已启用时参与鉴权，仅允许 `POST /api/knowledge/agent/{search,read,expand,sources}`，其它受保护 API 返回 403 |
| 免鉴权 | `GET /api/health`、`GET /api/server/status`（server 绑定状态 / 正常运行时间 / WS 数，不含敏感字段） |

`api_key=None` 时中间件全放行。鉴权实现见 [`crates/ha-server/src/middleware.rs`](../../crates/ha-server/src/middleware.rs)（constant-time 比较）。

## WebSocket 端点

| Path | 用途 | 消息格式 |
|---|---|---|
| `/ws/events` | 全局事件广播（EventBus → WS，多客户端同步） | JSON：`{ name: string, payload: unknown }` |

**HTTP 模式重连**：前端 `/ws/events` 指数退避（1s→30s 封顶），只在有活跃 listener 时维持连接；首次 listener 注册自动连上，最后一个取消订阅自动关闭。详见 `src/lib/transport-http.ts` 的 `ensureEventWs` / `scheduleReconnect` / `teardownEventWs`。

## EventBus 事件清单

所有事件由 `ha-core::EventBus` 发射（`BroadcastEventBus`，256 容量），桌面和 HTTP 两条桥各自订阅：
- **Tauri 桥** `src-tauri/src/setup.rs:141` — subscriber 转 `app_handle.emit(name, payload)`
- **HTTP 桥** `crates/ha-server/src/ws/events.rs:34` — subscriber 转 `/ws/events` 文本帧

### 聊天与流式

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `chat:stream_delta` | chat_engine streaming | `{ sessionId, seq, event }`，`seq` 用于重载恢复去重 |
| `chat:stream_end` | tool loop 末轮结束 | `{ sessionId, streamId }` |
| `process:output` | legacy exec process 运行中输出 | `{ process_id, parent_session_id, stream, chunk, truncated, status }`，仅保留下来的 process-session 兼容面使用；普通后台 exec 走 `job:*` + `output_tail` |
| `process:completed` | legacy exec process 终态 | `{ process_id, parent_session_id, status, exit_code?, exit_signal? }`，前端用于收尾 process 卡片，模型侧结果走 `<process-notification>` 注入 |
| `channel:stream_start` / `delta` / `end` | IM 渠道消息生成 | `{ accountId, messageId, ... }` |
| `channel:message_update` | IM 会话有新消息 | `{ accountId, sessionId }` |

### 审批与用户交互

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `approval_required` | tools/approval.rs | `{ requestId, command, cwd, sessionId }` |
| `ask_user_request` | tools/ask_user_question.rs | 结构化问答组 |
| `session_pending_interactions_changed` | 审批 + ask_user 合流 | `{ sessionId, count }` |

### 计划模式

| 事件名 | 触发点 |
|---|---|
| `plan_mode_changed` / `plan_content_updated` / `plan_step_updated` | plan/ 模块 |
| `plan_submitted` / `plan_amended` / `plan_subagent_status` | 同上 |

### Workflow

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `workflow:created` | `workflow::db::create_workflow_run` | `WorkflowRun` 快照 |
| `workflow:updated` | run 状态转换、pause/resume/approve/cancel、launch/recovery owner claim | `WorkflowRun` 快照 |
| `workflow:op_updated` | `workflow_ops` started/completed/failed | `WorkflowOp` 快照 |
| `workflow:event` | `append_workflow_event` | `WorkflowEvent`；大 payload 已在落库前截断到 preview |

### Domain Workflow

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `domain_evidence:recorded` | `domain_workflow::record_domain_evidence` 成功写入后 | `{ id, sessionId, goalId?, projectId?, domain, evidenceType, title, createdAt }`，只广播摘要，不携带完整 `summary` / `sourceMetadata` |

### Managed Worktree

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `worktree:created` | `worktree::create_managed_worktree` | `ManagedWorktree` 快照 |
| `worktree:updated` | `worktree::link_managed_worktree_to_workflow_run` 等元数据更新 | `ManagedWorktree` 快照 |
| `worktree:archived` | `worktree::archive_managed_worktree` | `ManagedWorktree` 快照，含 dirty snapshot |
| `worktree:restored` | `worktree::restore_managed_worktree` | `ManagedWorktree` 快照 |
| `worktree:handoff` | `worktree::handoff_managed_worktree` | `ManagedWorktree` 快照；session working dir 已切换 |
| `project:bootstrap_progress` | `project_bootstrap::bootstrap_project_session` / `worktree::create_managed_worktree` | `{ requestId, status, stage, sessionId?, worktreeId?, message?, errorCode? }` |
| `project:bootstrap_completed` | 首轮 Bootstrap 已交给 Chat Engine | `{ requestId }` |
| `session:git_progress` | `git_control` 长 Git 操作 | `{ requestId, sessionId, operation, status, stage, message?, errorCode? }` |
| `session:git_changed` | stage/branch/commit/push/PR/Handoff 成功 | `{ sessionId, operation, requestId? }` |
| `session:git_completed` | Git operation run 进入终态 | 与 progress 终态同形 |

### LSP

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `lsp:diagnostics` | language server `textDocument/publishDiagnostics` | `{ server, workspaceRoot, uri, count, diagnostics }`；Workspace 面板收到后重拉 owner 快照 |

### Review Engine

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `review:created` | `review::create_review_run` | `ReviewRun` 快照 |
| `review:updated` | review run completed / failed | `ReviewRun` 快照 |
| `review:finding_updated` | finding created / status changed | `ReviewFinding` 快照 |
| `review:event` | `append_review_event` | `ReviewEvent`；大 payload 已在落库前截断到 preview |

### Smart Verification

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `verification:created` | `verification::create_verification_run` | `VerificationRun` 快照 |
| `verification:updated` | verification run planned / completed / failed | `VerificationRun` 快照 |
| `verification:step_updated` | step selected / started / completed | `VerificationStep` 快照 |
| `verification:event` | `append_verification_event` | `VerificationEvent`；大 payload 已在落库前截断到 preview |

### Domain Quality

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `domain_quality:created` | `domain_quality::create_domain_quality_run` | `DomainQualityRun` 快照 |
| `domain_quality:updated` | domain quality run completed / failed | `DomainQualityRun` 快照 |
| `domain_quality:check_updated` | check recorded | `DomainQualityCheck` 快照 |
| `domain_quality:event` | `append_domain_quality_event` | `DomainQualityEvent`；大 payload 已在落库前截断到 preview |

### Goal

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `goal:created` | `goal::create_goal` | `Goal` 快照 |
| `goal:updated` | Goal 状态转换或 final audit 更新 | `Goal` 快照 |
| `goal:event` | `append_goal_event` | `GoalEvent`；大 payload 已在落库前截断到 preview |
| `goal:link_updated` | `link_goal_target` | `GoalLink` 快照 |

### 子代理与团队

| 事件名 | 触发点 |
|---|---|
| `subagent_event` | subagent/helpers.rs 生命周期 |
| `parent_agent_stream` | 子代理结果注入主对话（`eventType: started/delta/done/error`） |
| `team_event` | team/ 模块（`type: created/dissolved/paused/resumed/member_joined/message/...`） |

### 记忆与 Cron

| 事件名 | 触发点 |
|---|---|
| `core_memory_updated` / `memory_extracted` | tools/memory.rs 及自动提取 |
| `dreaming:cycle_started` / `dreaming:cycle_complete` | dreaming 固化周期开始 / 结束（payload 含 `runId`） |
| `cron:run_completed` | cron/executor.rs |
| `cron:unread_changed` | cron 未读聚合数变化（`cron_mark_all_read` 清除时发 `{ total: 0 }`）；前端 cron 未读 store 收到后刷新侧边栏角标 |
| `job:created` / `job:updated` / `job:progress` / `job:completed` / `job:mark_injected_failed` | **统一后台任务事件（R3，替代旧 `async_tool_job:*`）**。`async_jobs::events` 发射；kind-tagged（payload `{ job_id, kind: "tool"\|"group", tool, status, session_id }`），覆盖后台**工具 + Group** 生命周期。`created`=新任务出现（running/queued）；`updated`=非终态变化（如 cancelling）；`progress`=`{ job_id, kind, session_id, current, total }`（目前 Group 报 N/M 子完成）；`completed`=终态；`mark_injected_failed`=结果注入主对话失败告警 `{ job_id, error }`。**`subagent` kind 沿用 `subagent:*` 流**（不双发），R4 面板合并两路 + `job_status list`。 |
| `app_update:progress` / `app_update:completed` | 自升级 (`app_update` 工具) 进度上报。`progress` payload `{ job_id, label, phase, percent?, written?, total? }`（每 5% / 1s 节流）；`completed` payload `{ job_id, status: "done"|"failed", outcome?, error? }`，详见 [`self-update.md`](self-update.md) |

### 项目（Project CRUD）

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `project:created` / `project:updated` / `project:deleted` | `src-tauri/src/commands/project.rs` 调 `bus.emit(...)` | `{ projectId }` |
| `project:file_uploaded` / `project:file_deleted` | 同上文件子命令 | `{ projectId, fileId }` |

> 这些事件经 EventBus 广播，HTTP / Tauri 两条桥都会转发，前端在 server 模式下也能收到（虽然 `bus.emit` 调用点目前只落在 src-tauri 的命令薄壳里）。

### 配置与系统

| 事件名 | 触发点 |
|---|---|
| `config:changed` | `mutate_config()` 写路径（`category: app/user/shortcuts`） |
| `weather-cache-updated` | 天气缓存刷新 |
| `agent:send_notification` | tools/notification.rs（`{ title, body }`） |
| `acp_control_event` | ACP 运行生命周期 |
| `skills:auto_review_complete` | skills 草稿审核完成 |
| `skills:curator_proposals_ready` | auto-curator 周期扫描完成，payload 为 `CuratorReport` |
| `recap_progress` | `/recap` 深度复盘进度 |
| `local_model_job:created` / `:updated` / `:log` / `:completed` | 后台本地模型任务（Ollama 安装、模型拉取、Embedding 拉取）的全生命周期事件，payload 见 `LocalModelJobSnapshot` / `LocalModelJobLogEntry` |
| `local_model:missing_alert` | 默认 chat / embedding 模型文件丢失，payload 见 `LocalModelMissingAlert`（kind / missingModelId / alternatives / canRedownload / canDisableEmbedding） |

### Canvas

| 事件名 | 触发点 |
|---|---|
| `canvas_show` / `canvas_hide` / `canvas_reload` / `canvas_deleted` | 画布面板 |
| `canvas_snapshot_request` / `canvas_eval_request` | 画布工具流 |
| `browser:frame` | 浏览器活动 tab 的实时 JPEG 帧。Payload `{ sessionId?, targetId?, url?, title?, jpegBase64, capturedAt, backend }`。在 `act` / `navigate` / `tabs.new|select|claim` 后由后端自动 emit；BrowserPanel 同时以 1Hz 轮询 `browser_capture_frame` 兜底并按当前会话过滤 |

### MCP

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `mcp:server_status_changed` | `client.rs` set_state 之后 | `{ id, name, state, reason? }` — state ∈ `disabled`/`idle`/`connecting`/`ready`/`needsAuth`/`failed` |
| `mcp:catalog_refreshed` | `refresh_catalog` 完成 | `{ id, name, tools, resources, prompts }` 三项计数 |
| `mcp:auth_required` | OAuth 流程生成 authorize URL | `{ id, name, authUrl }` — 前端 toast + 浏览器打开 |
| `mcp:auth_completed` | OAuth 流程结束 | `{ id, name, ok: bool, error? }` |
| `mcp:servers_changed` | CRUD 写入完成 | `{}` — 触发前端重拉列表（debounced） |
| `mcp:server_log` | 预留（stderr / 生命周期） | `{ id, name, level, line }` |

### Slash 命令副作用

| 事件名 | 触发点 | Payload |
|---|---|---|
| `slash:effort_changed` / `slash:plan_changed` / `slash:session_cleared` | `crates/ha-core/src/channel/worker/slash.rs` 经 `bus.emit(...)` | effort 字段 / sessionId 等（具体见各调用点） |
| `session:model_updated` | `crates/ha-core/src/channel/worker/slash.rs` (IM `/model`)、`src-tauri/src/commands/session.rs::set_session_model`、`crates/ha-server/src/routes/sessions.rs::set_session_model` | `{ sessionId, providerId, modelId }` — 桌面 GUI 仅在 `sessionId == currentSessionId` 时同步 ModelPicker UI |

> 这些事件经 EventBus 广播，HTTP / Tauri 两条桥都会转发。

### 仅 Tauri 直发（不经 EventBus）

| 事件名 | 触发点 |
|---|---|
| `new-session` / `open-settings` | 菜单与快捷键（`src-tauri/src/tray.rs` / `setup.rs` 调 `app_handle.emit(...)`） |
| `chord-first-pressed` / `chord-timeout` / `shortcut-triggered` | 全局快捷键（`src-tauri/src/shortcuts.rs`） |

## 前端 Transport 抽象

接口定义：[`src/lib/transport.ts`](../../src/lib/transport.ts)。

| 方法 | Tauri 实现 | HTTP 实现 |
|---|---|---|
| `call<T>(command, args)` | `invoke(command, args)` | REST 查表 + JSON；multipart 走特例分支 |
| `prepareFileData(buffer, mime)` | `Array.from(Uint8Array)` — JSON 传输（~4× 膨胀） | `new Blob([buffer], {type})` — 零拷贝 |
| `startChat(args, onEvent)` | `new Channel<string>()` + `invoke("chat", { ...args, onEvent })` | `POST /api/chat`；流式 delta 走 `/ws/events` 的 `chat:stream_delta`，仅合成 `session_created` 给 `onEvent` 做新会话 cache rename |
| `listen(eventName, handler)` | `@tauri-apps/api/event.listen` | 全局 `/ws/events` + name 匹配 + 指数退避重连 |
| `resolveMediaUrl(item)` | `convertFileSrc(localPath)` → `tauri://` | 仅支持 `/api/` 或 `http(s)://`，本地绝对路径返 `null` |
| `resolveAssetUrl(path)` | `convertFileSrc` | 正则识别 `avatars`/`image_generate`/`canvas` → `/api/avatars/{n}?token=...` 等 |
| `openMedia(item)` | `invoke("open_directory", {path})` | 临时 `<a download>` 触发浏览器下载 |
| `revealMedia(item)` | `invoke("reveal_in_folder", {path})` | no-op |
| `previewReadText(path,{sessionId})` | `invoke("preview_read_text", {path})` | `GET /api/sessions/{id}/files/read?path=`（会话鉴权） |
| `previewExtractDoc(path,{sessionId})` | `invoke("preview_extract", {path})` | `GET /api/sessions/{id}/files/extract?path=`（会话鉴权） |
| `previewRawUrl(path,{sessionId},download)` | `resolveAssetUrl(path)`（`convertFileSrc`） | tokened `/api/sessions/{id}/files/by-path?...&download=` |
| `supportsLocalFileOps()` | `true` | `false` |
| `pickLocalImage()` | `@tauri-apps/plugin-dialog.open` | 隐藏 `<input type="file">` + blob URL |

**文件上传特殊路径**（在 `HttpTransport.call()` 中走 multipart/form-data 而非 JSON）：

| 命令 | HTTP 端点 |
|---|---|
| `save_attachment` | `POST /api/chat/attachment` |
| `project_fs_upload` | `POST /api/fs/upload`（workspace-scoped，前端 `projectFsUpload` 专用方法） |
| `save_avatar` | `POST /api/avatars`（服务端返 `{path}`，前端解包为 `string` 匹配 Tauri `-> String` 契约） |

## 命令对照表（按功能域分组）

> 所有路径省略 scheme/host，默认 `http(s)://<host>:<port>` 前缀。路径参数 `{id}` / `{sessionId}` 等在请求时 URL 编码。Tauri 模式下命令名即 `invoke()` 的第一个参数。

### Projects

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_projects_cmd` | `GET /api/projects` | ✅ |
| `get_project_cmd` | `GET /api/projects/{id}` | ✅ |
| `create_project_cmd` | `POST /api/projects` | ✅ |
| `update_project_cmd` | `PATCH /api/projects/{id}` | ✅ |
| `delete_project_cmd` | `DELETE /api/projects/{id}` | ✅ |
| `archive_project_cmd` | `POST /api/projects/{id}/archive` | ✅ |
| `list_project_sessions_cmd` | `GET /api/projects/{id}/sessions` | ✅ |
| `move_session_to_project_cmd` | `PATCH /api/sessions/{sessionId}/project` | ✅ |
| `list_project_memories_cmd` | `GET /api/projects/{id}/memories` | ✅ |

`list_projects_cmd` / `GET /api/projects` 接受可选 `active_session_id`（HTTP query `activeSessionId`）：正在打开的那个会话会从其所属项目的未读聚合里排除（在 SQL 里按已读处理），使项目徽标与“当前会话读作 0”一致，无需前端跨数据源相减。

**项目文件浏览器（workspace-scoped filesystem）**——上传/读写改走作用域文件管理 API（旧的 `list_project_files_cmd` / `upload_project_file_cmd` / `delete_project_file_cmd` / `rename_project_file_cmd` / `read_project_file_content_cmd` 五条命令与对应 `/api/projects/{id}/files*` 路由已删除）。命令以 `{ scope: "session"|"project", scopeId, ... }` 寻址，后端 `WorkspaceScope` 解析工作目录并做越界校验：

| Tauri 命令 | HTTP 路由 | 对齐 |
|---|---|---|
| `project_fs_list` | `GET /api/fs/list?scope=&scopeId=&path=` | ✅ |
| `project_fs_read_text` | `GET /api/fs/read?...` | ✅ |
| `project_fs_extract` | `GET /api/fs/extract?...` | ✅ (PDF/Office 提取预览) |
| `project_fs_write_text` | `PUT /api/fs/file` | ✅ (写闸门) |
| `project_fs_delete` | `DELETE /api/fs/entry?...&recursive=` | ✅ (写闸门) |
| `project_fs_rename` | `POST /api/fs/rename` | ✅ (写闸门) |
| `project_fs_mkdir` | `POST /api/fs/mkdir` | ✅ (写闸门) |
| `project_fs_upload` | `POST /api/fs/upload` (multipart) | ✅ (写闸门，`projectFsUpload` 专用方法) |
| `project_fs_resolve` | —（Tauri-only，图片预览 `convertFileSrc`） | N/A |
| —（HTTP-only raw serve） | `GET /api/fs/raw?...&download=` | N/A (`projectFsRawUrl` 专用方法) |
| `preview_read_text` | `GET /api/sessions/{id}/files/read?path=` | ✅ (preview-by-path，绝对路径，会话鉴权) |
| `preview_extract` | `GET /api/sessions/{id}/files/extract?path=` | ✅ (preview-by-path，绝对路径，会话鉴权) |

### Knowledge Base（知识空间）

**Owner / 管理平面**——全局 API key 持有者 = owner-equivalent，看自己全部 KB，**不经 `effective_kb_access`**（那是 agent `note_*` 工具平面）。Knowledge Agent read token 只在 owner API key 保护开启时生效，只允许下表中的 `knowledge_agent_{search,read,expand,sources}_cmd` HTTP 路由，不能访问 compile/propose 或任何管理端点；若 server 处于 no-auth 模式，read token 不会单独启用鉴权。详见 [knowledge-base.md](./knowledge-base.md)（实现 + 设计契约 D1–D20）。

| Tauri 命令 | HTTP 路由 | 对齐 |
|---|---|---|
| `list_kbs_cmd` | `GET /api/knowledge?includeArchived=` | ✅ |
| `get_kb_cmd` | `GET /api/knowledge/{id}` | ✅ |
| `create_kb_cmd` | `POST /api/knowledge` (`{ input }`) | ✅ |
| `update_kb_cmd` | `PATCH /api/knowledge/{id}` (`{ patch }`) | ✅ |
| `delete_kb_cmd` | `DELETE /api/knowledge/{id}` | ✅ (级联 registry+index+磁盘) |
| `reindex_kb_cmd` | `POST /api/knowledge/{id}/reindex` | ✅ |
| `attach_session_kb_cmd` | `POST /api/knowledge/attach` (`{ sessionId, kbId, access }`) | ✅ |
| `attach_project_kb_cmd` | `POST /api/knowledge/attach` (`{ projectId, kbId, access }`) | ✅ |
| `detach_session_kb_cmd` | `POST /api/knowledge/detach` (`{ sessionId, kbId }`) | ✅ |
| `detach_project_kb_cmd` | `POST /api/knowledge/detach` (`{ projectId, kbId }`) | ✅ |
| `list_session_kbs_cmd` | `GET /api/knowledge/attachments?sessionId=&projectId=` | ✅ (当前生效 KB 列表) |
| `list_project_kbs_cmd` | `GET /api/knowledge/project-attachments?projectId=` | ✅ (项目级绑定列表，项目设置 UI) |
| `list_kb_notes_cmd` | `GET /api/knowledge/{kbId}/notes` | ✅ |
| `kb_note_read_cmd` | `GET /api/knowledge/{kbId}/note?path=` | ✅ (含出链/反链/标签) |
| `kb_note_save_cmd` | `PUT /api/knowledge/{kbId}/note` | ✅ (写闸门 + stale-write guard) |
| `kb_note_delete_cmd` | `DELETE /api/knowledge/{kbId}/note?path=` | ✅ (写闸门) |
| `kb_note_rename_cmd` | `POST /api/knowledge/{kbId}/note/rename` | ✅ (写闸门 + **改写入站 `[[ ]]` 链接** #9，返回 `RenameOutcome`) |
| `kb_list_dirs_cmd` | `GET /api/knowledge/{kbId}/dirs` | ✅ (含空目录，读盘) |
| `kb_list_tags_cmd` | `GET /api/knowledge/{kbId}/tags` | ✅ (owner 平面，编辑器 `#tag` 补全词表) |
| `knowledge_embedding_get_cmd` | `GET /api/knowledge/embedding` | ✅ (D7 独立 selector 状态) |
| `knowledge_embedding_set_default_cmd` | `POST /api/knowledge/embedding/set-default` | ✅ (装 embedder + 后台 KnowledgeReembed) |
| `knowledge_embedding_disable_cmd` | `POST /api/knowledge/embedding/disable` | ✅ (pause 语义，清 index embedder) |
| `knowledge_embedding_rebuild_cmd` | `POST /api/knowledge/embedding/rebuild` | ✅ (强制全 KB 重建，无 same-signature 短路) |
| `knowledge_chunk_get_cmd` | `GET /api/knowledge/chunk` | ✅ (D12 分块参数，GUI-only) |
| `knowledge_chunk_set_cmd` | `POST /api/knowledge/chunk` | ✅ (写参数 clamp + 触发全 KB 重切) |
| `knowledge_search_config_get_cmd` | `GET /api/knowledge/search-config` | ✅ (混合检索排序参数 `KnowledgeSearchConfig`：融合权重 / RRF-k / MMR-λ / 候选倍数) |
| `knowledge_search_config_set_cmd` | `POST /api/knowledge/search-config` | ✅ (body `{config}`，clamp 后保存、无重索引；发默认值即恢复默认) |
| `reindex_note_cmd` | `POST /api/knowledge/{kbId}/note/reindex` | ✅ (单篇重建，同步) |
| `reindex_dir_cmd` | `POST /api/knowledge/{kbId}/dir/reindex` | ✅ (文件夹子树重建，同步) |
| `kb_mkdir_cmd` | `POST /api/knowledge/{kbId}/dir` | ✅ (写闸门) |
| `kb_rename_dir_cmd` | `POST /api/knowledge/{kbId}/dir/rename` | ✅ (写闸门 + reindex + **改写入站路径式链接** #9，返回 `RenameOutcome`) |
| `kb_delete_dir_cmd` | `DELETE /api/knowledge/{kbId}/dir?path=` | ✅ (写闸门，rm -rf + prune) |
| `kb_backlinks_cmd` | `GET /api/knowledge/{kbId}/backlinks?path=` | ✅ |
| `kb_broken_links_cmd` | `GET /api/knowledge/{kbId}/broken-links` | ✅ (维护面板：悬空 `[[ ]]` 清单) |
| `kb_orphans_cmd` | `GET /api/knowledge/{kbId}/orphans` | ✅ (维护面板：无链接孤岛笔记) |
| `kb_graph_cmd` | `GET /api/knowledge/{kbId}/graph` | ✅ (WS1 图谱视图：nodes+edges，含 degree，节点上限 2000 截断标 `truncated`) |
| `kb_graph_layout_get_cmd` | `GET /api/knowledge/{kbId}/graph/layout` | ✅ (Batch J 用户拖拽固定的节点坐标，按 `relPath` 键，落 sessions.db) |
| `kb_graph_layout_save_cmd` | `POST /api/knowledge/{kbId}/graph/layout` | ✅ (Batch J 整体替换布局，body `{positions:[{relPath,x,y}]}`，空数组=重置) |
| `kb_chat_thread_get_cmd` | `GET /api/knowledge/{kbId}/chat/thread?note=` | ✅ (侧边栏对话默认加载：某笔记最近一次 `kind=knowledge` 会话 `SessionMeta`，无则 `null`) |
| `kb_chat_threads_list_cmd` | `GET /api/knowledge/{kbId}/chat/threads?query=&limit=&offset=` | ✅ (历史对话列表分页：KB 内对话线程 `KbChatThread[]`，`query` 非空时 FTS 过滤；`limit` 默认 50 钳 1..=200、`offset` 翻页，FTS 走 `IN` 子查询使 `LIMIT` 作用于命中集) |
| `kb_ai_rewrite_cmd` | `POST /api/knowledge/ai/rewrite` | ✅ (快捷改写：body `{text, instruction, modelOverride?}` → side_query 返回改写后 Markdown；不落盘，GUI 走 diff 确认后经 `note_save`) |
| `kb_rewrite_log_cmd` | `POST /api/knowledge/rewrite/log` | ✅ (快捷改写统计：body `{kbId, notePath?, instruction, model?, charsBefore, charsAfter, accepted}` → 落 `learning_events`(`kind="kb_quick_rewrite"`)，best-effort) |
| `kb_maintenance_run_cmd` | `POST /api/knowledge/maintenance/run` | ✅ (WS6 手动跑一轮维护：扫全部内部 KB 生成 draft 提案；返回 `MaintenanceReport`) |
| `kb_maintenance_status_cmd` | `GET /api/knowledge/maintenance/status` | ✅ (running 标志 + 上轮 report) |
| `kb_maintenance_list_cmd` | `GET /api/knowledge/{kbId}/maintenance/proposals?status=` | ✅ (某 KB 的提案，可按 draft/applied/rejected/failed 过滤) |
| `kb_maintenance_pending_count_cmd` | `GET /api/knowledge/{kbId}/maintenance/pending-count` | ✅ (待审提案数，维护面板徽章) |
| `kb_maintenance_approve_cmd` | `POST /api/knowledge/maintenance/proposals/{id}/approve` | ✅ (批准并经 owner 平面落地，返回更新后的提案) |
| `kb_maintenance_reject_cmd` | `POST /api/knowledge/maintenance/proposals/{id}/reject` | ✅ (忽略单条提案) |
| `kb_maintenance_reject_all_cmd` | `POST /api/knowledge/{kbId}/maintenance/reject-all` | ✅ (清空某 KB 待审队列，返回清除数) |
| `kb_maintenance_config_get_cmd` | `GET /api/knowledge/maintenance/config` | ✅ (维护配置，GUI 面板；也可经 `get_settings(knowledge_maintenance)` 读) |
| `kb_maintenance_config_set_cmd` | `POST /api/knowledge/maintenance/config` | ✅ (写维护配置，clamp 后返回；emit `config:changed` 唤醒 cron loop) |
| `kb_passive_recall_config_get_cmd` | `GET /api/knowledge/passive-recall/config` | ✅ (读取桥③ 被动相关笔记配置，GUI 面板；也可经 `get_settings(knowledge_passive_recall)` 读) |
| `kb_passive_recall_config_set_cmd` | `POST /api/knowledge/passive-recall/config` | ✅ (写被动相关笔记配置，clamp 后返回) |
| `knowledge_media_retention_config_get_cmd` | `GET /api/knowledge/media-retention/config` | ✅ (读取原始媒体可选留存配置；默认关闭，HIGH/privacy，也可经 `get_settings(knowledge_media_retention)` 读) |
| `knowledge_media_retention_config_set_cmd` | `POST /api/knowledge/media-retention/config` | ✅ (写原始媒体可选留存配置，clamp 后返回；只影响未来 source 导入) |
| `knowledge_vision_config_get_cmd` | `GET /api/knowledge/vision/config` | ✅ (读取图片 OCR 模型链配置，GUI 面板；也可经 `get_settings(knowledge_vision)` 读) |
| `knowledge_vision_config_set_cmd` | `POST /api/knowledge/vision/config` | ✅ (写图片 OCR 模型链配置) |
| `note_tools_config_get_cmd` | `GET /api/knowledge/note-tools/config` | ✅ (读取笔记三件套共享模型链配置，GUI 面板；也可经 `get_settings(note_tools)` 读) |
| `note_tools_config_set_cmd` | `POST /api/knowledge/note-tools/config` | ✅ (写笔记三件套共享模型链配置) |
| `kb_sprite_observe_cmd` | `POST /api/knowledge/sprite/observe` | ✅ (精灵编辑空闲触发，fire-and-forget；节流 + side_query 后建议经 `sprite:suggestion` 事件返回) |
| `sprite_config_get_cmd` | `GET /api/knowledge/sprite/config` | ✅ (读精灵配置，GUI 面板；也可经 `get_settings(sprite)` 读) |
| `sprite_config_set_cmd` | `POST /api/knowledge/sprite/config` | ✅ (写精灵配置，clamp 后返回) |
| `kb_source_import_batch_cmd` | `POST /api/knowledge/{kbId}/sources/batch` | ✅ (资料舱批量导入：文本 / Markdown / PDF / DOCX / 音视频转录 / 图片 OCR / URL，创建 import run + item 后返回 `running` run；后台执行，重复内容标 `duplicate`) |
| `kb_source_import_session_attachment_cmd` | `POST /api/knowledge/{kbId}/sources/session-attachment` | ✅ (把已落到会话附件目录的聊天 / IM 附件归档为 raw source；后端校验 `sessionId + path` 位于该 session attachments dir，再复用文本 / PDF / DOCX / STT / OCR 导入链路) |
| `kb_source_asset_link_cmd` | `GET /api/knowledge/{kbId}/sources/{sourceId}/assets/{original\|thumbnail}/link` | ✅ (返回 retained source asset metadata + owner-plane local path；文件流走同路径去掉 `/link`，可加 `?download=1`) |
| `kb_source_import_runs_list_cmd` | `GET /api/knowledge/{kbId}/sources/import-runs?limit=` | ✅ (导入历史，limit 默认 20、钳 1..=200) |
| `kb_source_import_run_detail_cmd` | `GET /api/knowledge/{kbId}/sources/import-runs/{runId}` | ✅ (导入 run 明细 + item 状态，不回显原始 `input_json`) |
| `kb_source_import_retry_failed_cmd` | `POST /api/knowledge/{kbId}/sources/import-runs/{runId}/retry-failed` | ✅ (重试 failed item，校验 run 属于目标 KB，复用原 input_json) |
| `kb_source_ocr_pages_cmd` | `GET /api/knowledge/{kbId}/sources/{sourceId}/ocr-pages` | ✅ (扫描版 PDF 逐页 OCR 状态账本，见 knowledge-base.md 扫描版 PDF OCR 兜底一节) |
| `kb_source_ocr_retry_cmd` | `POST /api/knowledge/{kbId}/sources/{sourceId}/ocr-retry` | ✅ (重试当前失败页，后台执行、立即返回 source) |
| `kb_source_similarity_groups_cmd` | `GET /api/knowledge/{kbId}/sources/similar` | ✅ (资料去重治理：同 KB shingle/Jaccard 相似分组 + 跨 KB exact duplicate 提示，过滤已忽略 fingerprint) |
| `kb_source_similarity_dismiss_cmd` | `POST /api/knowledge/{kbId}/sources/similar/dismiss` | ✅ (按 fingerprint 持久忽略相似/重复 source 建议) |
| `kb_source_similarity_resolve_cmd` | `POST /api/knowledge/{kbId}/sources/similar/resolve` | ✅ (保留一个 source、删除当前 KB 内选定重复 source，并把该 fingerprint 记为已解决；不跨 KB 删除) |
| `kb_source_sync_external_raw_cmd` | `POST /api/knowledge/{kbId}/sources/sync-external-raw` | ✅ (把已有 source/version 文本快照镜像到外部 vault 的 `raw/` 或 `sources/`；仅外部 KB + 外部写 opt-in + `externalRawSync` 开启时可用，返回 synced/failed 计数) |
| `kb_evidence_coverage_cmd` | `GET /api/knowledge/{kbId}/evidence/coverage` | ✅ (Evidence 派生索引覆盖率：compiled note 数、claim-level evidence 命中数、missing/stale refs，用于维护面板) |
| `kb_evidence_source_claims_cmd` | `GET /api/knowledge/{kbId}/evidence/sources/{sourceId}/claims` | ✅ (按 raw source 反查引用它的 compiled claims，实时 hydrate missing/stale/superseded 状态) |
| `kb_evidence_rebuild_cmd` | `POST /api/knowledge/{kbId}/evidence/rebuild` | ✅ (从 `.md` 全量重建 `knowledge_evidence_refs` / `knowledge_evidence_claims` 派生索引，返回扫描 note/ref/claim 数) |
| `kb_note_read_ref_cmd` | `GET /api/knowledge/{kbId}/note/resolve?reference=` | ✅ (WS2 transclusion：按 `[[ ]]` ref 经 resolver 取目标 `NoteReadResult`，broken 返回 `null`；Batch G 起按 ref 的 `#anchor` 切片——`^id`→块、heading→标题段，未命中降级整篇) |
| `kb_search_cmd` | `GET /api/knowledge/search?query=&kbId=&limit=` | ✅ (FTS+向量混合) |
| `knowledge_agent_search_cmd` | `POST /api/knowledge/agent/search` | ✅ (`knowledge.search`；body 可为 `{input}` 或裸 input；notes-first，返回 `truncated`；read token 允许；`includeSources=true` 时 raw source 单独返回且必须传 `kbId`) |
| `knowledge_agent_read_cmd` | `POST /api/knowledge/agent/read` | ✅ (`knowledge.read`；read token 允许；`path`/`reference` 二选一，返回全文 + links/tags/source refs + `kind`) |
| `knowledge_agent_expand_cmd` | `POST /api/knowledge/agent/expand` | ✅ (`knowledge.expand`；read token 允许；读取 note + related notes) |
| `knowledge_agent_sources_cmd` | `POST /api/knowledge/agent/sources` | ✅ (`knowledge.sources`；read token 允许；list 默认 metadata/snippet，返回 `truncated`；只有显式 `sourceId + includeContent` 返回 source 全文) |
| `knowledge_agent_compile_propose_cmd` | `POST /api/knowledge/agent/compile/propose` | ✅ (`knowledge.compile.propose`；owner API key required，read token 禁止；启动 compile run，仅产 Review Diff proposals，不直接写 `.md`) |
| `kb_file_read_cmd` | `GET /api/knowledge/{kbId}/files/read?path=` | ✅ (纯 owner 平面 + scope contains) |
| `kb_file_extract_cmd` | `GET /api/knowledge/{kbId}/files/extract?path=` | ✅ |
| `kb_file_resolve_cmd` | —（Tauri-only，`convertFileSrc`） | N/A |
| —（HTTP-only raw serve） | `GET /api/knowledge/{kbId}/files/raw?path=&download=` | N/A |

KB 文件预览端点是**纯 owner 平面，无 session 参数、无 owner fallback**——与 `/api/sessions/{id}/files/*` 物理隔离，不放宽其判定。外部绑定 vault 默认只读（写经 `WorkspaceScope::resolve_writable` 拒绝 + HTTP `allow_remote_writes` 闸门双拒）。agent 读笔记不经此端点，走 `note_*` 工具（`effective_kb_access` 校验）。`knowledge:changed` 事件 `{ kbId, op }` 经 EventBus fan-out 到两端前端。

**preview-by-path（文件操作统一）**：`preview_read_text` / `preview_extract` 按**绝对路径**读取，供 Markdown 链接 / 下挂文件 / 工作台产物文件统一预览。桌面信任本机路径直接读；HTTP 经 `/api/sessions/{id}/files/{read,extract}`，与既有 `/files/by-path` 共用授权 `authorized_canonical_file_path` = **被会话 tool 消息引用 ∪ 落在会话工作目录内**，二者皆非的主机任意路径一律 403。详见 [file-operations.md](./file-operations.md)。

写端点（write/delete/rename/mkdir/upload）在 HTTP handler 层读 `filesystem.allow_remote_writes`（默认 false）闸门，为 false 返 403；桌面 Tauri 不受限。配置读写：`get_filesystem_config` / `save_filesystem_config` ↔ `GET/PUT /api/config/filesystem`。

`Project` 支持 `workingDir: string | null` 字段，作为该项目下会话的默认工作目录。运行时合并优先级 `session.working_dir > project 显式 working_dir > 默认 workspace`，lazy ensure 创建——编辑项目工作目录后未单独设置的已有会话立即跟随。详见 [`AGENTS.md`](../../AGENTS.md) 「项目（Project）容器」段与 [project.md](./project.md)。

**Project ↔ IM Channel 反向认领已废弃**。`Project.boundChannel` / `BoundChannel` 类型 + `projects.bound_channel_id` / `bound_channel_account_id` DB 列 + `idx_projects_bound_channel` 索引 + `find_by_bound_channel` API 全部删除；`UpdateProjectInput` 不再有 `boundChannel` 字段。IM 入站消息不再自动归属项目，新会话以 `project_id = NULL` 创建。要把会话归项目，从 IM chat 内 `/project <id>` 显式触发：handler 检测 `session.channel_info` 后发 `AssignProject` action，channel worker 调 `SessionDB::set_session_project` 直接 UPDATE 现有 `sessions.project_id`，**不创建新 session**。详见 [im-channel.md](./im-channel.md) 「Session 路由」章节。

### Sessions

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_sessions_cmd` | `GET /api/sessions?agentId=&projectId=&unassigned=&parentSession=&limit=&offset=&activeSessionId=` | ✅（`parentSession=true/false` 分别只取子会话/顶层会话，过滤发生在分页前） |
| `create_session_cmd` | `POST /api/sessions` | ✅ |
| `get_session_cmd` | `GET /api/sessions/{id}` | ✅ |
| `set_session_incognito` | `PATCH /api/sessions/{sessionId}/incognito` | ✅ |
| `set_session_working_dir` | `PATCH /api/sessions/{sessionId}/working-dir` | ✅ |
| `update_session_agent_cmd` | `PATCH /api/sessions/{sessionId}/agent` | ✅ |
| `set_session_model` | `PATCH /api/sessions/{sessionId}/model` | ✅ |
| `get_execution_mode` | `GET /api/sessions/{sessionId}/execution-mode` | ✅ |
| `set_execution_mode` | `POST /api/sessions/{sessionId}/execution-mode` | ✅ |
| `purge_session_if_incognito` | `POST /api/sessions/{sessionId}/purge-if-incognito` | ✅ |
| `search_sessions_cmd` | `GET /api/sessions/search` | ✅ |
| `search_session_messages_cmd` | `GET /api/sessions/{sessionId}/messages/search` | ✅ |
| `load_session_messages_latest_cmd` | `GET /api/sessions/{sessionId}/messages` | ✅ |
| `load_session_messages_around_cmd` | `GET /api/sessions/{sessionId}/messages/around` | ✅ |
| `load_session_messages_before_cmd` | `GET /api/sessions/{sessionId}/messages/before` | ✅ |
| `load_session_messages_after_cmd` | `GET /api/sessions/{sessionId}/messages/after` | ✅ |
| `load_session_artifacts_cmd` | `GET /api/sessions/{sessionId}/artifacts` | ✅ |
| `list_background_jobs` | `GET /api/sessions/{sessionId}/background-jobs` | ✅ |
| `get_background_job` | `GET /api/background-jobs/{jobId}` | ✅ |
| `get_session_stream_state` | `GET /api/sessions/{sessionId}/stream-state` | ✅ |
| `delete_session_cmd` | `DELETE /api/sessions/{sessionId}` | ✅ |
| `rename_session_cmd` | `PATCH /api/sessions/{sessionId}` | ✅ |
| `mark_session_read_cmd` | `POST /api/sessions/{sessionId}/read` | ✅ |
| `mark_session_read_batch_cmd` | `POST /api/sessions/read-batch` | ✅ |
| `mark_all_sessions_read_cmd` | `POST /api/sessions/read-all` | ✅ |
| `compact_context_now` | `POST /api/sessions/{sessionId}/compact` | ✅ |
| `export_session_cmd` | `GET /api/sessions/{sessionId}/export` | ✅ |
| `write_export_file` | `POST /api/misc/write-export-file` | ✅ |
| `get_dangerous_mode_status` | `GET /api/security/dangerous-status` | ✅ |
| `set_dangerous_skip_all_approvals` | `POST /api/security/dangerous-skip-all-approvals` | ✅ |

`create_session_cmd` 与 `chat` 在自动创建新会话时都支持可选 `incognito: boolean`，返回的 `SessionMeta` 也会包含 `incognito` 字段；主聊天 UI 将 incognito 视为“新会话预设”，只在尚未 materialize session 的草稿态提供入口，已有会话不再暴露切换按钮。`set_session_incognito` 保留给兼容调用和非主 UI 适配，但不应作为常规会话内开关使用。当请求同时带了 `project_id` 时 `incognito` 被强制为 `false`（互斥）。`list_sessions_cmd` / `search_sessions_cmd` / `list_project_sessions_cmd` 接受可选 `active_session_id` 参数：默认会过滤掉所有 incognito 会话，`active_session_id` 让正在打开的那个无痕会话仍出现在 sidebar / 搜索结果里。`purge_session_if_incognito` 在前端 `handleSwitchSession / handleNewChat / handleNewChatInProject` 切走当前 session 之前调用，仅当目标 session 当前为 incognito 时硬删，否则 no-op。

`update_session_agent_cmd` 接受 `{ agentId: string }`，后端在 SQL 层校验 `messages` 表中该 session 没有 `role IN ('user','assistant')` 的记录，否则返回 400。前端 `ChatTitleBar` 的 `AgentSwitcher` dropdown 在 `messages.length > 0` 时会把触发器降级为只读 `<span>`，作为 UX 防御层。

`set_session_model` 接受 `{ providerId, modelId }`，把模型固定到当前会话（写 `sessions.provider_id` / `provider_name` / `model_id`），不写 `AppConfig.active_model`——v0.2.1 起这是「会话内切模型」的唯一合法入口。`get_active_model` / `POST /api/models/active` 仍然存在，但**只该被 Settings 「模型」面板 / onboarding wizard / 本地 LLM 安装路径**调用，用来修改应用全局默认；任何 chat 内或 IM 内的"切模型"语义都必须落到 session 级。chat_engine 解析优先级 `plan_model > 本轮 model_override > sessions.provider_id > agent.model.primary > AppConfig.active_model`（详见 [`provider-system.md` § 7.2](provider-system.md#72-模型链解析)）。写入后 emit `session:model_updated`，桌面 GUI 仅在 `sessionId == currentSessionId` 时同步 ModelPicker。

`get_execution_mode` / `set_execution_mode` 是会话级执行模式入口，对应 `/mode off|guarded|deep|autonomous` 与 Workspace/Workflow 面板中的 Execution Mode 控件，写 `sessions.execution_mode`。该值会进入下一轮 system prompt 的动态段，控制长任务的观察、计划、验证、修复和停止策略；它不是 `/loop`，也不负责定时、重复触发或条件轮询。

### Managed Worktrees

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_managed_worktrees` | `GET /api/sessions/{sessionId}/worktrees` | ✅ |
| `create_managed_worktree` | `POST /api/sessions/{sessionId}/worktrees` | ✅ |
| `get_managed_worktree` | `GET /api/worktrees/{worktreeId}` | ✅ |
| `archive_managed_worktree` | `POST /api/worktrees/{worktreeId}/archive` | ✅ |
| `restore_managed_worktree` | `POST /api/worktrees/{worktreeId}/restore` | ✅ |
| `handoff_managed_worktree` | `POST /api/worktrees/{worktreeId}/handoff` | ✅ |
| `get_project_bootstrap_run` | `GET /api/project-bootstrap/{requestId}` | ✅ |
| `cancel_project_bootstrap` | `POST /api/project-bootstrap/{requestId}/cancel` | ✅ |

Managed Worktree owner API 管理 session-scoped durable git worktree。`create_managed_worktree` 拒绝 incognito session，默认在 `~/.hope-agent/worktrees/<repo-slug>/<wt-id>` 创建 detached worktree，并支持 `WorktreeCreate` hook 接管创建；`archive` 会记录 dirty snapshot，clean worktree 才 best-effort remove；`restore` 可重建已清理路径；生命周期兼容 `handoff` 只负责绑定父 session cwd，不复制 Git 改动。`chat` / `POST /api/chat` 的新项目草稿可带 `projectBootstrap`；Bootstrap 查询与取消接口用于断线恢复和停止准备。完整契约见 [Managed Worktree 控制平面](worktree.md)。

`POST /api/chat` 的 `projectBootstrap` 可能执行本地分支切换或创建 Worktree，因此与 Git 写端点共用 `filesystem.allow_remote_writes` 默认关闭闸门，并在创建临时 Session 前返回 403；普通聊天以及 Bootstrap 状态查询/取消不受此闸门影响。

### Session Git

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `load_session_git_control_cmd` | `GET /api/sessions/{id}/git` | ✅ |
| `load_session_git_diff_snapshot_cmd` | `GET /api/sessions/{id}/git/diff?scope=unstaged\|staged\|all` | ✅ |
| `mutate_session_git_index_cmd` | `POST /api/sessions/{id}/git/index` | ✅ |
| `switch_session_git_branch_cmd` | `POST /api/sessions/{id}/git/branch/switch` | ✅ |
| `create_session_git_branch_cmd` | `POST /api/sessions/{id}/git/branch/create` | ✅ |
| `commit_session_git_cmd` | `POST /api/sessions/{id}/git/commit` | ✅ |
| `push_session_git_cmd` | `POST /api/sessions/{id}/git/push` | ✅ |
| `session_git_pr_preflight_cmd` | `GET /api/sessions/{id}/git/pull-request` | ✅ |
| `load_session_git_pr_feedback_cmd` | `GET /api/sessions/{id}/git/pull-request/feedback` | ✅ |
| `create_session_git_pr_cmd` | `POST /api/sessions/{id}/git/pull-request` | ✅ |
| `enable_session_git_pr_auto_merge_cmd` | `POST /api/sessions/{id}/git/pull-request/auto-merge` | ✅ |
| `handoff_session_git_cmd` | `POST /api/sessions/{id}/git/handoff` | ✅ |
| `get_git_operation_run_cmd` | `GET /api/git-runs/{requestId}` | ✅ |

所有端点只按 session 解析 cwd，不接受客户端指定仓库根目录。HTTP 写端点受 `filesystem.allow_remote_writes` 闸门；PR 网络读取通过已认证的本机 `gh` 获取当前 PR 详情、checks、顶层 reviews 与未解决 review threads，不接受客户端传入 PR 标识。Feedback 的 checks/comments 独立容错并分别返回截断和错误字段；PR 外部文本按不可信数据处理。“修复”只填入当前会话输入框，不自动发送或执行。自动合并必须携带 revision、合并方式和显式确认，存在冲突时拒绝，并纳入 `requestId` 幂等记录。完整 DTO、锁、幂等、Handoff 与失败恢复契约见 [Session Git 控制平面](git-control.md)。

### LSP / Diagnostics

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_lsp_status` | `GET /api/sessions/{sessionId}/lsp/status` | ✅ |
| `get_lsp_diagnostics` | `GET /api/sessions/{sessionId}/lsp/diagnostics` | ✅ |

LSP owner API 返回当前 session working dir 对应 workspace 的 language server 状态和 diagnostics 快照。Agent 侧语义导航走 builtin `lsp` 工具；owner API 只服务 Workspace GUI / HTTP client 读取状态。无痕会话不启动 LSP，也不会注入 diagnostics prompt 后缀。完整契约见 [LSP 与语义代码智能](lsp.md)。

### Context Retrieval v2

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_context_retrieval` | `GET /api/sessions/{sessionId}/context-retrieval?query=&limit=&domain=&templateId=` | ✅ |
| `get_session_ide_context` | `GET /api/sessions/{sessionId}/ide-context` | ✅ |
| `save_session_ide_context` | `PUT /api/sessions/{sessionId}/ide-context` | ✅ |
| `clear_session_ide_context` | `DELETE /api/sessions/{sessionId}/ide-context` | ✅ |

Context Retrieval owner API 返回当前 session 的任务感知推荐上下文。后端聚合 Git diff、历史 artifacts、LSP diagnostics / workspace symbols、Review findings、Smart Verification steps、Goal evidence、tasks、Workflow ops、IDE/ACP context、file search v2 与 URL 来源，并按信号强度 + query boost 排序。Phase 7.3 起可选 `domain/templateId` 会启用 Domain Context Retrieval；未显式传入时也会从 `workflow_runs.kind=domain:<domain>`、domain evidence 或 Goal objective / criteria 推断 domain profile，返回 document / email thread / calendar event / sheet range / knowledge note / web source / decision / artifact 候选、`domainContext` 与 `accessIssues`。无工作目录时仍返回 Goal / Task / Workflow / Domain evidence 等通用候选，只跳过 workspace 信号；无痕会话返回空 snapshot。候选 `metadata.actions.focusPaths` 表示 GUI 可触发 focused review / verification，`metadata.domainActions` 表示引用 / evidence / 摘要 / ask-user / conflict / task 等领域动作入口，但查询本身仍只读。

`session_ide_context` owner API 管理当前 session 的 IDE / ACP 快照：current file、selection、open tabs、active diagnostic、active symbol。HTTP `PUT` body 为 `{ "context": SessionIdeContext }`，以对齐 generic transport 的命令参数形态。它只作为 Review / Context 的推荐和 evidence 信号，不进入 system prompt；无痕会话拒绝持久化。完整契约见 [Context Retrieval v2](context-retrieval.md)。

### Review Engine

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_review_runs` | `GET /api/sessions/{sessionId}/review-runs` | ✅ |
| `run_code_review` | `POST /api/sessions/{sessionId}/review-runs` | ✅ |
| `get_review_run` | `GET /api/review-runs/{runId}` | ✅ |
| `update_review_finding_status` | `POST /api/review-findings/{findingId}/status` | ✅ |

Review owner API 管理 durable local code review。`run_code_review` 读取当前 session workspace 的 uncommitted diff，按 `profiles[]` 生成 deterministic / optional Deep Review candidate findings，经 verifier 三态落 `review_findings`，并把 P0/P1 open finding 写回 Goal evidence。请求可带 `focusPaths[]`，用于在同一 local diff 内做 focused review；也可带 `ideContext`，用于本次 run 的 finding evidence 与 stats。无痕会话不创建 durable review run。完整契约见 [Review Engine 控制平面](review-engine.md)。

### Smart Verification

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_verification_runs` | `GET /api/sessions/{sessionId}/verification-runs` | ✅ |
| `plan_smart_verification` | `POST /api/sessions/{sessionId}/verification-runs/plan` | ✅ |
| `run_smart_verification` | `POST /api/sessions/{sessionId}/verification-runs/run` | ✅ |
| `get_verification_run` | `GET /api/verification-runs/{runId}` | ✅ |

Smart Verification owner API 管理 durable validation run。`plan_smart_verification` 只持久化推荐命令；`run_smart_verification` 创建 running run 后后台执行低风险 auto-run steps，并把 `validation_passed` / `validation_failed` / `validation_completed` 写回 Goal evidence。请求可带 `focusPaths[]`，用于在同一 local diff 内选择 focused verification steps；无痕会话不创建 durable verification run。完整契约见 [Smart Verification 控制平面](verification-engine.md)。

### Domain Quality

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_domain_quality_runs` | `GET /api/sessions/{sessionId}/domain-quality-runs` | ✅ |
| `run_domain_quality` | `POST /api/domain-quality-runs/run` | ✅ |
| `get_domain_quality_run` | `GET /api/domain-quality-runs/{runId}` | ✅ |

Domain Quality owner API 管理 durable non-coding review / verification run。`run_domain_quality` 基于 Domain Workflow template、domain evidence、approval gates 和输入 metadata 同步生成 `domain_quality_runs/checks/events`，并把 `domain_quality_passed` / `domain_quality_blocked` / `domain_quality_failed` / `domain_quality_needs_user` / `domain_quality_check` 写回 Goal evidence。请求可显式带 `templateId/templateVersion` 或 `domain`；未指定时优先使用 active / 指定 Goal 绑定的 `workflow_template_id/version`，run 与 stats 会保留 template id/version 便于审计。无工作目录也可运行；无痕会话拒绝持久化。高风险动作只有在 `sourceMetadata.requestedAction` 匹配 approval gate 或 `highRiskAction=true` 时要求 `explicitUserApproval`，缺失时 run 进入 `needs_user` 并阻塞 Goal。完整契约见 [Domain Quality 控制平面](domain-quality.md)。

### Domain Eval / Quality Gate

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_domain_eval_tasks` | `POST /api/domain-eval/tasks` | ✅ |
| `run_domain_eval_task` | `POST /api/domain-eval/runs/run` | ✅ |
| `run_domain_eval_fixture` | `POST /api/domain-eval/fixtures/run` | ✅ |
| `import_domain_eval_case` | `POST /api/domain-eval/cases/import` | ✅ |
| `record_domain_eval_calibration` | `POST /api/domain-eval/calibrations/record` | ✅ |
| `list_domain_eval_calibrations` | `POST /api/domain-eval/calibrations` | ✅ |
| `list_domain_eval_runs` | `POST /api/domain-eval/runs` | ✅ |
| `list_domain_eval_fixture_runs` | `POST /api/domain-eval/fixture-runs` | ✅ |
| `create_domain_eval_campaign` | `POST /api/domain-eval/campaigns/create` | ✅ |
| `list_domain_eval_campaigns` | `POST /api/domain-eval/campaigns` | ✅ |
| `get_domain_eval_campaign` | `GET /api/domain-eval/campaigns/{campaign_id}` | ✅ |
| `run_domain_eval_campaign` | `POST /api/domain-eval/campaigns/run` | ✅ |
| `cancel_domain_eval_campaign` | `POST /api/domain-eval/campaigns/{campaign_id}/cancel` | ✅ |
| `get_domain_eval_campaign_leaderboard` | `POST /api/domain-eval/campaigns/leaderboard` | ✅ |
| `evaluate_domain_quality_gate` | `POST /api/domain-quality-gate/evaluate` | ✅ |
| `evaluate_domain_readiness_gate` | `POST /api/domain-readiness-gate/evaluate` | ✅ |

Domain Eval owner API 管理 non-coding eval / gate。`list_domain_eval_tasks` 返回内置 15 个 Research / Writing / Data Analysis / Meeting Prep / Knowledge Curation task 以及显式导入的 active task，并附加 user/project calibration；`import_domain_eval_case` 把已晋升 `domain_eval_case` proposal 的 JSON artifact 导入 `domain_eval_tasks`；`record_domain_eval_calibration` / `list_domain_eval_calibrations` 记录与查询人工校准 / 复核历史；`run_domain_eval_task` 读取 Goal、Workflow、Domain Evidence 与 Domain Quality trace 做 deterministic scoring，并写入 `domain_eval_runs(source_type='live')`；`run_domain_eval_fixture` 支持 `executionMode="trace_fixture"` 和 `executionMode="agent"`：前者创建 `SessionKind::EvalFixture` session / goal / evidence / workflow / quality trace 后调用同一 scorer，后者要求 fixture 显式传 `execution.providers` / `execution.modelChain`，创建真实 user message + chat turn，经 `run_chat_engine` 执行后再进入同一 scorer，且不会自动写入 `fixture.evidence` / `fixture.workflow`，执行失败不写 eval run 但会写 `domain_eval_fixture_runs`；`create_domain_eval_campaign` / `run_domain_eval_campaign` 把多个 task × model/execution item 持久化为 durable campaign，可取消、可 retry failed/interrupted/cancelled item，并通过 item 指向最新 `fixtureRunId` / `evalRunId`；campaign history 只保存 provider/model/label，不保存 provider secret；`get_domain_eval_campaign_leaderboard` 按 provider/model/label/execution 聚合 campaign item，返回 rank、pass rate、average score、warnings 与可追溯 evidence；`generate_coding_improvement_proposals` 可用 `sourceType="domain_eval_campaign"` + campaign id 把 failed/cancelled/interrupted item 生成 `domain_eval_case` 与 `domain_guidance` draft proposal；`list_domain_eval_runs` 默认排除 `fixture_*` synthetic 数据，`list_domain_eval_fixture_runs` 专供 Dashboard Smoke Run Center；`evaluate_domain_quality_gate` 默认只读 live `domain_eval_runs`、`domain_quality_runs/checks` 与 evidence coverage，`includeSynthetic=true` 才纳入 fixture/smoke 数据，输出 `passed` / `failed` / `insufficient_data` 三态；`evaluate_domain_readiness_gate` 进一步只读 Quality Gate、Campaign、Leaderboard 和 Campaign Learning Closure，输出通用领域可交付 readiness 三态、blockers 与 recommended next steps，不自动生成 proposal、不自动 retry campaign。该 API 与 coding benchmark 分表、分路径、分 Dashboard 区块展示；无痕会话 fail-closed。完整契约见 [Domain Eval 与 Quality Gate 控制平面](domain-eval.md)。

### Coding Eval

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `run_coding_task_eval_fixture` | `POST /api/coding-eval/task-fixtures/run` | ✅ |
| `list_coding_eval_gold_tasks` | `GET /api/coding-eval/gold-tasks` | ✅ |
| `run_coding_eval_gold_task_pack` | `POST /api/coding-eval/gold-tasks/run` | ✅ |
| `evaluate_coding_eval_strategy_effect` | `POST /api/coding-eval/strategy-effects/evaluate` | ✅ |

Coding Eval owner API 运行一份完整 fixture JSON，创建临时 git repo 与真实 session / goal / task / workflow seed。`runs.execution.mode="agent"` 会按 fixture 提供的 `providers` / `modelChain` 调用 `run_chat_engine`，创建 user message + chat turn，让 agent 从 task prompt 开始执行；`mode="fixture_patch"` 用于无模型回归，只在执行阶段写入 `repo.changes`。随后 API 调用生产 Review / Smart Verification / Context Retrieval，并按 `fixture.task` 对候选 diff 做 task-level scoring。它返回 `FixtureReport`，可包含 `execution` / `task` report；`execution.toolCalls` / `metrics.executionToolCalls` 记录真实 tool message 名称，fixture 可用 `checks.execution.expectedToolCalls` / `minToolCalls` 断言模型确实调用了预期工具；`runs.task.recordEvalRun` 默认把结果写入 `coding_eval_runs(suite='task_level_coding_eval')` 供 Improvement Loop / Dashboard 消费。Phase 5.6 的 mock Responses 基线不访问外部服务，但会驱动真实 `write` 工具在临时 repo 产出 candidate diff。

Gold Task Pack API 是 Phase 5.3 的批量入口：`list_coding_eval_gold_tasks` 返回内置 active gold task registry；`run_coding_eval_gold_task_pack` / `POST /api/coding-eval/gold-tasks/run` 接收 `{ "input": { "ids": [], "statuses": [], "taskTypes": [], "maxTasks": 2, "executionMode": "fixture_patch", "recordEvalRuns": true, "recordPackRun": true, "baselineKind": "deterministic_mock", "evaluateGoal": true } }`，把自动化 gold tasks materialize 成普通 fixture 后批量运行，返回 `GoldTaskPackReport`。默认只跑已自动化的 active cases，且默认走 `fixture_patch`，不访问外部模型。Phase 5.9 起可传 `executionMode="agent"`、`providers`、`modelChain`、`autoApproveTools=true` 跑受控外部模型基线；这会创建真实 chat turn，让模型通过工具产生 diff，再进入同一 scorer。Phase 5.7 起 `recordPackRun` 默认把 pack summary 写入 `coding_eval_pack_runs` 并返回 `packRunId`；外部真实模型基线必须用 `baselineKind="external_model"` 标明，且必须具备 agent execution 配置，不能只改标签。

Strategy Effect API 是 Phase 5.4 的对比入口：`evaluate_coding_eval_strategy_effect` / `POST /api/coding-eval/strategy-effects/evaluate` 接收 `{ "input": { "strategyType": "workflow_policy", "baseline": GoldTaskPackReport, "candidate": GoldTaskPackReport, "recordRun": false } }`，返回 `StrategyEffectReport`。它只比较两份报告中的共同 case，candidate 漏掉 baseline case 视为回归风险，candidate 新增 case 只展示、不参与聚合；不跑模型、不执行项目命令。纯函数 `evaluate_strategy_effect()` 仍无 DB 副作用；owner API 仅在 `recordRun=true` 时写入 `coding_strategy_effect_runs` 并返回 `runId`。完整契约见 [Coding Eval 控制面评测](coding-eval.md)。

### Coding Improvement Loop

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_coding_trend_report` | `GET /api/sessions/{sessionId}/coding-trend?windowDays=30` | ✅ |
| `list_coding_improvement_proposals` | `GET /api/sessions/{sessionId}/coding-improvement/proposals` | ✅ |
| `generate_coding_improvement_proposals` | `POST /api/sessions/{sessionId}/coding-improvement/proposals` | ✅ |
| `distill_coding_improvement_proposals` | `POST /api/sessions/{sessionId}/coding-improvement/distill` | ✅ |
| `update_coding_improvement_proposal_status` | `POST /api/coding-improvement/proposals/{proposalId}/status` | ✅ |
| `preview_coding_improvement_proposal_action` | `GET /api/coding-improvement/proposals/{proposalId}/action-preview` | ✅ |
| `apply_coding_improvement_proposal` | `POST /api/coding-improvement/proposals/{proposalId}/apply` | ✅ |
| `preview_coding_improvement_proposal_promotion` | `GET /api/coding-improvement/proposals/{proposalId}/promotion-preview` | ✅ |
| `promote_coding_improvement_proposal` | `POST /api/coding-improvement/proposals/{proposalId}/promote` | ✅ |
| `record_coding_eval_run` | `POST /api/coding-improvement/eval-runs` | ✅ |
| `evaluate_coding_eval_release_gate` | `POST /api/coding-improvement/release-gate/evaluate` | ✅ |
| `evaluate_coding_learning_generalization` | `POST /api/coding-improvement/generalization/evaluate` | ✅ |
| `get_coding_benchmark_center` | `POST /api/coding-benchmark/center` | ✅ |
| `create_coding_benchmark_campaign` | `POST /api/coding-benchmark/campaigns/create` | ✅ |
| `list_coding_benchmark_campaigns` | `POST /api/coding-benchmark/campaigns` | ✅ |
| `get_coding_benchmark_campaign` | `GET /api/coding-benchmark/campaigns/{campaignId}` | ✅ |
| `cancel_coding_benchmark_campaign` | `POST /api/coding-benchmark/campaigns/{campaignId}/cancel` | ✅ |
| `run_coding_benchmark_campaign` | `POST /api/coding-benchmark/campaigns/run` | ✅ |
| `get_benchmark_leaderboard` | `POST /api/coding-benchmark/leaderboard` | ✅ |
| `compare_benchmark_models` | `POST /api/coding-benchmark/compare` | ✅ |
| `import_benchmark_task_pack` | `POST /api/coding-benchmark/corpus/import` | ✅ |
| `list_benchmark_task_packs` | `POST /api/coding-benchmark/corpus/packs` | ✅ |
| `get_benchmark_task_pack` | `GET /api/coding-benchmark/corpus/packs/{packId}/{version}` | ✅ |
| `update_benchmark_task_pack_status` | `POST /api/coding-benchmark/corpus/packs/status` | ✅ |
| `validate_benchmark_task_pack` | `POST /api/coding-benchmark/corpus/packs/validate` | ✅ |
| `get_benchmark_corpus_health` | `POST /api/coding-benchmark/corpus/health` | ✅ |
| `generate_benchmark_report` | `POST /api/coding-benchmark/reports/generate` | ✅ |
| `list_benchmark_reports` | `POST /api/coding-benchmark/reports` | ✅ |
| `get_benchmark_report` | `GET /api/coding-benchmark/reports/{reportId}` | ✅ |
| `mark_benchmark_report_release_evidence` | `POST /api/coding-benchmark/reports/release-evidence` | ✅ |
| `evaluate_continuous_benchmark_gate` | `POST /api/coding-benchmark/continuous-gate/evaluate` | ✅ |
| `materialize_benchmark_backlog` | `POST /api/coding-benchmark/backlog/materialize` | ✅ |
| `list_benchmark_backlog` | `POST /api/coding-benchmark/backlog` | ✅ |
| `update_benchmark_backlog_status` | `POST /api/coding-benchmark/backlog/status` | ✅ |

Coding Improvement owner API 基于 durable Goal / Workflow / Review / Smart Verification / Coding Eval / transcript 数据生成 trend report、workflow retro、failure taxonomy、transcript distillation 和 proposal 队列。`generate_coding_improvement_proposals` 从 report 派生候选；`distill_coding_improvement_proposals` 显式扫描 transcript、tool error、workflow ops 与 failure feedback 后只写 `coding_improvement_proposals(status='draft')`；`preview_coding_improvement_proposal_action` 返回确定性 action plan；`apply_coding_improvement_proposal` 先原子 claim draft proposal，再仅应用成 reviewable draft artifact 或 managed draft skill，目标已存在或并发创建都 fail-closed，不直接修改 project guidance、AGENTS、memory 或生产 eval fixture。`preview_coding_improvement_proposal_promotion` / `promote_coding_improvement_proposal` 只对已应用草稿显式晋升，目标冲突 fail-closed。`evaluate_coding_eval_release_gate` 只读 pack / strategy / tool-call history，输出发布质量三态；`evaluate_coding_learning_generalization` 只读 promoted learning、pack history 和 strategy history，输出跨项目学习泛化三态；`get_coding_benchmark_center` 只读 pack history 并嵌入 release / generalization gate，输出 Benchmark Run Center 三态；Benchmark Campaign API 创建/运行/取消/重试 durable campaign，history 不保存 provider configs 或 API key；leaderboard / compare API 只读 campaign item history，并保留 campaign item / packRunId evidence；Benchmark Corpus API 只保存显式 owner-provided manifest，要求 import consent，并验证 active task 的来源、版本、成功标准、验证命令和 redaction 状态；Benchmark Report API 把 campaign / comparison / release benchmark 生成 Markdown / JSON / HTML snapshot，记录 report history，并允许 owner 显式标记 release evidence；Continuous Benchmark Gate API 只读 release evidence、campaign、corpus、leaderboard、backlog、可靠性和预算 history，输出持续发布守门结论；Benchmark Backlog API 把失败 campaign item 物化为可处理 backlog，并通过显式状态更新关闭。无痕会话 fail-closed。完整契约见 [Coding Improvement Loop](coding-improvement-loop.md)。

### Workflow Runs

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_workflow_runs` | `GET /api/sessions/{sessionId}/workflow-runs` | ✅ |
| `list_workflow_watchdog_findings` | `GET /api/sessions/{sessionId}/workflow-runs/watchdog?staleSecs=300` | ✅ |
| `preview_workflow_script` | `POST /api/sessions/{sessionId}/workflow-runs/preview` | ✅ |
| `create_workflow_run` | `POST /api/sessions/{sessionId}/workflow-runs` | ✅ |
| `get_workflow_run` | `GET /api/workflow-runs/{runId}` | ✅ |
| `run_workflow_run` | `POST /api/workflow-runs/{runId}/run` | ✅ |
| `pause_workflow_run` | `POST /api/workflow-runs/{runId}/pause` | ✅ |
| `resume_workflow_run` | `POST /api/workflow-runs/{runId}/resume` | ✅ |
| `approve_workflow_run` | `POST /api/workflow-runs/{runId}/approve` | ✅ |
| `cancel_workflow_run` | `POST /api/workflow-runs/{runId}/cancel` | ✅ |
| `get_workflow_mode` | `GET /api/sessions/{sessionId}/workflow-mode` | ✅ |
| `set_workflow_mode` | `POST /api/sessions/{sessionId}/workflow-mode` | ✅ |

Workflow Mode 是 session 级能力开关：开启后模型才会在后续回合看到 `workflow_run` 工具，并自行判断是否需要动态编排。Workflow owner API 管理 durable `workflow_runs`。`preview_workflow_script` 不落库，只返回 Script Gate + permission preview；`create_workflow_run` 会强制复用同一 preflight，Gate 不通过或 permission preview 有确定 deny 时拒绝创建，并可选接收 `worktreeId` 绑定 managed worktree、`goalCriterionId` 绑定 active Goal 的具体完成标准，默认 kind 为 `general.workflow`。`create_workflow_run(runImmediately=true)` / `run_workflow_run` / `approve_workflow_run` / `resume_workflow_run` 都先要求当前进程是 primary launcher，再把启动请求交给 runtime；API 返回值只表示 launch accepted，不承诺同步进入 `running`，真实进度以后续 `workflow:*` 事件和 snapshot 为准。`cancel_workflow_run` 会先转 `cancelled`，再 best-effort 取消 workflow-owned async tool / validation / subagent children。`list_workflow_watchdog_findings` 只读返回 `workflow_recoverable_owner` / `workflow_no_recent_progress` 诊断，供 GUI 和后续模型 status/trace 面提示“需要确认”，不触发恢复、不执行脚本、不绕过 primary-only。完整技术契约见 [Workflow Mode、Workflow Run 与 Execution Mode](workflow.md)。

### Goals

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_active_goal` | `GET /api/sessions/{sessionId}/goal` | ✅ |
| `list_goal_watchdog_findings` | `GET /api/sessions/{sessionId}/goal/watchdog` | ✅ |
| `create_goal` | `POST /api/sessions/{sessionId}/goal` | ✅ |
| `get_goal` | `GET /api/goals/{goalId}` | ✅ |
| `update_goal` | `PATCH /api/goals/{goalId}` | ✅ |
| `pause_goal` | `POST /api/goals/{goalId}/pause` | ✅ |
| `resume_goal` | `POST /api/goals/{goalId}/resume` | ✅ |
| `clear_goal` | `POST /api/goals/{goalId}/clear` | ✅ |
| `evaluate_goal` | `POST /api/goals/{goalId}/evaluate` | ✅ |
| `close_goal` | `POST /api/goals/{goalId}/close` | ✅ |
| `append_goal_follow_up` | `POST /api/goals/{goalId}/follow-ups` | ✅ |

Goal owner API 管理 session-scoped 顶层目标。`create_goal` 会拒绝 incognito session，并保证同一 session 只有一个 open Goal 或 pending closure Goal；`update_goal` 更新 objective / completion criteria 后清空旧 final audit，并让 `blocked` / `evaluating` / pending `completed` 回到 `active`；`append_goal_follow_up` 把非阻塞后续项写入 durable follow-up pool，规范化去重并拒绝 sealed 终态 Goal；`evaluate_goal` 基于 linked workflow runs、tasks、validation/diff/file evidence 与 budget snapshot 生成 deterministic final audit；`close_goal` 记录用户 closure decision（`accepted_v1` / `needs_strict_evidence` / `cancelled` / `superseded`），其中 `clear_goal` 走 `cancelled` closure 而不是只改 state。`list_goal_watchdog_findings` 只读返回 `goal_no_recent_progress` / `goal_stale_evaluating` 诊断：它复用 runner stop rules，且在 active workflow/task/background job 存在时不误报，不排 wakeup、不恢复、不修改 Goal。`create_workflow_run` 可接收可选 `goalId`，省略时自动绑定当前 open Goal 或 pending closure Goal；`create_workflow_run` / `create_loop_schedule` 可接收 `goalCriterionId`，后端校验 Goal revision 并把 criteria 快照写进 run/schedule/evidence metadata；创建前会执行 Goal budget hard stop，workflow 终态和关键 op 会 best-effort 回写 Goal link 并触发 audit。完整契约见 [Goal 控制平面](goal.md)。

### Domain Workflow

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_domain_workflow_templates` | `POST /api/domain-workflows/templates` | ✅ |
| `save_domain_workflow_template` | `POST /api/domain-workflows/templates/save` | ✅ |
| `preview_domain_workflow` | `POST /api/domain-workflows/preview` | ✅ |
| `record_domain_evidence` | `POST /api/domain-evidence/record` | ✅ |
| `list_domain_evidence` | `POST /api/domain-evidence` | ✅ |

Domain Workflow owner API 是 Phase 7.1-7.2 的通用场景入口。`list_domain_workflow_templates` 合并内置 Research / Writing / Data Analysis / Meeting Prep / Knowledge Curation / Inbox / Project Ops 模板与用户/项目自定义模板；`save_domain_workflow_template` 要求 `explicitSaveConsent=true`，并禁止覆盖 built-in 同 id/version；`preview_domain_workflow` 从模板生成 `workflow.js` draft，走既有 Script Gate / permission preview，但不创建 run、不执行脚本；`record_domain_evidence` 写入通用 evidence，可把 `source_cited`、`claim_checked`、`user_decision`、`artifact_reviewed`、`data_quality_checked` 等 relation 链回 Goal，成功后通过 `domain_evidence:recorded` 事件通知 Workspace Context 与通用任务工作台刷新；`list_domain_evidence` 供 owner 面按 goal/session/project/domain/type 查询。无痕会话 fail-closed。完整契约见 [Domain Workflow 控制平面](domain-workflow.md)。

### Loop Schedules

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_loop_schedules` | `GET /api/sessions/{sessionId}/loops` | ✅ |
| `list_loop_watchdog_findings` | `GET /api/sessions/{sessionId}/loops/watchdog?graceSecs=120` | ✅ |
| `create_loop_schedule` | `POST /api/sessions/{sessionId}/loops` | ✅ |
| `get_loop_schedule` | `GET /api/loops/{loopId}` | ✅ |
| `pause_loop_schedule` | `POST /api/loops/{loopId}/pause` | ✅ |
| `resume_loop_schedule` | `POST /api/loops/{loopId}/resume` | ✅ |
| `stop_loop_schedule` | `POST /api/loops/{loopId}/stop` | ✅ |
| `run_loop_schedule_now` | `POST /api/loops/{loopId}/run-now` | ✅ |
| `update_loop_schedule_policy` | `PATCH /api/loops/{loopId}/policy` | ✅ |

Loop owner API 管理 session-scoped recurring triggers。`create_loop_schedule` 会拒绝 incognito session，并要求绑定 open/pending closure Goal 或提供明确 recurring prompt；可选 `goalCriterionId` 会绑定当前 Goal 的具体完成标准；可选 `executionStrategy` 默认为 `continue`，触发时通过 parent injection 回到原会话；设为 `workflow` 时仅支持 interval loop，且要求绑定 Goal 已选择 Domain Workflow template，Cron tick 会创建并启动 `origin=loop:<loop_id>` 的 durable WorkflowRun，并继承 Loop 的 criteria 绑定。Loop v2 字段 `maxNoProgressRuns` / `maxFailures` / `backoffSecs` 默认 3 / 3 / 300s；run terminal 会写 `progress_state`、`progress_delta_json`、`no_progress_reason`、`scheduling_decision`，并基于 durable Goal evidence delta 做 backoff / blocked。`triggerKind=dynamic` 支持 prompt-only `/loop <prompt>` 和裸 `/loop` maintenance 入口：`triggerSpec={ fallbackSecs, fallbackUsed, maintenancePrompt? }`；模型每轮优先用 internal tools `loop_reschedule` / `loop_stop` 写 `loop_runs.trace_json.dynamicDecision`，兼容 marker `LOOP_RESCHEDULE_AFTER` / `LOOP_STOP` / `LOOP_BLOCKED`；缺决策只 fallback 一次，再缺则 blocked。裸 `/loop` 的默认 prompt 可来自 session working dir / Hope Agent home 的 `loop.md`（最多 25KB）或内置通用维护 prompt，且 maintenance Loop 会在每次 Cron trigger admission 前刷新该来源顺序；刷新后的来源 metadata 写入 `loop_runs.trace_json.maintenancePrompt`。`triggerKind=event` 接受内部 EventBus 白名单事件 `workflow:updated` / `goal:updated` / `task_updated`，`triggerSpec={ eventName, filters, debounceSecs }`；匹配事件会先写 durable tick，再通过 Cron immediate primary-only path 执行，并把 `eventContext` 写入 `loop_runs.trace_json`。`list_loop_schedules` / `get_loop_schedule` 派生返回 Cron `nextRunAt` / `cronStatus`，Event Loop 返回空 `nextRunAt` 与 `cronStatus=event`。`list_loop_watchdog_findings` 是只读高可用诊断：backing Cron 缺失返回 `loop_cron_missing`，最新 Loop run 仍是 `running` 但 Cron 已无 `running_at` 且超过 grace 返回 `loop_run_maybe_interrupted`，active Loop 到点超过 grace 但 Cron 未 running 且没有 active Loop run 返回 `loop_due_not_claimed`；它不修复、不触发 run、不绕过 primary-only。`run_loop_schedule_now` 复用 Cron immediate primary-only path，但只接受 active Loop，不绕过 paused / blocked；`update_loop_schedule_policy` 更新 Loop budget / guard，并同步 Cron `max_failures` / `job_timeout_secs`。Loop run 会写 `loop_runs` trace，绑定 Goal 时写 `loop_run` evidence；`pause/resume/stop` 同步暂停或恢复底层 Cron job（Event Loop 的底层 Cron job 保持 paused，只响应事件 watcher / active run-now）。模型侧 `loop_status` / `loop_reschedule` / `loop_stop` / `loop_record_progress` 只操作当前 session Loop 控制面，不开放 `manage_cron` 写权限。完整契约见 [Loop 控制平面](loop.md)。

`export_session_cmd` / `GET /api/sessions/{sessionId}/export` 是两端**形态不对称**的特例：Tauri 端走 IPC，由前端先弹原生 save dialog 拿到 `output_path` 再传进来，后端写盘后返回最终路径字符串；HTTP 端走 GET 直接返回二进制流（`Content-Type` + `Content-Disposition: attachment; filename*=UTF-8''<percent>`），浏览器用 `URL.createObjectURL` + `<a download>` 触发下载。两端共用 [`ha_core::session::export::export_session`](../../crates/ha-core/src/session/export.rs) 序列化器，Query 参数 `format ∈ {md,json,html}` / `includeThinking` / `includeTools` 与 Tauri 命令的字段一一对应。前端 Transport 抽象 [`exportSession`](../../src/lib/transport.ts) 是这一对端点的统一入口，调用方不需要分支。

`set_session_working_dir` 接受 `{ workingDir: string | null }`，后端 `canonicalize` 路径并校验是否为存在的目录，返回 `{ updated: true, workingDir: <canonical> }`；`null` 或空串清除选择。该字段以 `SessionMeta.workingDir` 呈现，被 `system_prompt::build` 注入到 "# Working Directory" 段落（位于 Project / Project Files 之后、Memory 之前）。执行层也会把它作为 path-aware 工具的默认根：`read` / `write` / `edit` / `ls` / `grep` / `find` / `apply_patch` 的相对路径，以及 `exec.cwd` 的相对路径，均按「显式绝对路径 > Session working dir > Agent home」解析；`exec` 无 `cwd` 时再回退到用户 home。与 Project / Incognito 正交：三者可同时启用。在 HTTP 模式下前端没有原生目录选择器，改走 `GET /api/filesystem/list-dir`（见 Filesystem 域）的服务端目录浏览器。

新会话尚未 materialize 时也允许选目录：前端把选择存为 `draftWorkingDir`，首条消息发送时通过 `chat` 命令的可选 `workingDir` 字段（Tauri / `POST /api/chat` 同名）随请求带过去；后端只在自动创建 session 的分支应用，复用 `update_session_working_dir` 的 canonicalize + `is_dir` 校验，无效路径直接 400。已有 sessionId 的 `chat` 调用会忽略此字段，避免覆盖现成的工作目录设置。

**项目会话懒创建**：进项目「新建对话」不再预先 `create_session_cmd` 落库，而是停在草稿态（`currentSessionId=null`，前端记 `draftProjectId`），与普通对话对称。首条消息发送时通过 `chat` 命令的可选 `projectId` 字段（Tauri / `POST /api/chat` 同名 camelCase）把项目绑定带过去；后端只在自动创建 session 的分支用它 `create_session_with_project(agent, project_id, …)`，并在 `agent_id` 缺省时按 `project.default_agent_id` 解析 agent（对齐 `create_session_cmd`）。已有 sessionId 的调用忽略此字段；`project_id` 与 `incognito` 互斥（后端强制 incognito off）。好处：进项目不再产生未发消息的空会话行，且草稿态走与普通对话相同的模型 / 权限模式 seeding。

`chat` 命令还有两个知识空间侧边栏对话用的可选字段（Tauri / `POST /api/chat` 同名 camelCase）：`toolScope: "knowledge"` 把本轮注入工具集收窄到笔记 / 检索 / 记忆白名单（与 source / `effective_kb_access` 正交，只动 schema 可见性）；`kbAnchorNote` 仅在自动创建 session 的分支生效——配合单条 `kbAttachments`(write) 把新会话提升为 `kind=knowledge` 的对话线程并锚定该笔记。已有 sessionId 的调用忽略 `kbAnchorNote`。

### Chat

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `chat` | `POST /api/chat`；流式输出经 `/ws/events` 的 `chat:stream_delta` | ✅ |
| `queue_turn_user_message` | `POST /api/chat/turn-message` | ✅ 持久入队，附件在入队时转 session-owned 引用 |
| `list_queued_turn_user_messages` | `GET /api/chat/turn-message/{sessionId}` | ✅ UI/恢复单一查询入口 |
| `update_queued_turn_user_message` | `PATCH /api/chat/turn-message` | ✅ CAS 拒绝 inserting/dispatching |
| `delete_queued_turn_user_message` | `DELETE /api/chat/turn-message/{sessionId}/{requestId}` | ✅ CAS 拒绝 inserting/dispatching |
| `insert_queued_turn_user_message` | `POST /api/chat/turn-message/insert` | ✅ 绑定活跃 turn 的工具边界 |
| `cancel_queued_turn_user_message` | `POST /api/chat/turn-message/cancel` | ✅ 仅 waiting_tool_boundary 可撤销 |
| `stop_chat` | `POST /api/chat/stop` | ✅ |
| `set_permission_mode` | `POST /api/chat/permission-mode` | ✅ 替代旧 `set_tool_permission_mode` |
| `respond_to_approval` | `POST /api/chat/approval` | ✅ |
| `save_attachment` | `POST /api/chat/attachment` | ✅ (multipart) |
| `list_builtin_tools` | `GET /api/chat/tools` | ✅ |
| `list_session_tasks` | `GET /api/sessions/{sessionId}/tasks` | ✅ TaskProgressPanel 用户控件 |
| `create_session_task` | `POST /api/sessions/{sessionId}/tasks` | ✅ Workspace Context 候选转任务 |
| `update_task_status` | `PATCH /api/tasks/{id}/status` | ✅ TaskProgressPanel 用户控件 |
| `delete_task` | `DELETE /api/tasks/{id}` | ✅ TaskProgressPanel 用户控件 |

#### Chat `attachments` wire format

`chat` / `POST /api/chat` 与 `queue_turn_user_message` / `POST /api/chat/turn-message` 共用同一份 `attachments` 数组；Tauri IPC 和 HTTP 都按 snake_case 原样序列化。每项的基础字段为 `{ name, mime_type, source?, data?, file_path? }`。图片使用 base64 `data`，普通文件须先经 `save_attachment` / `POST /api/chat/attachment` 落盘并以 `file_path` 引用。

对话消息引用使用独立来源，不可复用文件引用语义：

```json
{
  "name": "message-quote",
  "mime_type": "text/plain",
  "source": "message_quote",
  "data": "用户实际选中的可见纯文本",
  "quote_role": "user"
}
```

`quote_role` 只能是 `user` 或 `assistant`。`message_quote` 不带 `file_path` / `quote_lines`，不会被当成上传文件、URL 来源或知识空间归档来源；后端将其作为已转义的 `<message_quote role="…">…</message_quote>` 用户上下文处理。历史消息会以 `{ kind: "message_quote", role, content }` 元数据恢复为引用卡片。旧客户端可忽略未知 `source`。

### macOS Control

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `mac_control_status` | `GET /api/mac-control/status` | ✅ |
| `mac_control_permissions` | `GET /api/mac-control/permissions` | ✅ |
| `mac_control_snapshot` | `POST /api/mac-control/snapshot` | ✅ |
| `mac_control_capture_frame` | `POST /api/mac-control/capture-frame` | ✅ |

这些是前端 Transport 层的桌面状态 / 权限 / 画面镜像入口；聊天里的 builtin tool 统一叫 `mac_control`，其 `wait/apps/windows/act/menu/dialog` 等动作在 ha-core 工具执行层分发，不按每个 op 增加 Tauri / HTTP command。HTTP/server 模式保持同形状响应，但本机桌面控制返回 `supported=false`。

### Providers

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_providers` | `GET /api/providers` | ✅ |
| `add_provider` | `POST /api/providers` | ✅ |
| `update_provider` | `PUT /api/providers/{providerId}` | ✅ |
| `delete_provider` | `DELETE /api/providers/{providerId}` | ✅ |
| `reorder_providers` | `POST /api/providers/reorder` | ✅ |
| `test_provider` | `POST /api/providers/test` | ✅ |
| `test_embedding` | `POST /api/providers/test-embedding` | ✅ |
| `test_image_generate` | `POST /api/providers/test-image` | ✅ |
| `test_model` | `POST /api/providers/test-model` | ✅ |
| `test_proxy` | `POST /api/config/proxy/test` | ✅ |
| `has_providers` | `GET /api/providers/has-any` | ✅ |
| `get_system_timezone` | `GET /api/system/timezone` | ✅ |
| `list_local_embedding_models` | `GET /api/memory/local-embedding-models` | ✅ |
| `check_auth_status` | `GET /api/auth/codex/status` | ✅ |
| `logout_codex` | `POST /api/auth/codex/logout` | ✅ |
| `try_restore_session` | `POST /api/auth/session/restore` | ✅ |
| `list_canvas_projects` | `GET /api/canvas/projects` | ✅ |
| `get_canvas_project` | `GET /api/canvas/projects/{projectId}` | ✅ |
| `delete_canvas_project` | `DELETE /api/canvas/projects/{projectId}` | ✅ |

### Models

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_available_models` | `GET /api/models` | ✅ |
| `get_active_model` | `GET /api/models/active` | ✅ |
| `set_active_model` | `POST /api/models/active` | ✅ |
| `get_fallback_models` | `GET /api/models/fallback` | ✅ |
| `set_fallback_models` | `POST /api/models/fallback` | ✅ |
| `get_vision_model` | `GET /api/models/vision` | ✅ |
| `set_vision_model` | `PUT /api/models/vision` | ✅ |
| `get_automation_model_chain` | `GET /api/models/automation` | ✅ |
| `set_automation_model_chain` | `PUT /api/models/automation` | ✅ |
| `set_reasoning_effort` | `POST /api/models/reasoning-effort` | ✅ |
| `get_current_settings` | `GET /api/models/settings` | ✅ |
| `get_global_temperature` | `GET /api/models/temperature` | ✅ |
| `set_global_temperature` | `POST /api/models/temperature` | ✅ |

### Agents

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_agents` | `GET /api/agents` | ✅ |
| `list_all_agents` | `GET /api/agents/all` | ✅ owner 设置面，包含 disabled |
| `get_agent_template` | `GET /api/agents/template` | ✅ |
| `initialize_agent` | `POST /api/agents/initialize` | ✅ (见 §7.4 语义差异) |
| `get_agent_config` | `GET /api/agents/{id}` | ✅ |
| `save_agent_config_cmd` | `PUT /api/agents/{id}` | ✅ `create=true` 仅用于显式新建/重用已删 id；普通保存受删除墓碑保护 |
| `preview_agent_delete` | `GET /api/agents/{id}/delete-preview` | ✅ 引用/活动工作/保留数据预检 |
| `set_agent_enabled` | `PATCH /api/agents/{id}/enabled` | ✅ 主 Agent 不可禁用；仍被全局 / Project / Channel / Cron / Wakeup 实时路由引用时拒绝禁用 |
| `delete_agent` | `DELETE /api/agents/{id}?replacementAgentId=...` | ✅ 活动工作 fail closed、含待触发 Wakeup 的引用重绑与精确回滚、备份 + 可恢复回收站；无持久化 Wakeup 作为活动工作阻断 |
| `get_agent_markdown` | `GET /api/agents/{id}/markdown` | ✅ |
| `save_agent_markdown` | `PUT /api/agents/{id}/markdown` | ✅ |
| `render_persona_to_soul_md` | `POST /api/agents/{id}/persona/render-soul-md` | ✅ |
| `get_agent_memory_md` | `GET /api/agents/{id}/memory-md` | ✅ |

Agent 执行准入采用两层 guard：Desktop / HTTP / Channel / Cron 等调用方必须在创建会话、写 turn / 注入消息等持久化副作用前取得外层 guard，`run_chat_engine` 入口再取得内层 backstop。删除与两层准入共用同一生命周期锁；禁止退化为“先 `ensure_agent_runnable`、落库后再进引擎”，否则检查与删除之间会留下 TOCTOU 窗口。删除重绑 Subagent allowlist 时，若 replacement 已在 denylist 必须同步移除（deny 优先于 allow）。
| `save_agent_memory_md` | `PUT /api/agents/{id}/memory-md` | ✅ |
| `dreaming_run_now` | `POST /api/dreaming/run` | ✅ |
| `dreaming_run_resolver` | `POST /api/dreaming/resolver` | ✅ owner 平面；Deep resolver（phase=deep）：valid_until 过期确定性 expire + 同主谓多对象组 LLM 判定 duplicates→merge / conflict→needs_review / independent→no_op，绝不自动 supersede 或硬删 |
| `dreaming_run_profile` | `POST /api/dreaming/profile/run` | ✅ owner 平面；Memory Profile 合成（phase=profile）：从 active claims 按 scope 规则式聚合（manual 触发额外 LLM 重写），写 `memory_profile_snapshots`（version=MAX+1）；受 `dreaming.profileSynthesis.enabled`（默认开）门控 |
| `dreaming_list_profile_snapshots` | `GET /api/dreaming/profile` | ✅ owner 平面；每 scope 最新 profile 快照（只读视图，global/agent/project） |
| `dreaming_list_diaries` | `GET /api/dreaming/diaries` | ✅ |
| `dreaming_read_diary` | `GET /api/dreaming/diaries/{filename}` | ✅ |
| `dreaming_is_running` | `GET /api/dreaming/status` | ✅ |
| `dreaming_last_report` | `GET /api/dreaming/last-report` | ✅ |
| `dreaming_idle_status` | `GET /api/dreaming/idle-status` | ✅ |
| `dreaming_list_runs` | `GET /api/dreaming/runs` | ✅ |
| `dreaming_get_run` | `GET /api/dreaming/runs/{id}` | ✅ |
| `dreaming_evidence_quote` | `GET /api/dreaming/evidence/quote` | ✅ owner 平面；incognito 来源归零（后端门控） |
| `claim_list` | `GET /api/claims` | ✅ 结构化 claim 只读（`scopeType`+`scopeId`/status/claimType 过滤；无效 scopeType → 400，不 fail-open；status 按 **effective** 计算并返回——`active` 且已过 `valid_until` 视为 `expired`，`status=active`/`expired` 过滤同步对齐） |
| `claim_get` | `GET /api/claims/{id}` | ✅ claim + evidence + links（`status` 同为 effective 值） |
| `claim_update` | `PATCH /api/claims/{id}` | ✅ owner 平面；用户纠错（Lucid Review §5.2）：edit content/triple/tags、改 status（approve→active / reject→archived / mark-outdated→expired / flag→needs_review）、move scope、pin/unpin（salience 越过 0.7 阈值）。写 `manual_correction` evidence（approve 用 `user_confirmed`）+ `user_correction` decision log + 发 `memory:claim_changed`；content 变更触发 re-embed。`id` 走 path（覆盖 body 的 `claimId`），其余字段为 body |
| `claim_forget` | `POST /api/claims/{id}/forget` | ✅ owner 平面；`{permanent?,note?}`。`permanent=false`（默认）archive（保留 evidence 作审计，linked legacy memory 停止注入）；`true` 硬删 claim 图谱（claim+evidence+link+vector）+ 仅本 claim 独管的 legacy memory。写 decision log + 发 `memory:claim_changed` |
| `memory_backfill_plan` | `GET /api/memory/backfill/plan` | ✅ owner 平面；dry-run 把 legacy memory 确定性映射为 claim 预览（精确计数 + 截断预览，不写） |
| `memory_backfill_apply` | `POST /api/memory/backfill/apply` | ✅ owner 平面；确定性重扫，事务内 check（memory 存在 + 未 link，竞态/重入幂等→skipped）后写入 claim + `source_type=memory` evidence + **detached** link（不改变现有注入），仅 pinned 的 user/feedback 自动 active、其余 needs_review；返回 created/skipped/failed |
| `scan_openclaw_agents` | `GET /api/agents/openclaw/scan` | ✅ legacy（agents-only） |
| `import_openclaw_agents` | `POST /api/agents/openclaw/import` | ✅ legacy（agents-only） |
| `scan_openclaw_full` | `GET /api/agents/openclaw/scan-full` | ✅ providers + agents + memories |
| `import_openclaw_full` | `POST /api/agents/openclaw/import-full` | ✅ providers + agents + memories |

### Memory

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `memory_search` | `POST /api/memory/search` | ✅ |
| `memory_list` | `GET /api/memory` | ✅ |
| `memory_count` | `GET /api/memory/count` | ✅ |
| `memory_stats` | `GET /api/memory/stats` | ✅ |
| `memory_add` | `POST /api/memory` | ✅ |
| `memory_get` | `GET /api/memory/{id}` | ✅ |
| `memory_update` | `PUT /api/memory/{id}` | ✅ |
| `memory_delete` | `DELETE /api/memory/{id}` | ✅ |
| `memory_toggle_pin` | `POST /api/memory/{id}/pin` | ✅ |
| `memory_delete_batch` | `POST /api/memory/delete-batch` | ✅ |
| `memory_reembed` | `POST /api/memory/reembed` | ✅ (CLI / 同步) |
| `memory_reembed_start` | `POST /api/memory/reembed-start` | ✅ |
| `memory_export` | `POST /api/memory/export` | ✅ |
| `memory_import` | `POST /api/memory/import` | ✅ |
| `memory_find_similar` | `POST /api/memory/find-similar` | ✅ |
| `memory_get_import_from_ai_prompt` | `GET /api/memory/import-from-ai-prompt` | ✅ |
| `get_global_memory_md` | `GET /api/memory/global-md` | ✅ |
| `save_global_memory_md` | `PUT /api/memory/global-md` | ✅ |

### Memory config

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_embedding_config` | `GET /api/config/embedding` | ✅ |
| `save_embedding_config` | `PUT /api/config/embedding` | ✅ |
| `get_embedding_presets` | `GET /api/config/embedding/presets` | ✅ |
| `embedding_model_config_list` | `GET /api/config/embedding-models` | ✅ |
| `embedding_model_config_templates` | `GET /api/config/embedding-models/templates` | ✅ |
| `embedding_model_config_save` | `PUT /api/config/embedding-models` | ✅ |
| `embedding_model_config_delete` | `POST /api/config/embedding-models/delete` | ✅ |
| `embedding_model_config_test` | `POST /api/config/embedding-models/test` | ✅ |
| `memory_embedding_get` | `GET /api/config/memory-embedding` | ✅ |
| `memory_embedding_set_default` | `POST /api/config/memory-embedding/default` | ✅ |
| `memory_embedding_disable` | `POST /api/config/memory-embedding/disable` | ✅ |
| `get_embedding_cache_config` | `GET /api/config/embedding-cache` | ✅ |
| `save_embedding_cache_config` | `PUT /api/config/embedding-cache` | ✅ |
| `get_dedup_config` | `GET /api/config/dedup` | ✅ |
| `save_dedup_config` | `PUT /api/config/dedup` | ✅ |
| `get_hybrid_search_config` | `GET /api/config/hybrid-search` | ✅ |
| `save_hybrid_search_config` | `PUT /api/config/hybrid-search` | ✅ |
| `get_mmr_config` | `GET /api/config/mmr` | ✅ |
| `save_mmr_config` | `PUT /api/config/mmr` | ✅ |
| `get_multimodal_config` | `GET /api/config/multimodal` | ✅ |
| `save_multimodal_config` | `PUT /api/config/multimodal` | ✅ |
| `get_temporal_decay_config` | `GET /api/config/temporal-decay` | ✅ |
| `save_temporal_decay_config` | `PUT /api/config/temporal-decay` | ✅ |
| `get_extract_config` | `GET /api/config/extract` | ✅ |
| `save_extract_config` | `PUT /api/config/extract` | ✅ |

### User config

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_user_config` | `GET /api/config/user` | ✅ |
| `save_user_config` | `PUT /api/config/user` | ✅ |
| `get_default_agent_id` | `GET /api/config/default-agent` | ✅ |
| `set_default_agent_id` | `PUT /api/config/default-agent` | ✅ |

`get_default_agent_id` 返回 `Option<String>`（HTTP body 为标量 `"my-agent"` 或 `null`）；`set_default_agent_id` 接受 `{ agentId: string | null }`，空串 / null 清除全局默认（resolver 链路回退到硬编码 `"ha-main"`，见 `agent_loader::DEFAULT_AGENT_ID`）。新建会话时按「显式参数 → project.default_agent_id → channel_account.agent_id → AppConfig.default_agent_id → "ha-main"」链路解析（统一 helper：`crate::agent::resolver::resolve_default_agent_id`）。

### Context compaction

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_compact_config` | `GET /api/config/compact` | ✅ |
| `save_compact_config` | `PUT /api/config/compact` | ✅ |
| `get_hooks_config` | `GET /api/config/hooks` | ✅ |
| `save_hooks_config` | `PUT /api/config/hooks` | ✅ |
| `get_session_title_config` | `GET /api/config/session-title` | ✅ |
| `save_session_title_config` | `PUT /api/config/session-title` | ✅ |

### Behavior awareness

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_awareness_config` | `GET /api/config/awareness` | ✅ |
| `save_awareness_config` | `PUT /api/config/awareness` | ✅ |
| `get_session_awareness_override` | `GET /api/sessions/{sessionId}/awareness-config` | ✅ |
| `set_session_awareness_override` | `PATCH /api/sessions/{sessionId}/awareness-config` | ✅ |

### Plan mode

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_plan_mode` | `GET /api/plan/{sessionId}/mode` | ✅ |
| `set_plan_mode` | `POST /api/plan/{sessionId}/mode` | ✅ |
| `get_plan_content` | `GET /api/plan/{sessionId}/content` | ✅ |
| `save_plan_content` | `PUT /api/plan/{sessionId}/content` | ✅ |
| `get_plan_file_path` | `GET /api/plan/{sessionId}/file-path` | ✅ |
| `get_plan_checkpoint` | `GET /api/plan/{sessionId}/checkpoint` | ✅ |
| `get_plan_versions` | `GET /api/plan/{sessionId}/versions` | ✅ |
| `load_plan_version_content` | `POST /api/plan/version/load` | ✅ |
| `restore_plan_version` | `POST /api/plan/{sessionId}/version/restore` | ✅ |
| `plan_rollback` | `POST /api/plan/{sessionId}/rollback` | ✅ |
| `cancel_plan_subagent` | `POST /api/plan/{sessionId}/cancel` | ✅ |
| `list_plans` | `POST /api/plan/list` | ✅ |
| `resolve_plan_mention` | `POST /api/plan/resolve-mention` | ✅ |
| `create_owner_ask_user_question` | `POST /api/ask_user/owner-question` | ✅ |
| `respond_ask_user_question` | `POST /api/ask_user/respond` | ✅ |
| `get_pending_ask_user_group` | `GET /api/plan/{sessionId}/pending-ask-user` | ✅ |
| `set_plan_subagent` | `POST /api/config/plan-subagent` | ✅ |
| `get_plan_subagent` | `GET /api/config/plan-subagent` | ✅ |
| `set_ask_user_question_timeout_enabled` | `POST /api/config/ask-user-question-timeout-enabled` | ✅ |
| `get_ask_user_question_timeout_enabled` | `GET /api/config/ask-user-question-timeout-enabled` | ✅ |
| `set_ask_user_question_timeout` | `POST /api/config/ask-user-question-timeout` | ✅ |
| `get_ask_user_question_timeout` | `GET /api/config/ask-user-question-timeout` | ✅ |

`create_owner_ask_user_question` 创建 owner-plane durable elicitation：它复用 ask_user UI，但不等待模型工具 oneshot；请求自带 `ownerResponse`，用户通过 `respond_ask_user_question` 回答后由后端记录对应 durable evidence（当前用于 Context Retrieval 的 `user_decision`）。普通工具型 ask_user 仍要求 live in-memory receiver，owner-side question 则可跨会话切换和重启保留；incognito session 禁用。

### Cron

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `cron_list_jobs` | `GET /api/cron/jobs` | ✅ |
| `cron_get_job` | `GET /api/cron/jobs/{id}` | ✅ |
| `cron_create_job` | `POST /api/cron/jobs` | ✅ |
| `cron_update_job` | `PUT /api/cron/jobs/{id}` | ✅ |
| `cron_toggle_job` | `POST /api/cron/jobs/{id}/toggle` | ✅ |
| `cron_delete_job` | `DELETE /api/cron/jobs/{id}` | ✅ |
| `cron_run_now` | `POST /api/cron/jobs/{id}/run` | ✅ |
| `cron_jobs_referencing_account` | `GET /api/cron/jobs-referencing-account/{accountId}` | ✅ |
| `cron_get_run_logs` | `GET /api/cron/jobs/{jobId}/logs` | ✅ |
| `cron_get_calendar_events` | `GET /api/cron/calendar` | ✅ |
| `cron_run_timeline` | `GET /api/cron/timeline?limit=&offset=` | ✅ (跨 job 运行时间线，cron 面板「对话」视图) |
| `cron_unread_total` | `GET /api/cron/unread` | ✅ (cron 未读聚合数，侧边栏角标) |
| `cron_mark_all_read` | `POST /api/cron/read-all` | ✅ (一键清除 cron 未读，emit `cron:unread_changed`) |

### Dashboard

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `dashboard_overview` | `POST /api/dashboard/overview` | ✅ |
| `dashboard_overview_delta` | `POST /api/dashboard/overview-delta` | ✅ |
| `dashboard_insights` | `POST /api/dashboard/insights` | ✅ |
| `dashboard_token_usage` | `POST /api/dashboard/token-usage` | ✅ |
| `dashboard_tool_usage` | `POST /api/dashboard/tool-usage` | ✅ |
| `dashboard_sessions` | `POST /api/dashboard/sessions` | ✅ |
| `dashboard_errors` | `POST /api/dashboard/errors` | ✅ |
| `dashboard_tasks` | `POST /api/dashboard/tasks` | ✅ |
| `dashboard_system_metrics` | `GET /api/dashboard/system-metrics` | ✅ |
| `dashboard_session_list` | `POST /api/dashboard/session-list` | ✅ |
| `dashboard_message_list` | `POST /api/dashboard/message-list` | ✅ |
| `dashboard_tool_call_list` | `POST /api/dashboard/tool-call-list` | ✅ |
| `dashboard_error_list` | `POST /api/dashboard/error-list` | ✅ |
| `dashboard_agent_list` | `POST /api/dashboard/agent-list` | ✅ |
| `dashboard_local_model_usage` | `POST /api/dashboard/local-model-usage` | ✅ |

#### Dashboard Learning

`session.db.learning_events` 表 + `dashboard::learning` 查询，支持 7/14/30/60/90 天窗口。埋点来自 `skills::author` CRUD 与 `tool_recall_memory` 命中等。前端 Dashboard "Learning" Tab 消费。

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `dashboard_learning_overview` | `POST /api/dashboard/learning/overview` | ✅ |
| `dashboard_learning_timeline` | `POST /api/dashboard/learning/timeline` | ✅ |
| `dashboard_top_skills` | `POST /api/dashboard/learning/top-skills` | ✅ |
| `dashboard_recall_stats` | `POST /api/dashboard/learning/recall-stats` | ✅ |
| `dashboard_coding_improvement` | `POST /api/dashboard/learning/coding-improvement` | ✅ |
| `evaluate_coding_eval_release_gate` | `POST /api/coding-improvement/release-gate/evaluate` | ✅ |
| `evaluate_coding_learning_generalization` | `POST /api/coding-improvement/generalization/evaluate` | ✅ |
| `evaluate_domain_quality_gate` | `POST /api/domain-quality-gate/evaluate` | ✅ |
| `get_coding_benchmark_center` | `POST /api/coding-benchmark/center` | ✅ |
| `create_coding_benchmark_campaign` | `POST /api/coding-benchmark/campaigns/create` | ✅ |
| `list_coding_benchmark_campaigns` | `POST /api/coding-benchmark/campaigns` | ✅ |
| `get_coding_benchmark_campaign` | `GET /api/coding-benchmark/campaigns/{campaignId}` | ✅ |
| `cancel_coding_benchmark_campaign` | `POST /api/coding-benchmark/campaigns/{campaignId}/cancel` | ✅ |
| `run_coding_benchmark_campaign` | `POST /api/coding-benchmark/campaigns/run` | ✅ |
| `get_benchmark_leaderboard` | `POST /api/coding-benchmark/leaderboard` | ✅ |
| `compare_benchmark_models` | `POST /api/coding-benchmark/compare` | ✅ |
| `import_benchmark_task_pack` | `POST /api/coding-benchmark/corpus/import` | ✅ |
| `list_benchmark_task_packs` | `POST /api/coding-benchmark/corpus/packs` | ✅ |
| `get_benchmark_task_pack` | `GET /api/coding-benchmark/corpus/packs/{packId}/{version}` | ✅ |
| `update_benchmark_task_pack_status` | `POST /api/coding-benchmark/corpus/packs/status` | ✅ |
| `validate_benchmark_task_pack` | `POST /api/coding-benchmark/corpus/packs/validate` | ✅ |
| `get_benchmark_corpus_health` | `POST /api/coding-benchmark/corpus/health` | ✅ |
| `generate_benchmark_report` | `POST /api/coding-benchmark/reports/generate` | ✅ |
| `list_benchmark_reports` | `POST /api/coding-benchmark/reports` | ✅ |
| `get_benchmark_report` | `GET /api/coding-benchmark/reports/{reportId}` | ✅ |
| `mark_benchmark_report_release_evidence` | `POST /api/coding-benchmark/reports/release-evidence` | ✅ |
| `evaluate_continuous_benchmark_gate` | `POST /api/coding-benchmark/continuous-gate/evaluate` | ✅ |
| `materialize_benchmark_backlog` | `POST /api/coding-benchmark/backlog/materialize` | ✅ |
| `list_benchmark_backlog` | `POST /api/coding-benchmark/backlog` | ✅ |
| `update_benchmark_backlog_status` | `POST /api/coding-benchmark/backlog/status` | ✅ |
| `dashboard_plan_stats` | `POST /api/dashboard/plan-stats` | ✅ |

`dashboard_coding_improvement` 是只读全局学习聚合，按 DashboardFilter 返回 workflow / case eval / pack eval / strategy effect / tool-call failure / review / verification / proposal / retro 的 overview、timeline、project buckets、failure modes、tool call failures、proposal status、latest strategy effects 和 latest retros；`get_coding_benchmark_center` 进一步聚合 benchmark history、baseline buckets、recent runs、Release Gate 与 Generalization Gate；Benchmark Campaign API 为 Dashboard 提供 durable campaign 列表、Run/Cancel/Retry 控制和 item-level evidence；leaderboard / compare API 提供同 task pack / source / execution / baseline 下的 provider/model ranking；Benchmark Corpus API 为 Dashboard 提供 task pack import/list/validate/activate/archive 和 corpus health；Benchmark Report API 为 Dashboard 提供 report history、生成 Markdown / JSON / HTML snapshot、复制路径和 release evidence 标记；Continuous Gate / Backlog API 为 Dashboard 提供持续守门状态、阻塞原因、推荐下一步、失败 item 物化和 resolved 操作。它们都不生成 proposal、不 apply、不 promotion。

`evaluate_coding_eval_release_gate` 接收 `{ "input": { "sessionId": "...", "projectId": "...", "windowDays": 30, "minPackRuns": 1, "minStrategyEffectRuns": 0, "minPackPassRate": 1.0, "requireExternalModelPack": false } }`，返回 `CodingEvalReleaseGateReport`。报告包含 `status = passed | failed | insufficient_data`、归一化 `thresholds`、pack / strategy / tool-call `summary` 和逐条 `checks`。它只读 `coding_eval_pack_runs`、`coding_strategy_effect_runs`、`coding_eval_runs`，不跑模型、不执行项目命令、不写 DB。

### Async / Deferred tools + Memory selection

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_async_tools_config` | `GET /api/config/async-tools` | ✅ |
| `save_async_tools_config` | `PUT /api/config/async-tools` | ✅ |
| `get_cron_config` | `GET /api/config/cron` | ✅ |
| `save_cron_config` | `PUT /api/config/cron` | ✅ |
| `get_deferred_tools_config` | `GET /api/config/deferred-tools` | ✅ |
| `save_deferred_tools_config` | `PUT /api/config/deferred-tools` | ✅ |
| `get_memory_selection_config` | `GET /api/config/memory-selection` | ✅ |
| `save_memory_selection_config` | `PUT /api/config/memory-selection` | ✅ |
| `get_memory_budget_config` | `GET /api/config/memory-budget` | ✅ |
| `save_memory_budget_config` | `PUT /api/config/memory-budget` | ✅ |

### Recap

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_recap_config` | `GET /api/config/recap` | ✅ |
| `save_recap_config` | `PUT /api/config/recap` | ✅ |
| `get_recall_summary_config` | `GET /api/config/recall-summary` | ✅ (读取召回摘要配置，含 `enabled` 主开关，GUI 面板；也可经 `get_settings(recall_summary)` 读) |
| `save_recall_summary_config` | `PUT /api/config/recall-summary` | ✅ (写召回摘要配置) |
| `get_dreaming_config` | `GET /api/config/dreaming` | ✅ |
| `save_dreaming_config` | `PUT /api/config/dreaming` | ✅ |
| `validate_cron_expression` | `POST /api/cron/validate` | ✅ |
| `recap_generate` | `POST /api/recap/generate` | ✅ |
| `recap_list_reports` | `POST /api/recap/reports` | ✅ |
| `recap_get_report` | `GET /api/recap/reports/{id}` | ✅ |
| `recap_delete_report` | `DELETE /api/recap/reports/{id}` | ✅ |
| `recap_export_html` | `POST /api/recap/reports/{id}/export` | ✅ |

### Logging

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `query_logs_cmd` | `POST /api/logs/query` | ✅ |
| `frontend_log` | `POST /api/logs/frontend` | ✅ |
| `frontend_log_batch` | `POST /api/logs/frontend-batch` | ✅ |
| `get_log_stats_cmd` | `GET /api/logs/stats` | ✅ |
| `get_log_config_cmd` | `GET /api/logs/config` | ✅ |
| `save_log_config_cmd` | `PUT /api/logs/config` | ✅ |
| `list_log_files_cmd` | `GET /api/logs/files` | ✅ |
| `read_log_file_cmd` | `GET /api/logs/file` | ✅ |
| `get_log_file_path_cmd` | `GET /api/logs/file-path` | ✅ |
| `export_logs_cmd` | `POST /api/logs/export` | ✅ |
| `clear_logs_cmd` | `POST /api/logs/clear` | ✅ |

### Notifications / Server / Proxy / Shortcuts / Sandbox

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_notification_config` | `GET /api/config/notification` | ✅ |
| `save_notification_config` | `PUT /api/config/notification` | ✅ |
| `get_auto_update_config` | `GET /api/config/auto-update` | ✅ |
| `set_auto_update_config` | `PUT /api/config/auto-update` | ✅ |
| `get_startup_notification_config` | `GET /api/config/startup-notification` | ✅ |
| `save_startup_notification_config` | `PUT /api/config/startup-notification` | ✅ |
| `get_server_config` | `GET /api/config/server` | ✅ |
| `save_server_config` | `PUT /api/config/server` | ✅ |
| `get_server_runtime_status` | `GET /api/server/status` | ✅ (免鉴权) — 返回 `{ boundAddr, startedAt, uptimeSecs, startupError, eventsWsCount, chatWsCount, localDesktopClient, activeChatStreams, activeChatCounts: { desktop, http, channel, total } }`。`activeChatStreams` 是 `activeChatCounts.total` 的 back-compat 别名（在跑的 `run_chat_engine` 数量）。`chatWsCount` 当前仍是独立的 `Arc<AtomicU32>` 计数器（`crates/ha-core/src/server_status.rs::chat_ws_counter`），per-session chat WS 端点已下线但 counter 字段未拆——历史遗留，目前没有 handler 在递增，实测恒为 0。`localDesktopClient` 在 Tauri 命令恒 `true`（桌面 webview 通过 IPC 与后端通信，不走 WS），HTTP 路由恒 `false`，前端把它计入"活跃连接" |
| `get_proxy_config` | `GET /api/config/proxy` | ✅ |
| `save_proxy_config` | `PUT /api/config/proxy` | ✅ |
| `get_shortcut_config` | `GET /api/config/shortcuts` | ✅ |
| `save_shortcut_config` | `PUT /api/config/shortcuts` | ✅ |
| `set_shortcuts_paused` | `POST /api/config/shortcuts/pause` | ✅ |
| `get_sandbox_config` | `GET /api/config/sandbox` | ✅ |
| `set_sandbox_config` | `PUT /api/config/sandbox` | ✅ |
| `check_sandbox_available` | `GET /api/config/sandbox/status` | ✅ |

### Canvas

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_canvas_config` | `GET /api/config/canvas` | ✅ |
| `save_canvas_config` | `PUT /api/config/canvas` | ✅ |
| `canvas_submit_snapshot` | `POST /api/canvas/snapshot/{requestId}` | ✅ |
| `canvas_submit_eval_result` | `POST /api/canvas/eval/{requestId}` | ✅ |
| `show_canvas_panel` | `POST /api/canvas/show` | ✅ |
| `list_canvas_projects_by_session` | `GET /api/canvas/by-session/{sessionId}` | ✅ |

### Image generation / Web search / Web fetch / SSRF / SearXNG

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_image_generate_config` | `GET /api/config/image-generate` | ✅ |
| `save_image_generate_config` | `PUT /api/config/image-generate` | ✅ |
| `get_web_search_config` | `GET /api/config/web-search` | ✅ |
| `save_web_search_config` | `PUT /api/config/web-search` | ✅ |
| `get_web_fetch_config` | `GET /api/config/web-fetch` | ✅ |
| `save_web_fetch_config` | `PUT /api/config/web-fetch` | ✅ |
| `get_ssrf_config` | `GET /api/config/ssrf` | ✅ |
| `save_ssrf_config` | `PUT /api/config/ssrf` | ✅ |
| `searxng_docker_status` | `GET /api/searxng/status` | ✅ |
| `searxng_docker_deploy` | `POST /api/searxng/deploy` | ✅ |
| `searxng_docker_start` | `POST /api/searxng/start` | ✅ |
| `searxng_docker_stop` | `POST /api/searxng/stop` | ✅ |
| `searxng_docker_remove` | `DELETE /api/searxng` | ✅ |

### Local LLM assistant

轻量级探测、已安装模型管理、Ollama Library 搜索与模型加载控制接口。长耗时安装和模型拉取统一走「Local model background jobs」（见下表），通过 `local_model_job:*` 事件订阅进度。Windows 不支持脚本安装 Ollama，需引导用户去 ollama.com 手动安装。

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `local_llm_detect_hardware` | `GET /api/local-llm/hardware` | ✅ |
| `local_llm_recommend_model` | `GET /api/local-llm/recommendation` | ✅ |
| `local_llm_chat_catalog` | `GET /api/local-llm/chat-catalog` | ✅ |
| `local_llm_detect_ollama` | `GET /api/local-llm/ollama-status` | ✅ |
| `local_llm_detect_ollama_version` | `GET /api/local-llm/ollama-version` | ✅ |
| `local_llm_known_backends` | `GET /api/local-llm/known-backends` | ✅ |
| `local_llm_start_ollama` | `POST /api/local-llm/start` | ✅ |
| `local_llm_list_models` | `GET /api/local-llm/models` | ✅ |
| `local_llm_search_library` | `GET /api/local-llm/library/search` | ✅ |
| `local_llm_get_library_model` | `POST /api/local-llm/library/model` | ✅ |
| `local_llm_preload_model` | `POST /api/local-llm/preload` | ✅ |
| `local_llm_stop_model` | `POST /api/local-llm/stop-model` | ✅ |
| `local_llm_delete_model` | `POST /api/local-llm/delete-model` | ✅ |
| `local_llm_add_provider_model` | `POST /api/local-llm/provider-model` | ✅ |
| `local_llm_set_default_model` | `POST /api/local-llm/default-model` | ✅ |
| `local_llm_add_embedding_config` | `POST /api/local-llm/embedding-config` | ✅ |
| `local_embedding_list_models` | `GET /api/local-embedding/models` | ✅ |

### Local model background jobs

本地模型安装 / 拉取的统一后台任务接口，进度走 `local_model_job:created` / `:updated` / `:log` / `:completed` 事件。前端用 `transport.listen` 订阅；ha-core `~/.hope-agent/local_model_jobs.db` 持久化。

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `local_model_job_start_chat_model` | `POST /api/local-model-jobs/chat-model` | ✅ |
| `local_model_job_start_embedding` | `POST /api/local-model-jobs/embedding` | ✅ |
| `local_model_job_start_ollama_install` | `POST /api/local-model-jobs/ollama-install` | ✅ |
| `local_model_job_start_ollama_pull` | `POST /api/local-model-jobs/ollama-pull` | ✅ |
| `local_model_job_list` | `GET /api/local-model-jobs` | ✅ |
| `local_model_job_get` | `GET /api/local-model-jobs/{id}` | ✅ |
| `local_model_job_logs` | `GET /api/local-model-jobs/{id}/logs` | ✅ |
| `local_model_job_cancel` | `POST /api/local-model-jobs/{id}/cancel` | ✅ |
| `local_model_job_pause` | `POST /api/local-model-jobs/{id}/pause` | ✅ |
| `local_model_job_retry` | `POST /api/local-model-jobs/{id}/retry` | ✅ |
| `local_model_job_clear` | `DELETE /api/local-model-jobs/{id}` | ✅ |

### Local model auto-maintenance

后台 watchdog（[`crates/ha-core/src/local_llm/auto_maintainer.rs`](../../crates/ha-core/src/local_llm/auto_maintainer.rs)）监测默认 chat / embedding 模型。模型停止时自动 preload；模型文件丢失时 emit `local_model:missing_alert` 事件，前端顶层 `MissingModelDialog` 弹窗。

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_local_llm_auto_maintenance_enabled` | `GET /api/local-model/auto-maintenance` | ✅ |
| `set_local_llm_auto_maintenance_enabled` | `PUT /api/local-model/auto-maintenance` | ✅ |
| `local_model_alert_dismiss_temporary` | `POST /api/local-model/alert/dismiss-temporary` | ✅ |
| `local_model_alert_silence_session` | `POST /api/local-model/alert/silence-session` | ✅ |
| `local_model_auto_maintenance_disable` | `POST /api/local-model/auto-maintenance/disable` | ✅ |
| `local_model_auto_maintenance_trigger` | `POST /api/local-model/auto-maintenance/trigger` | ✅ |

### Skills

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_skills` | `GET /api/skills` | ✅ |
| `list_mentionable_skills` | `GET /api/skills/mentionable` | ✅ |
| `get_skill_detail` | `GET /api/skills/{name}` | ✅ |
| `toggle_skill` | `POST /api/skills/{name}/toggle` | ✅ |
| `get_extra_skills_dirs` | `GET /api/skills/extra-dirs` | ✅ |
| `add_extra_skills_dir` | `POST /api/skills/extra-dirs` | ✅ |
| `remove_extra_skills_dir` | `DELETE /api/skills/extra-dirs` | ✅ |
| `discover_preset_skill_sources` | `GET /api/skills/preset-sources` | ✅ |
| `get_skill_env` | `GET /api/skills/{name}/env` | ✅ |
| `set_skill_env_var` | `POST /api/skills/{skill}/env` | ✅ |
| `remove_skill_env_var` | `DELETE /api/skills/{skill}/env` | ✅ |
| `get_skills_env_status` | `GET /api/skills/env-status` | ✅ |
| `get_skills_status` | `GET /api/skills/status` | ✅ |
| `get_skill_env_check` | `GET /api/skills/env-check` | ✅ |
| `set_skill_env_check` | `PUT /api/skills/env-check` | ✅ |
| `install_skill_dependency` | `POST /api/skills/{skillName}/install` | ✅ |
| `list_draft_skills` | `GET /api/skills/drafts` | ✅ |
| `activate_draft_skill` | `POST /api/skills/{name}/activate` | ✅ |
| `discard_draft_skill` | `DELETE /api/skills/{name}/draft` | ✅ |
| `trigger_skill_review_now` | `POST /api/skills/review/run` | ✅ |
| `get_skills_auto_review_promotion` | `GET /api/skills/auto-review/promotion` | ✅ |
| `set_skills_auto_review_promotion` | `PUT /api/skills/auto-review/promotion` | ✅ |
| `get_skills_auto_review_enabled` | `GET /api/skills/auto-review/enabled` | ✅ |
| `set_skills_auto_review_enabled` | `PUT /api/skills/auto-review/enabled` | ✅ |
| `get_skills_auto_review_config` | `GET /api/skills/auto-review/config` | ✅ |
| `set_skills_auto_review_config` | `PATCH /api/skills/auto-review/config` | ✅ |
| `reset_skills_auto_review_config` | `POST /api/skills/auto-review/config/reset` | ✅ |
| `get_skills_auto_review_recent_rejects` | `GET /api/skills/auto-review/recent-rejects` | ✅ |
| `run_skills_curator_now` | `POST /api/skills/curator/run` | ✅ |
| `apply_skills_curator_merge` | `POST /api/skills/curator/apply` | ✅ |

### Slash commands

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_slash_commands` | `GET /api/slash-commands` | ✅ |
| `execute_slash_command` | `POST /api/slash-commands/execute` | ✅ |
| `is_slash_command` | `POST /api/slash-commands/is-slash` | ✅ |

#### `/status` output 字段（与 GUI 弹层对齐）

`execute_slash_command` 返回的 `content` markdown 渲染按如下顺序拼接（值缺失时整行省略）；与 `ChatTitleBar.tsx` 的 Session Status popover 字段一一对应。

| 字段 | 数据源 | 格式 |
|---|---|---|
| Hope Agent 版本 | `env!("CARGO_PKG_VERSION")` | `- **Hope Agent**: v0.1.0` |
| Model + Auth type | `AppConfig.active_model` + `AvailableModel.api_type` | `- **Model**: Anthropic / Claude 3.7 Sonnet (api-key)`；Codex provider → `(oauth)` |
| Agent | 调用方传入 `agent_id` | `- **Agent**: \`default\`` |
| Title | `sessions.title` | `- **Title**: ...`（仅当非空） |
| Session ID | 调用方传入 `session_id` | `- **Session ID**: \`<uuid>\`` |
| Messages | `count_user_assistant_messages` | `- **Messages**: M user, N assistant` |
| Permission Mode | `sessions.permission_mode` | `- **Permission Mode**: \`default\` \| \`smart\` \| \`yolo\`` |
| Thinking | `sessions.reasoning_effort` → `live_reasoning_effort()` → `medium` | `- **Thinking**: high` |
| Context | `messages` 最后一条 assistant 行的 `tokens_in_last`（fallback `tokens_in`）vs 该行 `model` 对应的 `context_window` | `- **Context**: 42k / 200k (21%)`；window=0 时仅显示已用值 |
| Cache (last round) | 最后一条 assistant 的 `tokens_cache_creation` / `tokens_cache_read`（**不累计**；来自该 turn 最后一次 API round） | `- **Cache (last round)**: write 2k · hit 38k`；字段存在时即使两值都是 0 也显示，字段缺失时整行省略 |
| Updated | `sessions.updated_at` 相对时间 | `- **Updated**: just now` / `Nm ago` / `Nh ago` / `Nd ago` |
| Current Project | `sessions.project_id` | 单独一段（项目名 / desc / agent / working dir / instructions / agent source） |
| Attached IM Channels | `channel_db.list_attached(session_id)` | 单独一段（每行 `★` primary 标记 + channel:account:chat:thread + `attached_at`） |

Context / Cache 共用单 SQL `get_session_last_assistant_token_row`，避免渲染时多次扫表。Context window 在当前激活模型与该行 `model` 列名不同时，按 `cached_config().providers` 反查兜底。

### MCP servers

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `mcp_list_servers` | `GET /api/mcp/servers` | ✅ |
| `mcp_add_server` | `POST /api/mcp/servers` | ✅ |
| `mcp_reorder_servers` | `POST /api/mcp/servers/reorder` | ✅ |
| `mcp_update_server` | `PUT /api/mcp/servers/{id}` | ✅ |
| `mcp_remove_server` | `DELETE /api/mcp/servers/{id}` | ✅ |
| `mcp_get_server_status` | `GET /api/mcp/servers/{id}/status` | ✅ |
| `mcp_test_connection` | `POST /api/mcp/servers/{id}/test` | ✅ |
| `mcp_reconnect_server` | `POST /api/mcp/servers/{id}/reconnect` | ✅ |
| `mcp_start_oauth` | `POST /api/mcp/servers/{id}/oauth/start` | ✅ |
| `mcp_sign_out` | `POST /api/mcp/servers/{id}/oauth/sign-out` | ✅ |
| `mcp_list_tools` | `GET /api/mcp/servers/{id}/tools` | ✅ |
| `mcp_get_recent_logs` | `GET /api/mcp/servers/{id}/logs` | ✅ |
| `mcp_import_claude_desktop_config` | `POST /api/mcp/import/claude-desktop` | ✅ |
| `mcp_get_global_settings` | `GET /api/mcp/global` | ✅ |
| `mcp_update_global_settings` | `PUT /api/mcp/global` | ✅ |

### Channels (IM)

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `channel_list_plugins` | `GET /api/channel/plugins` | ✅ |
| `channel_list_accounts` | `GET /api/channel/accounts` | ✅ |
| `channel_add_account` | `POST /api/channel/accounts` | ✅ |
| `channel_update_account` | `PUT /api/channel/accounts/{accountId}` | ✅ |
| `channel_remove_account` | `DELETE /api/channel/accounts/{accountId}` | ✅ |
| `channel_start_account` | `POST /api/channel/accounts/{accountId}/start` | ✅ |
| `channel_stop_account` | `POST /api/channel/accounts/{accountId}/stop` | ✅ |
| `channel_sync_commands` | `POST /api/channel/sync-commands` | ✅ |
| `channel_health` | `GET /api/channel/accounts/{accountId}/health` | ✅ |
| `channel_health_all` | `GET /api/channel/health` | ✅ |
| `channel_validate_credentials` | `POST /api/channel/validate` | ✅ |
| `channel_send_test_message` | `POST /api/channel/accounts/{accountId}/test-message` | ✅ |
| `channel_list_sessions` | `GET /api/channel/sessions` | ✅ |
| `channel_wechat_start_login` | `POST /api/channel/wechat/login/start` | ✅ |
| `channel_wechat_wait_login` | `POST /api/channel/wechat/login/wait` | ✅ |
| `channel_handover_session` | `POST /api/channel/handover` | ✅ |

### Subagent / Team

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_subagent_runs` | `GET /api/subagent/runs` | ✅ |
| `get_subagent_run` | `GET /api/subagent/runs/{runId}` | ✅ |
| `get_subagent_runs_batch` | `POST /api/subagent/runs/batch` | ✅ |
| `kill_subagent` | `POST /api/subagent/runs/{runId}/kill` | ✅ |
| `list_teams` | `GET /api/teams` | ✅ |
| `create_team` | `POST /api/teams` | ✅ |
| `get_team` | `GET /api/teams/{teamId}` | ✅ |
| `get_team_members` | `GET /api/teams/{teamId}/members` | ✅ |
| `get_team_messages` | `GET /api/teams/{teamId}/messages` | ✅ |
| `get_team_messages_before` | `GET /api/teams/{teamId}/messages/before` | ✅ |
| `get_team_tasks` | `GET /api/teams/{teamId}/tasks` | ✅ |
| `send_user_team_message` | `POST /api/teams/{teamId}/messages` | ✅ |
| `pause_team` | `POST /api/teams/{teamId}/pause` | ✅ |
| `resume_team` | `POST /api/teams/{teamId}/resume` | ✅ |
| `dissolve_team` | `POST /api/teams/{teamId}/dissolve` | ✅ |
| `list_team_templates` | `GET /api/team-templates` | ✅ |
| `save_team_template` | `POST /api/team-templates` | ✅ |
| `delete_team_template` | `DELETE /api/team-templates/{templateId}` | ✅ |

### Weather / URL preview / Embedded browser

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `geocode_search` | `GET /api/weather/geocode` | ✅ |
| `preview_weather` | `POST /api/weather/preview` | ✅ |
| `detect_location` | `GET /api/weather/detect-location` | ✅ |
| `get_current_weather` | `GET /api/weather/current` | ✅ |
| `refresh_weather` | `POST /api/weather/refresh` | ✅ |
| `fetch_url_preview` | `POST /api/url-preview` | ✅ |
| `fetch_url_previews` | `POST /api/url-preview/batch` | ✅ |
| `browser_get_status` | `GET /api/browser/status` | ✅ |
| `browser_extension_status` | `GET /api/browser/extension/status` | ✅ |
| `browser_install_native_host_manifest` | `POST /api/browser/extension/install-native-host` | ✅ |
| `browser_extension_stop_control` | `POST /api/browser/extension/stop-control` | ✅ |
| `browser_list_profiles` | `GET /api/browser/profiles` | ✅ |
| `browser_create_profile` | `POST /api/browser/profiles` | ✅ |
| `browser_delete_profile` | `DELETE /api/browser/profiles/{name}` | ✅ |
| `browser_launch` | `POST /api/browser/launch` | ✅ |
| `browser_connect` | `POST /api/browser/connect` | ✅ |
| `browser_disconnect` | `POST /api/browser/disconnect` | ✅ |
| `browser_capture_frame` | `POST /api/browser/capture-frame`，body 可带 `{ sessionId? }` | ✅ |
| `browser_spawn_user_chrome` | `POST /api/browser/spawn-user-chrome` | ✅ |
| `browser_doctor` | `GET /api/browser/doctor` | ✅ |
| `browser_get_config` | `GET /api/browser/config` | ✅ |
| `browser_set_config` | `POST /api/browser/config` | ✅ |
| `browser_install_chromium_runtime` | `POST /api/browser/install-chromium-runtime` | ✅ |

### Theme / Language / UI

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_theme` | `GET /api/config/theme` | ✅ |
| `set_theme` | `POST /api/config/theme` | ✅ |
| `set_window_theme` | `POST /api/config/window-theme` | ✅ |
| `get_language` | `GET /api/config/language` | ✅ |
| `set_language` | `POST /api/config/language` | ✅ |
| `get_ui_effects_enabled` | `GET /api/config/ui-effects` | ✅ |
| `set_ui_effects_enabled` | `POST /api/config/ui-effects` | ✅ |
| `get_prevent_sleep_enabled` | `GET /api/config/prevent-sleep` | ✅ |
| `set_prevent_sleep_enabled` | `POST /api/config/prevent-sleep` | ✅ |
| `get_tool_call_narration_enabled` | `GET /api/config/tool-call-narration` | ✅ |
| `set_tool_call_narration_enabled` | `POST /api/config/tool-call-narration` | ✅ |
| `get_autostart_enabled` | `GET /api/config/autostart` | ✅ |
| `set_autostart_enabled` | `POST /api/config/autostart` | ✅ |

### Tools

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_tool_timeout` | `GET /api/config/tool-timeout` | ✅ |
| `set_tool_timeout` | `POST /api/config/tool-timeout` | ✅ |
| `get_approval_timeout` | `GET /api/config/approval-timeout` | ✅ |
| `set_approval_timeout` | `POST /api/config/approval-timeout` | ✅ |
| `get_approval_timeout_enabled` | `GET /api/config/approval-timeout-enabled` | ✅ |
| `set_approval_timeout_enabled` | `POST /api/config/approval-timeout-enabled` | ✅ |
| `get_approval_timeout_action` | `GET /api/config/approval-timeout-action` | ✅ |
| `set_approval_timeout_action` | `POST /api/config/approval-timeout-action` | ✅ |
| `get_unattended_approval_action` | `GET /api/config/unattended-approval-action` | ✅ |
| `set_unattended_approval_action` | `POST /api/config/unattended-approval-action` | ✅ |
| `get_tool_result_disk_threshold` | `GET /api/config/tool-result-threshold` | ✅ |
| `set_tool_result_disk_threshold` | `POST /api/config/tool-result-threshold` | ✅ |
| `get_tool_limits` | `GET /api/config/tool-limits` | ✅ |
| `set_tool_limits` | `POST /api/config/tool-limits` | ✅ |

### Permission（v2 权限/审批引擎）

详见 [`docs/architecture/permission-system.md`](permission-system.md)。

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_global_yolo_status` | `GET /api/permission/global-yolo` | ✅ 返回 `{ cliFlag, configFlag, active }` |
| `set_dangerous_skip_all_approvals` | `POST /api/security/dangerous-skip-all-approvals` | ✅ 切换 `permission.global_yolo`（兼容历史路径） |
| `get_smart_mode_config` | `GET /api/permission/smart` | ✅ 读 SmartModeConfig |
| `set_smart_mode_config` | `POST /api/permission/smart` | ✅ 写 SmartModeConfig |
| `get_protected_paths` | `GET /api/permission/protected-paths` | ✅ 返回 `{ current, defaults }` |
| `set_protected_paths` | `POST /api/permission/protected-paths` | ✅ 全量替换 |
| `reset_protected_paths` | `POST /api/permission/protected-paths/reset` | ✅ 恢复硬编码默认 |
| `get_dangerous_commands` | `GET /api/permission/dangerous-commands` | ✅ |
| `set_dangerous_commands` | `POST /api/permission/dangerous-commands` | ✅ |
| `reset_dangerous_commands` | `POST /api/permission/dangerous-commands/reset` | ✅ |
| `get_edit_commands` | `GET /api/permission/edit-commands` | ✅ |
| `set_edit_commands` | `POST /api/permission/edit-commands` | ✅ |
| `reset_edit_commands` | `POST /api/permission/edit-commands/reset` | ✅ |

### Crash / Recovery

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_crash_recovery_info` | `GET /api/crash/recovery-info` | ✅ |
| `get_config_health` | `GET /api/settings/config-health` | ✅ |
| `get_crash_history` | `GET /api/crash/history` | ✅ |
| `clear_crash_history` | `DELETE /api/crash/history` | ✅ |
| `list_backups_cmd` | `GET /api/crash/backups` | ✅ |
| `create_backup_cmd` | `POST /api/crash/backups` | ✅ |
| `restore_backup_cmd` | `POST /api/crash/backups/restore` | ✅ |
| `list_settings_backups_cmd` | `GET /api/settings/backups` | ✅ |
| `restore_settings_backup_cmd` | `POST /api/settings/backups/restore` | ✅ |
| `get_guardian_enabled` | `GET /api/crash/guardian` | ✅ |
| `set_guardian_enabled` | `PUT /api/crash/guardian` | ✅ |
| `request_app_restart` | `POST /api/system/restart` | ✅ |

### Developer（桌面专用，HTTP 端点亦保留供测试）

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `dev_clear_sessions` | `POST /api/dev/clear-sessions` | ✅ |
| `dev_clear_cron` | `POST /api/dev/clear-cron` | ✅ |
| `dev_clear_memory` | `POST /api/dev/clear-memory` | ✅ |
| `dev_reset_config` | `POST /api/dev/reset-config` | ✅ |
| `dev_clear_all` | `POST /api/dev/clear-all` | ✅ |

### ACP / Auth

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `acp_list_backends` | `GET /api/acp/backends` | ✅ |
| `acp_health_check` | `GET /api/acp/backends` | ✅ |
| `acp_refresh_backends` | `POST /api/acp/refresh` | ✅ |
| `acp_list_runs` | `GET /api/acp/runs` | ✅ |
| `acp_kill_run` | `POST /api/acp/runs/{runId}/kill` | ✅ |
| `acp_get_run_result` | `GET /api/acp/runs/{runId}/result` | ✅ |
| `acp_get_config` | `GET /api/acp/config` | ✅ |
| `acp_set_config` | `PUT /api/acp/config` | ✅ |
| `start_codex_auth` | `POST /api/auth/codex/start` | ✅ |
| `finalize_codex_auth` | `POST /api/auth/codex/finalize` | ✅ |
| `get_codex_models` | `GET /api/auth/codex/models` | ✅ |
| `set_codex_model` | `POST /api/auth/codex/models` | ✅ |

### Desktop-only（Web 模式 no-op）

| Tauri Command | HTTP | 说明 |
|---|---|---|
| `open_url` | `POST /api/desktop/open-url` | HTTP 端点保留但返回 no-op（浏览器无系统调用权限） |
| `open_directory` | `POST /api/desktop/open-directory` | 同上 |
| `reveal_in_folder` | `POST /api/desktop/reveal-in-folder` | 同上 |
| `set_dock_badge_cmd` | — | 仅桌面：把全局未读总数写到 app icon / Dock 角标（`count=0` 清除）；前端按 `isTauriMode()` 门控，Web 端不调用，无 HTTP 端点 |
| `get_system_prompt` | `POST /api/system-prompt` | 调试端点 |

### Filesystem

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `fs_list_dir` | `GET /api/filesystem/list-dir?path=<abs>` | ✅ |
| `fs_search_files` | `GET /api/filesystem/search-files?root=<abs>&q=<query>&limit=50` | ✅；path-aware fuzzy v2（精确 / 前缀 / 多 token / 路径分段 / 驼峰词感知，subsequence 兜底） |

`list-dir` 列出服务器本地目录单层条目，供 HTTP 模式目录浏览器驱动 `set_session_working_dir`，以及聊天输入框 `@` mention popper 的"路径模式"。参数要求绝对路径，后端会 canonicalize 并校验 `is_dir`；无 `path` 参数时返回平台默认根（Unix: `/`，Windows: `USERPROFILE`）。响应 `{ path, parent, entries: [{ name, isDir, isSymlink, size, modifiedMs }], truncated }`，按目录优先 + 名字升序排序，单次最多 5000 条（超出 `truncated=true`）。

`search-files` 在 `root` 下做 fuzzy 搜索，供聊天输入框 `@` mention popper 的"搜索模式"使用——用户输入 `@chat` 这种不含 `/` 的非空 token 时调用。后端用 `ignore::WalkBuilder` 遍历，遵守 `.gitignore` / `.git/info/exclude` / `.ignore` / 隐藏文件规则；`q` 按子序列匹配 + 评分（name 命中 +1000、path 命中 +200，靠近开头 + 跨度紧凑得分高）。响应 `{ root, matches: [{ name, path, relPath, isDir, score }], truncated }`，按 score desc + path asc 排序；`limit` 默认 50，最大 200；单次最多遍历 50000 条文件，超出 `truncated=true`。

两个 endpoint 桌面用 Tauri 原生 dialog（`pickLocalDirectory`）做目录初选时仍优先走 `@tauri-apps/plugin-dialog`，但 mention popper 在 Tauri 模式同样需要列目录 / 搜索能力，因此两个命令在桌面端也 invoke 注册。核心逻辑在 [`crates/ha-core/src/filesystem/mod.rs`](crates/ha-core/src/filesystem/mod.rs) 单一来源，axum / Tauri 两侧都是薄壳。

### First-run onboarding wizard

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_onboarding_state` | `GET /api/onboarding/state` | ✅ |
| `save_onboarding_draft` | `POST /api/onboarding/draft` | ✅ |
| `mark_onboarding_completed` | `POST /api/onboarding/complete` | ✅ |
| `mark_onboarding_skipped` | `POST /api/onboarding/skip` | ✅ |
| `reset_onboarding` | `POST /api/onboarding/reset` | ✅ |
| `apply_onboarding_language` | `POST /api/onboarding/language` | ✅ |
| `apply_onboarding_profile` | `POST /api/onboarding/profile` | ✅ |
| `apply_personality_preset_cmd` | `POST /api/onboarding/personality-preset` | ✅ |
| `apply_onboarding_safety` | `POST /api/onboarding/safety` | ✅ |
| `apply_onboarding_skills` | `POST /api/onboarding/skills` | ✅ |
| `apply_onboarding_server` | `POST /api/onboarding/server` | ✅ |
| `generate_api_key` | `POST /api/server/generate-api-key` | ✅ |
| `list_local_ips` | `GET /api/server/local-ips` | ✅ |

## 已知不对齐项

截至 2026-07-08 三端差集为 14 条（§7.3 的 5 条 Desktop-only 系统权限命令 + §7.3.1 的 `save_avatar` multipart / `fs_list_dir` / `fs_search_files` / `fs_create_dir` / `project_fs_upload` / `export_session_cmd` 六条 HTTP 已实现但走专用 Transport 方法 + `project_fs_resolve` / `kb_file_resolve_cmd` / `set_dock_badge_cmd` 三条 Tauri-only），没有"HTTP 漏写 COMMAND_MAP"或"HTTP 路由缺失"的破口。COMMAND_MAP 里的每一条都能在 `tauri::generate_handler!` 里找到对应命令；反向差 14 条均已在下文相应章节登记。`check_sandbox_available` 已对齐到 HTTP `GET /api/config/sandbox/status`，不再属于 Desktop-only。

### §7.3 Desktop-only（Tauri 专属，合法缺失，5 条）

| Tauri Command | 说明 |
|---|---|
| `check_system_permissions` | macOS 系统权限 v2 目录与状态查询 |
| `request_system_permission` | macOS 系统权限 v2 请求/跳转 |
| `check_all_permissions` | 权限 v1 兼容包装 |
| `check_permission` | 权限 v1 兼容包装 |
| `request_permission` | 权限 v1 兼容包装 |

前端必须在 `supportsLocalFileOps()` / `isTauriMode()` 或等价的运行模式判定保护下调用，HTTP 模式应 gate 住相关 UI。

### §7.3.1 不进 COMMAND_MAP 但 HTTP 已实现的合法非 REST 命令（6 条）

| Tauri Command | HTTP 端点 | 原因 |
|---|---|---|
| `save_avatar` | `POST /api/avatars` | multipart/form-data，HTTP 走 `HttpTransport.call()` 特殊分支 |
| `fs_list_dir` | `GET /api/filesystem/list-dir?path=<abs>` | query-string GET，HTTP 走 `HttpTransport.listServerDirectory()` 自定义方法（详见 Filesystem 域） |
| `fs_search_files` | `GET /api/filesystem/search-files?root=<abs>&q=<q>&limit=<n>` | 同上，走 `HttpTransport.searchFiles()` |
| `fs_create_dir` | `POST /api/filesystem/create-dir` | 绝对目录创建 + listing 返回，HTTP 走 `HttpTransport.createDirectory()` 专用方法 |
| `project_fs_upload` | `POST /api/fs/upload`（multipart） | 走 `HttpTransport.projectFsUpload()` 专用方法（详见 Filesystem 域） |
| `export_session_cmd` | `GET /api/sessions/{sessionId}/export` | 两端形态不对称（Tauri 走原生 save dialog，HTTP 返二进制流），统一入口 `exportSession`（详见 Session 域） |

这六条都是 HTTP 端有路由且前端两侧都能调用，只是不通过通用的 `COMMAND_MAP` JSON 路径。另有 `project_fs_resolve` / `kb_file_resolve_cmd`（Tauri-only `convertFileSrc`）与 `set_dock_badge_cmd`（Desktop-only Dock 角标）属 Tauri 专属、无 HTTP 对应，已分别在 Filesystem / 知识空间 / Desktop-only 章节登记。

### §7.4 命名/返回值语义差异

| 场景 | Tauri | HTTP | 备注 |
|---|---|---|---|
| `save_avatar` 返回值 | `-> String`（路径） | `{ path: string }` | `HttpTransport.call()` 特殊分支解包为 `string`，前端无感 |
| `openMedia` 底层命令 | `invoke("open_directory", {path})` | `POST /api/desktop/open-directory`（no-op） | 命令名与语义（"打开媒体"）不符，但保留以免破坏桌面行为 |
| `prepareFileData` mimeType 参数 | Tauri 实现忽略 | HTTP 用来构造 `Blob` | Tauri 侧不影响传输，语义差异仅限参数是否被使用 |
| 空响应 | 由具体命令决定 | 204 / 非 JSON content-type → `undefined as T` | 调用方需按命令契约处理 |
| `initialize_agent` | 写 config + 回填 `*state.agent.lock()` | 仅写 config（Anthropic provider + active_model），HTTP 模式 agent 按请求从 `cached_config` 重建 | 返回体相同 `{ ok: true }`；首次启动期之外一般不会被调用 |
| `set_codex_model` | 写 config + 如有内存 agent 重建 | 仅写 config；HTTP 模式每个 `POST /api/chat` 按配置新建 agent | 同上，返回 `{ ok: true }` |

## 新增接口 checklist

每次新增一个 Tauri 命令时，必须同 PR 完成以下四件事（AGENTS.md 亦强调）：

1. **后端实现**：在 `src-tauri/src/commands/` 或 `crates/ha-core/` 写业务函数；如果是核心逻辑放 `ha-core`
2. **Tauri 注册**：在 [`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs) 的 `tauri::generate_handler![...]` 加命令名
3. **HTTP 路由**：在 `crates/ha-server/src/routes/<domain>.rs` 加 handler，在 [`crates/ha-server/src/lib.rs`](../../crates/ha-server/src/lib.rs) 的 `Router::new()` 链式注册 `.route(...)`
4. **前端映射**：在 [`src/lib/transport-http.ts`](../../src/lib/transport-http.ts) 的 `COMMAND_MAP` 加一行 `command_name: { method, path }`
5. **本文档**：在对应功能域表格追加一行（可跑 §8 的验证脚本对账）

> 例外：仅桌面有意义（快捷键、托盘、权限探测）的命令可跳过步骤 3-4，但必须在 §7.3 登记。

## 验证脚本

以下 shell 段落可在项目根运行，本文档对照表的数据正确性依赖它们：

```bash
# 1. Tauri 命令总数（截至 2026-07-08：813）
awk 'BEGIN{flag=0} /tauri::generate_handler!\[/{flag=1;next} flag&&/^[[:space:]]*\]\)/{flag=0} flag' \
    src-tauri/src/lib.rs | grep -vE '^[[:space:]]*//|^[[:space:]]*$' | \
    grep -oE '::[a-z_][a-zA-Z0-9_]*,?[[:space:]]*$' | tr -d ':, ' | sort -u | wc -l

# 2. HTTP 路由总数（截至 2026-07-08：766）
grep -cE '^[[:space:]]+\.route\(' crates/ha-server/src/lib.rs

# 3. COMMAND_MAP 条目数（截至 2026-07-08：799，不含闭合 `}` 的行）
awk '/^const COMMAND_MAP/,/^};/' src/lib/transport-http.ts | \
    grep -cE '^[[:space:]]+[a-z_][a-zA-Z0-9_]*:[[:space:]]*\{'

# 4. 差集：Tauri 有、COMMAND_MAP 无（应与 §7.3 + §7.3.1 + 3 条 Tauri-only 总和一致）
comm -23 \
  <(awk 'BEGIN{flag=0} /tauri::generate_handler!\[/{flag=1;next} flag&&/^[[:space:]]*\]\)/{flag=0} flag' \
      src-tauri/src/lib.rs | grep -vE '^[[:space:]]*//|^[[:space:]]*$' | \
      grep -oE '::[a-z_][a-zA-Z0-9_]*,?[[:space:]]*$' | tr -d ':, ' | sort -u) \
  <(awk '/^const COMMAND_MAP/,/^};/' src/lib/transport-http.ts | \
      grep -oE '^[[:space:]]+[a-z_][a-zA-Z0-9_]*:' | tr -d ': ' | sort -u)
# 期望：14 行
#   check_system_permissions / request_system_permission
#   / check_all_permissions / check_permission / request_permission  （§7.3 Desktop-only）
#   / save_avatar / fs_list_dir / fs_search_files / fs_create_dir / project_fs_upload / export_session_cmd  （§7.3.1 HTTP 已实现走专用方法）
#   / project_fs_resolve / kb_file_resolve_cmd / set_dock_badge_cmd  （Tauri-only，无 HTTP）
```

## 运行模式快速回顾

详见 [backend-separation.md](backend-separation.md)。

| 模式 | 启动命令 | 前端通信 |
|---|---|---|
| 桌面 GUI（默认） | `hope-agent` | Tauri IPC + 内嵌 HTTP 可选 |
| HTTP/WS 守护 | `hope-agent server [--bind ...] [--api-key ...]` | REST + WebSocket |
| ACP stdio | `hope-agent acp` | JSON-RPC over stdio（不经本文档的接口） |

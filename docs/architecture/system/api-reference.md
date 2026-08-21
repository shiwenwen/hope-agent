# API 参考：Tauri ↔ HTTP/WebSocket 对照

> 返回 [文档索引](../../README.md) | 关联源码：[`src-tauri/src/lib.rs`](../../../src-tauri/src/lib.rs) · [`crates/ha-server/src/lib.rs`](../../../crates/ha-server/src/lib.rs) · [`src/lib/transport-http.ts`](../../../src/lib/transport-http.ts)

## 概述

Hope Agent 前端通过 `Transport` 抽象层和后端通信，内部根据运行环境自动在 Tauri IPC 和 HTTP/WebSocket 之间切换。本文档把两条通道上的**每一条接口**列成一一对应的表格，并标记对齐状态。

## 数据来源（截至 2026-08-05）

| 源 | 位置 | 数量 |
|---|---|---|
| Tauri 命令 | `src-tauri/src/lib.rs` 的 `tauri::generate_handler!` | **1128** |
| HTTP 路由 | `crates/ha-server/src/lib.rs` 的 `.route(...)` | **1068** |
| 前端 COMMAND_MAP | `src/lib/transport-http.ts::COMMAND_MAP` | **1108** |
| WebSocket 端点 | `crates/ha-server/src/ws/` | **1** |
| EventBus 事件 | 全代码 `emit_event` 调用 | **59+** |

## 对齐情况摘要

| 分类 | 数量 | 说明 |
|---|---|---|
| ✅ 两端完全对齐（在 COMMAND_MAP 中） | 1108 | 常规请求/响应命令，以及 HTTP-only 的 bound raw ticket 命令 |
| 🔧 特殊处理（不在 COMMAND_MAP 但 HTTP 已实现，走专用 Transport 方法） | 16 | multipart/二进制流/保存对话框类接口、HTTP-only 的短时 transport ticket 基础设施，以及远程服务端更新控制面 |
| 🖥️ Desktop-only / Tauri-only（HTTP 无对应） | 10 | macOS / legacy 系统权限探测（5 条）+ `project_fs_resolve` / `kb_file_resolve_cmd`（`convertFileSrc`）+ Dock / tray 未读提示 + browser-side save-as |
| ❌ HTTP 路由存在但 COMMAND_MAP 漏写 | 0 | — |
| ❌ HTTP 路由完全缺失 | 0 | — |

Tauri ↔ COMMAND_MAP 差集为 22 条合法非通用映射命令：5 条 Desktop-only 系统权限命令、12 条走专用 Transport 方法的 multipart/二进制/保存路径接口，以及 `project_fs_resolve` / `kb_file_resolve_cmd` / `set_dock_badge_cmd` / `set_tray_unread_cmd` / `save_exported_file` 5 条 Tauri-only 命令。HTTP 路由侧的非 REST endpoint（健康检查、静态文件、流式 chat、multipart 和二进制下载等）不要求映射为通用 `call()`；对齐状态在各功能域章节登记。新增 Tauri 命令时须同步补 HTTP 路由、COMMAND_MAP 或专用 Transport 说明。

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
| HTTP REST | `Authorization: Bearer <owner_token>` header |
| 浏览器 HTTP / WebSocket / 媒体 | Root Token 经 `POST /api/auth/session` 一次性交换为签名 `HttpOnly; SameSite=Strict` Cookie；Root Token 不进入 URL/localStorage |
| 跨源远程 GUI | Fetch 继续用 Bearer；`POST /api/auth/transport-tickets` 以独立随机签名密钥换 15 分钟 `events` 与非执行型 UI 静态资源票据（避免把弱 Root Token 变成离线猜测 oracle）。WebSocket 票据走 `Sec-WebSocket-Protocol`；Canvas / Design 可执行 iframe 另经 `POST /api/auth/preview-resource-ticket` 绑定到单个 project / artifact 子树，相对 CSS/JS/图片继承同一前缀但不能横跳其他资源；workspace / session raw preview 分别经 `/api/fs/raw-ticket` / `/api/sessions/{id}/files/by-path-ticket` 绑定到单个 canonical file；这些票据均不能调用 owner 控制面 |
| 自动化客户端 | `Authorization: Bearer <owner_token>`；不接受通用 `?token=` |
| Knowledge Agent 只读 token | `server.knowledgeAgentReadToken` 或 `HA_KNOWLEDGE_AGENT_READ_TOKEN`；仅在 Owner Token 已启用时参与鉴权，仅允许 `POST /api/knowledge/agent/{search,read,expand,sources}`，其它受保护 API 返回 403 |
| 免鉴权 | `GET /api/health`、浏览器登录引导 `/api/auth/{status,session,logout}`、显式创建的只读 Design Share capability URL，以及自带短时 scope 签名的 `/api/resource/{ticket}/...`；`GET /api/server/status` 已归入 Owner 保护面 |

`api_key=None` 仅允许回环监听；非回环启动默认 fail-closed（危险逃生开关除外），此时受保护路由才退化为无鉴权。鉴权实现见 [`crates/ha-server/src/middleware.rs`](../../../crates/ha-server/src/middleware.rs)（constant-time 比较）。

## WebSocket 端点

| Path | 用途 | 消息格式 |
|---|---|---|
| `/ws/events` | 全局事件广播（EventBus → WS，多客户端同步） | JSON：`{ name: string, payload: unknown }` |

**HTTP 模式重连**：前端 `/ws/events` 指数退避（1s→30s 封顶），只在有活跃 listener 时维持连接；同源浏览器握手携带 HttpOnly Cookie，跨源远程客户端把短时 `events` 票据放进 WebSocket 子协议（不进 URL）。首次 listener 注册自动连上，最后一个取消订阅自动关闭。详见 `src/lib/transport-http.ts` 的 `ensureEventWs` / `scheduleReconnect` / `teardownEventWs`。

## EventBus 事件清单

所有事件由 `ha-core::EventBus` 发射（`BroadcastEventBus`，256 容量），桌面和 HTTP 两条桥各自订阅：
- **Tauri 桥** `src-tauri/src/setup.rs` — subscriber 转 `app_handle.emit(name, payload)`
- **HTTP 桥** `crates/ha-server/src/ws/events.rs` — subscriber 转 `/ws/events` 文本帧

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

### 系统更新

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `app_update:available` | headless 自动检查发现新版 | `{ currentVersion, version, notes?, pubDate?, recommendedPath }` |
| `app_update:staged` | SelfContained 静默预下载完成 | `{ version }` |
| `app_update:progress` | 下载字节或安装阶段变化 | `{ job_id, phase, label, percent?, written?, total? }` |
| `app_update:completed` | 更新任务终态 | `{ job_id, status, error?, targetVersion?, remote? }` |

事件只负责低延迟提示，不重放。Web / 远程桌面首次连接、重连或 lag 后必须重读 `GET /api/app-update/status`；跨服务重启的状态以 `~/.hope-agent/updater/remote-update-state.json` 为准。

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
| `cron:run_completed` | ha-cron 的 cron/executor.rs |
| `cron:unread_changed` | cron 未读聚合数变化（`cron_mark_all_read` 清除时发 `{ total: 0 }`）；前端 cron 未读 store 收到后刷新侧边栏角标 |
| `session:unread_changed` | assistant 消息落库或任一会话水位线更新；payload `{ sessionId?: string, domain?: "regular"\|"channel"\|"cron" }` 只作精准失效提示，消费者必须重查各域权威值 |
| `job:created` / `job:updated` / `job:progress` / `job:completed` / `job:mark_injected_failed` | 统一后台任务事件（替代旧 `async_tool_job:*`）。`async_jobs::events` 发射；kind-tagged（payload `{ job_id, kind: "tool"\|"group", tool, status, session_id }`），覆盖后台**工具 + Group** 生命周期。`created`=新任务出现（running/queued）；`updated`=非终态变化（如 cancelling）；`progress`=`{ job_id, kind, session_id, current, total }`（目前 Group 报 N/M 子完成）；`completed`=终态；`mark_injected_failed`=结果注入主对话失败告警 `{ job_id, error }`。**`subagent` kind 沿用 `subagent:*` 流**（不双发），面板合并两路 + `job_status list`。 |
| `app_update:progress` / `app_update:completed` | 自升级 (`app_update` 工具) 进度上报。`progress` payload `{ job_id, label, phase, percent?, written?, total? }`（每 5% / 1s 节流）；`completed` payload `{ job_id, status: "done"|"failed", outcome?, error? }`，详见 [`self-update.md`](../infra/self-update.md) |

### 项目（Project CRUD）

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `project:created` / `project:updated` / `project:deleted` | `src-tauri/src/commands/project.rs` 调 `bus.emit(...)` | `{ projectId }` |
| `project:file_uploaded` / `project:file_deleted` | 同上文件子命令 | `{ projectId, fileId }` |

### 桌面宠物（Pet）

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `pet:config_changed` | `ha_pet::update_config` | `{ enabled, selectedPetRef, source }`（不含素材或路径） |
| `pet:library_changed` | pet install/delete/restore | library 失效提示，消费者重拉 |
| `pet:activity_changed` | 合格 chat turn / pending / read watermark 变化 | activity 失效提示，消费者重拉 snapshot |
| `pet:navigate` | PetWindow 点击 activity | typed `PetNavigationTarget`；Tauri 直发给主 App |
| `pet:install_link` | OS `hope-agent://pets/install` | `{ link }`；只打开 Settings preview，不自动 commit |
| `pet:inactive_pointer` | macOS 原生失焦指针桥 | `{ inside, x, y }`；进入/移动时 `inside=true` 且坐标为 logical px，离开固定发 `{ inside: false, x: 0, y: 0 }`；最多 30 Hz，仅发给 PetWindow |
| `pet:native_drag_ended` | macOS 原生拖拽释放桥 | 无 payload；仅发给 PetWindow |

> `pet:config_changed`、`pet:library_changed`、`pet:activity_changed` 是 Core EventBus 失效通知，经 HTTP / Tauri 两条桥转发，server 模式前端也能收到。`pet:navigate`、`pet:install_link`、`pet:inactive_pointer`、`pet:native_drag_ended` 是桌面壳内部事件，不经过 HTTP EventBus 桥，也不构成远端 API。

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
| `design:show` / `design:reload` / `design:artifact_ready` / `design:artifact_deleted` / `design:project_changed` / `design:system_changed` / `design:critiqued` / `design:code_drift` | 设计空间（产物生成 / 预览刷新 / 系统变更 / 质量评审 / code→design 回灌 stale 翻转） |
| `design:artifact_generating` / `design:generate_delta` / `design:generate_done` / `design:generate_error` | 设计空间真流式生成（建 generating 壳 / 逐帧回填预览 / 定稿受控 swap / 失败降级）。`generate_delta` payload `{ projectId, artifactId, streamId, seq, css, bodyHtml, done }` |
| `design:ffmpeg_download_progress` | MP4 导出编码器（ffmpeg）按需下载进度。Payload `{ stage: "downloading"\|"ready", percent?, downloadedBytes?, totalBytes?, binaryPath? }` |
| `browser:frame` | 浏览器活动 tab 的实时 JPEG 帧。Payload `{ sessionId?, targetId?, url?, title?, jpegBase64, capturedAt, backend, actionId? }`。在 `act` / `navigate` / `tabs.new|select|claim` 后由 `tool_browser` choke point 统一 emit（`actionId` 关联对应 action 事件；1Hz 轮询帧无 `actionId`）；BrowserPanel 同时以 1Hz 轮询 `browser_capture_frame` 兜底并按当前会话过滤 |
| `browser:action` / `mac_control:action` | 浏览器 / macOS 控制工具的逐步操作事件（面板执行历史时间线）。Payload = `ToolActionEvent`（camelCase）：`{ actionId, source, sessionId?, action, op?, target?, detail?, url?, app?, ok, error?, durationMs, startedAt, toolCallId?, hasFrame }`。输入文本经脱敏只记长度（`text(N chars)`）；历史落进程内 per-session ring buffer（200 条 / 缩略图最近 50 条），经 `tool_recent_actions` 拉取，会话删除 / 焚毁即清 |

### Artifacts

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `artifact:created` / `artifact:updated` | `ArtifactService` 创建、更新或 restore | `{ artifactId, version, detail? }` |
| `artifact:verified` | 当前版本完成确定性 verifier | `{ artifactId, version, detail }`，detail 为 passed/failed |
| `artifact:export_running` / `artifact:export_ready` / `artifact:export_failed` | HTML/ZIP/Markdown/PDF 导出生命周期 | `{ artifactId, version, detail? }`，ready 时 detail 为 receipt ID |
| `artifact:archived` / `artifact:deleted` | owner archive/delete | `{ artifactId, version, detail? }` |

Artifact 创建或 show 仍复用 `canvas_show`，当前投影变化复用 `canvas_reload`，因此 `CanvasPanel` 不需要第二套预览事件。缺少系统 Chrome 和 Hope runtime 的 PDF 导出还会触发 `browser:runtime_required`，由全局 runtime 安装对话框处理。

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
| `slash:effort_changed` / `slash:plan_changed` / `slash:session_cleared` | `crates/ha-channel/src/channel/worker/slash.rs` 经 `bus.emit(...)` | effort 字段 / sessionId 等（具体见各调用点） |
| `session:model_updated` | `crates/ha-channel/src/channel/worker/slash.rs` (IM `/model`)、`src-tauri/src/commands/session.rs::set_session_model`、`crates/ha-server/src/routes/sessions.rs::set_session_model` | `{ sessionId, providerId, modelId }` — 桌面 GUI 仅在 `sessionId == currentSessionId` 时同步 ModelPicker UI |
| `terminal:created` / `output` / `exit` / `closed` | `ha_core::terminal::TerminalManager` | created 带 snapshot；output 为 `{ terminalId, seq, dataBase64 }`；exit 为 `{ terminalId, exitCode, error }`；closed 带 terminalId |

> 这些事件经 EventBus 广播，HTTP / Tauri 两条桥都会转发。

### 仅 Tauri 直发（不经 EventBus）

| 事件名 | 触发点 |
|---|---|
| `new-session` / `open-settings` | 菜单与快捷键（`src-tauri/src/tray.rs` / `setup.rs` 调 `app_handle.emit(...)`） |
| `chord-first-pressed` / `chord-timeout` / `shortcut-triggered` | 全局快捷键（`src-tauri/src/shortcuts.rs`） |

## 前端 Transport 抽象

接口定义：[`src/lib/transport.ts`](../../../src/lib/transport.ts)。

| 方法 | Tauri 实现 | HTTP 实现 |
|---|---|---|
| `call<T>(command, args)` | `invoke(command, args)` | REST 查表 + JSON；multipart 走特例分支 |
| `prepareFileData(buffer, mime)` | `Array.from(Uint8Array)` — JSON 传输（~4× 膨胀） | `new Blob([buffer], {type})` — 零拷贝 |
| `startChat(args, onEvent)` | `new Channel<string>()` + `invoke("chat", { ...args, onEvent })` | Bundled UI 走 `POST /api/chat/ui`，非 incognito 在 Session/turn 持久化后返回 202 ACK，执行由服务端 task 持有；流式 delta 走 `/ws/events`，ACK 合成 `session_created` + `turn_started`，漏掉 end 时查 `GET /api/chat/turns/{turnId}`；公共 `POST /api/chat` 与 incognito 保留同步完成语义 |
| `listen(eventName, handler)` | `@tauri-apps/api/event.listen` | 全局 `/ws/events` + name 匹配 + 指数退避重连 |
| `resolveMediaUrl(item)` | `convertFileSrc(localPath)` → `tauri://` | 仅支持 `/api/` 或 `http(s)://`，本地绝对路径返 `null` |
| `resolveAssetUrl(path)` | `convertFileSrc` | 正则识别 `avatars`/`image_generate`/`canvas` → 同源 `/api/...`，浏览器 Cookie 自动鉴权 |
| `openMedia(item)` | `invoke("open_directory", {path})` | 临时 `<a download>` 触发浏览器下载 |
| `revealMedia(item)` | `invoke("reveal_in_folder", {path})` | no-op |
| `previewReadText(path,{sessionId})` | `invoke("preview_read_text", {path})` | `GET /api/sessions/{id}/files/read?path=`（会话鉴权） |
| `previewExtractDoc(path,{sessionId})` | `invoke("preview_extract", {path})` | `GET /api/sessions/{id}/files/extract?path=`（会话鉴权） |
| `previewRawUrl(path,{sessionId},download)` | `resolveAssetUrl(path)`（`convertFileSrc`） | `POST /api/sessions/{id}/files/by-path-ticket` 后用绑定单个 canonical file 的 `/api/resource/{ticket}/fs/raw` |
| `fileRuntime()` | `{workspaceHost:"local",openMode:"system",canReveal:true}` | `{workspaceHost:"remote",openMode:"browser",canReveal:false}` |
| `getWorkspaceAccess(scope)` | `project_fs_capabilities` | `GET /api/fs/capabilities`（统一返回最终写能力与 canonical `rootPath`） |
| `openWorkspaceFile` / `downloadWorkspaceFile` | 系统打开 | `POST /api/fs/raw-ticket` 后用绑定单个 canonical file 的 `/api/resource/{ticket}/fs/raw` 浏览/下载 |
| `revealWorkspaceFile` | `reveal_in_folder` | 不支持（capability disabled） |
| `uploadFile(file,purpose)` | `file_upload_start/status/chunk/complete`（chunk raw binary IPC） | `/api/file-uploads*`（chunk Blob body） |
| `discardFileUpload(id)` | `file_upload_discard` | `DELETE /api/file-uploads/{id}` |
| `stageChatAttachment(file)` | 委托 `uploadFile(...,"chat_attachment")` | 同左 |
| `discardChatAttachmentUpload(id)` | 委托 `discardFileUpload` | 同左 |
| `supportsLocalFileOps()` | `true` | `false` |
| `pickLocalImage()` | `@tauri-apps/plugin-dialog.open` | 隐藏 `<input type="file">` + blob URL |
| `saveFileAs(blob, filename)` | `@tauri-apps/plugin-dialog.save`（记住上次目录）→ `invoke("save_exported_file", {path, dataBase64})`；返回 `path` 供 reveal | `showSaveFilePicker`（Chromium + 安全上下文）否则回退 `<a download>`；`path=null`。**绝不写服务器磁盘** |
| `revealFile(path)` | `invoke("reveal_in_folder", {path})` | no-op（Web 沙箱无法 reveal 本机文件） |

**文件上传特殊路径**（在 `HttpTransport.call()` 中走 multipart/form-data 而非 JSON）：

| 命令 | HTTP 端点 |
|---|---|
| `save_attachment` | `POST /api/chat/attachment` |
| `stage_chat_attachment` | `POST /api/chat/attachment-stage`（仅旧客户端兼容，静态 20 MiB） |
| `project_fs_upload` | `POST /api/fs/upload`（仅旧客户端兼容，静态 20 MiB） |
| `save_avatar` | `POST /api/avatars`（服务端返 `{path}`，前端解包为 `string` 匹配 Tauri `-> String` 契约） |

新版聊天、Workspace、知识来源与客户端本地 Artifact 来源统一使用上传租约：`file_upload_start/status/chunk/complete/discard` ↔ `POST /api/file-uploads`、`GET|DELETE /api/file-uploads/{id}`、`PUT /api/file-uploads/{id}/chunk?offset=`、`POST /api/file-uploads/{id}/complete`。purpose 分别为 `chat_attachment`、`workspace_upload`、`knowledge_source`、`artifact_source`；固定 4 MiB 顺序 chunk，lease 1 小时过期，最终由对应业务 claim 消费。

## 命令对照表（按功能域分组）

> 所有路径省略 scheme/host，默认 `http(s)://<host>:<port>` 前缀。路径参数 `{id}` / `{sessionId}` 等在请求时 URL 编码。Tauri 模式下命令名即 `invoke()` 的第一个参数。

### Desktop Pet

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_pet_config_cmd` | `GET /api/pets/config` | ✅ |
| `save_pet_config_cmd` | `PUT /api/pets/config` | ✅（HTTP 不允许改变 overlay enabled） |
| `pet_set_enabled_cmd` | `POST /api/pets/enabled` | ✅（HTTP 返回 desktop-only） |
| `pet_activate_cmd` | `POST /api/pets/activate` | ✅（`{petRef}` 原子选择并启用；仅 desktop runtime，headless 返回 desktop-only） |
| `pet_list_cmd` | `GET /api/pets` | ✅ |
| `pet_asset_path_cmd` | `GET /api/pets/asset?assetId=` | ✅（HTTP 返回同源 raw URL，不泄露主机路径） |
| — | `GET /api/pets/sprite?assetId=` | HTTP raw bytes + ETag |
| `pet_codex_candidates_cmd` | `GET /api/pets/codex-candidates` | ✅ |
| `pet_candidate_thumbnail_cmd` | `GET /api/pets/codex-candidates/{candidateId}/thumbnail` | ✅ |
| `pet_preview_thumbnail_cmd` | `GET /api/pets/import/previews/{previewToken}/thumbnail` | ✅（1536×208 idle 动画条） |
| `pet_create_preview_cmd` | `POST /api/pets/create/preview` | ✅（显式 media generation） |
| `pet_upgrade_v2_cmd` | `POST /api/pets/upgrade-v2` | ✅（保留 v1、安装 v2 副本；持久化成功后仅当 v1 仍被选中时切换到 v2） |
| `pet_import_preview_cmd` | `POST /api/pets/import/preview` | ✅（HTTP 拒绝 LocalPath，只接受 upload/link/candidate capability；link 可来自任意公网 origin，支持直接 zip / JSON manifest / PNG·WebP、deep link，以及少量显式 page resolver；不解释任意 HTML） |
| `pet_import_preview_cancel_cmd` | `POST /api/pets/import/preview/cancel` | ✅（token 只放 JSON body；幂等释放 preview cache 与其 upload leases） |
| `pet_import_commit_cmd` | `POST /api/pets/import/commit` | ✅（HTTP 拒绝 `enableAfterImport=true`，不能启用桌面 overlay） |
| `pet_delete_cmd` | `POST /api/pets/delete` | ✅（expected package hash） |
| `pet_restore_cmd` | `POST /api/pets/restore` | ✅（短期 restore token） |
| `pet_export_cmd` | `POST /api/pets/export` | ✅（Codex-compatible zip base64 DTO） |
| `pet_activity_snapshot_cmd` | `GET /api/pets/activity` | ✅ |
| `pet_take_install_link_cmd` | `GET /api/pets/install-link/pending` | ✅（HTTP 恒 `null`，OS 协议由 desktop shell 接收） |
| `pet_apply_window_bounds_cmd` | `POST /api/pets/window/bounds` | ✅（HTTP 明确返回 overlay unsupported） |
| `pet_sync_window_cmd` | `POST /api/pets/window/sync` | ✅（HTTP 明确返回 overlay unsupported） |
| `pet_focus_target_cmd` | `POST /api/pets/focus-target` | ✅（HTTP 明确返回 overlay unsupported） |

Pet 的主对话身份由 chat 请求可选 `uiSurface` 传播并落 `chat_turns.ui_surface`；缺省值绝不推断为桌面主对话。HTTP 只有带浏览器不可由页面脚本伪造的 `Sec-Fetch-Mode: cors`、`Sec-Fetch-Dest: empty`，且 `Origin` 与 `Host` 同源或命中服务端显式 CORS allowlist 时才能进入 `/api/chat/ui`；普通 API、side-query 和 automation 一律走会清空字段的 `/api/chat`。详见 [Pet 架构](../core/pet.md)。

本机 CLI 复用同一导入实现：先用 `hope-agent pet capabilities --json` 验证协议握手，再以 `hope-agent pet preview --source <PATH|URL> [--source <PATH> ...] --json` 返回待确认的 `packageHash`，随后 `hope-agent pet import` 用完全相同的来源列表和 `--expected-package-hash <HASH>` 重新读取来源、比对 hash 后提交。PATH 可指目录、zip、manifest 或 atlas；同目录 loose manifest + sprite 用重复 `--source`；URL 可指任意公网 origin 上的直接 zip / manifest / atlas。Import 恒只安装库包，不启用 overlay；显式启用另走 `hope-agent pet activate --pet-ref <REF>`，其内部安全调用 desktop-only Bearer API。远程调用仍使用上表 Bearer-auth HTTP preview / commit / activate，preview/commit token 只放 JSON body，模型侧不得为建立 HTTP 鉴权而读取 Owner Token。

跨源 HTTP/WS GUI 只允许显式 origin：打包桌面 WebView 的 `tauri://localhost` / `http://tauri.localhost` 默认加入 allowlist；其他前端部署通过逗号分隔的 `HA_CORS_ORIGINS` 配置（例如 `https://ui.example`）。不接受 `*`，同源浏览器无需配置。Owner Token 仍只走 Bearer/登录请求体，禁止放 URL；WebSocket 与静态资源使用 15 分钟 scope ticket。

### Projects

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_projects_cmd` | `GET /api/projects` | ✅ |
| `get_project_cmd` | `GET /api/projects/{id}` | ✅ |
| `get_project_overview_cmd` | `GET /api/projects/{id}/overview` | ✅（用户会话 + 自动记忆主题 + 有效结构化记忆 + `AGENTS.md` 状态） |
| `create_project_cmd` | `POST /api/projects` | ✅ |
| `update_project_cmd` | `PATCH /api/projects/{id}` | ✅ |
| `inspect_project_instructions_cmd` | `POST /api/projects/instructions/inspect` | ✅（表单切换工作目录时只读检查，不创建缺失文件） |
| `get_project_instructions_cmd` | `GET /api/projects/{id}/instructions` | ✅（只读检查，缺失时不创建） |
| `save_project_instructions_cmd` | `PUT /api/projects/{id}/instructions` | ✅（owner 设置面，原子写入，不受通用文件写闸门影响） |
| `delete_project_cmd` | `DELETE /api/projects/{id}` | ✅ |
| `archive_project_cmd` | `POST /api/projects/{id}/archive` | ✅ |
| `list_project_sessions_cmd` | `GET /api/projects/{id}/sessions?pinned=` | ✅（`pinned=true/false` 在分页前筛选置顶状态） |
| `move_session_to_project_cmd` | `PATCH /api/sessions/{sessionId}/project` | ✅ |
| `list_project_memories_cmd` | `GET /api/projects/{id}/memories` | ✅ |
| `list_project_memory_files_cmd` | `GET /api/projects/{id}/memory-files` | ✅ |
| `read_project_memory_file_cmd` | `GET /api/projects/{id}/memory-files/{fileName}` | ✅ |
| `write_project_memory_file_cmd` | `PUT /api/projects/{id}/memory-files` | ✅ |
| `delete_project_memory_file_cmd` | `DELETE /api/projects/{id}/memory-files/{fileName}?expectedFileHash=` | ✅（stale-write guard） |
| `rebuild_project_memory_index_cmd` | `POST /api/projects/{id}/memory-files/rebuild-index` | ✅ |

`list_projects_cmd` / `GET /api/projects` 接受可选 `active_session_id`（HTTP query `activeSessionId`）：前端仅在该会话满足“聊天主视图已选中 + 窗口聚焦 + document 可见 + 消息列表在最新位置”的可读条件时传入，使项目徽标与会话行口径一致，无需前端跨数据源相减。项目列表不再返回旧 `memoryCount`，也不再逐项目查询记忆库；概览口径统一由 `get_project_overview_cmd` / `GET /api/projects/{id}/overview` 提供。

项目指令以项目工作目录根 `AGENTS.md` 为唯一真相源，`Project` / `CreateProjectInput` / `UpdateProjectInput` 均不再携带 `instructions`。新增 / 编辑表单通过独立 `instructions: { content, expectedFileHash, expectedExists }` 请求字段把文件草稿与项目元数据一起提交；创建接口另接受默认 `true` 的 `createInstructionsIfMissing`，添加已有目录时可显式保留缺失状态。文件步骤失败会回滚项目创建 / 元数据更新，内容仍不进 SQLite。切换目录前与 GET 都只读检查目标文件，缺失时返回空内容、空文件 hash 与 `exists: false`，不提前建文件；用户显式保存指令时才以 create-new 语义建立文件。GET 返回 `{ path, content, contentHash, exists, created }`，PUT body 为 `{ content, expectedFileHash, expectedExists }` 并原样保留 Markdown 空白。保存前同时校验磁盘存在状态与 raw BLAKE3，任一不一致都返回冲突，防止覆盖 Agent / 外部编辑器的并发创建、删除或修改；旧客户端缺少 `expectedExists` 时按 `true` fail closed。

**项目文件浏览器（workspace-scoped filesystem）**——上传/读写改走作用域文件管理 API（旧的 `list_project_files_cmd` / `upload_project_file_cmd` / `delete_project_file_cmd` / `rename_project_file_cmd` / `read_project_file_content_cmd` 五条命令与对应 `/api/projects/{id}/files*` 路由已删除）。命令以 `{ scope: "session"|"project"|"project_folder"|"path", scopeId, ... }` 寻址，后端 `WorkspaceScope` 解析工作目录并做越界校验。`project_folder` 绑定基础 project/session scope、`linkedDirs` 索引和期望路径，并在每次请求时按 live Project 重新授权；目录移除、换序或项目失效后旧 scope fail closed，不构成任意绝对路径入口：

| Tauri 命令 | HTTP 路由 | 对齐 |
|---|---|---|
| `project_fs_list` | `GET /api/fs/list?scope=&scopeId=&path=` | ✅ |
| `project_fs_capabilities` | `GET /api/fs/capabilities?scope=&scopeId=` | ✅（最终写能力） |
| `project_fs_read_text` | `GET /api/fs/read?...` | ✅ |
| `project_fs_extract` | `GET /api/fs/extract?...` | ✅ (PDF/Office 提取预览) |
| —（HTTP-only raw ticket） | `POST /api/fs/raw-ticket` | N/A（Owner 保护；绑定单个 canonical file，`no-store`） |
| —（HTTP-only session raw ticket） | `POST /api/sessions/{id}/files/by-path-ticket` | N/A（会话路径授权后绑定单个 canonical file，`no-store`） |
| `project_fs_write_text` | `PUT /api/fs/file` | ✅（`expectedFileHash` / `createOnly` / 结构化冲突 + 写闸门） |
| `project_fs_delete` | `DELETE /api/fs/entry?...&recursive=` | ✅ (写闸门) |
| `project_fs_rename` | `POST /api/fs/rename` | ✅ (写闸门) |
| `project_fs_mkdir` | `POST /api/fs/mkdir` | ✅ (写闸门) |
| `project_fs_upload` | `POST /api/fs/upload` (multipart) | ✅（旧客户端兼容，静态 20 MiB） |
| `project_fs_claim_upload` | `POST /api/fs/upload-claim` | ✅（`workspace_upload` lease，最终 scope/大小复检 + 原子 publish） |
| `project_fs_resolve` | —（Tauri-only，图片预览 `convertFileSrc`） | N/A |
| —（HTTP-only raw serve） | `GET /api/fs/raw?...&download=`（同源 session）/ `GET /api/resource/{ticket}/fs/raw`（跨源单文件 capability） | N/A (`projectFsRawUrl` 专用方法) |
| `preview_read_text` | `GET /api/sessions/{id}/files/read?path=` | ✅ (preview-by-path，绝对路径，会话鉴权) |
| `preview_extract` | `GET /api/sessions/{id}/files/extract?path=` | ✅ (preview-by-path，绝对路径，会话鉴权) |

### Knowledge Base（知识空间）

**Owner / 管理面**——全局 API key 持有者 = owner-equivalent，看自己全部 KB，**不经 `effective_kb_access`**（那是 agent `note_*` 工具侧）。Knowledge Agent read token 只在 owner API key 保护开启时生效，只允许下表中的 `knowledge_agent_{search,read,expand,sources}_cmd` HTTP 路由，不能访问 compile/propose 或任何管理端点；若 server 处于 no-auth 模式，read token 不会单独启用鉴权。详见 [knowledge-base.md](../core/knowledge-base.md)（实现 + 设计契约 D1–D20）。

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
| `kb_list_tags_cmd` | `GET /api/knowledge/{kbId}/tags` | ✅ (面向用户本人，编辑器 `#tag` 补全词表) |
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
| `kb_maintenance_approve_cmd` | `POST /api/knowledge/maintenance/proposals/{id}/approve` | ✅ (批准并以用户本人身份落地，返回更新后的提案) |
| `kb_maintenance_reject_cmd` | `POST /api/knowledge/maintenance/proposals/{id}/reject` | ✅ (忽略单条提案) |
| `kb_maintenance_reject_all_cmd` | `POST /api/knowledge/{kbId}/maintenance/reject-all` | ✅ (清空某 KB 待审队列，返回清除数) |
| `kb_maintenance_config_get_cmd` | `GET /api/knowledge/maintenance/config` | ✅ (维护配置，GUI 面板；也可经 `get_settings(knowledge_maintenance)` 读) |
| `kb_maintenance_config_set_cmd` | `POST /api/knowledge/maintenance/config` | ✅ (写维护配置，clamp 后返回；emit `config:changed` 唤醒 cron loop) |
| `kb_passive_recall_config_get_cmd` | `GET /api/knowledge/passive-recall/config` | ✅ (读取桥③ 被动相关笔记配置，GUI 面板；也可经 `get_settings(knowledge_passive_recall)` 读) |
| `kb_passive_recall_config_set_cmd` | `POST /api/knowledge/passive-recall/config` | ✅ (写被动相关笔记配置，clamp 后返回) |
| `knowledge_media_retention_config_get_cmd` | `GET /api/knowledge/media-retention/config` | ✅ (读取原始媒体可选留存配置；默认关闭，HIGH/privacy，也可经 `get_settings(knowledge_media_retention)` 读) |
| `knowledge_media_retention_config_set_cmd` | `POST /api/knowledge/media-retention/config` | ✅ (写原始媒体可选留存配置，clamp 后返回；只影响未来 source 导入) |
| `knowledge_source_limits_config_get_cmd` | `GET /api/knowledge/source-limits/config` | ✅（文本/二进制/URL 三类来源 MiB 限制，MEDIUM） |
| `knowledge_source_limits_config_set_cmd` | `POST /api/knowledge/source-limits/config` | ✅（clamp 后保存并返回） |
| `knowledge_vision_config_get_cmd` | `GET /api/knowledge/vision/config` | ✅ (读取图片 OCR 模型链配置，GUI 面板；也可经 `get_settings(knowledge_vision)` 读) |
| `knowledge_vision_config_set_cmd` | `POST /api/knowledge/vision/config` | ✅ (写图片 OCR 模型链配置) |
| `note_tools_config_get_cmd` | `GET /api/knowledge/note-tools/config` | ✅ (读取笔记三件套共享模型链配置，GUI 面板；也可经 `get_settings(note_tools)` 读) |
| `note_tools_config_set_cmd` | `POST /api/knowledge/note-tools/config` | ✅ (写笔记三件套共享模型链配置) |
| `kb_sprite_observe_cmd` | `POST /api/knowledge/sprite/observe` | ✅ (精灵编辑空闲触发，fire-and-forget；节流 + side_query 后建议经 `sprite:suggestion` 事件返回) |
| `sprite_config_get_cmd` | `GET /api/knowledge/sprite/config` | ✅ (读精灵配置，GUI 面板；也可经 `get_settings(sprite)` 读) |
| `sprite_config_set_cmd` | `POST /api/knowledge/sprite/config` | ✅ (写精灵配置，clamp 后返回) |
| `kb_source_import_batch_cmd` | `POST /api/knowledge/{kbId}/sources/batch` | ✅（资料舱批量导入；本地文件输入使用 `uploadId`，与 `content` / `dataBase64` / `url` 互斥；创建 import run + item 后返回 `running` run） |
| `kb_source_import_session_attachment_cmd` | `POST /api/knowledge/{kbId}/sources/session-attachment` | ✅ (把已落到会话附件目录的聊天 / IM 附件归档为 raw source；后端校验 `sessionId + path` 位于该 session attachments dir，再复用文本 / PDF / DOCX / STT / OCR 导入链路) |
| `kb_source_asset_link_cmd` | `GET /api/knowledge/{kbId}/sources/{sourceId}/assets/{original\|thumbnail}/link` | ✅ (返回 retained source asset metadata + 用户本人可见的本地路径；文件流走同路径去掉 `/link`，可加 `?download=1`) |
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
| `kb_file_read_cmd` | `GET /api/knowledge/{kbId}/files/read?path=` | ✅ (仅面向用户本人 + scope contains) |
| `kb_file_extract_cmd` | `GET /api/knowledge/{kbId}/files/extract?path=` | ✅ |
| `kb_file_resolve_cmd` | —（Tauri-only，`convertFileSrc`） | N/A |
| —（HTTP-only raw serve） | `GET /api/knowledge/{kbId}/files/raw?path=&download=` | N/A |

KB 文件预览端点**仅面向用户本人，无 session 参数、无 owner fallback**——与 `/api/sessions/{id}/files/*` 物理隔离，不放宽其判定。外部绑定 vault 默认只读（写经 `WorkspaceScope::resolve_writable` 拒绝 + HTTP `allow_remote_writes` 闸门双拒）。agent 读笔记不经此端点，走 `note_*` 工具（`effective_kb_access` 校验）。`knowledge:changed` 事件 `{ kbId, op }` 经 EventBus fan-out 到两端前端。

**preview-by-path（文件操作统一）**：`preview_read_text` / `preview_extract` 按**绝对路径**读取，供 Markdown 链接 / 下挂文件 / 工作台产物文件统一预览。桌面信任本机路径直接读；HTTP 经 `/api/sessions/{id}/files/{read,extract}`，与既有 `/files/by-path` 共用授权 `authorized_canonical_file_path` = **被会话 tool 消息引用 ∪ 落在会话工作目录内**，二者皆非的主机任意路径一律 403。详见 [file-operations.md](../core/file-operations.md)。

写端点（write/delete/rename/mkdir/upload）在 HTTP handler 层读 `filesystem.allow_remote_writes`（默认 false）闸门，为 false 返 403；桌面 Tauri 不受限。`FilesystemConfig` 包含聊天附件、Workspace 上传、文本预览、文本编辑、文档预览五项 MiB 限制；`maxChatAttachmentMb` 同时约束用户聊天附件与 Agent `send_attachment`。配置读写：`get_filesystem_config` / `save_filesystem_config` / `patch_filesystem_config` ↔ `GET/PUT/PATCH /api/config/filesystem`；设置面使用 PATCH，避免不同风险面的字段互相覆盖。完整默认值与范围见 [file-operations.md](../core/file-operations.md#大小配置与硬上限)。

`Project` 支持 `workingDir: string | null` 与 `linkedDirs: string[]`。设置 UI 将它们统一呈现为“源文件夹”：`workingDir` 是主文件夹，决定 cwd、相对路径与根 `AGENTS.md`；`linkedDirs` 是最多 32 个 canonical 辅助根，可供 Agent 和文件浏览器搜索、读取与编辑。将辅助目录设为主目录时，客户端一次 PATCH 同时交换两字段。运行时主目录合并优先级 `session.working_dir > project 显式 working_dir > 默认 workspace`，lazy ensure 创建——编辑项目工作目录后未单独设置的已有会话立即跟随。详见 [`AGENTS.md`](../../../AGENTS.md) 「项目（Project）容器」段与 [project.md](../core/project.md)。

**Project ↔ IM Channel 反向认领已废弃**。`Project.boundChannel` / `BoundChannel` 类型 + `projects.bound_channel_id` / `bound_channel_account_id` DB 列 + `idx_projects_bound_channel` 索引 + `find_by_bound_channel` API 全部删除；`UpdateProjectInput` 不再有 `boundChannel` 字段。IM 入站消息不再自动归属项目，新会话以 `project_id = NULL` 创建。要把会话归项目，从 IM chat 内 `/project <id>` 显式触发：handler 检测 `session.channel_info` 后发 `AssignProject` action，channel worker 调 `SessionDB::set_session_project` 直接 UPDATE 现有 `sessions.project_id`，**不创建新 session**。详见 [im-channel.md](../integration/im-channel.md) 「Session 路由」章节。

### Sessions

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_sessions_cmd` | `GET /api/sessions?agentId=&projectId=&unassigned=&parentSession=&pinned=&limit=&offset=&activeSessionId=` | ✅（`parentSession=true/false` 与 `pinned=true/false` 均在分页前筛选；置顶分组用 `pinned=true` 跨项目读取） |
| `list_archived_sessions_cmd` | `GET /api/sessions/archived?limit=&offset=` | ✅（跨普通 / 项目 / IM / Subagent / Cron / Knowledge / Design 的归档管理列表） |
| `create_session_cmd` | `POST /api/sessions` | ✅ |
| `get_session_cmd` | `GET /api/sessions/{id}` | ✅ |
| `fork_session_cmd` | `POST /api/sessions/{sessionId}/fork` | ✅（body 的 `messageId` 为含边界；`beforeMessageId` 为不含边界，二者互斥且同时传入返回 400；响应保持 `SessionMeta` 扁平字段，并可附带 `draftAttachmentsMeta`） |
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
| `get_session_stream_state` | `GET /api/sessions/{sessionId}/stream-state` | ✅（返回流状态与 `admissionActive`；后者表示 exact Turn admission 尚未释放） |
| `set_session_archived_cmd` | `PATCH /api/sessions/{sessionId}/archived` | ✅（body `{ archived: boolean }`；归档 / 恢复，不删除消息） |
| `delete_session_cmd` | `DELETE /api/sessions/{sessionId}` | ✅（永久删除；同时清理独立 `cron.db` 中引用该会话的 run log） |
| `rename_session_cmd` | `PATCH /api/sessions/{sessionId}` | ✅ |
| `mark_session_read_cmd` | `POST /api/sessions/{sessionId}/read` | ✅ 可选 body `{throughMessageId}`；阅读面按已渲染上限推进，省略表示显式全部已读 |
| `mark_session_read_batch_cmd` | `POST /api/sessions/read-batch` | ✅ |
| `mark_all_sessions_read_cmd` | `POST /api/sessions/read-all` | ✅（仅普通顶层会话，不清 Cron / IM / Knowledge / Subagent / incognito） |
| `regular_unread_total_cmd` | `GET /api/sessions/unread?activeSessionId=` | ✅（全库普通未读 session 数） |
| `next_unread_session_cmd` | `GET /api/sessions/unread/next?activeSessionId=` | ✅（侧边栏视觉顺序中的首个普通未读 session + projectId + listOffset） |
| `compact_context_now` | `POST /api/sessions/{sessionId}/compact` | ✅ |
| `export_session_cmd` | `GET /api/sessions/{sessionId}/export` | ✅ |
| `write_export_file` | `POST /api/misc/write-export-file` | ✅ |
| `get_dangerous_mode_status` | `GET /api/security/dangerous-status` | ✅ |
| `set_dangerous_skip_all_approvals` | `POST /api/security/dangerous-skip-all-approvals` | ✅ |

`create_session_cmd` 与 `chat` 在自动创建新会话时都支持可选 `incognito: boolean`，返回的 `SessionMeta` 也会包含 `incognito` 字段；主聊天 UI 将 incognito 视为“新会话预设”，只在尚未 materialize session 的草稿态提供入口，已有会话不再暴露切换按钮。`set_session_incognito` 保留给兼容调用和非主 UI 适配，但不应作为常规会话内开关使用。当请求同时带了 `project_id` 时 `incognito` 被强制为 `false`（互斥）。`list_sessions_cmd` / `search_sessions_cmd` / `list_project_sessions_cmd` 接受可选 `active_session_id` 参数：默认会过滤掉所有 incognito 会话，`active_session_id` 让正在打开的那个无痕会话仍出现在 sidebar / 搜索结果里。`purge_session_if_incognito` 在前端 `handleSwitchSession / handleNewChat / handleNewChatInProject` 切走当前 session 之前调用，仅当目标 session 当前为 incognito 时硬删，否则 no-op。

普通删除入口采用“先归档”产品语义：归档会话从活跃列表、全局搜索和未读聚合消失，但 transcript、项目 / Agent 归属及 Knowledge / Design 等专属绑定仍保留；设置中的“已归档对话”可恢复。`delete_session_cmd` 仍是永久删除 API，仅在归档管理页通过二次确认暴露。无痕会话拒绝归档。

未读产品口径为 session 数：普通域仅包含 `kind=regular`、顶层、非 Cron、非 incognito、无 IM 绑定的会话（项目会话包含）。只有聊天主视图已选中、应用窗口聚焦、document 可见且消息列表停在最新位置时，当前 session 才按已读显示并推进水位线；组件仍挂载或仅持有 `currentSessionId` 不代表用户正在阅读。`regular_unread_total_cmd` 是对话入口、Dock 和状态栏的单一聚合来源，不得用当前分页列表求和；再次点击已经激活的“对话”入口时，用 `next_unread_session_cmd` 定位侧边栏视觉顺序中的首个未读会话，按返回的 `projectId + listOffset` 一次加载足够的前缀并滚动到目标行。会话行只显示点，项目与全局入口显示数量。

`update_session_agent_cmd` 接受 `{ agentId: string }`，后端在 SQL 层校验 `messages` 表中该 session 没有 `role IN ('user','assistant')` 的记录，否则返回 400。前端 `ChatTitleBar` 的 `AgentSwitcher` dropdown 在 `messages.length > 0` 时会把触发器降级为只读 `<span>`，作为 UX 防御层。

`set_session_model` 接受 `{ providerId, modelId }`，把模型固定到当前会话（写 `sessions.provider_id` / `provider_name` / `model_id`），不写 `AppConfig.active_model`——v0.2.1 起这是「会话内切模型」的唯一合法入口。`get_active_model` / `POST /api/models/active` 仍然存在，但**只该被 Settings 「模型」面板 / onboarding wizard / 本地 LLM 安装路径**调用，用来修改应用全局默认；任何 chat 内或 IM 内的"切模型"语义都必须落到 session 级。chat_engine 解析优先级 `plan_model > 本轮 model_override > sessions.provider_id > agent.model.primary > AppConfig.active_model`（详见 [`provider-system.md` § 7.2](../core/provider-system.md#72-模型链解析)）。写入后 emit `session:model_updated`，桌面 GUI 仅在 `sessionId == currentSessionId` 时同步 ModelPicker。

`get_execution_mode` / `set_execution_mode` 是会话级执行模式入口，对应 `/mode off|guarded|deep|autonomous` 与 Workspace/Workflow 面板中的 Execution Mode 控件，写 `sessions.execution_mode`。该值会在下一轮以 trusted run instruction 提供给模型，控制长任务的观察、计划、验证、修复和停止策略，但不改变稳定 system 前缀；它不是 `/loop`，也不负责定时、重复触发或条件轮询。

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

Managed Worktree owner API 管理 durable git worktree：`create_managed_worktree` 拒绝 incognito，且只接受 `manual/workflow/subagent` purpose（Scheduled provenance 只由 kernel typed API 铸造）；默认在 `~/.hope-agent/worktrees/<repo-slug>/<wt-id>` 建 detached worktree（可由 `WorktreeCreate` hook 接管）。返回 DTO 可含 session/task owner、runtime 与 handoff 绑定；`archive` 记 dirty snapshot且 clean 才 best-effort remove，`restore` 重建已清理路径，`handoff` 只绑父 session cwd 不复制 Git 改动；`chat` / `POST /api/chat` 新项目草稿可带 `projectBootstrap`，配套查询 / 取消接口用于断线恢复。完整契约见 [Managed Worktree 控制平面](../agent/worktree.md)。

HTTP Managed Worktree 的 create/archive/restore/handoff 与 `POST /api/chat` 的 `projectBootstrap` 都可能写宿主 Git，统一受默认关闭的 `filesystem.allow_remote_writes` 闸门；普通聊天、读取及 Bootstrap 状态查询/取消不受影响。

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

所有端点只按 session 解析 cwd、不接受客户端指定仓库根，HTTP 写端点受 `filesystem.allow_remote_writes` 闸门；PR 读取通过已认证的本机 `gh`（不接受客户端传 PR 标识），外部文本按不可信数据处理，“修复”只填入输入框不自动发送，自动合并须带 revision / 合并方式 / 显式确认并纳入 `requestId` 幂等、冲突时拒绝。完整 DTO、锁、幂等、Handoff 与失败恢复契约见 [Session Git 控制平面](../agent/git-control.md)。

### LSP / Diagnostics

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_lsp_status` | `GET /api/sessions/{sessionId}/lsp/status` | ✅ |
| `get_lsp_diagnostics` | `GET /api/sessions/{sessionId}/lsp/diagnostics` | ✅ |

LSP owner API 返回当前 session working dir 对应 workspace 的 language server 状态与 diagnostics 快照，只服务 Workspace GUI / HTTP client 读取（agent 侧语义导航走 builtin `lsp` 工具）；无痕会话不启动 LSP、不注入 diagnostics prompt 后缀。完整契约见 [LSP 与语义代码智能](../agent/lsp.md)。

### 上下文检索

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_context_retrieval` | `GET /api/sessions/{sessionId}/context-retrieval?query=&limit=&domain=&templateId=` | ✅ |
| `get_session_ide_context` | `GET /api/sessions/{sessionId}/ide-context` | ✅ |
| `save_session_ide_context` | `PUT /api/sessions/{sessionId}/ide-context` | ✅ |
| `clear_session_ide_context` | `DELETE /api/sessions/{sessionId}/ide-context` | ✅ |

Context Retrieval owner API 返回当前 session 的任务感知推荐上下文（聚合 Git diff、artifacts、LSP diagnostics、Review / Verification / Goal / Task / Workflow 信号、文件搜索与 URL 来源，按信号强度 + query boost 排序），可选 `domain/templateId` 启用领域候选；查询本身只读，无工作目录时只跳过 workspace 信号、无痕会话返回空 snapshot。`session_ide_context` owner API 管理当前 session 的 IDE / ACP 快照（current file / selection / open tabs / active diagnostic / active symbol），只作为推荐和 evidence 信号、不进 system prompt，无痕会话拒绝持久化。完整契约见 [上下文检索](../agent/context-retrieval.md)。

### Review Engine

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_review_runs` | `GET /api/sessions/{sessionId}/review-runs` | ✅ |
| `run_code_review` | `POST /api/sessions/{sessionId}/review-runs` | ✅ |
| `get_review_run` | `GET /api/review-runs/{runId}` | ✅ |
| `update_review_finding_status` | `POST /api/review-findings/{findingId}/status` | ✅ |

Review owner API 管理 durable 本地代码复查：`run_code_review` 按 `profiles[]` 复查当前 session workspace 的 uncommitted diff，经 verifier 三态落 `review_findings` 并把 P0/P1 open finding 写回 Goal evidence，可带 `focusPaths[]` / `ideContext`；无痕会话不创建 durable run。完整契约见 [Review Engine 控制平面](../agent/review-engine.md)。

### Smart Verification

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_verification_runs` | `GET /api/sessions/{sessionId}/verification-runs` | ✅ |
| `plan_smart_verification` | `POST /api/sessions/{sessionId}/verification-runs/plan` | ✅ |
| `run_smart_verification` | `POST /api/sessions/{sessionId}/verification-runs/run` | ✅ |
| `get_verification_run` | `GET /api/verification-runs/{runId}` | ✅ |

Smart Verification owner API 管理 durable 验证 run：`plan_smart_verification` 只持久化推荐命令，`run_smart_verification` 后台执行低风险 auto-run steps 并把 `validation_passed` / `validation_failed` / `validation_completed` 写回 Goal evidence，可带 `focusPaths[]`；无痕会话不创建 durable run。完整契约见 [Smart Verification 控制平面](../agent/verification-engine.md)。

### Domain Quality

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_domain_quality_runs` | `GET /api/sessions/{sessionId}/domain-quality-runs` | ✅ |
| `run_domain_quality` | `POST /api/domain-quality-runs/run` | ✅ |
| `get_domain_quality_run` | `GET /api/domain-quality-runs/{runId}` | ✅ |

Domain Quality owner API 管理 durable 非编码复查 / 验证 run：`run_domain_quality` 基于 Domain Workflow template、domain evidence 和 approval gates 同步生成 `domain_quality_runs/checks/events` 并回写 Goal evidence，可显式带 `templateId/templateVersion` 或 `domain`；高风险动作命中 approval gate 或 `highRiskAction=true` 时要求 `explicitUserApproval`，缺失则进入 `needs_user` 阻塞 Goal；无痕会话拒绝持久化。完整契约见 [Domain Quality 控制平面](../agent/domain-quality.md)。

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

Domain Eval owner API 管理非编码 eval / gate：task 列表（内置 15 个 Research / Writing / Data Analysis / Meeting Prep / Knowledge Curation task + 导入 task）、`trace_fixture` 与 `agent` 两种 fixture 执行、durable campaign（可取消 / retry）、leaderboard，以及 `evaluate_domain_quality_gate` / `evaluate_domain_readiness_gate` 三态（`passed` / `failed` / `insufficient_data`）只读守门。它与 coding benchmark 分表、分路径、分 Dashboard 区块，只做确定性只读打分、不自动生成 proposal / retry campaign，campaign history 不保存 provider secret，无痕会话 fail-closed。完整契约见 [Domain Eval 与 Quality Gate 控制平面](../agent/domain-eval.md)。

### Coding Eval

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `run_coding_task_eval_fixture` | `POST /api/coding-eval/task-fixtures/run` | ✅ |
| `list_coding_eval_gold_tasks` | `GET /api/coding-eval/gold-tasks` | ✅ |
| `run_coding_eval_gold_task_pack` | `POST /api/coding-eval/gold-tasks/run` | ✅ |
| `evaluate_coding_eval_strategy_effect` | `POST /api/coding-eval/strategy-effects/evaluate` | ✅ |

Coding Eval owner API 在临时 git repo + 真实 session / goal / task / workflow seed 上运行完整 fixture JSON：`mode="agent"` 按 fixture 的 `providers` / `modelChain` 经 `run_chat_engine` 让模型产出 candidate diff，`mode="fixture_patch"` 用于无模型回归，随后调用生产 Review / Smart Verification / Context Retrieval 做 task-level 打分并可写 `coding_eval_runs`。Gold Task Pack API（`list_coding_eval_gold_tasks` / `run_coding_eval_gold_task_pack`）批量 materialize 内置 gold task，Strategy Effect API（`evaluate_coding_eval_strategy_effect`）纯函数比较两份 pack 报告的共同 case、不跑模型、不执行命令。外部真实模型基线必须显式标 `baselineKind="external_model"` 且带 agent 执行配置，不能只改标签。完整契约见 [Coding Eval 控制面评测](../agent/coding-eval.md)。

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

Coding Improvement owner API 基于 durable Goal / Workflow / Review / Verification / Coding Eval / transcript 生成 trend report、failure taxonomy 和 proposal 队列：`generate_*` / `distill_*` 只写 `status='draft'` proposal，`apply_*` 原子 claim 后仅落 reviewable draft artifact 或 managed draft skill、不直改生产 guidance / AGENTS / memory / eval fixture，`promote_*` 仅对已应用草稿显式晋升（目标冲突 fail-closed）。Release / Generalization Gate、Benchmark Center、Campaign / Leaderboard / Corpus / Report / Continuous Gate / Backlog 一组 API 只读打分与守门，不自动 apply / promotion，history 不保存 provider secret，无痕会话 fail-closed。完整契约见 [Coding Improvement Loop](../agent/coding-improvement-loop.md)。

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

Workflow Mode 是 session 级能力开关：开启后模型才在后续回合看到 `workflow_run` 工具。Workflow owner API 管理 durable `workflow_runs`：`preview_workflow_script` 不落库只返回 Script Gate + permission preview，`create_workflow_run` 复用同一 preflight（Gate 不通过或有确定 deny 即拒创建），可绑定 `worktreeId` / `goalCriterionId`；`run` / `approve` / `resume` 均要求 primary launcher，返回值只表示 launch accepted、真实进度以 `workflow:*` 事件为准，`cancel` 先转 `cancelled` 再 best-effort 取消子 run；watchdog 只读诊断不恢复、不执行脚本。完整契约见 [Workflow Mode、Workflow Run 与 Execution Mode](../agent/workflow.md)。

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

Goal owner API 管理 session-scoped 顶层目标：`create_goal` 拒绝 incognito 并保证同一 session 只有一个 open / pending-closure Goal，`update_goal` 改 objective / criteria 后清空旧 final audit，`append_goal_follow_up` 写非阻塞后续项、拒绝 sealed 终态，`evaluate_goal` 基于 linked runs / tasks / evidence / budget 生成 deterministic final audit，`close_goal` / `clear_goal` 记录 closure decision（`clear_goal` 走 `cancelled`），watchdog 只读诊断不恢复。`create_workflow_run` / `create_loop_schedule` 可带 `goalId` / `goalCriterionId` 绑定当前 Goal 的具体完成标准并把 criteria 快照写进 evidence metadata，创建前执行 Goal budget hard stop。完整契约见 [Goal 控制平面](../agent/goal.md)。

### Domain Workflow

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_domain_workflow_templates` | `POST /api/domain-workflows/templates` | ✅ |
| `save_domain_workflow_template` | `POST /api/domain-workflows/templates/save` | ✅ |
| `preview_domain_workflow` | `POST /api/domain-workflows/preview` | ✅ |
| `record_domain_evidence` | `POST /api/domain-evidence/record` | ✅ |
| `list_domain_evidence` | `POST /api/domain-evidence` | ✅ |

Domain Workflow owner API 是通用（非编码）场景入口：`list_domain_workflow_templates` 合并内置（Research / Writing / Data Analysis / Meeting Prep / Knowledge Curation / Inbox / Project Ops）与自定义模板，`save_domain_workflow_template` 要求 `explicitSaveConsent=true` 且禁改 built-in，`preview_domain_workflow` 只从模板出 draft（走 Script Gate / permission preview）、不建 run 不执行，`record_domain_evidence` / `list_domain_evidence` 写 / 查通用 evidence 并可链回 Goal（成功后 emit `domain_evidence:recorded`）。无痕会话 fail-closed。完整契约见 [Domain Workflow 控制平面](../agent/domain-workflow.md)。

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

Loop owner API 管理 session-scoped 重复触发器：`create_loop_schedule` 拒绝 incognito，要求绑定 open / pending-closure Goal 或明确 recurring prompt，可选 `goalCriterionId` / `executionStrategy`（默认 `continue`，`workflow` 时每次 tick 建并启动 `origin=loop:<id>` 的 durable WorkflowRun）；`maxNoProgressRuns` / `maxFailures` / `backoffSecs` 默认 3 / 3 / 300s，并基于 Goal evidence delta 做 backoff / blocked。三种 `triggerKind`：`interval` 定时、`dynamic`（prompt-only `/loop <prompt>` 与裸 `/loop` maintenance，模型经 `loop_reschedule` / `loop_stop` 自定节奏、缺决策只 fallback 一次）、`event`（订阅内部 EventBus 白名单事件，`triggerSpec={ eventName, filters, debounceSecs }`，经 Cron immediate primary-only path 执行）。派生返回 Cron `nextRunAt` / `cronStatus`，watchdog 只读诊断不恢复，`pause` / `resume` / `stop` 同步底层 Cron job；模型侧 `loop_*` 工具只操作当前 session Loop、不开放 `manage_cron` 写权限。事件白名单及各自 filter 字段的权威清单见 [Loop 控制平面](../agent/loop.md)。

`export_session_cmd` / `GET /api/sessions/{sessionId}/export` 是两端**形态不对称**的特例：Tauri 端走 IPC，由前端先弹原生 save dialog 拿到 `output_path` 再传进来，后端写盘后返回最终路径字符串；HTTP 端走 GET 直接返回二进制流（`Content-Type` + `Content-Disposition: attachment; filename*=UTF-8''<percent>`），浏览器用 `URL.createObjectURL` + `<a download>` 触发下载。两端共用 [`ha_core::session::export::export_session`](../../../crates/ha-core/src/session/export.rs) 序列化器，Query 参数 `format ∈ {md,json,html}` / `includeThinking` / `includeTools` 与 Tauri 命令的字段一一对应。前端 Transport 抽象 [`exportSession`](../../../src/lib/transport.ts) 是这一对端点的统一入口，调用方不需要分支。

`set_session_working_dir` 接受 `{ workingDir: string | null }`，后端 `canonicalize` 路径并校验是否为存在的目录，返回 `{ updated: true, workingDir: <canonical> }`；`null` 或空串清除选择。该字段以 `SessionMeta.workingDir` 呈现，被 `system_prompt::build` 注入到 "# Working Directory" 段落（位于 Project / Project Files 之后、Memory 之前）。执行层也会把它作为 path-aware 工具的默认根：`read` / `write` / `edit` / `ls` / `grep` / `find` / `apply_patch` 的相对路径，以及 `exec.cwd` 的相对路径，均按「显式绝对路径 > Session working dir > Agent home」解析；`exec` 无 `cwd` 时再回退到用户 home。与 Project / Incognito 正交：三者可同时启用。在 HTTP 模式下前端没有原生目录选择器，改走 `GET /api/filesystem/list-dir`（见 Filesystem 域）的服务端目录浏览器。

新会话尚未 materialize 时也允许选目录：前端把选择存为 `draftWorkingDir`，首条消息发送时通过 `chat` 命令的可选 `workingDir` 字段（Tauri / `POST /api/chat` 同名）随请求带过去；后端只在自动创建 session 的分支应用，复用 `update_session_working_dir` 的 canonicalize + `is_dir` 校验，无效路径直接 400。已有 sessionId 的 `chat` 调用会忽略此字段，避免覆盖现成的工作目录设置。

**项目会话懒创建**：进项目「新建对话」不再预先 `create_session_cmd` 落库，而是停在草稿态（`currentSessionId=null`，前端记 `draftProjectId`），与普通对话对称。首条消息发送时通过 `chat` 命令的可选 `projectId` 字段（Tauri / `POST /api/chat` 同名 camelCase）把项目绑定带过去；后端只在自动创建 session 的分支用它 `create_session_with_project(agent, project_id, …)`，并在 `agent_id` 缺省时按 `project.default_agent_id` 解析 agent（对齐 `create_session_cmd`）。已有 sessionId 的调用忽略此字段；`project_id` 与 `incognito` 互斥（后端强制 incognito off）。好处：进项目不再产生未发消息的空会话行，且草稿态走与普通对话相同的模型 / 权限模式 seeding。

`chat` 命令还有两个知识空间侧边栏对话用的可选字段（Tauri / `POST /api/chat` 同名 camelCase）：`toolScope: "knowledge"` 把本轮注入工具集收窄到笔记 / 检索 / 记忆白名单（与 source / `effective_kb_access` 正交，只动 schema 可见性）；`kbAnchorNote` 仅在自动创建 session 的分支生效——配合单条 `kbAttachments`(write) 把新会话提升为 `kind=knowledge` 的对话线程并锚定该笔记。已有 sessionId 的调用忽略 `kbAnchorNote`。

### Chat

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `chat` | Bundled Transport：`POST /api/chat/ui`（浏览器来源校验；非 incognito 返回 202 ACK 后服务端托管）；公共 owner API：`POST /api/chat`（忽略 `uiSurface`、同步完成）；流式输出均经 `/ws/events` | ✅（已有会话可带 `editMessageId`，仅允许最后一条非排队 user 且旧 turn 已终止；旧分支回退、replacement user 落库、新 turn 登记同一事务提交并保留 Bundled UI surface） |
| — | `GET /api/chat/turns/{turnId}` | ✅ 查询精确 ChatTurn 终态，供 202 ACK 后断线/竞态恢复 |
| `queue_turn_user_message` | `POST /api/chat/turn-message` | ✅ 持久入队，附件在入队时转 session-owned 引用 |
| `list_queued_turn_user_messages` | `GET /api/chat/turn-message/{sessionId}` | ✅ UI/恢复单一查询入口；`canForceInsert` 由后端按当前状态与完整 turn sidecar 边界权威投影，客户端不得自行猜测；backend-owned IM 行带 `managedBy: "channel"`，可展示但不可由客户端 edit/delete/insert/claim |
| `update_queued_turn_user_message` | `PATCH /api/chat/turn-message` | ✅ CAS 拒绝 inserting/dispatching |
| `delete_queued_turn_user_message` | `DELETE /api/chat/turn-message/{sessionId}/{requestId}` | ✅ CAS 拒绝 inserting/dispatching |
| `insert_queued_turn_user_message` | `POST /api/chat/turn-message/insert` | ✅ 绑定活跃 turn 的工具边界 |
| `cancel_queued_turn_user_message` | `POST /api/chat/turn-message/cancel` | ✅ 仅 waiting_tool_boundary 可撤销 |
| `control_model_recovery` | `POST /api/chat/recovery/control` | ✅ 精确匹配 `sessionId + recoveryId`；`action=skip_wait\|switch_model` |
| `stop_chat` | `POST /api/chat/stop` | ✅ |
| `continue_chat` | `POST /api/chat/continue` | ✅ 精确匹配 `sessionId + pauseId`，迟到请求不得消费更新一代 Stop receipt |
| `set_permission_mode` | `POST /api/chat/permission-mode` | ✅ 替代旧 `set_tool_permission_mode` |
| `respond_to_approval` | `POST /api/chat/approval` | ✅ |
| `save_attachment` | `POST /api/chat/attachment` | ✅ (multipart) |
| `stage_chat_attachment` | `POST /api/chat/attachment-stage` | ✅（旧 multipart 兼容，静态 20 MiB） |
| `discard_chat_attachment_upload` | `DELETE /api/chat/attachment-stage/{uploadId}` | ✅ |
| `list_builtin_tools` | `GET /api/chat/tools` | ✅ |
| `list_capability_mentions` | `GET /api/chat/capability-mentions?agentId=` | ✅（仅返回已注册 Plugin/Connector 的有界非敏感选择器元数据；不连接远端、不授权调用或外发） |
| `list_session_tasks` | `GET /api/sessions/{sessionId}/tasks` | ✅ TaskProgressPanel 用户控件 |
| `create_session_task` | `POST /api/sessions/{sessionId}/tasks` | ✅ Workspace Context 候选转任务 |
| `update_task_status` | `PATCH /api/tasks/{id}/status` | ✅ TaskProgressPanel 用户控件 |
| `delete_task` | `DELETE /api/tasks/{id}` | ✅ TaskProgressPanel 用户控件 |

`chat` 的可选 `clientRequestId` 是前端生成的不透明请求 id。Bundled HTTP UI 把它和 payload
指纹随 `chat_turn` 持久化（与 user message 同一 SQLite 事务），进程内 registry 只合并尚未提交的
并发 waiter：相同 id + 相同 payload 即使服务重启或 registry 淘汰也返回原 `sessionId/turnId`，
不同 payload 复用返回 409，避免 ACK 丢失造成重复消息。它同时用于主动停止定位：在懒创建会话的
`session_created` 尚未到达前，`stop_chat` / `POST /api/chat/stop` 可传
`{ clientRequestId }` 精确取消该请求，不会误停其他会话。已知会话时依旧传
`{ sessionId, turnId?, clientRequestId? }`：`turnId` 尚未发布时由 `clientRequestId`
封住 active-turn 注册前的竞态，已有 `turnId` 后以精确 turn 为准；session 与 request
绑定不一致时不得跨会话取消。`sessionId` 与 `clientRequestId` 都缺失才表示全局停止。

三种入口在定位 session 后共用 core Stop 编排：先同步发出 cancel/`cancelling`/watchdog，再以
有界等待并行收敛 DB、审批、`ask_user` 与 session-owned runtime；响应超时不取消已经启动的
后台清理。全局停止同样走 core `stop_all_sessions`，HTTP / Tauri 只先翻转 transport-local
cancel handle。精确 `turnId` 不匹配时 fail closed，不得误停同 session 的新回合。

`stop_chat` 与 `POST /api/chat/stop` 返回**同一个** `ha_core::chat_engine::stop::StopChatResult`
（camelCase），两个适配器不得各自造形状：

```ts
{
  stopped: boolean
  scope: "request" | "session" | "all"
  reason?: string | null
  turnMismatch: boolean          // 精确 turn 打到了另一个在跑的 turn
  activeTurnFound: boolean       // 后端 registry 当时确实有前台 turn
  completionSealed: boolean      // executor 已越过取消点，终态事件仍在路上
  terminalEventPending: boolean  // 本次调用武装了 cancelling 广播 + watchdog
  latched: boolean               // 预注册闩锁吃下了本次 Stop，turn 稍后才注册
  runtimeCancellations: CancelRuntimeTaskResult[]
  runtimeCancellationError?: string | null
  autonomyPaused: boolean
  autonomyPause?: SessionAutonomyPause | null
  autonomyPauseError?: string | null
  stoppedSessionCount?: number   // 仅全局停止
}
```

**Stop 只回答「这次调用做了什么」，绝不回答「会话现在什么状态」**——后者恒由
`get_session_stream_state` 唯一负责，否则两个真相源必然漂移。`latched` /
`completionSealed` / `terminalEventPending` **三者全为 false 即证明不会再有任何终态事件**
（例如请求的 turn 已是 `interrupted / crash_recovery`，或 session-only Stop 时根本没有活跃
turn），调用方必须据此自行收敛本地活动状态，不得继续空等（见
[chat-engine](../core/chat-engine.md)）。

`chat` / `/api/chat` 的可选 `incomingTurn` 是 typed composer sidecar，当前契约为 `promptContractVersion=3`、`mentionWireVersion=1`。它必须携带 canonical user text、SHA-256 digest、UTF-8 source anchor 与 `file|plan|note|skill|plugin|connector|agent` binding；后端逐项校验正文 token 与来源，typed 请求不回退为字符串猜测。`plan` 只由 Plans 页的 first-party 引用动作产生：服务端独立重解析 registry path、核对附件 canonical path 并 open-once 冻结。没有 sidecar 的普通或粘贴 `@...`（包括 `@plan:`）不产生引用，`[[note]]` 只作为单独登记的只读兼容语法。File/Plan binding 必须在 direct/queue 持久化前分别与 `source=mention|plan_mention` 的附件按 unique target 精确对齐（重复相同 target 只需一个附件）；typed attachment 禁止预装 `data`、upload/quote metadata，且 name/MIME/path 受有界元数据校验。额外、缺失或无 sidecar 的 typed source 直接返回 400/IPC error。队列入口把 `incomingTurn` 与显式 Skill 的 `skillAllowedTools` 一起持久化；编辑排队消息会在同一 DB 事务清除旧 binding/ceiling 和 `mention|plan_mention` 附件、保留普通 upload/quote，避免把旧 provenance 套到新正文或留下无法派发的无 sidecar typed source。

#### Chat `attachments` wire format

`chat` / `POST /api/chat` 与 `queue_turn_user_message` / `POST /api/chat/turn-message` 共用同一份 `attachments` 数组；Tauri IPC 和 HTTP 都按 snake_case 原样序列化。每项的基础字段为 `{ name, mime_type, source?, upload_id?, data?, file_path? }`。GUI 新客户端点击发送后统一经通用 `chat_attachment` 分块 lease，再只传 `upload_id`；`upload_id` 与 `data` / `file_path` 互斥。图片不再由 GUI 转 Base64。旧字段保留给 ACP、IM、历史客户端与历史消息，旧 stage/Base64 固定 20 MiB；HTTP 旧 `file_path` 必须 canonicalize 后位于对应 session 或 `_temp` 附件目录。单文件动态上限取当前后端 `filesystem.maxChatAttachmentMb`（默认 20 MiB），单消息最多 64 个，前后端双重校验；claim 采用全量 prepare + rollback，部分失败不会消费其他 lease。

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

文件浏览器的 `source: "quote"` 除 `file_path` / `quote_lines` 外，可带 `quote_revealable`、`quote_project_root: { index, path }` 与 `quote_worktree_root`。`quote_revealable=false` 表示视觉／合成来源不能在文件浏览器重新打开；缺失时保持旧客户端的默认可揭示语义。后两者只作为编辑、分叉、重发时恢复源文件夹及 Git worktree 的持久 provenance；实际打开文件仍由当前 Project 行和后端 scope 校验，这些字段不扩大文件权限。历史元数据分别保存为 `revealable`、`project_root` / `worktree_root`。

### macOS Control

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `mac_control_status` | `GET /api/mac-control/status` | ✅ |
| `mac_control_permissions` | `GET /api/mac-control/permissions` | ✅ |
| `mac_control_snapshot` | `POST /api/mac-control/snapshot` | ✅ |
| `mac_control_capture_frame` | `POST /api/mac-control/capture-frame`（可带 `{ displayId? }` 指定捕获显示器） | ✅ |
| `mac_control_list_displays` | `GET /api/mac-control/displays`（面板快捷条显示器下拉；server 模式返回空 + error） | ✅ |
| `tool_recent_actions` | `GET /api/tool-actions?source=&sessionId=&limit=`（浏览器 / mac control 面板执行历史，读内存 ring buffer） | ✅ |

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
| `test_model` | `POST /api/providers/test-model` | ✅ |
| `test_proxy` | `POST /api/config/proxy/test` | ✅ |
| `has_providers` | `GET /api/providers/has-any` | ✅ |
| `get_system_timezone` | `GET /api/system/timezone` | ✅ |
| `get_toolchain_doctor_report` | `GET /api/system/toolchain-doctor` | ✅；只读固定探针，返回 `detected/supported/degraded/blocked`、版本与脱敏诊断代码，不安装/升级/启动/改配置 |
| `check_auth_status` | `GET /api/auth/codex/status` | ✅ |
| `logout_codex` | `POST /api/auth/codex/logout` | ✅ |
| `try_restore_session` | `POST /api/auth/session/restore` | ✅ |
| `list_canvas_projects` | `GET /api/canvas/projects` | ✅ |
| `get_canvas_project` | `GET /api/canvas/projects/{projectId}` | ✅ |
| `delete_canvas_project` | `DELETE /api/canvas/projects/{projectId}` | ✅ |

### Speech-to-Text

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_stt_providers` | `GET /api/stt/providers` | ✅（HTTP 响应脱敏） |
| `add_stt_provider` | `POST /api/stt/providers` | ✅ |
| `update_stt_provider` | `PUT /api/stt/providers/{providerId}` | ✅ |
| `delete_stt_provider` | `DELETE /api/stt/providers/{providerId}` | ✅ |
| `reorder_stt_providers` | `POST /api/stt/providers/reorder` | ✅ |
| `get_active_stt_model` | `GET /api/stt/active-model` | ✅ |
| `set_active_stt_model` | `PUT /api/stt/active-model` | ✅ |
| `clear_active_stt_model` | `DELETE /api/stt/active-model` | ✅ |
| `get_stt_fallback_models` | `GET /api/stt/fallback-models` | ✅ |
| `set_stt_fallback_models` | `PUT /api/stt/fallback-models` | ✅ |
| `get_im_fallback_stt_model` | `GET /api/stt/im-fallback-model` | ✅ |
| `set_im_fallback_stt_model` | `PUT /api/stt/im-fallback-model` | ✅ |
| `get_stt_default_options` | `GET /api/stt/default-options` | ✅ |
| `set_stt_default_options` | `PUT /api/stt/default-options` | ✅ |
| `list_known_local_stt_backends` | `GET /api/stt/local-backends` | ✅ |
| `probe_local_stt_backend` | `GET /api/stt/local-backends/{key}/probe` | ✅ |
| `upsert_known_local_stt_provider_cmd` | `POST /api/stt/local-backends/{backendKey}/upsert` | ✅ |
| `stt_transcribe_blob` | `POST /api/stt/transcribe` | ✅ |
| `stt_start_session` | `POST /api/stt/sessions` | ✅ |
| `stt_push_chunk` | `POST /api/stt/sessions/{sessionId}/chunk` | ✅ |
| `stt_finalize_session` | `POST /api/stt/sessions/{sessionId}/finalize` | ✅ |
| `stt_cancel_session` | `DELETE /api/stt/sessions/{sessionId}` | ✅ |

### Design Space（设计空间）

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_design_projects_cmd` | `GET /api/design/projects` | ✅ |
| `create_design_project_cmd` | `POST /api/design/projects` | ✅ |
| `update_design_project_cmd` | `PUT /api/design/projects` | ✅ |
| `get_design_project_cmd` | `GET /api/design/projects/{id}` | ✅ |
| `delete_design_project_cmd` | `DELETE /api/design/projects/{id}` | ✅ |
| `duplicate_design_project_cmd` | `POST /api/design/projects/{id}/duplicate` | ✅ |
| `list_design_artifacts_cmd` | `GET /api/design/projects/{projectId}/artifacts` | ✅ |
| `create_design_artifact_cmd` | `POST /api/design/artifacts` | ✅ |
| `import_design_image_cmd` | `POST /api/design/artifacts/import-image`（拖入导入：base64 图片→image 产物） | ✅ |
| `generate_design_brand_pack_cmd` | `POST /api/design/artifacts/brand-pack`（一 brief 批量生成一组共享系统的协调产物；可带 `referenceImages`（每件产物真看参考图、逐件规整成功计数钳 ≤5 张）+ `modelOverride`（单模型不降级）） | ✅ |
| `set_design_presenter_notes_cmd` | `PUT /api/design/artifacts/{artifactId}/presenter-notes`（deck 演讲者备注，存 metadata） | ✅ |
| `set_design_artifact_dir_cmd` | `PUT /api/design/artifacts/{id}/dir`（RTL/LTR 文本方向，存 metadata.dir + 重渲染） | ✅ |
| `patch_design_page_style_cmd` | `PUT /api/design/artifacts/{id}/page-style`（页面级 body 样式，CSS 标记块 + 落版本） | ✅ |
| `inpaint_design_image_cmd` | `POST /api/design/artifacts/{id}/inpaint`（image 蒙版局部重绘，经 `media_gen::execute_image` 只投给声明 `supports_mask` 的模型） | ✅ |
| `export_design_pptx_outline_cmd` | `GET /api/design/artifacts/{id}/pptx-outline`（deck 结构化可编辑文本 PPTX，服务端抽大纲） | ✅ |
| `review_design_artifact_cmd` | `GET /api/design/artifacts/{id}/quality-review`（确定性多镜头质量审查：a11y/内容/语义） | ✅ |
| `generate_design_artifact_cmd` | `POST /api/design/artifacts/generate`（`referenceImages`（≤5 张，优先于单张 `referenceImageB64`）作视觉附件真多模态；`modelOverride` 单模型不降级；image 形态可带 `aspectRatio` / `imageSize` / `imageResolution`，audio 形态可带 `audioKind` / `audioVoice` / `audioDurationSecs`，缺省落 `media_gen` 的 `imageDefaults` / `audioDefaults`） | ✅ |
| `design_ffmpeg_doctor_cmd` | `GET /api/design/ffmpeg/doctor` | ✅ |
| `design_install_ffmpeg_cmd` | `POST /api/design/ffmpeg/install` | ✅ |
| `design_browser_doctor_cmd` | `GET /api/design/browser/doctor` | ✅ |
| `design_install_browser_cmd` | `POST /api/design/browser/install` | ✅ |
| `list_all_design_artifacts_cmd` | `GET /api/design/artifacts` | ✅ |
| `get_design_artifact_cmd` | `GET /api/design/artifacts/{id}` | ✅ |
| `ensure_design_artifact_fresh_cmd` | `POST /api/design/artifacts/{id}/ensure-fresh`（打开时自愈渲染版本，返回是否重渲染） | ✅ |
| `delete_design_artifact_cmd` | `DELETE /api/design/artifacts/{id}` | ✅ |
| `rename_design_artifact_cmd` | `PUT /api/design/artifacts/{id}/title`（轻量改名，不重渲染/不新版本） | ✅ |
| `duplicate_design_artifact_cmd` | `POST /api/design/artifacts/{id}/duplicate`（同项目内深拷贝） | ✅ |
| `reorder_design_artifacts_cmd` | `POST /api/design/projects/{projectId}/artifacts/reorder`（拖动排序，body `orderedIds`） | ✅ |
| `list_design_folders_cmd` | `GET /api/design/projects/{projectId}/folders`（页面分组文件夹路径） | ✅ |
| `create_design_folder_cmd` | `POST /api/design/projects/{projectId}/folders`（新建空文件夹，body `name`） | ✅ |
| `rename_design_folder_cmd` | `PUT /api/design/projects/{projectId}/folders`（文件夹改名/移动，body `from`/`to`） | ✅ |
| `delete_design_folder_cmd` | `DELETE /api/design/projects/{projectId}/folders?path=`（删文件夹，页面移到根） | ✅ |
| `move_design_artifact_cmd` | `PUT /api/design/artifacts/{id}/folder`（把页面移到文件夹，body `folder`） | ✅ |
| `list_design_artifact_versions_cmd` | `GET /api/design/artifacts/{id}/versions` | ✅ |
| `get_design_artifact_version_html_cmd` | `GET /api/design/artifacts/{artifactId}/versions/{versionNumber}/html` | ✅ |
| `create_design_share_cmd` | `POST /api/design/artifacts/{artifactId}/share` | ✅ |
| `get_design_share_cmd` | `GET /api/design/artifacts/{artifactId}/share` | ✅ |
| `revoke_design_share_cmd` | `DELETE /api/design/artifacts/{artifactId}/share` | ✅ |
| _(公开只读快照，无鉴权)_ | `GET /api/design/share/{token}` | ✅ HTTP-only |
| `run_design_visual_regression_cmd` | `POST /api/design/artifacts/{artifactId}/visual-regression`（固定三视口截图 + 像素差 + 静态 DOM/a11y） | ✅ |
| `accept_design_visual_baseline_cmd` | `POST /api/design/visual-baseline`（显式 owner 接受，`expectedArtifactHash` 防陈旧写） | ✅ |
| `get/save_design_scenarios_cmd` | `GET/PUT /api/design/artifacts/{artifactId}/scenarios`（≤12 场景、≤4 视口、本地 route） | ✅ |
| `get_design_components_manifest_cmd` | `GET /api/design/projects/{projectId}/components?draft=` | ✅ |
| `save_design_components_draft_cmd` | `PUT /api/design/projects/{projectId}/components/draft` | ✅ |
| `publish_design_components_manifest_cmd` | `POST /api/design/components/publish`（expected published hash） | ✅ |
| `scan_design_components_cmd` | `POST /api/design/projects/{projectId}/components/scan`（绑定仓库只读扫描） | ✅ |
| `preview_figma_roundtrip_cmd` | `POST /api/design/figma-roundtrip/preview`（10 分钟一次性预览，无外部调用） | ✅ |
| `commit_figma_roundtrip_cmd` | `POST /api/design/figma-roundtrip/commit`（消费预览后调用 Figma MCP） | ✅ |
| `list_figma_roundtrip_reconciliations_cmd` | `GET /api/design/artifacts/{artifactId}/figma-roundtrip/reconciliations`（列未决外部副作用回执） | ✅ |
| `resolve_figma_roundtrip_reconciliation_cmd` | `POST /api/design/figma-roundtrip/reconcile`（显式确认已发生/未发生，CAS 校验后写审计记录并解锁） | ✅ |
| `list_figma_roundtrip_links_cmd` | `GET /api/design/artifacts/{artifactId}/figma-roundtrip`（不含凭据的链接元数据） | ✅ |
| `create_design_review_space_cmd` | `POST /api/design/review-spaces`（viewer/commenter，token 仅回一次） | ✅ |
| `list_design_review_spaces_cmd` | `GET /api/design/artifacts/{artifactId}/review-spaces` | ✅ |
| `revoke_design_review_space_cmd` | `DELETE /api/design/artifacts/{artifactId}/review-spaces/{grantId}` | ✅ |
| _(固定版本评审，review bearer)_ | `GET /api/design/review-space` · `POST /api/design/review-space/comments` | ✅ HTTP-only |
| `save_cf_deploy_config_cmd` | `PUT /api/design/deploy/config` | ✅ |
| `get_cf_deploy_config_cmd` | `GET /api/design/deploy/config` | ✅ |
| `deploy_design_artifact_cmd` | `POST /api/design/artifacts/{artifactId}/deploy` | ✅ |
| `probe_design_deploy_cmd` | `POST /api/design/deploy/probe`（探测部署 URL 是否已生效，body `{url}` 回 `{ready,status}`） | ✅ |
| `bind_design_domain_cmd` | `POST /api/design/artifacts/{artifactId}/domains`（绑定 CF Pages 自定义域名，回域名+验证态） | ✅ |
| `list_design_domains_cmd` | `GET /api/design/artifacts/{artifactId}/domains`（列已绑定域名+验证态） | ✅ |
| `preflight_design_deploy_cmd` | `GET /api/design/artifacts/{artifactId}/deploy/preflight`（部署预检：空/超限阻断、外部引用告警） | ✅ |
| `list_design_deployments_cmd` | `GET /api/design/artifacts/{artifactId}/deployments`（部署历史，跨 provider，最新在前） | ✅ |
| `save_vercel_deploy_config_cmd` | `PUT /api/design/deploy/vercel/config`（保存 Vercel token 0600 + team） | ✅ |
| `get_vercel_deploy_config_cmd` | `GET /api/design/deploy/vercel/config`（读配置，token 脱敏） | ✅ |
| `deploy_design_artifact_vercel_cmd` | `POST /api/design/artifacts/{artifactId}/deploy/vercel`（部署到 Vercel，回 `{url}`） | ✅ |
| `restore_design_version_cmd` | `POST /api/design/artifacts/{artifactId}/restore` | ✅ |
| `restyle_design_artifact_cmd` | `POST /api/design/artifacts/{id}/restyle`（换设计系统重染，新版本快照） | ✅ |
| `patch_design_element_cmd` | `POST /api/design/patch` | ✅ |
| `remove_design_element_cmd` | `POST /api/design/artifacts/{id}/remove-element`（删元素+回传重建上下文，结构 undo） | ✅ |
| `insert_design_element_cmd` | `POST /api/design/artifacts/{id}/insert-element`（重插被删元素，结构 undo 撤销侧，owner-only） | ✅ |
| `cancel_design_generation_cmd` | `POST /api/design/artifacts/{id}/cancel`（停止在途流式生成、降级占位，不删） | ✅ |
| `export_design_artifact_cmd` | `GET /api/design/artifacts/{id}/export`（format=html 干净自包含 / markdown HTML→MD） | ✅ |
| `export_design_handoff_cmd` | `GET /api/design/artifacts/{id}/handoff`（代码交付包 ZIP：index.html + source/ + 多平台 tokens/ + HANDOFF.md，base64） | ✅ |
| `bind_design_code_project_cmd` | `POST /api/design/bindings`（绑定设计系统→代码工程目录；HTTP 受 `allowRemoteWrites` 门） | ✅ |
| `sync_design_code_binding_cmd` | `POST /api/design/bindings/{id}/sync`（把多平台 token 写入绑定目录；HTTP 受 `allowRemoteWrites` 门） | ✅ |
| `list_design_code_bindings_cmd` | `GET /api/design/bindings?systemId=`（列出代码绑定） | ✅ |
| `unbind_design_code_project_cmd` | `DELETE /api/design/bindings/{id}`（解绑，不删已写文件） | ✅ |
| `get_design_project_code_binding_cmd` | `GET /api/design/projects/{id}/code-binding`（项目级代码仓库绑定状态：来源/生效目录/stale） | ✅ |
| `set_design_project_code_binding_cmd` | `PUT /api/design/projects/{id}/code-binding`（设置/清除双源绑定，`codeDir` 与 `haProjectId` 互斥；仅面向用户本人） | ✅ |
| `design_implement_to_code_cmd` | `POST /api/design/artifacts/{id}/implement`（组 handoff pack + 建实现会话，返回 `{sessionId, prompt, codeDir}`） | ✅ |
| `design_check_code_drift_cmd` | `POST /api/design/projects/{id}/code-drift/check`（code→design 回灌：收割承接会话写盘 + 逐文件比对，标 `metadata.codeDrift`；body `{artifactId?}`） | ✅ |
| `design_code_drift_changes_cmd` | `GET /api/design/artifacts/{id}/code-drift`（逐 stale 文件 diff 喂 DiffPanel + 带到对话 quote） | ✅ |
| `design_code_drift_sync_cmd` | `POST /api/design/artifacts/{id}/code-drift/sync`（重置基线为当前磁盘态 + 清 drift 标记） | ✅ |
| `mark_design_artifact_opened_cmd` | `POST /api/design/artifacts/{id}/opened`（上报「最近查看」，MCP `design_get_active_context` 事实源；不动 updated_at） | ✅ |
| `export_design_pptx_cmd` | `POST /api/design/pptx`（前端整页 PNG → OOXML 组装） | ✅ |
| `export_design_zip_cmd` | `POST /api/design/zip`（artifactId=单产物源码包 / projectId=项目级全产物包） | ✅ |
| `export_design_selected_zip_cmd` | `POST /api/design/zip/selected`（body `artifactIds` → 选中产物打成一个 ZIP + 画廊，文件面批量导出） | ✅ |
| `critique_design_artifact_cmd` | `POST /api/design/artifacts/{id}/critique` | ✅ |
| `list_design_systems_cmd` | `GET /api/design/systems` | ✅ |
| `get_design_system_cmd` | `GET /api/design/systems/{id}` | ✅ |
| `save_design_system_cmd` | `POST /api/design/systems` | ✅ |
| `extract_design_system_cmd` | `POST /api/design/systems/extract`（brief/codebase/url/image 反向提取；`image` 走 `run_vision` 真视觉 + 可选 `modelOverride` 单模型不降级） | ✅ |
| `import_design_md_cmd` | `POST /api/design/systems/import`（导入 DESIGN.md 规范文本） | ✅ |
| `import_figma_system_cmd` | `POST /api/design/systems/figma`（从 Figma 文件导入设计系统；面向用户本人，令牌按次传不落盘） | ✅ |
| `export_design_md_cmd` | `GET /api/design/systems/{id}/design-md`（导出为规范 DESIGN.md） | ✅ |
| `export_design_tokens_cmd` | `GET /api/design/systems/{id}/tokens/export`（Token 导出多平台代码：CSS/SCSS/TS/Swift/Android XML/DTCG） | ✅ |
| `propose_design_directions_cmd` | `POST /api/design/directions` | ✅ |
| `list_design_recipes_cmd` | `GET /api/design/recipes`（内置设计模板目录，首屏模板快选） | ✅ |
| `get_design_recipe_demo_cmd` | `GET /api/design/recipes/{id}/demo?systemId=`（模板骨架 demo HTML：工具箱 hover 预览，注入设计系统配色） | ✅ |
| `export_design_native_cmd` | `GET /api/design/artifacts/{id}/native?format=pdf\|png`（真实浏览器原生捕获：矢量 PDF / 全保真 PNG；无后端时前端回退客户端栅格化） | ✅ |
| `delete_design_system_cmd` | `DELETE /api/design/systems/{id}` | ✅ |
| `rename_design_system_cmd` | `PATCH /api/design/systems/{id}`（重命名用户设计系统，body `{name}`，内置拒改） | ✅ |
| `get_design_config_cmd` | `GET /api/config/design` | ✅ |
| `save_design_config_cmd` | `PUT /api/config/design` | ✅ |
| `design_comment_add_cmd` | `POST /api/design/artifacts/{artifactId}/comments` | ✅ |
| `design_comment_list_cmd` | `GET /api/design/artifacts/{artifactId}/comments` | ✅ |
| `design_comment_relocate_cmd` | `POST /api/design/artifacts/{artifactId}/comments/{commentId}/relocate` | ✅ |
| `design_comment_update_cmd` | `PUT /api/design/artifacts/{artifactId}/comments/{commentId}` | ✅ |
| `design_comment_resolve_cmd` | `POST /api/design/artifacts/{artifactId}/comments/{commentId}/resolve` | ✅ |
| `design_comment_delete_cmd` | `DELETE /api/design/artifacts/{artifactId}/comments/{commentId}` | ✅ |
| `design_comment_refine_cmd` | `POST /api/design/artifacts/{artifactId}/comments/{commentId}/refine`（回灌对话：AI 按批注精修产物、落新版本） | ✅ |
| `design_review_artifact_cmd` | `POST /api/design/artifacts/{artifactId}/review`（反-slop 自查复查：`action ∈ recheck\|dismiss`） | ✅ |
| `get_design_system_kit_cmd` | `GET /api/design/systems/{id}/kit`（设计系统套件视图自包含 HTML，返回 JSON 字符串） | ✅ |
| `design_chat_thread_get_cmd` | `GET /api/design/projects/{projectId}/chat/thread`（设计对话默认加载目标：该项目最近一条对话线程的 SessionMeta，无则空） | ✅ |
| `design_chat_threads_list_cmd` | `GET /api/design/projects/{projectId}/chat/threads`（设计对话历史选择器分页，`query` FTS 过滤） | ✅ |
| （静态托管，iframe 直连） | `GET /api/design/projects/{pid}/artifacts/{aid}/{*rest}` | ✅ |

Figma 往返在 Tauri / HTTP 两端共用按制品串行化语义：同一制品仅最新预览可提交；`commit` 持锁覆盖回执检查、外部 MCP 调用与最终落盘，未解决的 `.indeterminate` 回执会阻断新预览和后续提交。用户核对 Figma 后须对精确回执选择“已发生”或“未发生”，后端以回执 ID + 本地哈希做 CAS 校验，先原子持久化 `reconciled/<receiptId>.json` 审计记录，再删除阻断标记；写入或删除失败继续 fail closed。

### Artifacts

| Tauri Command / Transport | HTTP | 状态 |
|---|---|---|
| `list_artifacts` | `GET /api/artifacts?limit=&offset=&kind=&lifecycleState=` | ✅ |
| `get_artifact` | `GET /api/artifacts/{id}` | ✅ |
| `list_artifact_versions` | `GET /api/artifacts/{id}/versions` | ✅ |
| `import_artifact` | `POST /api/artifacts/import` | ✅（`filePath` 与 `artifact_source uploadId` 互斥；同一路径同时承载 create/update，update 必须带 `artifactId+expectedVersion`） |
| `restore_artifact` | `POST /api/artifacts/{id}/restore` | ✅ |
| `verify_artifact` | `POST /api/artifacts/{id}/verify` | ✅ |
| `review_artifact_export` | `POST /api/artifacts/{id}/export-review` | ✅ |
| `export_artifact` | `POST /api/artifacts/{id}/exports` + `GET /api/artifact-exports/{exportId}/download` | ✅（Tauri 保存对话框 + 原子复制；HTTP 受管文件 streaming） |
| `archive_artifact` | `POST /api/artifacts/{id}/archive` | ✅ |
| `delete_artifact` | `DELETE /api/artifacts/{id}` | ✅ |

Artifact owner API 的完整不变量、输入白名单、本地导出与未来 Publisher Guard 边界、receipt 见 [Artifacts 本地优先产物平台](../infra/artifacts.md)。列表响应包含 metadata/source/verification 摘要但不返回 canonical payload 或大 dataset；正文由受保护的 Canvas 静态路径加载。

`POST /api/artifacts/{id}/exports` 接受 `format` 与可选 `expectedVersion`；Gallery 总是传入风险确认时读取到的当前版本。Core 在生成快照前原子比较版本，不一致返回 `artifact_conflict`，调用方应刷新产物后重新确认，而不是自动重试导出新版本。

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
| `dreaming_run_resolver` | `POST /api/dreaming/resolver` | ✅ 面向用户本人；Deep resolver（phase=deep）：valid_until 过期确定性 expire + 同主谓多对象组 LLM 判定 duplicates→merge / conflict→needs_review / independent→no_op，绝不自动 supersede 或硬删 |
| `dreaming_run_profile` | `POST /api/dreaming/profile/run` | ✅ 面向用户本人；Memory Profile 合成（phase=profile）：从 active claims 按 scope 规则式聚合（manual 触发额外 LLM 重写），写 `memory_profile_snapshots`（version=MAX+1）；受 `dreaming.profileSynthesis.enabled`（默认开）门控 |
| `dreaming_list_profile_snapshots` | `GET /api/dreaming/profile` | ✅ 面向用户本人；每 scope 最新 profile 快照（只读视图，global/agent/project） |
| `dreaming_list_diaries` | `GET /api/dreaming/diaries` | ✅ |
| `dreaming_read_diary` | `GET /api/dreaming/diaries/{filename}` | ✅ |
| `dreaming_is_running` | `GET /api/dreaming/status` | ✅ |
| `dreaming_last_report` | `GET /api/dreaming/last-report` | ✅ |
| `dreaming_idle_status` | `GET /api/dreaming/idle-status` | ✅ |
| `dreaming_list_runs` | `GET /api/dreaming/runs` | ✅ |
| `dreaming_get_run` | `GET /api/dreaming/runs/{id}` | ✅ |
| `dreaming_evidence_quote` | `GET /api/dreaming/evidence/quote` | ✅ 面向用户本人；incognito 来源归零（后端门控） |
| `claim_list` | `GET /api/claims` | ✅ 结构化 claim 只读（`scopeType`+`scopeId`/status/claimType 过滤；无效 scopeType → 400，不 fail-open；status 按 **effective** 计算并返回——`active` 且已过 `valid_until` 视为 `expired`，`status=active`/`expired` 过滤同步对齐） |
| `claim_get` | `GET /api/claims/{id}` | ✅ claim + evidence + links（`status` 同为 effective 值） |
| `claim_update` | `PATCH /api/claims/{id}` | ✅ 面向用户本人；用户纠错（Lucid Review §5.2）：edit content/triple/tags、改 status（approve→active / reject→archived / mark-outdated→expired / flag→needs_review）、move scope、pin/unpin（salience 越过 0.7 阈值）。写 `manual_correction` evidence（approve 用 `user_confirmed`）+ `user_correction` decision log + 发 `memory:claim_changed`；content 变更触发 re-embed。`id` 走 path（覆盖 body 的 `claimId`），其余字段为 body |
| `claim_forget` | `POST /api/claims/{id}/forget` | ✅ 面向用户本人；`{permanent?,note?}`。`permanent=false`（默认）archive（保留 evidence 作审计，linked legacy memory 停止注入）；`true` 硬删 claim 图谱（claim+evidence+link+vector）+ 仅本 claim 独管的 legacy memory。写 decision log + 发 `memory:claim_changed` |
| `memory_backfill_plan` | `GET /api/memory/backfill/plan` | ✅ 面向用户本人；dry-run 把 legacy memory 确定性映射为 claim 预览（精确计数 + 截断预览，不写） |
| `memory_backfill_apply` | `POST /api/memory/backfill/apply` | ✅ 面向用户本人；确定性重扫，事务内 check（memory 存在 + 未 link，竞态/重入幂等→skipped）后写入 claim + `source_type=memory` evidence + **detached** link（不改变现有注入），仅 pinned 的 user/feedback 自动 active、其余 needs_review；返回 created/skipped/failed |
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

`get_hooks_config` / `GET /api/config/hooks` 只返回已信任工作区路径，不返回内容哈希；保存时已有路径保留服务端旧哈希，仅新加入（或先移除并保存后重新加入）的路径按当前 Hook 文件内容生成新信任记录。Tauri 与 HTTP 适配器共用这一 fail-closed 语义。

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

`create_owner_ask_user_question` 创建面向用户本人的 durable elicitation：它复用 ask_user UI，但不等待模型工具 oneshot；请求自带 `ownerResponse`，用户通过 `respond_ask_user_question` 回答后由后端记录对应 durable evidence（当前用于 Context Retrieval 的 `user_decision`）。普通工具型 ask_user 仍要求 live in-memory receiver，用户本人侧的问题则可跨会话切换和重启保留；incognito session 禁用。

### Cron

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `cron_list_jobs` | `GET /api/cron/jobs` | ✅（每行附 `lastRun` 最近一次运行摘要，供列表搜索与异常入口，无需逐行拉运行日志） |
| `cron_get_job` | `GET /api/cron/jobs/{id}` | ✅ |
| `cron_get_job_snapshot` | `GET /api/cron/jobs/{id}/snapshot` | ✅（返回 `{ job, deleted }`；逻辑删除的 Task 仍可读，供保留历史展示与「复制为新任务」草稿，绝不重新进入排程面） |
| `cron_preflight` | `POST /api/cron/preflight` | ✅（只读；body `{ request }`，支持 create/update/runNow，返回下三次触发、实际执行摘要和 blocker/warning/info） |
| `cron_create_job` | `POST /api/cron/jobs` | ✅ |
| `cron_update_job` | `PUT /api/cron/jobs/{id}` | ✅（`expectedRevision` CAS；冲突 HTTP 409 / code=`cron_revision_conflict` + `currentJob`，Tauri 返回同形结果） |
| `cron_toggle_job` | `POST /api/cron/jobs/{id}/toggle` | ✅ |
| `cron_delete_job` | `DELETE /api/cron/jobs/{id}` | ✅（逻辑删除；保留运行历史与普通 / legacy Session） |
| `cron_run_now` | `POST /api/cron/jobs/{id}/run` | ✅（body `{ expectedRevision }`；live preflight 后同步精确 claim，返回 `started` 或带新报告的 `rejected`） |
| `cron_cancel_run` | `POST /api/cron/runs/{runLogId}/cancel` | ✅（按 immutable run-log occurrence 精确取消 standalone `AgentTurn`；terminal 幂等 no-op；无 `turnId` 的 legacy / SessionLoop 返回 `code=cron_run_cancel_unsupported`、`cancelRequested=false`） |
| `cron_jobs_referencing_account` | `GET /api/cron/jobs-referencing-account/{accountId}` | ✅ |
| `cron_get_run_logs` | `GET /api/cron/jobs/{jobId}/logs` | ✅（按可见行分页，排除已归档运行对话；standalone 行含 exact `turnId`） |
| `cron_get_calendar_events` | `GET /api/cron/calendar` | ✅ |
| `cron_run_timeline` | `GET /api/cron/timeline?limit=&offset=` | ✅（跨 job 运行时间线，含已删 Task 历史；`jobDeleted` 标记，按可见行分页并排除已归档对话） |
| `cron_unread_total` | `GET /api/cron/unread` | ✅（未读 Cron 运行 session 数，侧边栏独立角标） |
| `cron_mark_all_read` | `POST /api/cron/read-all` | ✅ (一键清除 cron 未读，emit `cron:unread_changed`) |
| `cron_workspace_resources` | `GET /api/cron/workspaces?jobId=` | ✅（有界的待处理 Worktree 资源，包含后端裁决的 owner action availability） |
| `cron_workspace_resource_for_run` | `GET /api/cron/runs/{runLogId}/workspace` | ✅（按保留 run log 查询，Task 逻辑删除后仍可用） |
| `cron_workspace_takeover` | `POST /api/cron/jobs/{jobId}/workspace/takeover` | ✅（body `{ sessionId }`） |
| `cron_workspace_return` | `POST /api/cron/jobs/{jobId}/workspace/return` | ✅（body `{ sessionId, resume }`） |
| `cron_workspace_discard_run` | `POST /api/cron/runs/{runLogId}/workspace/discard` | ✅（body `{ sessionId, confirm: true }`） |
| `cron_workspace_discard_task` | `POST /api/cron/jobs/{jobId}/workspace/discard` | ✅（body `{ confirm: true }`） |

`cron_preflight` 不联网、不创建 Worktree、不写数据库；create/update 壳在临写前重跑同一 evaluator 并拒绝 blocker。非 Project 任务的远程 create/update/enable/run-now 与资源接管、归还、丢弃还必须实时命中 `filesystem.allowRemoteWrites=true`，否则 403。Scheduled Worktree 读取端点同样不写盘；可用性及拒绝原因由 `CronWorkspaceResource.actions` 返回，客户端不根据本地状态猜测；真正的 Diff / branch / commit / push / PR 继续复用普通 Session Workspace / Git 控制面。

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
| `dashboard_control_plane` | `POST /api/dashboard/control-plane` | ✅（Goal / Workflow / Loop / Task / Plan 聚合；独立 Agent/项目筛选） |
| `dashboard_system_metrics` | `GET /api/dashboard/system-metrics` | ✅ |
| `dashboard_session_list` | `POST /api/dashboard/session-list` | ✅ |
| `dashboard_message_list` | `POST /api/dashboard/message-list` | ✅ |
| `dashboard_tool_call_list` | `POST /api/dashboard/tool-call-list` | ✅ |
| `dashboard_error_list` | `POST /api/dashboard/error-list` | ✅ |
| `dashboard_agent_list` | `POST /api/dashboard/agent-list` | ✅ |
| `dashboard_local_model_usage` | `POST /api/dashboard/local-model-usage` | ✅ |

`dashboard_control_plane` 请求体为 `{ "filter": { "startDate": ISO8601|null, "endDate": ISO8601|null, "agentId": string|null, "projectId": string|null } }`；返回 `{ summary, goals, workflows, loops, tasks, plans, attention }`。`projectId="__unassigned__"` 表示未分配项目。比例零分母序列化为 `null`；`attention.items` 最多 20 条、`attention.total` 保留未截断总数。旧 `dashboard_tasks` 只统计 Cron / Subagent（前端名“自动化”），`dashboard_plan_stats` 保持兼容。

#### Dashboard Learning

`session.db.learning_events` 表 + `dashboard::learning` 只读聚合查询（写入面 `learning_events::emit`），支持 7/14/30/60/90 天窗口。埋点来自 `skills::author` CRUD 与 `tool_recall_memory` 命中等。前端 Dashboard "Learning" Tab 消费。

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

`dashboard_coding_improvement` 是只读全局学习聚合（按 DashboardFilter 返回 workflow / eval / strategy / tool-call failure / review / verification / proposal / retro 的 overview、timeline、project buckets 等）；`get_coding_benchmark_center` 及 Benchmark Campaign / Leaderboard / Compare / Corpus / Report / Continuous Gate / Backlog 一组 API 为 Dashboard 提供 durable benchmark 历史、守门状态与 item-level evidence。它们都不生成 proposal、不 apply、不 promotion。

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
| `get_external_memory_providers_config` | `GET /api/config/external-memory-providers` | ✅ |
| `get_external_memory_providers_preflight` | `GET /api/config/external-memory-providers/preflight` | ✅（零网络） |
| `run_external_memory_provider_sync` | `POST /api/config/external-memory-providers/sync` | ✅ |
| `test_external_memory_provider_connection` | `POST /api/config/external-memory-providers/{providerId}/test` | ✅（owner 显式触发版本/能力探测） |
| `get_external_memory_provider_credential_status` | `GET /api/config/external-memory-providers/{providerId}/credentials` | ✅ |
| `save_external_memory_provider_credentials` | `PUT /api/config/external-memory-providers/{providerId}/credentials` | ✅ |
| `clear_external_memory_provider_credentials` | `DELETE /api/config/external-memory-providers/{providerId}/credentials` | ✅ |
| `save_external_memory_providers_config` | `PUT /api/config/external-memory-providers` | ✅ |

`get_deferred_tools_config` 会把旧配置（缺少 `mode`）按 `DeferredToolsConfig::effective_mode()` 归一化后返回，确保 Tauri / HTTP 界面展示与运行时策略一致；磁盘兼容字段保持原样，只有显式保存才写入 V2 `mode`。

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
| — | `GET /api/app-update/status` | HTTP-only：读取远程服务更新快照、活动任务与最近任务 |
| — | `POST /api/app-update/check` | HTTP-only：立即检查远程服务更新并持久化快照 |
| — | `POST /api/app-update/prepare` | HTTP-only、Owner Token 必须启用：创建 5 分钟、进程绑定的一次性安装计划 |
| — | `POST /api/app-update/confirm` | HTTP-only、Owner Token 必须启用：消费一次性计划并启动持久化更新任务；请求体只含 `planId` |
| — | `GET /api/app-update/jobs/{jobId}` | HTTP-only：按 ID 重读持久化任务，供断线/重启恢复 |
| `get_startup_notification_config` | `GET /api/config/startup-notification` | ✅ |
| `save_startup_notification_config` | `PUT /api/config/startup-notification` | ✅ |
| `get_server_config` | `GET /api/config/server` | ✅ |
| `save_server_config` | `PUT /api/config/server` | ✅ |
| `get_server_runtime_status` | `GET /api/server/status` | ✅（Owner 保护面）— 返回 `{ boundAddr, startedAt, uptimeSecs, startupError, eventsWsCount, chatWsCount, localDesktopClient, activeChatStreams, activeChatCounts: { desktop, http, channel, total } }`。`activeChatStreams` 是 `activeChatCounts.total` 的 back-compat 别名（在跑的 `run_chat_engine` 数量）。`chatWsCount` 当前仍是独立的 `Arc<AtomicU32>` 计数器（`crates/ha-core/src/server_status.rs::chat_ws_counter`），per-session chat WS 端点已下线但 counter 字段未拆——历史遗留，目前没有 handler 在递增，实测恒为 0。`localDesktopClient` 在 Tauri 命令恒 `true`（桌面 webview 通过 IPC 与后端通信，不走 WS），HTTP 路由恒 `false`，前端把它计入"活跃连接" |
| `get_proxy_config` | `GET /api/config/proxy` | ✅ |
| `save_proxy_config` | `PUT /api/config/proxy` | ✅ |
| `get_shortcut_config` | `GET /api/config/shortcuts` | ✅ |
| `save_shortcut_config` | `PUT /api/config/shortcuts` | ✅ |
| `set_shortcuts_paused` | `POST /api/config/shortcuts/pause` | ✅ |
| `get_sandbox_config` | `GET /api/config/sandbox` | ✅ |
| `set_sandbox_config` | `PUT /api/config/sandbox` | ✅ |
| `check_sandbox_available` | `GET /api/config/sandbox/status` | ✅ — 返回 `{ installed, running, hostOs, backend, wslInstalled, wslDistributionInstalled, wslDockerInstalled, connectionError, containerized, isolatedModeOnly }`；`connectionError` 为 `socket_missing` / `permission_denied` / `daemon_unreachable` / `client_error` / `null`。`backend` 为 `native` / `wsl` / `null`。原生 daemon 健康时不唤醒 WSL；`HA_DEPLOYMENT=docker` 时仅 `isolated` mode 可执行 |

### Canvas

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_canvas_config` | `GET /api/config/canvas` | ✅ |
| `save_canvas_config` | `PUT /api/config/canvas` | ✅ |
| `canvas_submit_snapshot` | `POST /api/canvas/snapshot/{requestId}` | ✅ |
| `canvas_submit_eval_result` | `POST /api/canvas/eval/{requestId}` | ✅ |
| `show_canvas_panel` | `POST /api/canvas/show` | ✅ |
| `list_canvas_projects_by_session` | `GET /api/canvas/by-session/{sessionId}` | ✅ |

### Media generation / Web search / Web fetch / SSRF / SearXNG

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_media_gen_config` | `GET /api/config/media-gen` | ✅ |
| `add_media_provider` | `POST /api/config/media-gen/providers` | ✅ |
| `update_media_provider` | `PUT /api/config/media-gen/providers/{providerId}` | ✅ |
| `delete_media_provider` | `DELETE /api/config/media-gen/providers/{providerId}` | ✅ |
| `reorder_media_providers` | `PUT /api/config/media-gen/providers/reorder` | ✅ |
| `set_media_default_chain` | `PUT /api/config/media-gen/chains/{function}` | ✅ |
| `update_media_gen_defaults` | `PUT /api/config/media-gen/defaults` | ✅ |
| `get_media_provider_templates` | `GET /api/config/media-gen/templates` | ✅ |
| `list_media_voices` | `GET /api/config/media-gen/voices` | ✅ |
| `test_media_provider` | `POST /api/config/media-gen/test` | ✅ |
| `get_media_gen_overview` | `GET /api/config/media-gen/overview` | ✅ |
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

后台 watchdog（[`crates/ha-local-llm/src/local_llm/auto_maintainer.rs`](../../../crates/ha-local-llm/src/local_llm/auto_maintainer.rs)）监测默认 chat / embedding 模型。模型停止时自动 preload；模型文件丢失时 emit `local_model:missing_alert` 事件，前端顶层 `MissingModelDialog` 弹窗。

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_local_llm_auto_maintenance_enabled` | `GET /api/local-model/auto-maintenance` | ✅ |
| `set_local_llm_auto_maintenance_enabled` | `PUT /api/local-model/auto-maintenance` | ✅ |
| `local_model_alert_dismiss_temporary` | `POST /api/local-model/alert/dismiss-temporary` | ✅ |
| `local_model_alert_silence_session` | `POST /api/local-model/alert/silence-session` | ✅ |
| `local_model_auto_maintenance_disable` | `POST /api/local-model/auto-maintenance/disable` | ✅ |
| `local_model_auto_maintenance_trigger` | `POST /api/local-model/auto-maintenance/trigger` | ✅ |

### 内置用户手册（帮助中心）

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_manual_bundle` | `GET /api/manual/bundle?lang=` | ✅ |
| `search_manual` | `GET /api/manual/search?lang=&query=` | ✅ |

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
| `list_slash_commands` | `GET /api/slash-commands?sessionId=…`（可选） | ✅ |
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
| Current Project | `sessions.project_id` | 单独一段（项目名 / desc / agent / working dir / `AGENTS.md` 指令预览 / agent source） |
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
| `channel_validate_credentials` | `POST /api/channel/validate` | ✅（body `{ channelId, credentials, settings? }`；`settings` 让 Telegram setup 校验走草稿中的 `apiRoot`，缺省保持旧版只验 credentials） |
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

Team 生命周期 HTTP 响应保持原有 `status` 字段并追加可观测结果：pause 为 `status="paused"`，dissolve 为 `status="dissolved"`；两者的 `cancellation` 会区分 `requestedCount`、`terminalCount`、`pendingCount` 与逐 run 状态，取消请求获接受不等于 run 已终止。成员启动走 durable prepare → roster attach → roster-aware launch CAS；pause/dissolve 先提交时不会 fire `SubagentStart` 或启动模型，排队项也保留同一 Team fence。resume 仅在所有被引用的旧成员 run 已终态（或成员 `run_id=NULL`）时启动 fresh attempt；任一旧 run 仍 `queued|spawning|running` 时整队保持 Paused、零 spawn，返回 `status="paused", disposition="refused", retryable=true` 及 `failures[].reason="old_attempt_still_active"`。非空 run_id 对应记录缺失时同样 fail-closed，reason 为 `old_attempt_unknown`、oldAttemptStatus 为 `missing_run_record`。若旧 run 在 pause 提交后的取消竞态中变成 Completed，resume 会把成员惰性对齐为 Completed 并排除重跑；对齐后仅当 roster 非空且全员 Completed 时返回 `status="already_complete", teamStatus="paused", disposition="no_op"`，带完整且重试稳定的 `completedDuringPauseCount/completedMembers`，可发幂等 `member_status=completed`，但不发 `resumed`、不 spawn。兼容字段名 `completedDuringPauseCount` 在该 no-op 中等于 `completedMembers.length`，表示本次 resume 已完成并被跳过的完整 roster 数（含 pause 前已完成成员），不是本次新对齐增量；刷新或重复调用保持同一 no-op。空 roster 或含任意非 Completed 成员的混合 roster 不得归入该结果。通过 gate 后，全成功为 `status="resumed", teamStatus="active", disposition="resumed"`；启动部分成功为 `status="partially_resumed", teamStatus="active", disposition="partial"` 并带 `resumedMembers/failures`；未启动任何成员时为 `status="paused"|"refused", disposition="refused"`。Tauri `resume_team` 与 HTTP 返回同一份结构化 `ResumeTeamResult`；`resumed/partial/refused/no_op` 都是成功传输的领域结果，UI 必须读取 `disposition` 与 durable `teamStatus`，只有传输或系统错误才返回 `CmdError`。`team_event` 仍负责刷新已经发生的 durable 状态变化。

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
| `browser_panel_navigate` | `POST /api/browser/panel-navigate`，body `{ op: "go"\|"back"\|"reload", url?, sessionId? }`（面板快捷条；`go` 过 SSRF 检查，缺 scheme 默认 https） | ✅ |
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

### Permission（权限/审批引擎）

详见 [`docs/architecture/permission-system.md`](../agent/permission-system.md)。

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
| `save_exported_file` | — | 仅桌面：把导出字节（base64）写到原生保存框选定的路径（设计空间导出）。**故意无 HTTP 端点**——远端经 File System Access / 浏览器下载在本机保存，绝不写服务器磁盘（防远程写盘 / 外泄） |
| `set_dock_badge_cmd` | — | 仅桌面：把全局未读总数写到 app icon / Dock 角标（`count=0` 清除）；前端按 `isTauriMode()` 门控，Web 端不调用，无 HTTP 端点 |
| `set_tray_unread_cmd` | — | 仅桌面：状态栏 / tray 图标按普通未读会话是否大于 0 显示红点；紧凑图标不绘制数字 |
| `get_system_prompt` | `POST /api/system-prompt` | 调试端点 |

### Filesystem

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `fs_list_dir` | `GET /api/filesystem/list-dir?path=<abs>` | ✅ |
| `fs_search_files` | `GET /api/filesystem/search-files?root=<abs>&q=<query>&limit=50` | ✅；path-aware fuzzy（精确 / 前缀 / 多 token / 路径分段 / 驼峰词感知，subsequence 兜底） |

`list-dir` 列出服务器本地目录单层条目，供 HTTP 模式目录浏览器驱动 `set_session_working_dir`，以及聊天输入框 `@` mention popper 的"路径模式"。参数要求绝对路径，后端会 canonicalize 并校验 `is_dir`；无 `path` 参数时返回平台默认根（Unix: `/`，Windows: `USERPROFILE`）。响应 `{ path, parent, entries: [{ name, isDir, isSymlink, size, modifiedMs }], truncated }`，按目录优先 + 名字升序排序，单次最多 5000 条（超出 `truncated=true`）。

`search-files` 在 `root` 下做 fuzzy 搜索，供聊天输入框 `@` mention popper 的"搜索模式"使用——用户输入 `@chat` 这种不含 `/` 的非空 token 时调用。后端用 `ignore::WalkBuilder` 遍历，遵守 `.gitignore` / `.git/info/exclude` / `.ignore` / 隐藏文件规则；`q` 按子序列匹配 + 评分（name 命中 +1000、path 命中 +200，靠近开头 + 跨度紧凑得分高）。响应 `{ root, matches: [{ name, path, relPath, isDir, score }], truncated }`，按 score desc + path asc 排序；`limit` 默认 50，最大 200；单次最多遍历 50000 条文件，超出 `truncated=true`。

两个 endpoint 桌面用 Tauri 原生 dialog（`pickLocalDirectory`）做目录初选时仍优先走 `@tauri-apps/plugin-dialog`，但 mention popper 在 Tauri 模式同样需要列目录 / 搜索能力，因此两个命令在桌面端也 invoke 注册。核心逻辑在 [`crates/ha-core/src/filesystem/mod.rs`](../../../crates/ha-core/src/filesystem/mod.rs) 单一来源，axum / Tauri 两侧都是薄壳。

### Terminal

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `terminal_create` | `POST /api/terminals` | ✅ 创建 PTY，body `{ request: { cwd?, cols, rows } }` |
| `terminal_list` | `GET /api/terminals` | ✅ 列出进程内会话元数据（不携带输出） |
| `terminal_snapshot` | `GET /api/terminals/{terminalId}` | ✅ 重载/事件缺口恢复 |
| `terminal_write` | `POST /api/terminals/{terminalId}/input` | ✅ 写 stdin |
| `terminal_resize` | `POST /api/terminals/{terminalId}/resize` | ✅ 更新 PTY 行列 |
| `terminal_close` | `DELETE /api/terminals/{terminalId}` | ✅ 终止 shell 并移除会话 |

终端是高权限 owner 写入面，HTTP 端点必须保留在 Bearer 保护的 `/api` 路由内，并统一要求 `filesystem.allowRemoteWrites=true`；不得向 Knowledge Agent 只读 token、公开路由开放或绕过远程写入门。生命周期、上限和重放协议详见 [terminal.md](terminal.md)。

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
| — | `GET /api/auth/status` | 公开最小启动探针：仅返回 required/authenticated；Token 指纹仅对已认证请求返回，避免弱 Token 离线猜测 oracle |
| — | `POST /api/auth/session` | 公开 Token→HttpOnly 会话交换；同源检查、失败限速、`no-store` |
| — | `POST /api/auth/logout` | 清除浏览器会话 Cookie |
| — | `POST /api/auth/transport-tickets` | Owner 保护；给跨源远程 GUI 签发 15 分钟 `events` / `resources` scope 票据，`no-store` |
| — | `POST /api/auth/preview-resource-ticket` | Owner 保护；给跨源 Canvas / Design 签发 15 分钟 project / artifact 子树绑定票据，`no-store`；可执行预览不能复用通用 `resources` 票据 |
| — | `POST /api/fs/raw-ticket` | Owner 保护；把 15 分钟 capability 绑定到单个已授权 canonical workspace file，`no-store` |
| — | `POST /api/sessions/{id}/files/by-path-ticket` | Owner 保护；按会话引用/工作目录授权后把 15 分钟 capability 绑定到单个 canonical file，`no-store` |
| — | `GET /api/resource/{ticket}/{*path}` | 公开 capability 入口；票据仅分派到静态预览/附件/授权文件的只读 allowlist，访问日志隐藏票据段 |
| `rotate_server_token` | `POST /api/auth/token/rotate` | ✅；Owner 保护，返回新 Token 一次，立即作废旧会话；外部托管 Token 拒绝 |
| `list_local_ips` | `GET /api/server/local-ips` | ✅ |

## 已知不对齐项

截至 2026-08-05 三端差集为 31 条：§7.3 的 6 条 Desktop-only 系统权限命令、§7.3.1 的 12 条 HTTP 已实现但走专用 Transport 方法、3 条 HTTP-only transport ticket 基础设施、5 条 HTTP-only 远程服务更新端点，以及 `project_fs_resolve` / `kb_file_resolve_cmd` / `set_dock_badge_cmd` / `set_tray_unread_cmd` / `save_exported_file` 5 条 Tauri-only 命令。没有“HTTP 漏写 COMMAND_MAP”或“HTTP 路由缺失”的破口；COMMAND_MAP 每一条顶层命令都能在 `tauri::generate_handler!` 找到对应命令。

### §7.3 Desktop-only（Tauri 专属，合法缺失，6 条）

| Tauri Command | 说明 |
|---|---|
| `check_system_permissions` | macOS 系统权限目录与状态查询 |
| `request_system_permission` | macOS 系统权限请求/跳转 |
| `reset_system_permission` | macOS 系统权限重置 TCC 记录（`tccutil`，白名单 3 项、仅打包应用；**owner/GUI-only，刻意无模型工具面**） |
| `check_all_permissions` | 旧版权限接口兼容包装 |
| `check_permission` | 旧版权限接口兼容包装 |
| `request_permission` | 旧版权限接口兼容包装 |

前端必须在 `supportsLocalFileOps()` / `isTauriMode()` 或等价的运行模式判定保护下调用，HTTP 模式应 gate 住相关 UI。

### §7.3.1 不进 COMMAND_MAP 但 HTTP 已实现的合法专用入口（15 条）

| Tauri Command | HTTP 端点 | 原因 |
|---|---|---|
| `stage_chat_attachment` | `POST /api/chat/attachment-stage` | multipart/form-data，HTTP 走 `HttpTransport.stageChatAttachment()` 专用方法 |
| `save_avatar` | `POST /api/avatars` | multipart/form-data，HTTP 走 `HttpTransport.call()` 特殊分支 |
| `fs_list_dir` | `GET /api/filesystem/list-dir?path=<abs>` | query-string GET，HTTP 走 `HttpTransport.listServerDirectory()` 自定义方法（详见 Filesystem 域） |
| `fs_search_files` | `GET /api/filesystem/search-files?root=<abs>&q=<q>&limit=<n>` | 同上，走 `HttpTransport.searchFiles()` |
| `fs_create_dir` | `POST /api/filesystem/create-dir` | 绝对目录创建 + listing 返回，HTTP 走 `HttpTransport.createDirectory()` 专用方法 |
| `project_fs_upload` | `POST /api/fs/upload`（multipart） | 走 `HttpTransport.projectFsUpload()` 专用方法（详见 Filesystem 域） |
| `export_session_cmd` | `GET /api/sessions/{sessionId}/export` | 两端形态不对称（Tauri 走原生 save dialog，HTTP 返二进制流），统一入口 `exportSession`（详见 Session 域） |
| `export_artifact` | `POST /api/artifacts/{id}/exports` + `GET /api/artifact-exports/{exportId}/download` | Tauri 走 save dialog + 受管文件复制；HTTP 走 receipt + 二进制流，统一入口 `exportArtifact` |
| `memory_backup_export_archive` | `POST /api/memory/backup/export-archive` | Tauri 保存到用户路径；HTTP 返回 ZIP blob，统一入口 `exportMemoryBackupArchive` |
| `memory_backup_preview_archive` | `POST /api/memory/backup/preview-archive` | HTTP body 为 ZIP bytes，走 `previewMemoryBackupArchive` |
| `memory_backup_restore_legacy_archive` | `POST /api/memory/backup/restore-legacy-archive` | HTTP body 为 ZIP bytes，走 `restoreMemoryBackupLegacyArchive` |
| `memory_backup_restore_structured_archive` | `POST /api/memory/backup/restore-structured-archive` | HTTP body 为 ZIP bytes，走 `restoreMemoryBackupStructuredArchive` |
| — | `POST /api/auth/transport-tickets` | HTTP-only：Bearer 换短时、scope 受限的远程传输票据 |
| — | `POST /api/auth/preview-resource-ticket` | HTTP-only：Bearer 为单个 Canvas project / Design artifact 换子树绑定的可执行预览票据 |
| — | `GET /api/resource/{ticket}/{*path}` | HTTP-only：只读资源 allowlist 分派，供 `<img>` / iframe / 下载直链使用 |

前 12 条是 HTTP 端有路由且前端两侧都能调用、但不通过通用 `COMMAND_MAP` JSON 路径的命令；后 3 条是仅 HTTP Transport 需要的鉴权基础设施。另有 `project_fs_resolve` / `kb_file_resolve_cmd`（Tauri-only `convertFileSrc`）、`set_dock_badge_cmd`（Desktop-only Dock 数字角标）、`set_tray_unread_cmd`（Desktop-only tray 红点）与 `save_exported_file`（浏览器在客户端保存）属 Tauri 专属、无 HTTP 对应。

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
2. **Tauri 注册**：在 [`src-tauri/src/lib.rs`](../../../src-tauri/src/lib.rs) 的 `tauri::generate_handler![...]` 加命令名
3. **HTTP 路由**：在 `crates/ha-server/src/routes/<domain>.rs` 加 handler，在 [`crates/ha-server/src/lib.rs`](../../../crates/ha-server/src/lib.rs) 的 `Router::new()` 链式注册 `.route(...)`
4. **前端映射**：在 [`src/lib/transport-http.ts`](../../../src/lib/transport-http.ts) 的 `COMMAND_MAP` 加一行 `command_name: { method, path }`
5. **本文档**：在对应功能域表格追加一行（可跑 §8 的验证脚本对账）

> 例外：仅桌面有意义（快捷键、托盘、权限探测）的命令可跳过步骤 3-4，但必须在 §7.3 登记。

## 验证脚本

以下 shell 段落可在项目根运行，本文档对照表的数据正确性依赖它们：

```bash
# 1. Tauri 命令总数（截至 2026-07-30：1128）
awk 'BEGIN{flag=0} /tauri::generate_handler!\[/{flag=1;next} flag&&/^[[:space:]]*\]\)/{flag=0} flag' \
    src-tauri/src/lib.rs | grep -vE '^[[:space:]]*//|^[[:space:]]*$' | \
    grep -oE '::[a-z_][a-zA-Z0-9_]*,?[[:space:]]*$' | tr -d ':, ' | sort -u | wc -l

# 2. HTTP 路由总数（截至 2026-08-05：1068）
grep -cE '^[[:space:]]+\.route\(' crates/ha-server/src/lib.rs

# 3. COMMAND_MAP 条目数（截至 2026-07-30：1107，不含闭合 `}` 的行）
awk '/^const COMMAND_MAP/,/^};/' src/lib/transport-http.ts | \
    grep -cE '^[[:space:]]+[a-z_][a-zA-Z0-9_]*:[[:space:]]*\{'

# 4. 差集：Tauri 有、COMMAND_MAP 无（应与 §7.3 + §7.3.1 + 5 条 Tauri-only 总和一致）
comm -23 \
  <(awk 'BEGIN{flag=0} /tauri::generate_handler!\[/{flag=1;next} flag&&/^[[:space:]]*\]\)/{flag=0} flag' \
      src-tauri/src/lib.rs | grep -vE '^[[:space:]]*//|^[[:space:]]*$' | \
      grep -oE '::[a-z_][a-zA-Z0-9_]*,?[[:space:]]*$' | tr -d ':, ' | sort -u) \
  <(awk '/^const COMMAND_MAP/,/^};/' src/lib/transport-http.ts | \
      grep -oE '^[[:space:]]+[a-z_][a-zA-Z0-9_]*:' | tr -d ': ' | sort -u)
# 期望：23 行
#   check_system_permissions / request_system_permission / reset_system_permission
#   / check_all_permissions / check_permission / request_permission  （§7.3 Desktop-only）
#   / save_avatar / fs_list_dir / fs_search_files / fs_create_dir / project_fs_upload / export_session_cmd
#   / export_artifact / memory_backup_*_archive  （§7.3.1 HTTP 已实现走专用方法）
#   / stage_chat_attachment  （兼容聊天 staging 专用方法）
#   / project_fs_resolve / kb_file_resolve_cmd / set_dock_badge_cmd / set_tray_unread_cmd
#   / save_exported_file  （Tauri-only，无 HTTP；Web 在浏览器侧保存）
```

## 运行模式快速回顾

详见 [backend-separation.md](backend-separation.md)。

| 模式 | 启动命令 | 前端通信 |
|---|---|---|
| 桌面 GUI（默认） | `hope-agent` | Tauri IPC + 内嵌 HTTP 可选 |
| HTTP/WS 守护 | `hope-agent server [--bind ...] [--api-key-file ...]` | REST + WebSocket；`server token show/rotate` 用于恢复与轮换 |
| ACP stdio | `hope-agent acp` | JSON-RPC over stdio（不经本文档的接口） |

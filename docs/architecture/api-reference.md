# API 参考：Tauri ↔ HTTP/WebSocket 对照

> 返回 [文档索引](../README.md) | 关联源码：[`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs) · [`crates/ha-server/src/lib.rs`](../../crates/ha-server/src/lib.rs) · [`src/lib/transport-http.ts`](../../src/lib/transport-http.ts)

## 概述

Hope Agent 前端通过 `Transport` 抽象层和后端通信，内部根据运行环境自动在 Tauri IPC 和 HTTP/WebSocket 之间切换。本文档把两条通道上的**每一条接口**列成一一对应的表格，并标记对齐状态。

## 数据来源（截至 2026-04-24）

| 源 | 位置 | 数量 |
|---|---|---|
| Tauri 命令 | `src-tauri/src/lib.rs` 的 `tauri::generate_handler!` | **383** |
| HTTP 路由 | `crates/ha-server/src/lib.rs` 的 `.route(...)` | **387** |
| 前端 COMMAND_MAP | `src/lib/transport-http.ts::COMMAND_MAP` | **378** |
| WebSocket 端点 | `crates/ha-server/src/ws/` | **1** |
| EventBus 事件 | 全代码 `emit_event` 调用 | **52+** |

## 对齐情况摘要

| 分类 | 数量 | 说明 |
|---|---|---|
| ✅ 两端完全对齐（在 COMMAND_MAP 中） | 378 | 常规请求/响应命令（所有 COMMAND_MAP 条目都有 Tauri 命令对应） |
| 🔧 特殊 multipart 处理（不在 COMMAND_MAP 但 HTTP 已实现） | 1 | `save_avatar` |
| 🖥️ Desktop-only（Tauri 专属，HTTP 无对应） | 4 | 权限 / 沙箱本地探测 |
| ❌ HTTP 路由存在但 COMMAND_MAP 漏写 | 0 | — |
| ❌ HTTP 路由完全缺失 | 0 | — |

Tauri ↔ COMMAND_MAP 差集稳定在 5 条合法非 REST 命令（4 条 Desktop-only 权限 + `save_avatar` multipart）；HTTP 路由侧 387 - 378 = 9 条非 REST endpoint（`/api/health` / `/api/server/status` / `/api/filesystem/list-dir` / `/api/avatars/{...}` multipart / `/api/auth` 登录系列 / `/api/chat` 流式等）本身不映射到单条 Tauri 命令，它们的对齐状态在各自功能域章节单独说明。新增 Tauri 命令时须同步补 HTTP 路由 + COMMAND_MAP，保持差集不变——详见下文"新增接口 checklist"与"验证脚本"两节。

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
| `dreaming:cycle_complete` | dreaming 固化周期 |
| `cron:run_completed` | cron/executor.rs |
| `async_tool_job:completed` / `async_tool_job:failed` | 异步 tool 执行器 |

### 配置与系统

| 事件名 | 触发点 |
|---|---|
| `config:changed` | `mutate_config()` 写路径（`category: app/user/shortcuts`） |
| `weather-cache-updated` | 天气缓存刷新 |
| `agent:send_notification` | tools/notification.rs（`{ title, body }`） |
| `acp_control_event` | ACP 运行生命周期 |
| `skills:auto_review_complete` | skills 草稿审核完成 |
| `recap_progress` | `/recap` 深度复盘进度 |
| `local_model_job:created` / `:updated` / `:log` / `:completed` | 后台本地模型任务（Ollama 安装、模型拉取、Embedding 拉取）的全生命周期事件，payload 见 `LocalModelJobSnapshot` / `LocalModelJobLogEntry` |

### Canvas

| 事件名 | 触发点 |
|---|---|
| `canvas_show` / `canvas_hide` / `canvas_reload` / `canvas_deleted` | 画布面板 |
| `canvas_snapshot_request` / `canvas_eval_request` | 画布工具流 |

### MCP

| 事件名 | 触发点 | Payload 关键字段 |
|---|---|---|
| `mcp:server_status_changed` | `client.rs` set_state 之后 | `{ id, name, state, reason? }` — state ∈ `disabled`/`idle`/`connecting`/`ready`/`needsAuth`/`failed` |
| `mcp:catalog_refreshed` | `refresh_catalog` 完成 | `{ id, name, tools, resources, prompts }` 三项计数 |
| `mcp:auth_required` | OAuth 流程生成 authorize URL | `{ id, name, authUrl }` — 前端 toast + 浏览器打开 |
| `mcp:auth_completed` | OAuth 流程结束 | `{ id, name, ok: bool, error? }` |
| `mcp:servers_changed` | CRUD 写入完成 | `{}` — 触发前端重拉列表（debounced） |
| `mcp:server_log` | 预留（stderr / 生命周期） | `{ id, name, level, line }` |

### 仅 Tauri 直发（不经 EventBus）

| 事件名 | 触发点 |
|---|---|
| `new-session` / `open-settings` | 菜单与快捷键 |
| `chord-first-pressed` / `chord-timeout` / `shortcut-triggered` | 全局快捷键 |
| `slash:model_switched` / `slash:effort_changed` / `slash:plan_changed` / `slash:session_cleared` | Slash 命令副作用 |
| `project:created` / `updated` / `deleted` / `file_uploaded` / `file_deleted` | 项目 CRUD |

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
| `supportsLocalFileOps()` | `true` | `false` |
| `pickLocalImage()` | `@tauri-apps/plugin-dialog.open` | 隐藏 `<input type="file">` + blob URL |

**文件上传特殊路径**（在 `HttpTransport.call()` 中走 multipart/form-data 而非 JSON）：

| 命令 | HTTP 端点 |
|---|---|
| `save_attachment` | `POST /api/chat/attachment` |
| `upload_project_file_cmd` | `POST /api/projects/{projectId}/files` |
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
| `list_project_files_cmd` | `GET /api/projects/{projectId}/files` | ✅ |
| `upload_project_file_cmd` | `POST /api/projects/{projectId}/files` | ✅ (multipart) |
| `delete_project_file_cmd` | `DELETE /api/projects/{projectId}/files/{fileId}` | ✅ |
| `rename_project_file_cmd` | `PATCH /api/projects/{projectId}/files/{fileId}` | ✅ |
| `read_project_file_content_cmd` | `GET /api/projects/{projectId}/files/{fileId}/content` | ✅ |
| `list_project_memories_cmd` | `GET /api/projects/{id}/memories` | ✅ |

### Sessions

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_sessions_cmd` | `GET /api/sessions` | ✅ |
| `create_session_cmd` | `POST /api/sessions` | ✅ |
| `get_session_cmd` | `GET /api/sessions/{id}` | ✅ |
| `set_session_incognito` | `PATCH /api/sessions/{sessionId}/incognito` | ✅ |
| `set_session_working_dir` | `PATCH /api/sessions/{sessionId}/working-dir` | ✅ |
| `purge_session_if_incognito` | `POST /api/sessions/{sessionId}/purge-if-incognito` | ✅ |
| `search_sessions_cmd` | `GET /api/sessions/search` | ✅ |
| `search_session_messages_cmd` | `GET /api/sessions/{sessionId}/messages/search` | ✅ |
| `load_session_messages_latest_cmd` | `GET /api/sessions/{sessionId}/messages` | ✅ |
| `load_session_messages_around_cmd` | `GET /api/sessions/{sessionId}/messages/around` | ✅ |
| `load_session_messages_before_cmd` | `GET /api/sessions/{sessionId}/messages/before` | ✅ |
| `get_session_stream_state` | `GET /api/sessions/{sessionId}/stream-state` | ✅ |
| `delete_session_cmd` | `DELETE /api/sessions/{sessionId}` | ✅ |
| `rename_session_cmd` | `PATCH /api/sessions/{sessionId}` | ✅ |
| `mark_session_read_cmd` | `POST /api/sessions/{sessionId}/read` | ✅ |
| `mark_session_read_batch_cmd` | `POST /api/sessions/read-batch` | ✅ |
| `mark_all_sessions_read_cmd` | `POST /api/sessions/read-all` | ✅ |
| `compact_context_now` | `POST /api/sessions/{sessionId}/compact` | ✅ |
| `write_export_file` | `POST /api/misc/write-export-file` | ✅ |
| `get_dangerous_mode_status` | `GET /api/security/dangerous-status` | ✅ |
| `set_dangerous_skip_all_approvals` | `POST /api/security/dangerous-skip-all-approvals` | ✅ |

`create_session_cmd` 与 `chat` 在自动创建新会话时都支持可选 `incognito: boolean`，返回的 `SessionMeta` 也会包含 `incognito` 字段；主聊天 UI 将 incognito 视为“新会话预设”，只在尚未 materialize session 的草稿态提供入口，已有会话不再暴露切换按钮。`set_session_incognito` 保留给兼容调用和非主 UI 适配，但不应作为常规会话内开关使用。当请求同时带了 `project_id` 时 `incognito` 被强制为 `false`（互斥）。`list_sessions_cmd` / `search_sessions_cmd` / `list_project_sessions_cmd` 接受可选 `active_session_id` 参数：默认会过滤掉所有 incognito 会话，`active_session_id` 让正在打开的那个无痕会话仍出现在 sidebar / 搜索结果里。`purge_session_if_incognito` 在前端 `handleSwitchSession / handleNewChat / handleNewChatInProject` 切走当前 session 之前调用，仅当目标 session 当前为 incognito 时硬删，否则 no-op。

`set_session_working_dir` 接受 `{ workingDir: string | null }`，后端 `canonicalize` 路径并校验是否为存在的目录，返回 `{ updated: true, workingDir: <canonical> }`；`null` 或空串清除选择。该字段以 `SessionMeta.workingDir` 呈现，被 `system_prompt::build` 注入到 "# Working Directory" 段落（位于 Project / Project Files 之后、Memory 之前）。执行层也会把它作为 path-aware 工具的默认根：`read` / `write` / `edit` / `ls` / `grep` / `find` / `apply_patch` 的相对路径，以及 `exec.cwd` 的相对路径，均按「显式绝对路径 > Session working dir > Agent home」解析；`exec` 无 `cwd` 时再回退到用户 home。与 Project / Incognito 正交：三者可同时启用。在 HTTP 模式下前端没有原生目录选择器，改走 `GET /api/filesystem/list-dir`（见 Filesystem 域）的服务端目录浏览器。

新会话尚未 materialize 时也允许选目录：前端把选择存为 `draftWorkingDir`，首条消息发送时通过 `chat` 命令的可选 `workingDir` 字段（Tauri / `POST /api/chat` 同名）随请求带过去；后端只在自动创建 session 的分支应用，复用 `update_session_working_dir` 的 canonicalize + `is_dir` 校验，无效路径直接 400。已有 sessionId 的 `chat` 调用会忽略此字段，避免覆盖现成的工作目录设置。

### Chat

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `chat` | `POST /api/chat`；流式输出经 `/ws/events` 的 `chat:stream_delta` | ✅ |
| `stop_chat` | `POST /api/chat/stop` | ✅ |
| `set_tool_permission_mode` | `POST /api/chat/tool-permission-mode` | ✅ |
| `respond_to_approval` | `POST /api/chat/approval` | ✅ |
| `save_attachment` | `POST /api/chat/attachment` | ✅ (multipart) |
| `list_builtin_tools` | `GET /api/chat/tools` | ✅ |

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
| `set_reasoning_effort` | `POST /api/models/reasoning-effort` | ✅ |
| `get_current_settings` | `GET /api/models/settings` | ✅ |
| `get_global_temperature` | `GET /api/models/temperature` | ✅ |
| `set_global_temperature` | `POST /api/models/temperature` | ✅ |

### Agents

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_agents` | `GET /api/agents` | ✅ |
| `get_agent_template` | `GET /api/agents/template` | ✅ |
| `initialize_agent` | `POST /api/agents/initialize` | ✅ (见 §7.4 语义差异) |
| `get_agent_config` | `GET /api/agents/{id}` | ✅ |
| `save_agent_config_cmd` | `PUT /api/agents/{id}` | ✅ |
| `delete_agent` | `DELETE /api/agents/{id}` | ✅ |
| `get_agent_markdown` | `GET /api/agents/{id}/markdown` | ✅ |
| `save_agent_markdown` | `PUT /api/agents/{id}/markdown` | ✅ |
| `render_persona_to_soul_md` | `POST /api/agents/{id}/persona/render-soul-md` | ✅ |
| `get_agent_memory_md` | `GET /api/agents/{id}/memory-md` | ✅ |
| `save_agent_memory_md` | `PUT /api/agents/{id}/memory-md` | ✅ |
| `dreaming_run_now` | `POST /api/dreaming/run` | ✅ |
| `dreaming_list_diaries` | `GET /api/dreaming/diaries` | ✅ |
| `dreaming_read_diary` | `GET /api/dreaming/diaries/{filename}` | ✅ |
| `dreaming_is_running` | `GET /api/dreaming/status` | ✅ |
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
| `memory_reembed` | `POST /api/memory/reembed` | ✅ |
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

### Context compaction

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_compact_config` | `GET /api/config/compact` | ✅ |
| `save_compact_config` | `PUT /api/config/compact` | ✅ |
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
| `get_plan_steps` | `GET /api/plan/{sessionId}/steps` | ✅ |
| `update_plan_step_status` | `POST /api/plan/{sessionId}/steps/update` | ✅ |
| `get_plan_content` | `GET /api/plan/{sessionId}/content` | ✅ |
| `save_plan_content` | `PUT /api/plan/{sessionId}/content` | ✅ |
| `get_plan_file_path` | `GET /api/plan/{sessionId}/file-path` | ✅ |
| `get_plan_checkpoint` | `GET /api/plan/{sessionId}/checkpoint` | ✅ |
| `get_plan_versions` | `GET /api/plan/{sessionId}/versions` | ✅ |
| `load_plan_version_content` | `POST /api/plan/version/load` | ✅ |
| `restore_plan_version` | `POST /api/plan/{sessionId}/version/restore` | ✅ |
| `plan_rollback` | `POST /api/plan/{sessionId}/rollback` | ✅ |
| `cancel_plan_subagent` | `POST /api/plan/{sessionId}/cancel` | ✅ |
| `respond_ask_user_question` | `POST /api/ask_user/respond` | ✅ |
| `get_pending_ask_user_group` | `GET /api/plan/{sessionId}/pending-ask-user` | ✅ |
| `set_plan_subagent` | `POST /api/config/plan-subagent` | ✅ |
| `get_plan_subagent` | `GET /api/config/plan-subagent` | ✅ |
| `set_ask_user_question_timeout` | `POST /api/config/ask-user-question-timeout` | ✅ |
| `get_ask_user_question_timeout` | `GET /api/config/ask-user-question-timeout` | ✅ |

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
| `cron_get_run_logs` | `GET /api/cron/jobs/{jobId}/logs` | ✅ |
| `cron_get_calendar_events` | `GET /api/cron/calendar` | ✅ |

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

### Async / Deferred tools + Memory selection

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_async_tools_config` | `GET /api/config/async-tools` | ✅ |
| `save_async_tools_config` | `PUT /api/config/async-tools` | ✅ |
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
| `get_server_config` | `GET /api/config/server` | ✅ |
| `save_server_config` | `PUT /api/config/server` | ✅ |
| `get_server_runtime_status` | `GET /api/server/status` | ✅ (免鉴权) — 返回 `{ boundAddr, startedAt, uptimeSecs, startupError, eventsWsCount, chatWsCount, localDesktopClient, activeChatStreams, activeChatCounts: { desktop, http, channel, total } }`；`activeChatStreams` 现在等于 `activeChatCounts.total`（在跑的 `run_chat_engine` 数量），不再是 WS 订阅者数；`localDesktopClient` 在 Tauri 命令恒 `true`（桌面 webview 通过 IPC 与后端通信，不走 WS），HTTP 路由恒 `false`，前端把它计入"活跃连接" |
| `get_proxy_config` | `GET /api/config/proxy` | ✅ |
| `save_proxy_config` | `PUT /api/config/proxy` | ✅ |
| `get_shortcut_config` | `GET /api/config/shortcuts` | ✅ |
| `save_shortcut_config` | `PUT /api/config/shortcuts` | ✅ |
| `set_shortcuts_paused` | `POST /api/config/shortcuts/pause` | ✅ |
| `get_sandbox_config` | `GET /api/config/sandbox` | ✅ |
| `set_sandbox_config` | `PUT /api/config/sandbox` | ✅ |

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
| `local_llm_detect_ollama` | `GET /api/local-llm/ollama-status` | ✅ |
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

### Skills

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_skills` | `GET /api/skills` | ✅ |
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
| `dashboard_learning_overview` | `POST /api/dashboard/learning/overview` | ✅ |
| `dashboard_learning_timeline` | `POST /api/dashboard/learning/timeline` | ✅ |
| `dashboard_top_skills` | `POST /api/dashboard/learning/top-skills` | ✅ |
| `dashboard_recall_stats` | `POST /api/dashboard/learning/recall-stats` | ✅ |

### Slash commands

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `list_slash_commands` | `GET /api/slash-commands` | ✅ |
| `execute_slash_command` | `POST /api/slash-commands/execute` | ✅ |
| `is_slash_command` | `POST /api/slash-commands/is-slash` | ✅ |

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
| `channel_health` | `GET /api/channel/accounts/{accountId}/health` | ✅ |
| `channel_health_all` | `GET /api/channel/health` | ✅ |
| `channel_validate_credentials` | `POST /api/channel/validate` | ✅ |
| `channel_send_test_message` | `POST /api/channel/accounts/{accountId}/test-message` | ✅ |
| `channel_list_sessions` | `GET /api/channel/sessions` | ✅ |
| `channel_wechat_start_login` | `POST /api/channel/wechat/login/start` | ✅ |
| `channel_wechat_wait_login` | `POST /api/channel/wechat/login/wait` | ✅ |

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
| `browser_list_profiles` | `GET /api/browser/profiles` | ✅ |
| `browser_create_profile` | `POST /api/browser/profiles` | ✅ |
| `browser_delete_profile` | `DELETE /api/browser/profiles/{name}` | ✅ |
| `browser_launch` | `POST /api/browser/launch` | ✅ |
| `browser_connect` | `POST /api/browser/connect` | ✅ |
| `browser_disconnect` | `POST /api/browser/disconnect` | ✅ |

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
| `get_approval_timeout_action` | `GET /api/config/approval-timeout-action` | ✅ |
| `set_approval_timeout_action` | `POST /api/config/approval-timeout-action` | ✅ |
| `get_tool_result_disk_threshold` | `GET /api/config/tool-result-threshold` | ✅ |
| `set_tool_result_disk_threshold` | `POST /api/config/tool-result-threshold` | ✅ |
| `get_tool_limits` | `GET /api/config/tool-limits` | ✅ |
| `set_tool_limits` | `POST /api/config/tool-limits` | ✅ |

### Crash / Recovery

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `get_crash_recovery_info` | `GET /api/crash/recovery-info` | ✅ |
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
| `get_system_prompt` | `POST /api/system-prompt` | 调试端点 |

### Filesystem

| Tauri Command | HTTP | 状态 |
|---|---|---|
| `fs_list_dir` | `GET /api/filesystem/list-dir?path=<abs>` | ✅ |
| `fs_search_files` | `GET /api/filesystem/search-files?root=<abs>&q=<query>&limit=50` | ✅ |

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

截至 2026-04-24 三端差集仍稳定为 5 条（§7.3 的 4 条 Desktop-only + `save_avatar` multipart），没有"HTTP 漏写 COMMAND_MAP"或"HTTP 路由缺失"的破口。COMMAND_MAP 里的每一条都能在 `tauri::generate_handler!` 里找到对应命令；反向差 5 条均已在下表登记。

### §7.3 Desktop-only（Tauri 专属，合法缺失，4 条）

| Tauri Command | 说明 |
|---|---|
| `check_all_permissions` | `tauri-plugin-permissions` 本地权限查询 |
| `check_permission` | 同上 |
| `request_permission` | 同上 |
| `check_sandbox_available` | Linux bubblewrap 本地探测 |

前端必须在 `supportsLocalFileOps()` 或等价的运行模式判定保护下调用，HTTP 模式应 gate 住相关 UI。

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
# 1. Tauri 命令总数（截至 2026-04-24：383）
awk 'BEGIN{flag=0} /tauri::generate_handler!\[/{flag=1;next} flag&&/^[[:space:]]*\]\)/{flag=0} flag' \
    src-tauri/src/lib.rs | grep -vE '^[[:space:]]*//|^[[:space:]]*$' | \
    grep -oE '::[a-z_][a-zA-Z0-9_]*,?[[:space:]]*$' | tr -d ':, ' | sort -u | wc -l

# 2. HTTP 路由总数（截至 2026-04-24：387）
grep -cE '^[[:space:]]+\.route\(' crates/ha-server/src/lib.rs

# 3. COMMAND_MAP 条目数（截至 2026-04-24：378，不含闭合 `}` 的行）
awk '/^const COMMAND_MAP/,/^};/' src/lib/transport-http.ts | \
    grep -cE '^[[:space:]]+[a-z_][a-zA-Z0-9_]*:[[:space:]]*\{'

# 4. 差集：Tauri 有、COMMAND_MAP 无（应与 §7.3 + save_avatar 总和一致）
comm -23 \
  <(awk 'BEGIN{flag=0} /tauri::generate_handler!\[/{flag=1;next} flag&&/^[[:space:]]*\]\)/{flag=0} flag' \
      src-tauri/src/lib.rs | grep -vE '^[[:space:]]*//|^[[:space:]]*$' | \
      grep -oE '::[a-z_][a-zA-Z0-9_]*,?[[:space:]]*$' | tr -d ':, ' | sort -u) \
  <(awk '/^const COMMAND_MAP/,/^};/' src/lib/transport-http.ts | \
      grep -oE '^[[:space:]]+[a-z_][a-zA-Z0-9_]*:' | tr -d ': ' | sort -u)
# 期望：5 行（check_all_permissions / check_permission / request_permission
#         / check_sandbox_available / save_avatar）
```

## 运行模式快速回顾

详见 [backend-separation.md](backend-separation.md)。

| 模式 | 启动命令 | 前端通信 |
|---|---|---|
| 桌面 GUI（默认） | `hope-agent` | Tauri IPC + 内嵌 HTTP 可选 |
| HTTP/WS 守护 | `hope-agent server [--bind ...] [--api-key ...]` | REST + WebSocket |
| ACP stdio | `hope-agent acp` | JSON-RPC over stdio（不经本文档的接口） |

# Hope Agent

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，内置丰富 Provider 模板与预设模型，GUI 傻瓜式配置。三种运行模式：桌面 GUI（Tauri）、HTTP/WS 守护进程（`hope-agent server`）、ACP stdio（`hope-agent acp`）。

子系统设计与实现细节见 [`docs/architecture/`](docs/architecture/)；本文只列**影响每个 PR 的契约和红线**，不重复实现细节。

## 开发命令

```bash
pnpm tauri dev         # 启动开发模式（前端 + Tauri 热重载）
pnpm dev               # 仅前端 Vite 开发服务器
pnpm tauri build       # 构建生产包
pnpm sync:version      # 以 package.json 为单一来源，同步 src-tauri 版本号
pnpm release:verify    # 校验 package.json / src-tauri 版本一致；可附 -- --tag vX.Y.Z
pnpm typecheck         # 前端类型检查（tsc -b）
pnpm lint              # Lint
pnpm test              # Vitest（一次性跑完）
pnpm test:watch        # Vitest watch 模式
node scripts/sync-i18n.mjs --check   # 检查各语言翻译缺失
node scripts/sync-i18n.mjs --apply   # 从翻译文件补齐缺失翻译

# Server 模式（HTTP/WS 守护进程）
hope-agent server start              # 前台启动 HTTP/WS 服务
hope-agent server install            # 注册系统服务（macOS launchd / Linux systemd）
hope-agent server uninstall          # 卸载系统服务
hope-agent server status             # 查看服务运行状态
hope-agent server stop               # 停止服务
```

## 提交前检查（强制）

`git push` 前**必须**本地跑以下六条；`pnpm install` 后 [`.husky/pre-push`](.husky/pre-push) 钩子按这个顺序自动跑：

```bash
cargo fmt --all --check                                                    # CI: rust.yml fmt
cargo clippy -p ha-core -p ha-server --all-targets --locked -- -D warnings # CI: rust.yml clippy
cargo test  -p ha-core -p ha-server --locked                               # CI: rust.yml test
pnpm typecheck                                                              # CI: lint.yml tsc
pnpm lint                                                                    # CI: lint.yml ESLint
pnpm test                                                                    # CI: lint.yml Vitest
```

- **clippy / test 只覆盖 `ha-core` + `ha-server`**（CI 也是如此）；`src-tauri` 不在钩子内，tauri-specific 问题用 `cargo {clippy,test} --workspace` 自查
- **Rust 版本**由 [`rust-toolchain.toml`](rust-toolchain.toml) 固定，本地 / CI 共用
- **应急开关**：`HA_SKIP_PREPUSH=1`（整段跳过，仅限纯 `.md` / 弱网紧急）/ `HA_SKIP_PREPUSH_TEST=1`（只跳 cargo test）。**禁止 `--no-verify`**——会绕过 GPG 等其它钩子

### Agent 开发期检查行为（强制）

上面六条是 push 前兜底，**Agent 在开发过程中不要主动跑全套检查**：

- **改代码过程中**：默认只做单点验证——Rust 用 `cargo check -p <crate>`，TS/TSX 用 `pnpm typecheck`；不要主动跑 clippy / cargo test / pnpm test / pnpm lint
- **想跑全套必须先问**：判断需要跑这四项之一时，先问用户「是否要跑 X？」并说明原因，等回复再跑
- **长任务收尾例外**：跨多文件多模块 / 完整 plan / 跨 crate 重构这类阶段性收尾时，可主动跑必要项，跑前说一句"改动较大，跑一下 X 收尾"
- **commit / push 前 / 用户明确要求**：直接按指示跑

## 项目结构

```
Cargo.toml              Workspace 根（members: crates/ha-core, crates/ha-server, src-tauri）
crates/
  ha-core/              核心业务逻辑（零 Tauri 依赖，纯 Rust 库）
  ha-server/            HTTP/WS 服务器（axum，REST API + WebSocket 流式推送）
src-tauri/              Tauri 桌面 Shell（薄壳，调用 ha-core）
src/                    前端（React + TypeScript）
  components/           chat/ settings/ dashboard/ cron/ common/ ui/ 等
  lib/                  Transport 抽象层：transport.ts + transport-tauri.ts + transport-http.ts
  i18n/locales/         12 种语言翻译文件
skills/                 内置技能（meta / 编程方法论 vendor / 办公方法论原创）
docs/architecture/      子系统设计文档（39 篇，跨 PR 必读单一真相源）
```

ha-core 主要领域：`agent/` `chat_engine/` `context_compact/` `memory/` `skills/` `tools/` `channel/` `subagent/` `team/` `cron/` `acp/` `dashboard/` `recap/` `awareness/` `config/` `session/` `project/` `plan/` `ask_user/` `async_jobs/` `failover/` `platform/` `security/` `logging/` `local_llm/`。Vendor skill 来源记录在 `THIRD_PARTY_NOTICES.md`。

## 技术栈

| 层     | 技术                                                                 |
| ------ | -------------------------------------------------------------------- |
| 前端   | React 19 + TypeScript, Vite 8, Tailwind CSS v4, shadcn/ui (Radix UI) |
| 桌面   | Tauri 2                                                              |
| 服务器 | axum (HTTP/WS), clap (CLI)                                           |
| 后端   | Rust, tokio, reqwest（ha-core 库，零 Tauri 依赖）                    |
| 渲染   | Streamdown + Shiki + KaTeX + Mermaid                                 |
| 多语言 | i18next (12 种语言)                                                  |

## 架构契约

每个子系统的细节都在对应 `docs/architecture/<name>.md`；本节只列跨 PR 必守的契约和红线。

### 分层 & 运行模式

详见 [`process-model.md`](docs/architecture/process-model.md) / [`backend-separation.md`](docs/architecture/backend-separation.md) / [`transport-modes.md`](docs/architecture/transport-modes.md)。

- **三 Crate 架构**：业务逻辑全进 `ha-core`（**零 Tauri 依赖**），`ha-server` 与 `src-tauri` 只做适配薄壳
- **EventBus + CoreState**：核心层用 `ha-core::EventBus` 替代 Tauri `APP_HANDLE`；状态走 `CoreState`，Tauri 用 `State<AppState>`，server 用 axum `Extension`
- **Transport 抽象**：前端走 [`src/lib/transport.ts`](src/lib/transport.ts)，**新 invoke 必须同时实现 Tauri + HTTP 两套适配**
- **桌面 release 单一来源**：`package.json`，`pnpm version` 钩子同步 `src-tauri/Cargo.toml` 与 `tauri.conf.json`；CI tag 构建前跑 `pnpm release:verify -- --tag vX.Y.Z`。Updater 私钥严禁入仓
- **API Key 鉴权**：HTTP/WS 走 Bearer Token（[`ha-server/middleware.rs`](crates/ha-server/src/middleware.rs)），`/api/health` 免鉴权；浏览器 WS 用 `?token=` 兼容
- **运行模式 getter**：`ha_core::runtime_role()` / `is_desktop()`，避免给共享函数加 mode 参数

### LLM 主对话

详见 [`provider-system.md`](docs/architecture/provider-system.md) / [`failover.md`](docs/architecture/failover.md) / [`side-query.md`](docs/architecture/side-query.md)。

- **Provider**：4 种（Anthropic / OpenAIChat / OpenAIResponses / Codex）
- **新会 spawn tool loop 的 chat 路径**走 `chat_engine::run_chat_engine`，不要绕过自包 `on_delta`
- **failover policy 三档**：`chat_engine_default` / `side_query_default` / `summarize_default`；**Codex 强制不参与 profile 轮换**
- **温度 / Think 三层覆盖**：会话 > Agent > 全局；`thinking_style` Provider 默认 + 模型级覆盖
- **Side Query**：复用主对话 system_prompt + history 前缀命中 cache，Tier 3 摘要 / 记忆提取成本降 ~90%

### Chat Engine & Streaming

详见 [`chat-engine.md`](docs/architecture/chat-engine.md)。

- **聊天流双写**：per-call `EventSink`（主路径）+ EventBus `chat:stream_delta`（带 `seq` 去重，重载恢复用）；IM 渠道独立 `channel:stream_delta`
- **API-Round 分组**：assistant + tool_result 通过 `_oc_round` 元数据成对，压缩切割对齐 round 边界；API 调用前 `prepare_messages_for_api()` 剥离元数据

### 上下文压缩

详见 [`context-compact.md`](docs/architecture/context-compact.md)。

- 5 层渐进式 + `ContextEngine` / `CompactionProvider` trait 可插拔
- `compact.cacheTtlSecs`（默认 300s）节流 Tier 2+，使用率 ≥ 95% 强制覆盖
- 反应式微压缩：每轮末尾使用率 ≥ `reactiveTriggerRatio`（默认 0.75）触发 Tier 0 清旧 tool 结果（cache-safe）
- Tier 3 摘要后自动注入最近 write/edit/apply_patch 的文件当前内容（最多 5 × 16KB）

### Memory

详见 [`memory.md`](docs/architecture/memory.md)。

- **优先级**：Project > Agent > Global，唯一入口 `effective_memory_budget(agent, global)`
- **`recall_memory` / `memory_get` 工具返回完整原文**，预算只约束 system prompt 注入
- Active Memory / Awareness / User Profile 都作为**独立 cache block** 注入，不作废静态前缀缓存
- **会话级无痕（`sessions.incognito`）**：单一真相源；不注入 Memory / Active Memory / Awareness、跳过自动提取；**关闭即焚**——不进侧边栏列表 / 全局 FTS / Dashboard 统计；**与 Project / IM Channel 互斥**（前端灰化 + 后端入口防御）

### 工具 & 审批

详见 [`permission-system.md`](docs/architecture/permission-system.md) / [`tool-system.md`](docs/architecture/tool-system.md)。

- **统一权限引擎 v2**：所有调用走 `permission::engine::resolve_async()`，优先级 **Plan > Internal > YOLO > Protected/Dangerous > AllowAlways > Session 模式 preset > 兜底 Allow**
- **Session 模式三选一**：`default | smart | yolo`，`PermissionModeSwitcher` / `/permission` 切换；`AgentConfig.capabilities.default_session_permission_mode` 决定新会话初始 mode
- **Smart 模式忽略 `custom_approval_tools`**——UI 必须显式提示
- **保护路径 / 危险命令 / 编辑命令**：三独立列表，存 `~/.hope-agent/permission/*.json`；非 YOLO 模式强制弹窗（AllowAlways 按钮置灰），YOLO 只 `app_warn!` 不弹
- **Global YOLO**：CLI flag `--dangerously-skip-all-approvals` 与 `permission.global_yolo` OR 组合；判定入口 `security::dangerous::is_dangerous_skip_active()`，**与 Plan Mode 正交**
- **审批超时**：`approval_timeout_secs`（默认 300s，`0` 不限）+ `approval_timeout_action ∈ deny|proceed`
- **Agent 工具过滤**：`AgentConfig.capabilities.tools` 在 system_prompt / schemas / tool_search / 执行层统一生效，internal 工具始终保留
- **工具结果磁盘持久化**：> `toolResultDiskThreshold`（默认 50KB）写盘，上下文留 head+tail 预览
- **异步 Tool 执行**：`exec` / `web_search` / `image_generate` 标 `async_capable=true`；落 `~/.hope-agent/async_jobs.db` + spool；`job_status` deferred 工具主动 poll/wait
- **SSRF 统一策略**：出站 HTTP 必须走 `security::ssrf::check_url`；**新出站入口严禁自写 IP 校验**
- **文件 Diff 元数据**：`write` / `edit` / `apply_patch` / `read` 通过 `ToolExecContext.metadata_sink` 旁路传出 JSON；持久化到 `messages.tool_metadata` 列；前端右侧 `DiffPanel` 渲染（与 PlanPanel / CanvasPanel 视觉互斥）

### Plan Mode

详见 [`plan-mode.md`](docs/architecture/plan-mode.md)。

- **5 状态机**：`Off / Planning / Review / Executing / Completed`，**没有 Paused**——挂起就 `/plan exit`
- **进入永远由用户拍板**：UI 按钮 / `/plan enter` / `set_plan_mode` Tauri / HTTP 是用户主动入口直接转 state；模型用 `enter_plan_mode` 工具走 `ask_user_question` Yes/No 审批，**模型不能自己转 state**
- **plan = 设计契约**（自由 markdown，存 `~/.hope-agent/plans/<agent>/<session>/`）；**task = 唯一进度真相**（`task_create` / `task_update` 三态）；**执行期不改 plan 文件**
- **Plan 完成自动转 Completed**：plan 期 task 全部终态时 `maybe_complete_plan` 收尾，按 `PlanMeta.executing_started_at` 切片避免误触发
- **git checkpoint**：审批转 Executing 时建，`Completed` / `Off` 时清（`Completed` 须显式清 `meta.checkpoint_ref`）
- **Plan 执行层兜底**：`resolve_tool_permission` 入口加 live state fallback，防 mid-turn 调 `enter_plan_mode` 后剩余工具绕过

### Skill 系统

详见 [`skill-system.md`](docs/architecture/skill-system.md)。

- **优先级**：bundled < extra < managed < project
- **激活入口**：`skill({name, args?})` 工具（`internal + always_load`）；斜杠 `/skillname args` 内联走 `[SYSTEM: ...]` + `display_text`（**当前未应用 `allowed-tools` / `check_requirements`**）
- **SKILL.md 字段**：`context: fork` 起子 session（可带 `agent:` / `effort:`）；`allowed-tools:` 白名单工具；`paths:` 条件激活默认不进 catalog；`status: active|draft|archived`，面向模型路径跳过非 active
- **Draft 审核**：`skills::author` CRUD + Jaccard 0.80 模糊 patch + `security_scan`；`auto_review.enabled=true / promotion=draft` 等用户确认

### MCP 客户端

详见 [`mcp.md`](docs/architecture/mcp.md)。

- 4 种 transport（stdio / Streamable HTTP / SSE / WebSocket），网络 transport 必须先过 SSRF 检查
- 命名空间 `mcp__<server>__<tool>`；工具默认 `deferred=true`，白名单 `always_load_servers` 强制 always_load
- OAuth 2.1 + PKCE 自实现（不用 `rmcp::auth_client`）；凭据 0600 落 `~/.hope-agent/credentials/mcp/{id}.json`
- **配置读写 contract**：读 `cached_config().mcp_servers`；写 `mutate_config(("mcp.<op>", source), ...)`，`op ∈ add|update|remove|reorder|settings`
- handshake 401/403 → `ServerState::NeedsAuth`（避免 watchdog 死循环）

### Subagent / Team / Cron

详见 [`subagent.md`](docs/architecture/subagent.md) / [`agent-team.md`](docs/architecture/agent-team.md) / [`cron.md`](docs/architecture/cron.md)。

- `subagent(action="spawn_and_wait")` 前台等待 `foreground_timeout`（默认 30s），超时自动转后台
- Agent Team 模板 GUI 预配 + 模型按需发现；`TeamTemplateMember.description` 注入子 session 身份段
- Cron `delivery_targets`：final assistant text fan-out 到 IM；IM 会话内未显式传时自动取当前会话，显式 `[]` 关闭

### IM Channel

详见 [`im-channel.md`](docs/architecture/im-channel.md)。

- 12 个插件，状态文件落 `~/.hope-agent/channels/`；入站媒体走 plug → worker → `Attachment` → `~/.hope-agent/attachments/{session_id}/`
- 工具审批通过 EventBus `approval_required` 监听，按 `supports_buttons` 走原生按钮或文本；`auto_approve_tools=true` 跳审批
- **Auto-start 失败统一走 [`channel/start_watchdog.rs`](crates/ha-core/src/channel/start_watchdog.rs)**——退避 30s/60s/2m/5m，sweep 15s，user 操作永远胜过 watchdog；失败日志带 `classify_channel_error` 分类
- **流式预览 Transport 三选一**（[`worker/streaming.rs`](crates/ha-core/src/channel/worker/streaming.rs) `select_stream_preview_transport`）：`Draft (Telegram DM 专属) > Card (capabilities.supports_card_stream，目前仅飞书 cardkit) > Message (send_message+edit_message)`。Card / Draft 失败有降级（Card 创建期失败 → 切 Message；中后期 `update_card_element` 失败 → `broken=true`，收尾走 `send_message` 兜底）。新增飞书风格"无编辑标记"流式靠 `ChannelPlugin` 上 4 个 default-impl=`Err` 的 cardkit trait 方法（`create_card_stream` / `send_card_message` / `update_card_element` / `close_card_stream`）—— 仅飞书实现，11 个非飞书 channel 的 `capabilities.supports_card_stream=false` 走旧路径不变
- **`ImReplyMode` 三态对所有渠道生效**（`ChannelAccountConfig.settings.imReplyMode`，默认 `split`，[`channel/types.rs`](crates/ha-core/src/channel/types.rs)）：
  - `split`（默认）：每 round 的 narration + 媒体按时序作为独立消息发送；流式渠道每条仍带打字机效果（每 round 起独立消息，不再是「一条不断增长」），媒体在产生它的 round 之后立即发
  - `final`：丢弃中间 round narration，只发最后 round 的 text + 末尾发所有媒体；不启用流式预览
  - `preview`：流式渠道用 preview transport 渲染合并文本（旧行为）；非流式渠道降级为 `final`

  实现要点：
  - **stream task transport 由 mode 决定**——dispatcher 在 spawn stream task 之前算 `account.im_reply_mode()`，仅 `Preview` 模式调 `select_stream_preview_transport`，其他模式给 stream task 传 `None`（drain events 不创建 preview）
  - **round 边界 + 媒体归组**靠 `ChannelStreamSink::round_texts: RoundTextAccumulator<RoundOutput { text, medias }>`（[`chat_engine/types.rs`](crates/ha-core/src/chat_engine/types.rs)）。state machine：`text_delta → current.text`；`tool_call → close round`（idempotent，多 tool_call 同 round 只关一次）；`tool_result(media) → 挂到刚关闭的 round`
  - **dispatcher** 在 [`channel/worker/dispatcher.rs`](crates/ha-core/src/channel/worker/dispatcher.rs) 按 mode 调 `deliver_split` / `deliver_final_only` / `deliver_preview_merged` 三选一；split 路径每 round 走 `send_message(text)` + `deliver_media(round.medias)`，最后 round 通过 `send_final_reply` 走 finalize 路径
- **配置入口**：GUI（`EditAccountDialog` 三选项 Select，`preview` 选到非流式 channel 时 hint「will degrade to Final」）+ `/imreply [split|final|preview]` 斜杠命令
- **`ChannelStreamSink` 短路条件用 `contains` 不能用 `starts_with`**：`emit_tool_result` 走 `serde_json::json!({...})` + 默认 `BTreeMap`，键按字母序输出（`call_id` 永远在前），任何 anchor 在 `{"type":...` 的 fast-path 都不会触发。`media_items` / `tool_result` / `text_delta` / `tool_call` 检测都用 `event.contains(...)`，rarer-needle-first

### Dashboard / Recap / Learning

详见 [`dashboard.md`](docs/architecture/dashboard.md) / [`recap.md`](docs/architecture/recap.md)。

- `dashboard/insights.rs`：overview delta / cost trend / heatmap / health score / `query_insights` orchestrator
- Learning Tracker 落 `session.db.learning_events`，目前埋点：`skills::author` CRUD + `tool_recall_memory` 命中 + MCP tool 调用
- `/recap` 独立 `~/.hope-agent/recap/recap.db` 缓存按 `last_message_ts` 失效；`recap.analysisAgent` 与主对话 Agent 解耦

### 跨会话 / 全局

详见 [`session.md`](docs/architecture/session.md) / [`behavior-awareness.md`](docs/architecture/behavior-awareness.md) / [`ask-user.md`](docs/architecture/ask-user.md) / [`prompt-system.md`](docs/architecture/prompt-system.md)。

- **数据存储**：所有数据在 `~/.hope-agent/`，[`paths.rs`](crates/ha-core/src/paths.rs) 集中管理
- **统一日志**：前后端走 [`logging.rs`](crates/ha-core/src/logging.rs)（SQLite + 文本双写），API 请求体 `redact_sensitive` + 32KB 截断；agent 自主排查入口见 [`skills/ha-logs/SKILL.md`](skills/ha-logs/SKILL.md)（用 `exec` + `sqlite3 -readonly` 直查 `~/.hope-agent/{logs,sessions,async_jobs}.db`）
- **延迟工具加载**：opt-in `deferredTools.enabled`，只发核心 ~10 个 schema，其余通过 `tool_search` 发现；execution dispatch 不变
- **会话搜索**：FTS5 + `<mark>` 高亮 + XSS 防御（escape → 白名单反解）；`Cmd+F` 复用同一 `search_messages` + session_id 过滤
- **ask_user_question**：1–4 题结构化问答（单选/多选/输入）；pending 持久化 SQLite，App 重启 replay 断点续答；IM 按 `supports_buttons` 走按钮或文本
- **会话级工作目录**：`sessions.working_dir` 注入 system_prompt `# Working Directory` 段；**v1 只做 prompt 注入**，不改 `exec`/`read_file` 的 cwd 解析
- **桌面专属 markdown 路径链接**：仅 `is_desktop()` 注入 `MARKDOWN_PATH_LINKS_GUIDANCE`，要求 LLM 写 `[名](绝对路径)`；前端按 `isLocalPath()` + Transport 分流（Tauri 走 `open_directory`；HTTP/server 早返回禁用）。**例外**：anchor `title` 用 native HTML 不用 shadcn Tooltip（一条流式消息可能渲染上百个）

### 项目（Project）容器

详见 [`project.md`](docs/architecture/project.md)。

- system_prompt 三层注入：目录清单永远注入 → 小文件（<4KB）8KB 预算内联 → `project_read_file` 工具按需
- 记忆优先级 Project > Agent > Global
- **默认工作目录合并**：优先级 `session > project > 不注入`，**lazy resolve**；唯一入口 [`session/helpers.rs::effective_session_working_dir`](crates/ha-core/src/session/helpers.rs)，写校验入口 [`util.rs::canonicalize_working_dir`](crates/ha-core/src/util.rs)
- 删除级联：unassign → 删 `projects` + `project_files`（FK） → `rm -rf projects/{id}/` → 删项目记忆（跨 db 单独执行）
- **绑定 IM Channel**：`Project.bound_channel` 让一个项目认领 (channel, account)，channel worker 创建会话时反查注入；同一 (channel, account) 只能被一个项目认领

### Agent 解析链（默认 Agent）

5 级：**显式参数 → `project.default_agent_id` → `channel_account.agent_id` → `AppConfig.default_agent_id` → 硬编码 `"default"`**。统一 helper：[`agent/resolver.rs::resolve_default_agent_id`](crates/ha-core/src/agent/resolver.rs)（带来源 tag 版本给 `/status`）。

### 本地 LLM 助手

详见 [`local-model-loading.md`](docs/architecture/local-model-loading.md)。

- 后端锁 Ollama（OpenAI 兼容端点）；模型目录硬编码 [`local_llm/types.rs::model_catalog`](crates/ha-core/src/local_llm/types.rs)，按预算从大到小取首个 ≤ budget
- 预算：macOS 统一内存 50% / Win+Linux dGPU VRAM 50% 优先回落系统内存 50%，扣 1 GiB runtime
- App **不接管** Ollama 进程；安装走官方 `install.sh`（macOS+Linux），Windows 引导用户去官网
- 后台任务统一走 [`local_model_jobs.rs`](crates/ha-core/src/local_model_jobs.rs) + `~/.hope-agent/local_model_jobs.db`
- **Provider 写入 contract（强制）**：所有 Provider 列表与 `active_model` 写入必须走 [`provider/crud.rs`](crates/ha-core/src/provider/crud.rs) helper（`add_provider` / `update_provider` / `delete_provider` / `reorder_providers` / `set_active_model` / `add_and_activate_provider` / `add_many_providers` / `ensure_codex_provider_persisted`）；本地 LLM 安装走 `upsert_known_local_provider_model`。**禁止直接 `providers.push` / `retain` / 手写 `active_model`**
- **Known local backend catalog** 在 [`provider/local.rs`](crates/ha-core/src/provider/local.rs)（ollama / litellm / vllm / lm-studio / sglang）；前端"是否已配本地后端"必须消费 catalog，**禁止硬编码 regex**

## 编码规范

### 通用

- **性能和用户体验是最高优先级**
- **核心逻辑必须在 ha-core 实现**：业务、数据、文件 IO、状态管理一律放 `crates/ha-core/`，`src-tauri/` / `crates/ha-server/` 只做薄壳，前端只负责展示和交互
- 操作即时反馈（乐观更新、loading 态），动效 60fps（优先 CSS transform/opacity）

### 前端

- 函数式组件 + hooks，不用 class 组件
- UI 组件统一用 `src/components/ui/`（shadcn/ui），不直接用 HTML 原生表单组件
- 样式只用 Tailwind utility class，不写行内 style 和自定义 CSS
- 动效优先复用 shadcn/ui / Radix UI / Tailwind 内置 utility，确认不够用才手写
- 路径别名：`@/` → `src/`
- 布局避免硬编码过小的 max-width（如 `max-w-md`），用 `max-w-4xl` 以上或弹性伸缩
- **i18n 当次改动涉及的翻译 key 必须 commit 时全 12 语言齐全**（存量缺失不强制）
- 避免不必要的重渲染（`React.memo` / `useMemo` / `useCallback`）
- **Tooltip 必须用 [`@/components/ui/tooltip`](src/components/ui/tooltip.tsx)**，禁止用 HTML 原生 `title`；优先 `<IconTip label={...}>`
- **保存按钮统一三态**：`saving`（Loader2 旋转 + disabled）→ `saved`（绿 + Check，2s 恢复）→ `failed`（红，2s 恢复），用 `saveStatus: "idle"|"saved"|"failed"` + `saving: boolean`
- **Think / Tool 流式块**：必须设合理 `max-height` 内部滚动；流式期间自动滚到底；实时显示耗时（结束保留最终耗时）

### 后端（Rust）

- 新功能放 `crates/ha-core/` 单独模块；Tauri 命令在 `src-tauri/src/lib.rs` 注册，HTTP 路由在 `crates/ha-server/src/router.rs` 注册
- 内部用 `anyhow::Result`；Tauri 命令边界用 `Result<T, CmdError>`（[`error.rs`](src-tauri/src/commands/error.rs)），`?` 直接传 `anyhow::Error`，不要 `.map_err(|e| e.to_string())`；HTTP 路由按 axum 习惯返 `Result<Json<T>, (StatusCode, String)>`
- 异步命令加 `async`，不要 `block_on`
- **禁止 `log` crate 宏**，必须用 `app_info!` / `app_warn!` / `app_error!` / `app_debug!`（[`logging.rs`](crates/ha-core/src/logging.rs)）。例外：`lib.rs::run()` 中 AppLogger 初始化前 + `main.rs` panic 恢复
- 用法：`app_info!("category", "source", "message {}", arg)`
- **核心业务路径必须埋点**（Provider 调用 / tool 执行 / 审批决策 / failover / compaction / channel / 记忆 / cron / 配置变更等）。日志服务人工排查，也是 **agent 自主修复**的首要信息源——带最小复现上下文，`category` / `source` 命名稳定便于 grep
- **禁止字节索引切片字符串**（如 `&s[..80]`），用 `crate::truncate_utf8(s, max_bytes)`
- **跨平台分支**：优先 `#[cfg(unix)]` / `#[cfg(windows)]`（macOS+Linux+BSD 共享 Unix 路径）。新跨平台原语统一放 [`platform/`](crates/ha-core/src/platform/)（`mod.rs` 门面 / `unix.rs` / `windows.rs`），调用方走 `crate::platform::xxx()` 单一入口

## 安全红线

- **API Key / OAuth Token 禁止出现在任何日志中**
- `tauri.conf.json` CSP 当前为 `null`，不要放行外部域名
- OAuth token 在 `~/.hope-agent/credentials/auth.json`，登出时必须 `clear_token()`

## 易错提醒

- 修改 Tauri 命令后须同步更新 `invoke_handler!` 注册列表
- 新增 HTTP 端点须在 `crates/ha-server/src/router.rs` 注册
- 新增核心功能须放 `crates/ha-core/`，禁止在 ha-core 中引入 Tauri 依赖
- Rust 依赖变更后 `cargo check --workspace` 先行验证
- 前端新增 invoke 调用须同步实现 Transport 的 Tauri + HTTP 两套适配
- 新增/修改接口须同步更新 [`api-reference.md`](docs/architecture/api-reference.md)（Tauri ↔ HTTP 对齐单一真相源）

## 设置（Settings）约定

所有用户可操作的配置必须同时具备 **GUI 入口** 和 **`ha-settings` 技能对应能力**，两者零偏差。新增/修改进入 `AppConfig` / `UserConfig` 且用户需要调整的字段时，**同一 PR 内三件事缺一不可**：

1. **GUI 控件**：[`src/components/settings/`](src/components/settings/) 对应面板，shadcn/ui + 三态保存按钮
2. **技能能力**：[`tools/settings.rs`](crates/ha-core/src/tools/settings.rs) 加读写分支 + 风险分级 + 副作用提示；同步更新 [`core_tools.rs`](crates/ha-core/src/tools/definitions/core_tools.rs) 的 `category` enum；含凭据需 read-only 的，加到 `BLOCKED_UPDATE_CATEGORIES` + `read_category` redact
3. **技能文档**：在 [`skills/ha-settings/SKILL.md`](skills/ha-settings/SKILL.md) 风险等级表登记

### 风险等级

- **LOW**：UI 偏好、显示配额（theme / language / notification / canvas 等）
- **MEDIUM**：行为调整，影响上下文 / 成本 / 输出质量（compact / memory_* / web_search / approval / multimodal / dreaming 等）
- **HIGH**：安全 / 网络暴露 / 全局键位 / 凭据 / 需要重启 / 权限规则 / 审批策略 / MCP 子系统级开关（proxy / embedding / shortcuts / server / skill_env / acp_control / `permission.global_yolo` / `smart_mode` / `mcp_global` / `protected_paths` / `dangerous_commands` 等）——技能在 `update_settings` 前**必须二次确认**

### 强制留 GUI 的例外（read-only via skill）

四类不进 `update_settings`（凭据安全 + 运行时稳定性）：**Provider 列表与 API Key**、**IM Channel 账号（`channels`）**、**MCP 服务器配置（`mcp_servers`）**、**`active_model` / `fallback_models` 写入**。`get_settings` 仍可读但敏感字段 redact（`channels.accounts[*].credentials/settings`、`mcp_servers.env/headers/oauth`）。

### 含凭据 category 的 read 脱敏（write 仍允许）

下列 category 允许 `update_settings`，但 `get_settings` 必须 redact 凭据字段，避免 LLM 把 history 当 leak 通道。**所有新增带凭据子字段的 `AppConfig` field 必须接入 [`tools::settings::redact_*_value`](crates/ha-core/src/tools/settings.rs) 同款 helper**：

- `web_search` — `providers[*].apiKey` / `apiKey2`
- `image_generate` — `providers[*].apiKey`
- `server` — `apiKey`（HTTP/WS Bearer Token）
- `acp_control` — `backends[*].env` 整张 map
- `skill_env` — secret 容器，技能层二次确认警示已在 SKILL.md

判定规则：read 时仅 `Some(non_empty_string)` 视为 secret 用 `"[REDACTED]"` 覆盖；`None` / 缺字段 / `Some("")` 保留原状（区分"未设"与"已设但被清空"）。

### 配置读写 contract（强制）

详见 [`config-system.md`](docs/architecture/config-system.md)。

- **读** 走 `ha_core::config::cached_config()`（`Arc<AppConfig>` 快照），禁止重新引入 `Mutex<AppConfig>` 或本地克隆
- **写** 走 `ha_core::config::mutate_config((category, source), |cfg| {...})`，禁止 `load_config()` + `save_config()` 手动克隆-改-存（无法防并发 lost-update）
- 写路径自动 emit `config:changed` 并落 autosave 备份，不要手动模拟

## 文档维护

技术文档索引见 [`docs/README.md`](docs/README.md)（`docs/architecture/` 架构 + `docs/research/` 调研）。

| 改动类型                                            | 需更新                                                                |
| --------------------------------------------------- | --------------------------------------------------------------------- |
| 新增/删除功能、命令、模块                           | `CHANGELOG.md`、`AGENTS.md`                                           |
| 技术栈/架构/规范变更                                | `AGENTS.md`                                                           |
| 已有子系统架构变更                                  | `docs/architecture/` 对应文档                                         |
| 新增架构级能力                                      | `docs/architecture/` 新建文档 + `docs/README.md` 索引                 |
| 新增/删除 Tauri 命令、HTTP 路由、`COMMAND_MAP` 条目 | [`api-reference.md`](docs/architecture/api-reference.md) 对应表格     |
| 功能变化导致 README 过时                            | `README.md` + `README.en.md`（同一 PR 双语同步）                      |
| 新增调研/对比分析                                   | `docs/research/` 新建调研文档                                         |
| 修改 README 任一语言版本                            | 同一 PR 同步另一语言（`README.md` ↔ `README.en.md`）                  |
| 新增/修改 Release Notes                             | 同一 PR 内中英双份（`docs/release-notes/vX.Y.Z.md` ↔ `vX.Y.Z.en.md`） |

- **AGENTS.md 是契约面**——只放跨 PR 必守的规则、红线、文件入口；**实现细节、内部数据结构、迁移逻辑、边角行为一律下沉到对应 architecture 文档**
- **架构文档强制**：子系统边界 / 数据流 / 持久化格式 / 跨模块 contract 改动须更新对应 `docs/architecture/`；新增架构级能力（新子系统 / 协议层）须同 PR 新建文档并登记到 `docs/README.md`
- **README 双语同步**：根目录 `README.md`（中文）+ `README.en.md`（英文），任一改动同次提交同步另一份
- **Release Notes 双语同步**：每版本 `vX.Y.Z.md` + `vX.Y.Z.en.md`，顶部互加 `简体中文 · English` 切换链接

### Review Followups 登记（强制）

**所有 code review（`/simplify` / `/codex:review` / `/codex:adversarial-review` / 人工 / subagent review）识别但本期 PR 不修的问题，必须登记到 [`docs/plans/review-followups.md`](docs/plans/review-followups.md)，与 PR 同 commit。**

适用：跨 PR 主题清理 / 存量债务 / 低优但确实是问题的"不优雅" / review 给的复用机会但当期不动。
不适用：误报（直接驳回）/ 当期已修 / 功能 backlog（去 issue tracker）。

格式见文件顶部的"条目模板"。**目的是让债务可见、可检索、可调度**——下次有人撞上同一处问题不该重新发现一遍。

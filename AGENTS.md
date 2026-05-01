# Hope Agent

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，内置丰富的 Provider 模板与预设模型，GUI 傻瓜式配置。支持三种运行模式：桌面 GUI（Tauri）、HTTP/WS 守护进程（`hope-agent server`）、ACP stdio（`hope-agent acp`）。

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

推送（`git push`）前**必须**本地跑一遍以下六条，任一失败都要先修再推——CI 会对同一组检查投红票，红在 CI 上要等 10 分钟反馈。`pnpm install` 后会自动启用 [`.husky/pre-push`](.husky/pre-push) 钩子，该钩子就按这个顺序跑这六条；裸跑也可以：

```bash
cargo fmt --all --check                                                    # 对应 CI: rust.yml fmt job
cargo clippy -p ha-core -p ha-server --all-targets --locked -- -D warnings # 对应 CI: rust.yml clippy job
cargo test  -p ha-core -p ha-server --locked                               # 对应 CI: rust.yml test job
pnpm typecheck                                                              # 对应 CI: lint.yml 前端类型（tsc -b）
pnpm lint                                                                    # 对应 CI: lint.yml ESLint
pnpm test                                                                    # 对应 CI: lint.yml Vitest
```

- **clippy / test 只覆盖 `ha-core` + `ha-server`**（CI 也是如此）—— `src-tauri` 不在本地钩子范围内，tauri-specific 的 lint / test 问题请用 `cargo {clippy,test} --workspace` 主动自查
- **fmt 用 `--all`**，覆盖整个 workspace；钩子用的是 `cargo fmt --all --check`，裸跑时用不用 `--all` 都能对齐 CI，不过 `--all` 更稳
- **Rust 版本由仓库根目录 [`rust-toolchain.toml`](rust-toolchain.toml) 固定**，本地与 CI 共用同一版本和组件集合；升级 Rust 时优先改这个文件，再验证 pre-push 六项检查全部通过
- **应急开关**：
  - `HA_SKIP_PREPUSH=1 git push` — 整段钩子跳过（仅限文档 / 纯 `.md` 改动 / 弱网紧急场合）
  - `HA_SKIP_PREPUSH_TEST=1 git push` — 只跳过 `cargo test`（WIP 分支快速推送，test 让 CI 兜底；fmt/clippy/tsc/eslint/vitest 仍然跑）
  - 禁止用 `--no-verify`，因为它会把 GPG 签名等其它钩子也一并绕过

### Agent 开发期检查行为（强制）

上面这六条是 **`git push` 前**的兜底检查，pre-push 钩子会自动跑。**Agent 在开发过程中不要主动跑全套检查**——耗时长，绝大多数改动不需要。具体规则：

- **改代码过程中**：默认**只跑单点验证**——改 Rust 文件用 `cargo check -p <影响的 crate>`；改 TS/TSX 用 `pnpm typecheck`。**不要**主动跑 `cargo clippy` / `cargo test` / `pnpm test` / `pnpm lint` 全量检查
- **想跑全套检查时必须先问**：如果 agent 判断需要跑 clippy / cargo test / pnpm test / pnpm lint 中任何一项，**先在对话里问用户**「是否要跑 X？」并说明原因（比如「改动了 streaming_loop 的并发逻辑，建议跑一次 cargo test -p ha-core 验证」），等用户回复再跑。**不要默认跑了再说**
- **长任务 / 计划完成时例外**：当本轮工作属于「改动覆盖多文件多模块」「执行完一个完整 plan / TodoList」「重构跨 crate」这类大块收尾节点时，agent 可以**主动适量**跑必要的检查（按改动范围挑相关项，不必跑全六条），跑前在文字里说一句「改动较大，跑一下 X 收个尾」。把握「适量」——不要每写完一个文件就跑一遍，要在阶段性收尾时跑
- **commit / push 前例外**：用户明确要求 `git commit` / `git push` 时，agent 可以按 pre-push 六条自查；或直接交给 [.husky/pre-push](.husky/pre-push) 钩子兜底
- **用户明确说「跑一下检查」/「验证一下」时**：按用户指示跑，不需要再问

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
skills/                 内置技能（bundled skills，随应用发行）
                          - meta：ha-settings / ha-skill-creator / ha-find-skills
                          - 编程方法论（vendor 自 Hermes Agent，MIT，带 `paths:` 限定 *.{rs,ts,py,go,...} 触发）：systematic-debugging / test-driven-development / writing-plans / code-review / subagent-driven-development
                          - 办公方法论（原创）：meeting-notes / email-draft / status-report / mermaid-diagram
                          - vendor 来源记录在仓库根 `THIRD_PARTY_NOTICES.md`，每个 vendor skill 子目录附 `ATTRIBUTION.md`
```

ha-core 按功能域拆分模块，具体用 `ls crates/ha-core/src/` / `Glob` 查看，无需在此维护清单。主要领域：`agent/`（AssistantAgent + Provider + Tool Loop）、`chat_engine/`、`context_compact/`、`memory/`、`skills/`、`tools/`、`channel/`（IM 渠道）、`subagent/`、`team/`、`cron/`、`acp/`、`dashboard/`、`recap/`、`awareness/`、`config/`、`session/`、`project/`、`plan/`、`ask_user/`、`async_jobs/`、`failover/`、`platform/`、`security/`、`logging/`、`local_llm/`（本地小模型一键安装助手）。

## 技术栈

| 层     | 技术                                                                 |
| ------ | -------------------------------------------------------------------- |
| 前端   | React 19 + TypeScript, Vite 8, Tailwind CSS v4, shadcn/ui (Radix UI) |
| 桌面   | Tauri 2                                                              |
| 服务器 | axum (HTTP/WS), clap (CLI)                                           |
| 后端   | Rust, tokio, reqwest（ha-core 库，零 Tauri 依赖）                    |
| 渲染   | Streamdown + Shiki + KaTeX + Mermaid                                 |
| 多语言 | i18next (12 种语言)                                                  |

## 架构约定

### 分层 & 运行模式

- **三 Crate 架构**：`ha-core`（核心业务逻辑，**零 Tauri 依赖**）/ `ha-server`（axum HTTP/WS 薄壳）/ `src-tauri`（Tauri 桌面薄壳）。业务逻辑全进 ha-core，其它两个只做适配
- **三种运行模式**：`hope-agent`（桌面 GUI）/ `hope-agent server`（HTTP/WS 守护进程 + 内嵌 Web GUI，含 install/uninstall/status/stop 子命令）/ `hope-agent acp`（stdio ACP 协议）
- **内嵌 Web GUI**：`crates/ha-server/src/web_assets.rs` 用 `rust-embed` 把 Vite 构建产物 `dist/` 打进 server 二进制，axum 路由 `fallback_service` 直接返回；浏览器打开 `http://<server>:port` 即得完整 React UI（含 SPA fallback，未知路径回 `index.html` 让 React Router 接管）。`HA_WEB_ROOT` 环境变量可指向本地 `dist/` 目录做 dev override；未构建前端时降级为诊断页提示跑 `pnpm build`。`/api` 与 `/ws` 仍走 Bearer Token 鉴权中间件，静态资源放行以便首屏加载
- **前后端通信**：前端走 Transport 抽象层（`src/lib/transport.ts`），桌面走 Tauri IPC，server 走 HTTP + WS。**新 invoke 必须同时补齐两套适配**
- **EventBus 事件总线**：`ha-core::EventBus` 替代原 Tauri `APP_HANDLE`，让核心逻辑脱离 Tauri 依赖；Tauri shell / axum server 各自订阅并转发到各自前端通道
- **状态管理**：`ha-core::CoreState`（`tokio::sync::Mutex`），Tauri 走 `State<AppState>`，server 走 axum `Extension`；前端保持轻量 React state
- **Guardian 心跳**：桌面 + server 共用 keepalive，保证后台任务（Channel 轮询、Cron 调度等）持续运行
- **桌面自动更新 / Release 版本单一来源**：`package.json` 是桌面版本的单一真相源；`pnpm version` 生命周期钩子会运行 `scripts/sync-version.mjs`，同步 `src-tauri/Cargo.toml` 与 `src-tauri/tauri.conf.json`。GitHub Release workflow 在 tag 构建前执行 `pnpm release:verify -- --tag vX.Y.Z`，并上传 Tauri updater `latest.json` 工件。Updater 公钥跟仓库 `src-tauri/updater.pub.pem`，私钥仅存本机 `~/.tauri/hope-agent-updater.key` 和 GitHub Secrets，严禁入仓
- **系统服务**：`server install` 在 macOS 注册 launchd plist（`~/Library/LaunchAgents/`），Linux 注册 systemd unit（`~/.config/systemd/user/`）
- **API Key 鉴权**：`ha-server/middleware.rs`，支持 `Authorization: Bearer` 头和 `?token=` 查询参数（浏览器 WS 不支持自定义头）。`/api/health` 免鉴权，`api_key=None` 全放行
- **内嵌服务器配置**：桌面内嵌 HTTP 的 bind + api_key 存 `config.json` 的 `server` 字段，修改后需重启；设置页、侧边栏状态指示器、托盘菜单状态行都以 `ha_core::server_status` 为运行时状态单一数据源

### LLM 主对话

- **Provider**：4 种（Anthropic / OpenAIChat / OpenAIResponses / Codex），集中在 `agent/` 模块
- **Think 模式两层配置**：Provider 级 `thinking_style` 定义默认推理参数格式；`ProviderConfig.models[*].thinking_style` 可按模型覆盖，`None`=继承 Provider，`reasoning=false` 或 style=`none` 都会强制 no-think。聊天 UI 显示遵循“会话模型 > Agent override > 全局 active model”
- **Tool Loop**：请求 → 解析 tool_call → 执行 → 回传 → 继续，默认上限 50 轮（`max_tool_rounds=0` 无限）。末轮去 `tools` 字段强制文本收尾。`concurrent_safe` 标只读工具并发、写入工具串行
- **温度三层覆盖**：会话 > Agent > 全局，`AssistantAgent.temperature` 在四 Provider 请求中统一注入（0.0–2.0，`None` = API 默认）
- **Side Query（缓存侧查询）**：`side_query()` 复用主对话 system_prompt + history 前缀命中 prompt cache，Tier 3 摘要 / 记忆提取成本降 ~90%
- **LlmApiAdapter / StreamingChatAdapter trait**：one-shot + streaming 两个抽象（`agent/llm_adapter.rs` + `agent/streaming_adapter.rs`）；各 Provider 实现 trait 避免重复，`streaming_loop.rs` 承担 compaction / cache snapshot / tool dispatch / steer / microcompact / event emit 公共编排。**新"会 spawn tool loop 的 chat 路径"走 `chat_engine::run_chat_engine`，不要绕过自包 `on_delta`**
- **failover executor**：`failover/executor.rs::execute_with_failover` 统一承担 profile 选择 + 错误分类 + 决策。三档 policy：`chat_engine_default` / `side_query_default` / `summarize_default`。**Codex 强制不参与 profile 轮换**
- **降级策略**：ContextOverflow 终止 → RateLimit/Overloaded/Timeout 指数退避重试 2 次 → Auth/Billing/ModelNotFound 跳下一模型
- **Auth Profile 轮换**：`ProviderConfig.auth_profiles` 同 Provider 多 API Key，失败先轮换同 Provider profile 再跳模型；纯内存 cooldown + session 级 sticky。Codex OAuth 不参与
- **连续消息合并**：`push_user_message()` 合并连续 user 消息兼容 Anthropic role 交替

### Chat Engine & Streaming

- **聊天流双写 + seq 去重（重载恢复）**：每个 stream delta 同时发 per-call `EventSink`（主路径）和 EventBus `chat:stream_delta`（带 `{sessionId, seq}`，重载保险）。主路径 + Bus 路径共享 `lastSeqRef` cursor 去重，Channel 死时 Bus 路径接管。IM 渠道独立 `ChannelStreamSink` 走 `channel:stream_delta` 不与主 chat 混淆
- **API-Round 消息分组**：tool loop 中 assistant + tool_result 通过 `_oc_round` 元数据分组，压缩切割（Tier 3/4）对齐 round 边界避免拆散 tool_use/tool_result 配对；API 调用前 `prepare_messages_for_api()` 剥离元数据

### 上下文压缩

- **5 层渐进式 + ContextEngine trait**：`context_compact/` 可插拔引擎，默认 `DefaultContextEngine` 行为不变
- **CompactionProvider trait**：Tier 3 摘要策略可插拔，`summarization_model` 配置驱动 `DedicatedModelProvider`
- **Cache-TTL 节流**：`compact.cacheTtlSecs`（默认 300s）Tier 2+ 冷却保护 API prompt cache；usage ≥ 95% 强制覆盖，`0` 禁用
- **反应式微压缩**：tool loop 每轮末尾使用率 ≥ `reactiveTriggerRatio`（默认 0.75）触发 Tier 0 清 `tool_policies=eager` 旧工具结果，cache-safe 不改消息顺序
- **后压缩文件恢复**：Tier 3 摘要后扫描 write/edit/apply_patch 自动注入最近文件当前内容（最多 5 × 16KB），省额外 read

### Memory

- **自动提取**：冷却 ≥ 5 min AND（Token ≥ 8000 OR 消息 ≥ 10）时在 assistant 最终消息落库后后台调度提取，不阻塞聊天流结束；检测到 save_memory/update_core_memory 跳过互斥；空闲超时（默认 30 min）兜底 flush
- **Memory Budget**：system_prompt Memory section 按 `Guidelines > Agent memory.md > Global memory.md > SQLite` 优先级消费总字符预算，`effective_memory_budget(agent, global)` 单入口解析。**`recall_memory` / `memory_get` 工具返回完整原文，预算仅约束 system prompt 注入路径**
- **Active Memory 主动召回**：每轮 user turn 前调 `refresh_active_memory_suffix(user_text)`，带 15s TTL 缓存，按 Project → Agent → Global scope 取候选 + bounded side_query 生成 `## Active Memory` suffix。作为独立 cache block 注入不作废静态前缀缓存
- **反省式记忆（COMBINED_EXTRACT_PROMPT）**：`memory_extract` 单次 side_query 同时返回 facts + profile；profile 渲染为独立 `## User Profile` 段（不用 "About You"——"You" 在 LLM system prompt 里默认指 assistant）
- **LLM 语义选择**：候选 > 阈值（默认 8）时 side_query 选最相关 ≤5 条，opt-in `memorySelection.enabled`
- **召回摘要**：命中 ≥ `min_hits`（默认 3）时 bounded side_query 压成 ≤400 字符洞察段落，opt-in
- **Dreaming 离线固化**：idle / cron / manual 三触发 bounded side_query 把候选交 LLM 评估，打分高的 pin，diary markdown 落 `~/.hope-agent/memory/dreams/`；`DREAMING_RUNNING` AtomicBool + RAII 保证并发
- **会话级无痕对话**：`sessions.incognito` 是单一真相源。除关闭被动 AI 行为（不注入 Memory / Active Memory / Awareness、不跑 inline/idle/flush-before-compact 自动记忆提取）外，**关闭即焚**——无痕会话不进侧边栏列表、不进全局 FTS、不进 Dashboard 统计（`list_sessions_paged` / `search_messages(session_id=None)` / `dashboard::filters::build_session_filter` 统一过滤；`active_session_id` 例外参数让"当前正在看的那个无痕"仍出现在 sidebar）。`handleSwitchSession / handleNewChat / handleNewChatInProject` 切走前端调 `purge_session_if_incognito` 硬删；`app_init::purge_orphan_incognito_sessions` 兜底 crash / kill -9 / 物理断电后下次启动残留。**当前会话内 Cmd+F 搜索仍工作**——`search_messages(session_id=Some(..))` 路径不应用 incognito 过滤。**Project / IM Channel 与 incognito 互斥**：前端 `IncognitoToggle` 灰化 + tooltip 解释，`update_session_incognito` 对 `project_id.is_some()` / `channel_info.is_some()` 直接 `Err`，`create_session_with_project` 在 project_id 存在时强制 `incognito=false`，`channel/db.rs::ensure_conversation` 入口防御式清零。`delete_session` 顺手清 `learning_events / subagent_runs / acp_runs` 三张无 FK cascade 的关联表，避免 silent orphan

### 工具 & 审批

详见 [`docs/architecture/permission-system.md`](docs/architecture/permission-system.md)。核心约定：

- **Agent 工具过滤**：`AgentConfig.capabilities.tools` 在 system_prompt / tool_schemas / tool_search / 执行层统一生效。internal 系统工具始终保留，`denied_tools` / skill allowlist / Plan Mode 继续收紧
- **工具结果磁盘持久化**：超过阈值（默认 50KB，`toolResultDiskThreshold`）写 `~/.hope-agent/tool_results/{session_id}/`，上下文仅留 head+tail 预览 + 路径引用
- **统一权限引擎（v2）**：所有工具调用走 `permission::engine::resolve_async(ctx) -> Decision { Allow | Ask { reason } | Deny { reason } }` 单入口判定。优先级 **Plan > Internal > YOLO > Protected/Dangerous > AllowAlways > Session 模式 preset > 兜底 Allow**。Plan Mode 是正交工作模式，能压过 Global YOLO；保护路径 + 危险命令在非 YOLO 模式下强制弹（`forbids_allow_always=true`，AllowAlways 按钮置灰），YOLO 模式只 `app_warn!` 不弹。**Smart 模式忽略 `custom_approval_tools`**——UI 必须显式提示
- **Session 模式三选一**：`sessions.permission_mode ∈ default | smart | yolo`，标题栏 `PermissionModeSwitcher` dropdown 切换；IM 渠道用户用 `/permission default|smart|yolo` 切换（IM 端经 `channel/worker/slash.rs` 写 SessionDB + emit `permission:mode_changed`），用 `/status` 查看当前模式。`AgentConfig.capabilities.default_session_permission_mode` 决定该 Agent 新会话的初始 mode（None = 跟随全局默认 `default`）
- **Smart 模式**：`SmartStrategy ∈ SelfConfidence | JudgeModel | Both`。SelfConfidence 读 `args._confidence == "high"` 直接 Allow；JudgeModel 起独立 `judge_one_shot` side_query（5s 硬超时 + 60s TTL cache，bare 模式不污染主对话 cache）；判官失败按 `SmartFallback ∈ Default | Ask | Allow` 降级。Smart guidance 在 system_prompt 末尾按 mode 注入（`system_prompt/build.rs` 与 `TOOL_CALL_NARRATION_GUIDANCE` 同位置）
- **保护路径 / 危险命令 / 编辑命令**：三个独立列表，文件落 `~/.hope-agent/permission/{protected-paths,dangerous-commands,edit-commands}.json`，缺失则用硬编码 `DEFAULT_*` const。共享 `permission::list_store` 的 `Arc<Vec<String>>` cache + `RwLock<Option<Arc<...>>>` 槽——hot path 只 bump refcount
- **Global YOLO**：CLI flag `--dangerously-skip-all-approvals`（进程内不持久化）和 `AppConfig.permission.global_yolo`（config.json 持久化）OR 组合。判定入口 `security::dangerous::is_dangerous_skip_active()`，跳过会落 `app_warn!` 审计。**与 Plan Mode 正交**（Plan Mode 限制工具/路径，能压过 YOLO；YOLO 只跳审批）
- **审批等待超时**：`AppConfig.permission.approval_timeout_secs`（默认 300s，`0` 不限时）；超时动作由 `approval_timeout_action` 控制（`deny` / `proceed`）。前端 `ApprovalDialog` 圆环倒计时 + 最后 30s 变红（`setState` 在 interval 回调里满足 `react-hooks/set-state-in-effect`）
- **AllowAlways 多作用域（v1 部分实现）**：`permission/allowlist.rs` 暴露了 `AllowScope ∈ Project | Session | AgentHome | Global` enum 骨架，但 GUI 编辑入口 + 4 作用域文件 IO 是 v1 待补——弹窗按钮目前固定走 "Allow Once"；exec 命令前缀 allowlist 走旧 `is_command_allowed` 路径兼容
- **工具调用前说明**：`AppConfig.tool_call_narration_enabled`（默认 `false`）开启后注入 `TOOL_CALL_NARRATION_GUIDANCE`
- **异步 Tool 执行**：`exec` / `web_search` / `image_generate` 标 `async_capable=true`。三道决策：模型显式 `run_in_background` / Agent `async_tool_policy` / 同步预算超时自动后台化。结果落独立 `async_jobs.db` + spool 到 `~/.hope-agent/async_jobs/`，完成后经 `subagent::injection` 注入主对话。`job_status` deferred 工具主动 poll/wait，用 per-job `tokio::sync::Notify` + DB 重读关闭 race；重启回放残留 running → interrupted
- **SSRF 统一策略**：出站 HTTP **必须**走 `security::ssrf::check_url(url, policy, &trusted_hosts)`，redirect 用 `check_host_blocking_sync`。三档 policy：`Strict` / `Default` / `AllowPrivate`，metadata IP 任何 policy 都拒；`trustedHosts` 优先于 policy。**新出站入口严禁自写 IP 校验**。LLM Provider 出站当前不走此检查（`allow_private_network` 仅落字段）
- **会话列表 pending-interaction 指示器**：`SessionMeta.pendingInteractionCount` 合并工具审批 + ask_user 两类。前端按 `!isActive && !channelInfo && pendingInteractionCount > 0` 叠加视觉提示；后端相关路径调 `emit_pending_interactions_changed()` 让前端 300ms 防抖刷新
- **文件 Diff 元数据 / 右侧 Diff 面板**：`write` / `edit` / `apply_patch` / `read` 走 `tools::diff_util` 计算行差并通过 `ToolExecContext.metadata_sink`（每次 dispatch 由 `agent::streaming_loop::execute_tool_with_cancel` 独立 `Arc::new` 一只 `tokio::sync::Mutex<Option<Value>>`，外层注入 → tool 执行调 `emit_metadata` → 外层 take）旁路传出 `kind=file_change|file_changes|file_read` JSON。事件层走 `chat:tool_result.tool_metadata`，持久化层落新增 `messages.tool_metadata` 列（迁移 `ALTER TABLE messages ADD COLUMN tool_metadata TEXT`），写入入口统一在 `SessionDB::update_tool_result_with_metadata`。前端 `ToolCallBlock` / `ToolCallGroup` header 渲染 `+N -M` 摘要 + GitCompare 按钮，点击经 `useDiffPanel` 触发 `ChatScreen` 右侧 `DiffPanel`（Unified / Split toggle，localStorage `ha:diff-panel:layout`），与 PlanPanel / CanvasPanel 视觉互斥（保留 state，重新点击恢复）。`metadata` 缺失时 ToolCallBlock 完全退回旧外观

### Skill 系统

- **Bundled Skills**：`skills/` 随应用发行，`discovery.rs::resolve_bundled_skills_dir()` 按"环境变量 → exe 同级/上级 → CARGO_MANIFEST_DIR"优先级定位。优先级 bundled < extra < managed < project
- **`skill` 工具（激活主入口）**：`skill({name, args?})` 取代"模型 read SKILL.md"老路径；inline / fork 两种分发路径复用同一 helper。工具标 `internal + always_load` 保证 deferred tool 场景恒可见
- **斜杠 Skill 内联**：`/skillname args` 内联模式包进 `[SYSTEM: ...]` 消息 + `---` 分隔线；`chat` 命令 `display_text` 让 DB / UI 显示原始斜杠、LLM history 存展开 prompt。**已知 gap**：斜杠内联当前未应用 `allowed-tools` 过滤和 `check_requirements`
- **Skill Fork**：SKILL.md `context: fork` 在 tool 调和斜杠两条路径都生效；`agent:` 指定身份（失败 fallback + warn），`effort:` 透传 `SpawnParams.reasoning_effort`
- **Skill 工具隔离**：SKILL.md `allowed-tools:` 激活时只保留白名单 schema，空列表 = 全部
- **Skill `paths:` 条件激活**：`paths:` 的 Skill 默认不进 catalog，命中路径感知工具时 `bump_skill_version()` 刷下一轮 prompt；kill switch `AppConfig.conditional_skills_enabled`
- **Skill Draft 审核**：`skills::author` 提供 CRUD + Jaccard 0.80 模糊 patch；`security_scan` 拦截 shell pipe / 不可见 Unicode / 凭证特征。SKILL.md `status: active|draft|archived`，面向模型路径跳过非 Active；`auto_review` 默认 enabled=true + promotion=draft 等用户确认。GUI `设置 → 技能` 拆两个 tab：「技能」管目录 + 开关，「自我进化」放主开关（`auto_review.enabled`，紫→品红 hero 卡片 + 状态 pill）+ 新技能自动激活（`auto_review.promotion`，主开关关闭时灰化）。后端新增 `get/set_skills_auto_review_enabled` Tauri 命令 + `GET/PUT /api/skills/auto-review/enabled` HTTP 路由
- **Plan 执行层权限强制**：`ToolExecContext.plan_mode_allowed_tools` 执行层白名单检查，与 schema 级过滤 defense-in-depth

### MCP 客户端

详见 [`docs/architecture/mcp.md`](docs/architecture/mcp.md)。核心约定：

- **模块位置**：`crates/ha-core/src/mcp/`，零 Tauri 依赖。Tauri / axum 通过 EventBus + `mcp::api::*` 调用
- **四种 transport**：stdio / Streamable HTTP / SSE（路由到 Streamable HTTP）/ WebSocket。所有网络 transport 先过 `security::ssrf::check_url`；ws/wss 先 rewrite 成 http/https 供 SSRF 分类
- **命名空间**：`mcp__<server>__<tool>`，`server` 校验 `^[a-z0-9_-]{1,32}$`；tool 名截断到 64 字符并清洗；碰撞追加 `_N`
- **工具默认 deferred**：MCP 服务器可能暴露几十个工具，`ToolDefinition.deferred=true` 走 `tool_search` 发现；`McpGlobalSettings.always_load_servers` 白名单强制 `always_load`
- **async_capable**：rmcp `ToolExecution.task_support ∈ Required|Optional` → `async_capable=true`，让 tool loop 的"同步预算超时自动后台化"分支可触发
- **OAuth 2.1 + PKCE**：`oauth.rs` 独立实现（不用 `rmcp::auth_client` 避免 reqwest 版本冲突）。PKCE S256 + CSRF state 32 bytes；loopback callback `127.0.0.1:0`（RFC 8252）+ DCR (RFC 7591) 按需触发。discovery/DCR/token/refresh 固定 `SsrfPolicy::Default`，走 `provider::apply_proxy`
- **凭据存储**：`~/.hope-agent/credentials/mcp/{server_id}.json`，0600 原子写（`platform::write_secure_file`）。load/clear 通过 `io::ErrorKind::NotFound` match 无 TOCTOU。删除 server 自动 `credentials::clear`；日志经 `redact_sensitive` 脱敏
- **NeedsAuth 状态**：handshake 401/403 → `McpError::Auth` → `ServerState::NeedsAuth`（不是 Failed），避免 watchdog 重试死循环；真实 authorize URL 由 `oauth::authorize_server` 点击时动态生成
- **WebSocket 硬上限**：`max_message_size=4 MiB` / `max_frame_size=1 MiB`（tungstenite 默认 64/16 MiB 对 JSON-RPC 过宽松）；`poll_next` 带 64-frame cooperative-yield budget 防恶意 peer 饿死调度器；per-read 5s timeout
- **Resources / Prompts**：独立于 tool call 的被动数据访问。`mcp_resource(action=list|read)` / `mcp_prompt(action=list|get)` 作为 internal tool 暴露；list 读 cached Ready catalog，read/get 走 RPC
- **System prompt snippet**：`catalog::system_prompt_snippet()` 在 `build_full_system_prompt` 末尾追加 "# MCP Capabilities"（sync-safe 通过 `cached_tool_defs` ArcSwap；无 MCP server 时不注入）
- **配置读写 contract**：读 `cached_config().mcp_servers` / `.mcp_global`；写 `mutate_config(("mcp.<op>", source), |cfg| { ... })` — `op ∈ add|update|remove|reorder|settings`
- **Dashboard Learning**：每次 MCP 工具调用发 `EVT_MCP_TOOL_CALLED` / `EVT_MCP_TOOL_FAILED`，meta 含 `{ server, tool, durationMs, error? }`；ref_id 为 namespaced name

### Subagent / Team / 定时任务

- **spawn_and_wait**：`subagent(action="spawn_and_wait")` 前台等待 `foreground_timeout`（默认 30s），超时自动转后台
- **Agent Team 模板**：GUI 预配 + 模型按需发现（`team(action="list_templates" / "create")`）。`TeamTemplateMember.description` 注入子 session 的 `### Your Role Identity` 段；删除模板不影响已实例化运行中 team（`teams.template_id` 悬空保留）
- **定时任务 delivery_targets**：`CronJob.delivery_targets` 把 final assistant text fan-out 到 IM 会话，每 target 10s 超时保护。IM 会话内隐式推断：未显式传时自动取当前 IM 会话；显式 `[]` 关闭发送

### IM Channel

- **架构**：`channel/` 下 12 个插件，状态文件统一落 `~/.hope-agent/channels/`。入站媒体管道：plug 下载 → worker 转 `Attachment` → 归档到 `~/.hope-agent/attachments/{session_id}/` → `agent.chat()` 多模态
- **工具审批交互**：`channel/worker/approval.rs` 监听 EventBus `approval_required`，按 `ChannelCapabilities.supports_buttons` 发原生按钮或文本 fallback；`ChannelAccountConfig.auto_approve_tools=true` 跳审批门控
- **启动握手 watchdog**：[`channel/start_watchdog.rs`](crates/ha-core/src/channel/start_watchdog.rs) 是所有 channel auto-start 失败的共享后台重试器。退避 30s → 60s → 2m → 5m，5m 上限封顶，sweep 周期 15s，直到握手成功或者用户 stop/disable/remove。入口：`register_failure(account, &err)` 排队 + `cancel_pending(account_id)` 撤销 + `mark_success(...)` 标记成功；`spawn_loop(registry)` 在 `app_init::start_background_tasks` 里幂等启动。`ChannelRegistry::stop_account` / `start_account` 与 `accounts::add_account` / `update_account` 已就地接入，user 操作永远胜过 watchdog。失败日志同时输出 `{:#}` 全错误链 + `classify_channel_error` 启发式分类（network / auth / proxy / TLS / DNS / timeout / forbidden 等），便于人工 grep 与 agent 自动诊断

### Dashboard / Recap / Learning

- **Insights**：`dashboard/insights.rs`——overview delta / cost trend / activity heatmap / hourly / top sessions / model efficiency / health score + orchestrator `query_insights`
- **Learning Tracker**：`session.db.learning_events` 表 + `dashboard::learning` 查询。埋点：`skills::author` CRUD + `tool_recall_memory` 命中；Dashboard Learning Tab 支持 7/14/30/60/90 天窗口
- **/recap 深度复盘**：`recap/` 模块 side_query 提取 facet + 11 个并行 AI 章节。独立 `~/.hope-agent/recap/recap.db` 缓存按 `last_message_ts` 失效；`recap.analysisAgent` 与主对话 Agent 解耦，`facetConcurrency` 限流

### 跨会话 / 全局

- **数据存储**：所有数据在 `~/.hope-agent/`，`paths.rs` 集中管理
- **统一日志**：前后端都进 `logging.rs`（SQLite + 文本双写），API 请求体 `redact_sensitive` 脱敏 + 32KB 截断
- **跨会话行为感知**：每会话 `SessionAwareness` 在 user turn 前按"脏位 / 时间节流 / 语义 hint"决定是否重建"其它会话在做什么" markdown suffix。默认 `structured`（零 LLM 成本），opt-in `llm_digest`（bounded 5s + 5min 节流）。作为独立 cache block 注入不作废静态前缀缓存
- **Awareness 与无痕联动**：session `incognito=true` 时会直接跳过 `refresh_awareness_suffix`，前端 session 级 AwarenessToggle 置灰但不改写 `awareness_config_json`；关闭无痕后恢复原有 awareness 配置
- **会话搜索**：侧边栏 FTS5 `messages_fts` + `<mark>` snippet 高亮 + `SessionTypeFilter` 跨会话类型筛选。结果命中 `load_session_messages_around_cmd` 加载窗口（40/20）+ 滚动定位 + pulse 高亮。**XSS 防御**：HTML escape → 白名单反解 `<mark>`。Find in Chat（`Cmd+F`）复用同一 `SessionDB::search_messages` + session_id 过滤
- **ask_user_question 工具**：1–4 题结构化问答（单选/多选/输入），选项支持 markdown/image/mermaid preview；pending 组持久化到 session SQLite，App 重启 replay 断点续答；IM 渠道按 `supports_buttons` 走按钮或文本
- **延迟工具加载**：opt-in `deferredTools.enabled`，只发核心 ~10 个 schema，其余通过 `tool_search` 元工具按需发现；execution dispatch 不变
- **Persona SoulMd 模式**：`PersonalityConfig.mode: Structured | SoulMd`。SoulMd 跳过结构化 personality section，注入 `~/.hope-agent/agents/{id}/soul.md` + `SOUL_EMBODIMENT_GUIDANCE`，身份行省略 `role_suffix` 避免与 markdown 自述身份双重声明
- **SearXNG Docker 代理**：`web_search.searxng_docker_use_proxy` 控制是否向容器 `settings.yml` 注入 proxies 和环境变量，下次启动生效
- **会话级工作目录**：`sessions.working_dir` 持久化用户为该对话指定的绝对路径，`system_prompt::build` 在 Project 段之后、Memory 段之前注入 `# Working Directory` 告诉模型默认操作目录。桌面端用 `@tauri-apps/plugin-dialog` 的 `{ directory: true }` 原生选择；HTTP 模式前端是 `ServerDirectoryBrowser` Dialog 驱动后端 `GET /api/filesystem/list-dir`（单级 + Bearer token）。`update_session_working_dir` 走 `canonicalize` + `is_dir` 校验，无效路径 400。与 Project / Incognito 正交可并存。v1 只做提示词注入，不改 `exec`/`read_file` 的 cwd 解析
- **运行模式 getter `runtime_role()` / `is_desktop()`**：`init_runtime(role: &'static str)` 把 role（`"desktop"|"server"|"acp"|"test"`）写入 `crates/ha-core/src/app_init.rs` 的 `RUNTIME_ROLE: OnceLock<&'static str>`（first-write-wins，早期返回前 set），通过 `pub fn runtime_role() -> Option<&'static str>` + `pub fn is_desktop() -> bool` 暴露给需要按运行模式分流的核心路径——避免给 `system_prompt::build` 这类被三种入口共用的函数加 mode 参数污染签名。当前消费方：`build.rs` 桌面专属 guidance 注入。**测试共享 OnceLock**——同 binary 内 `init_runtime("test")` 一旦 set，所有测试都看到 `Some("test")`，禁止写"必须等于 desktop"的断言
- **桌面专属 markdown 文件路径链接**：仅 `is_desktop()` 时 `system_prompt::build` 注入 `MARKDOWN_PATH_LINKS_GUIDANCE`，要求 LLM 把文件路径写成 `[显示名](Unix-style 绝对路径)` markdown 链接（含可选 `#L<line>` 锚点）而不是纯文本 / inline code，路径含空格时用 angle-bracket destination `[x](</abs/path with spaces>)`（CommonMark 唯一能保留 href 的形式）。前端 `MarkdownRenderer` 的 `MarkdownLink` 用 `isLocalPath()`（**只识别 `/` / `~/` 前缀**）+ `getTransport().supportsLocalFileOps()` 分流：本地路径 + Tauri → `open_directory` 走系统默认应用打开（`normalizeLocalPath` 剥掉 `#L42` 防 `open::that` 失败）；本地路径 + HTTP/server → `event.preventDefault()` 后早返回 + native `title` tooltip（i18n key `common.markdownLinkLocalDisabled`）+ `cursor-not-allowed`，避免在 server 主机上误开文件**或**浏览器把 `<a href="/Users/...">` 当 URL 跳走；非本地链接走 `openExternalUrl`（含 `window.open` fallback）。**Streamdown sanitize 限制**：内部用固定 `defaultSchema` 的 `rehype-sanitize`（不可外部配置），`file://` 和 Windows `C:\` 协议会被剥 href，所以 prompt 显式禁用这两种格式且 `isLocalPath` 不识别它们。`title` 用 native HTML attr 不用 shadcn Tooltip——Streamdown 一条流式消息可能渲染上百个 anchor，包 TooltipTrigger 既炸 DOM 也破坏 anchor 组件签名，是 AGENTS.md「Tooltip 必须用 ui/tooltip」规则的合理例外。v1 不接 IDE 协议（行号锚点被丢但保留在 wire format），不做正则后处理把存量纯文本路径自动转链接（误伤风险高）；v2 留给 `open_in_editor(path, line)` Tauri 命令 + `AppConfig.preferred_editor` 走 `vscode://file/<path>:<line>`

### 项目（Project）容器

- **三层 system prompt 注入**：目录清单永远注入 → 小文件（<4KB）8KB 预算内内联 → `project_read_file` 工具按需（强制 `project_extracted_dir` 内）
- **记忆优先级**：Project > Agent > Global，budget 裁剪时项目记忆最先保留；session 属项目时自动记忆提取默认写 Project scope
- **默认工作目录**：`Project.working_dir` 是该项目下会话的默认工作目录。运行时合并优先级 `session.working_dir > project.working_dir > 不注入`，**lazy resolve**（不复制快照）——改项目工作目录立即对未单独设置的已有会话生效。合并逻辑唯一入口 `crates/ha-core/src/session/helpers.rs::effective_session_working_dir`，被 system prompt 渲染（`agent/config.rs`）、工具执行 context（`agent/mod.rs::ToolExecContext.session_working_dir`、`slash_commands/handlers/mod.rs`）共同消费——`read` / `write` / `exec` 的相对路径解析、`write_file` 路径白名单都按合并后的有效值。`crates/ha-core/src/util.rs::canonicalize_working_dir` 是 session/project 共用的写入校验入口（canonicalize + is_dir，空串当清空）。前端 `ChatTitleBar` 与 `WorkingDirectoryButton` 都显示生效路径并区分「会话级」「继承自项目」两种 source；继承态下不渲染 clear 按钮（避免无效 no-op 操作）
- **删除级联顺序**：unassign 会话 → 删项目行 + `project_files`（FK）→ `rm -rf projects/{id}/` → 删项目记忆（跨 `memory.db` 单独执行，失败留不可达孤儿）
- **侧边栏树状渲染**：`ProjectSection.tsx` 把每个项目渲染为可折叠的 `ProjectGroup`，展开后嵌套该项目的会话列表（复用 `SessionItem`，按 `localStorage` 的 `ha:project-expanded:<id>` 持久化展开状态）。Hover 显示「新建对话」+「设置」两个按钮，右键菜单包含新建对话 / 设置 / 归档（复用 `AgentSection.tsx` 的 `group/agent` hover 与 `ContextMenu` 模式）。主区域 `SessionList` 自动排除 `projectId` 非空的会话，避免与树状项目下的会话重复展示。`ProjectOverviewDialog`（保留文件名以减少 import 改动）已重构为右侧 `Sheet`（来自 `src/components/ui/sheet.tsx`），删除冗余的 Sessions tab；Overview tab 内置「绑定 IM Channel」select。
- **绑定 IM Channel**：`Project.bound_channel = { channel_id, account_id } | None` 让一个项目认领一个 IM channel account。绑定后 channel worker 的 `crates/ha-core/src/channel/db.rs::resolve_or_create_session` 在创建新会话时反查 `projects` 表注入 `project_id`，并按 5 级解析链（项目 → channel account → AppConfig → "default"）选 agent_id；同一 (channel, account) 同时只允许被一个项目认领（DB 索引 + 写入冲突检测）。`UpdateProjectInput.bound_channel` 用 double-Option：字段缺省=不变，`null`=解绑，对象=设置。`/status` 在项目会话尾部追加项目摘要段并标注 Agent Source 来源。
- **`/project [name]` 斜杠命令**：无参时返回 `CommandAction::ShowProjectPicker`（前端渲染为 markdown event 消息），有参时模糊匹配项目名 → `EnterProject` → `handleNewChatInProject(project_id)`。Discord / Telegram 同步阶段过滤掉 `/project`，handler 用 `session.channel_info` 自检在 IM session 里直接拒绝（不下发 IM 用户）。
- **标题栏 Agent 切换**：`ChatTitleBar` 在项目会话前缀渲染项目 chip（点击打开设置 sheet）；Agent 名换成 `AgentSwitcher` dropdown，仅 `messages.length === 0` 时可换。后端 `update_session_agent_cmd` / `PATCH /api/sessions/:id/agent` 在 SQL 层校验 message_count==0 拒绝改动（前端 disabled 是 UX 防御）。

### Agent 解析链（默认 Agent）

- 新会话 agent_id 解析顺序统一为 5 级：**显式参数 → `project.default_agent_id` → `channel_account.agent_id` → `AppConfig.default_agent_id` → 硬编码 `"default"`**。统一 helper：[`crate::agent::resolver::resolve_default_agent_id`](crates/ha-core/src/agent/resolver.rs)（含带来源 tag 的 `_with_source` 版本，给 `/status` 显示链路命中位置）。
- `AppConfig.default_agent_id` 默认 `Some("default")`，可在「设置 → 常规 → 系统」里改；`ha-settings` 工具支持 `category="default_agent"` 读写（LOW 风险）。Tauri 命令 `get_default_agent_id` / `set_default_agent_id`，HTTP `GET/PUT /api/config/default-agent`。

### 本地 LLM 助手

- **后端锁 Ollama**：`crates/ha-core/src/local_llm/` 通过 Ollama 的 OpenAI 兼容端点注册 Provider（base URL 为 `http://127.0.0.1:11434`，实际 chat endpoint 由 `build_api_url` 拼到 `/v1/chat/completions`），`apiType=OpenaiChat` + `allow_private_network=true`，`ThinkingStyle::Qwen`。模型目录硬编码 Qwen3.6 / Gemma 4 默认量化的 on-disk 大小（覆盖 dense 与 MoE 两种形态），从大到小取首个 ≤ 预算的为推荐；尺寸数字以 `ollama.com/library/<model>/tags` 页为准，ollama 上游发新版本时同步更新 [`local_llm/types.rs::model_catalog`](crates/ha-core/src/local_llm/types.rs)
- **预算判定**：macOS 走统一内存的 50%；Windows/Linux 优先 dGPU VRAM 的 50%（探测顺序 `nvidia-smi` → `lspci` → PowerShell `Win32_VideoController`），失败回落系统内存的 50%；额外扣 1 GiB 给 Ollama runtime + KV cache 浮动
- **Ollama 生命周期**：app **不接管** Ollama 进程，启动时如未运行只是触发 `ollama serve` detached spawn，应用退出**不**杀（用户其它工具可能在用）。一键安装走官方 `install.sh`（macOS/Linux only），Windows 引导用户去 `ollama.com/download`
- **后台任务统一接口**：安装、模型拉取、Embedding 拉取都走 [`crates/ha-core/src/local_model_jobs.rs`](crates/ha-core/src/local_model_jobs.rs)，`~/.hope-agent/local_model_jobs.db` 持久化。前端通过 `local_model_job_*` Tauri 命令 / `/api/local-model-jobs/*` HTTP 路由发起，订阅 `local_model_job:created` / `:updated` / `:log` / `:completed` 事件读进度；进度自带 250 ms / phase-change 节流，对话模型完成后通过 `rebuild_active_agent_hook` 即时刷新 `AppState.agent`
- **Known local backend catalog**：本地后端目录维护在 `crates/ha-core/src/provider/local.rs`，当前包含 `ollama:11434`、`litellm:4000`、`vllm:8000`、`lm-studio:1234`、`sglang:30000`。匹配规则固定为 `apiType` 一致 + host/port 命中，URL path 忽略（所以 `http://localhost:11434/v1` 也算 Ollama）；Ollama 额外接受 `ollama.local`。Tauri command `local_llm_known_backends` 与 HTTP `GET /api/local-llm/known-backends` 同步暴露，前端判断"是否已配置本地后端"必须消费这个 catalog，禁止再写硬编码 regex
- **Provider 写入 contract**：所有 Provider 列表与 `active_model` 写入必须走 `crates/ha-core/src/provider/crud.rs` 的 helper（`add_provider` / `update_provider` / `delete_provider` / `reorder_providers` / `set_active_model` / `add_and_activate_provider` / `add_many_providers` / `ensure_codex_provider_persisted`），Tauri / HTTP 只做薄壳和运行时 agent 重建。普通 `add_provider` 必须保持"生成新 id + append"语义，避免破坏前端"新增后取最后一项"流程；只有本地 LLM 安装路径使用 `upsert_known_local_provider_model("ollama", ...)` 按 known backend host/port 去重、补模型、启用 provider、设置 `allow_private_network` 并切 active model。禁止在 Tauri/server/onboarding/importer/local_llm 路径里直接 `providers.push` / `retain` / 手写 `active_model`

## 编码规范

### 通用

- **性能和用户体验是最高优先级**
- **核心逻辑必须在 ha-core 实现**：业务逻辑、数据处理、文件 IO、状态管理等一律放 `crates/ha-core/`，`src-tauri/` 仅做 Tauri 命令薄壳，`crates/ha-server/` 仅做 HTTP 路由薄壳，前端只负责展示和交互
- 操作即时反馈（乐观更新、loading 态），动效 60fps（优先 CSS transform/opacity）

### 前端

- 函数式组件 + hooks，不用 class 组件
- UI 组件统一用 `src/components/ui/`（shadcn/ui），不直接用 HTML 原生表单组件
- 样式只用 Tailwind utility class，不写行内 style 和自定义 CSS
- 动效优先复用 shadcn/ui、Radix UI、Tailwind 内置 utility，确认不够用才手写
- 路径别名：`@/` → `src/`
- 布局避免硬编码过小的 max-width（如 `max-w-md`），使用 `max-w-4xl` 以上或弹性伸缩
- **i18n 当次改动涉及的翻译 key 必须 commit 时全 12 语言齐全**：新增或修改的 key 当次就要把所有语言文件补齐（存量缺失的 key 不强制处理）。
- 避免不必要的重渲染（`React.memo`、`useMemo`、`useCallback`）
- **Tooltip 必须使用 `@/components/ui/tooltip`**，禁止用 HTML 原生 `title` 属性。优先使用 `<IconTip label={...}>` 简洁包裹
- **保存按钮统一三态交互**：saving（Loader2 旋转 + disabled）→ saved（绿色 + Check 图标，2 秒恢复）→ failed（红色，2 秒恢复）。使用 `saveStatus: "idle" | "saved" | "failed"` + `saving: boolean` 管理
- **Think / Tool 流式块展示约定**：内容块必须设置合理 `max-height`，超出后内部滚动；流式增量期间需自动滚动至底部，并实时显示耗时（结束后保留最终耗时）

### 后端（Rust）

- 新功能放 `crates/ha-core/` 单独模块文件；Tauri 命令在 `src-tauri/src/lib.rs` 注册，HTTP 路由在 `crates/ha-server/src/router.rs` 注册
- 内部用 `anyhow::Result`；Tauri 命令边界统一用 `Result<T, CmdError>`（[`src-tauri/src/commands/error.rs`](src-tauri/src/commands/error.rs)），`?` 直接传播 `anyhow::Error`，不要再写 `.map_err(|e| e.to_string())`；HTTP 路由按 axum 习惯返回 `Result<Json<T>, (StatusCode, String)>`
- 异步命令加 `async`，不要自己 `block_on`
- **禁止使用 `log` crate 宏**（`log::info!` 等），必须使用 `app_info!` / `app_warn!` / `app_error!` / `app_debug!`（定义在 `logging.rs`）。唯一例外：`lib.rs` 的 `run()` 中 AppLogger 初始化之前，以及 `main.rs` 的 panic 恢复
- 日志宏用法：`app_info!("category", "source", "message {}", arg)`
- **核心业务路径必须埋点**：Provider 调用、tool 执行、审批决策、failover 切换、compaction 触发、channel 收发、记忆提取/召回、cron 执行、配置变更等关键节点都要打日志，不要静默成功或静默失败（成功走 `app_info!`、异常走 `app_warn!` / `app_error!`）。日志既服务人工排查，也作为 **agent 自主修复**的首要信息源——内容要带最小复现上下文（输入摘要 + 关键决策 + 结果/错误），`category` / `source` 命名保持稳定以便 grep 聚合
- **禁止对字符串使用字节索引切片**（如 `&s[..80]`），必须使用 `crate::truncate_utf8(s, max_bytes)` 安全截断
- **跨平台分支**：优先用 `#[cfg(unix)]` / `#[cfg(windows)]`（macOS + Linux + BSD 共享 Unix 路径），少写 `target_os = "linux"`。新增跨平台原语统一放 `crates/ha-core/src/platform/`（`mod.rs` 是门面，`unix.rs` / `windows.rs` 是实现），不要在业务代码里散落 `#[cfg]` 分支。调用方用 `crate::platform::xxx()` 单一入口

## 安全红线

- **API Key 和 OAuth Token 禁止出现在任何日志中**
- `tauri.conf.json` CSP 当前为 `null`，不要放行外部域名
- OAuth token 在 `~/.hope-agent/credentials/auth.json`，登出时必须 `clear_token()`

## 易错提醒

- 修改 Tauri 命令后须同步更新 `invoke_handler!` 宏注册列表
- 新增 HTTP API 端点后须在 `crates/ha-server/src/router.rs` 注册路由
- 新增核心功能须放 `crates/ha-core/`，禁止在 ha-core 中引入 Tauri 依赖
- Rust 依赖变更后 `cargo check` 先行验证（workspace 级别）
- 前端新增 invoke 调用时须同步实现 Transport 的 Tauri 和 HTTP 两种适配
- 新增/修改接口时同步更新 [`docs/architecture/api-reference.md`](docs/architecture/api-reference.md) 对应功能域表格，该文档是 Tauri ↔ HTTP 对齐的单一真相源

## 设置（Settings）约定

所有用户可操作的配置必须同时具备 **GUI 入口** 和 **`ha-settings` 技能对应能力**，两者零偏差。新增/修改进入 `AppConfig` / `UserConfig` 且用户需要调整的字段时，**同一 PR 内三件事缺一不可**：

1. **GUI 控件**：[`src/components/settings/`](src/components/settings/) 对应面板，shadcn/ui + 三态保存按钮
2. **技能能力**：[`crates/ha-core/src/tools/settings.rs`](crates/ha-core/src/tools/settings.rs) 加读写分支 + 风险分级 + 必要的副作用提示；同步更新 [`core_tools.rs`](crates/ha-core/src/tools/definitions/core_tools.rs) 里 `get_settings` / `update_settings` 的 `category` enum；如果字段含凭据/敏感数据需要 read-only，把 category 加到 `BLOCKED_UPDATE_CATEGORIES` 并在 `read_category` 里 redact
3. **技能文档**：在 [`skills/ha-settings/SKILL.md`](skills/ha-settings/SKILL.md) 风险等级表里登记

### 风险等级判定

- **LOW**：UI 偏好、显示配额，不影响成本/安全（theme / language / notification / canvas 等）
- **MEDIUM**：行为调整，影响上下文、成本或输出质量（compact / memory\_\* / web_search / approval / multimodal / dreaming 等）
- **HIGH**：安全、网络暴露、全局键位、凭据、需要重启、**权限规则改动**、**审批策略改动**、**MCP 子系统级开关**（proxy / embedding / shortcuts / server / skill_env / acp_control / `permission.global_yolo` / `smart_mode` / `mcp_global` / `protected_paths` / `dangerous_commands` 等）——技能在 `update_settings` 前必须向用户二次确认。`global_yolo` 跨整 process 影响所有会话；`smart_mode` 改 strategy/judgeModel/fallback 重塑哪些工具自动放行；`mcp_global.enabled=false` 短路所有 MCP 调用；`protected_paths` / `dangerous_commands` 写错或删除条目会让本不该放行的命令直接走 Allow，必须二次确认

### 强制留在 GUI 的例外（read-only via skill）

**Provider 列表与 API Key**、**IM Channel 账号（`channels`）**、**MCP 服务器配置（`mcp_servers`）**、**`active_model` / `fallback_models` 的写入**这四类不进 `update_settings` 工具（凭据安全 + 运行时稳定性）。`get_settings` 仍可读取以便模型描述用户当前状态——`channels` 读时 `accounts[*].credentials` / `accounts[*].settings` 字段 redact 为 `"[REDACTED]"`；`mcp_servers` 读时 `env` / `headers` / `oauth` 字段 redact。所有写路径必须走对应 GUI 面板，由 GUI 触发 0600 凭据写入和监听器重启。

### 含凭据 category 的 read 脱敏（write 仍允许）

下列 category 因运行时切换 / 用户主动用 skill 配置等场景仍允许 `update_settings`，但 `get_settings` 响应必须 redact 凭据字段，避免 LLM 把 history 当 leak 通道。**所有新增带凭据子字段的 `AppConfig` field 必须接入 [`crate::tools::settings::redact_*_value`](crates/ha-core/src/tools/settings.rs) 同款 helper**：

- `web_search` — `providers[*].apiKey` / `apiKey2` 替换为 `[REDACTED]`
- `image_generate` — `providers[*].apiKey` 替换为 `[REDACTED]`
- `server` — `apiKey`（HTTP/WS Bearer Token）替换为 `[REDACTED]`
- `acp_control` — `backends[*].env`（常含 `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`）整张 map 替换为 `[REDACTED]`
- `skill_env` — 文档化为 secret 容器，技能层的二次确认警示已在 SKILL.md 标注；如需进一步 redact 同样接 helper

判定规则：read 时仅 `Some(non_empty_string)` 视为 secret 用 `"[REDACTED]"` 覆盖；`None`、缺字段、`Some("")`（被用户清空但保留字段）都保留原状，让 LLM 能区分"未设"与"已设但被清空"两种状态。

### 配置读写 contract（强制）

详见 [`docs/architecture/config-system.md`](docs/architecture/config-system.md)。硬规则：

- **读** 走 `ha_core::config::cached_config()`（`Arc<AppConfig>` 快照），禁止重新引入 `Mutex<AppConfig>` 或本地克隆
- **写** 走 `ha_core::config::mutate_config((category, source), |cfg| {...})`，禁止 `load_config()` + `save_config()` 手动克隆-改-存模式——无法防并发 lost-update（历史 image_generate stale bug 的根因）
- 写路径自动 emit `config:changed` 事件并落 autosave 备份，不要手动模拟

旧的 `AppState::config: Mutex<AppConfig>` 字段已于 2026-04-20 删除，PR 里出现 `state.config.lock()` 一律 reject。

## 文档维护

技术文档索引见 [`docs/README.md`](docs/README.md)，分为架构文档（`docs/architecture/`）和调研文档（`docs/research/`）。

代码改动时**必须同步更新文档**：

| 改动类型                                            | 需更新                                                                |
| --------------------------------------------------- | --------------------------------------------------------------------- |
| 新增/删除功能、命令、模块                           | `CHANGELOG.md`、`AGENTS.md`                                           |
| 技术栈/架构/规范变更                                | `AGENTS.md`                                                           |
| 已有子系统架构变更                                  | `docs/architecture/` 对应文档                                         |
| 新增架构级能力                                      | `docs/architecture/` 新建文档 + `docs/README.md` 索引                 |
| 新增/删除 Tauri 命令、HTTP 路由、`COMMAND_MAP` 条目 | `docs/architecture/api-reference.md` 对应功能域表格                   |
| 功能变化导致 README 过时                            | `README.md` + `README.en.md`（同一 PR 双语同步）                      |
| 新增调研/对比分析                                   | `docs/research/` 新建调研文档                                         |
| 修改 README 任一语言版本                            | 同一 PR 内同步另一语言（`README.md` ↔ `README.en.md`）                |
| 新增/修改 Release Notes                             | 同一 PR 内中英双份（`docs/release-notes/vX.Y.Z.md` ↔ `vX.Y.Z.en.md`） |

- `CHANGELOG.md`：[Keep a Changelog](https://keepachangelog.com/) 格式
- `docs/README.md`：文档索引，新增/删除文档时同步更新
- **架构文档强制**：功能架构发生改动（子系统边界、数据流、持久化格式、跨模块 contract 等）必须回头更新 `docs/architecture/` 下对应文档；**新增架构级能力**（新子系统、新核心模块、新协议层）必须在同一 PR 内新建架构文档并登记到 `docs/README.md`，不能只在 `AGENTS.md` 里一笔带过
- **README 回写强制**：功能变化（新增/删除/重大行为改动）导致 `README.md` 里描述过时、截图失效、功能矩阵漂移时，必须在同一 PR 内更新 `README.md`（并连带 `README.en.md`），不能留"下次顺手改"
- **README 双语同步强制**：项目根目录只有 `README.md`（中文）和 `README.en.md`（英文）两个面向用户的 README。**任何对其一的文案/结构/截图/链接改动，必须在同一次提交内对另一份做等价修改**，不能留待后续"顺手补"，避免中英版本漂移
- **Release Notes 双语同步强制**：`docs/release-notes/` 下每个版本必须同时有 `vX.Y.Z.md`（中文）和 `vX.Y.Z.en.md`（英文），两份顶部互加 `简体中文 · English` 切换链接。任一改动在同一次提交内同步到另一份，与 README 规则一致

### Review Followups 登记（强制）

**所有 code review（`/simplify`、`/codex:review`、`/codex:adversarial-review`、人工审查、subagent review 等）识别出来但本期 PR 决定不修的问题，必须登记到 [`docs/plans/review-followups.md`](docs/plans/review-followups.md)，与 PR 同 commit。**

适用范围：

- 范围超出当期 PR 主题的清理（跨 crate 重构、统一错误类型、抽公共 helper 等）
- 存量债务（review 顺手翻出来的）
- 优先级低 / 风险低、但确实是问题的"不优雅"
- review 给出了改进方向但当期不动的复用机会

不适用：

- review 判定为误报、不是问题的（直接驳回，不登记）
- 当期 PR 已经修掉的（不登记）
- 功能 backlog / 用户需求（去 issue tracker，不登记）

登记格式见 [`review-followups.md`](docs/plans/review-followups.md) 顶部的"条目模板"。**目的是让债务可见、可检索、可调度——不登记就等于忘记**。下次有人撞上同一处问题不应该重新发现一遍。

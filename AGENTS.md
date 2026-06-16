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

# Docker 自托管（server 模式）—— 完整指南见 docs/deployment/docker.md
docker compose up -d                          # 起 hope-agent
docker compose --profile with-ollama up -d    # + Ollama 本地 LLM sidecar
```

## 提交前检查（强制）

以下六条是 `git push` 的强制门禁（对应 CI 8 项 status check）；`pnpm install` 后 [`.husky/pre-push`](.husky/pre-push) 钩子会在 push 时按此顺序自动跑——**无需在 push 前手动重复执行**：

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
- **push 由钩子兜底**：`git push` 时钩子自动跑全套，**Agent 不要在 push 前手动重复跑一遍**；只有用户明确要求跑某项时才手动跑

## 分支与发布

> 实操流程（PR 工作流、tag 推送、cherry-pick backport、避坑速查）见 [`docs/release-process.md`](docs/release-process.md)。本节仅列契约面。

`main` 承载下一个 minor 版本的开发，已发布的 minor 版本对应一条 `release/vX.Y` 维护分支用于 patch 修复。两条分支之间**只允许 cherry-pick，不允许 merge**——`merge main → release/vX.Y` 会把未发布功能拖入维护分支。

### 工作流

- **修 bug**：从 `release/vX.Y` 切 `fix/vX.Y-<topic>`，PR base 选 `release/vX.Y`；合并并发版后 cherry-pick 回 `main` 再单独发 PR
- **新功能**：从 `main` 切 `feat/<topic>`，PR base 选 `main`
- **新 minor 发版**：`main` 上 `pnpm version X.Y.0` 打 tag，再 `git branch release/vX.Y vX.Y.0 && git push -u origin release/vX.Y`，CI 与 protection 通过 ruleset 通配符自动覆盖

### CI 与 branch protection

- [`.github/workflows/lint.yml`](.github/workflows/lint.yml) 与 [`rust.yml`](.github/workflows/rust.yml) 触发条件包含 `[main, "release/**"]`
- GitHub ruleset `main-branch-protection` 的 `conditions.ref_name.include` 覆盖 `~DEFAULT_BRANCH` + `refs/heads/release/**`：必须 PR、必跑 8 项 status check、禁 force push、禁删分支、`enforce_admins: true`
- 修改 workflow 的 job 名或 matrix 时需同步通过 `gh api` 更新 ruleset 的 `required_status_checks` context 列表

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

ha-core 主要领域：`agent/` `chat_engine/` `context_compact/` `memory/` `knowledge/` `skills/` `tools/` `channel/` `subagent/` `team/` `cron/` `acp/` `dashboard/` `recap/` `awareness/` `config/` `session/` `project/` `plan/` `ask_user/` `async_jobs/` `wakeup/` `failover/` `platform/` `security/` `logging/` `local_llm/`。Vendor skill 来源记录在 `THIRD_PARTY_NOTICES.md`。

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
- **桌面 release 单一来源**：`package.json`，`pnpm version` 钩子（[`scripts/sync-version.mjs`](scripts/sync-version.mjs)）同步 `src-tauri/Cargo.toml` / `tauri.conf.json` / `crates/ha-server/Cargo.toml` / `crates/ha-core/Cargo.toml`。`ha-server` 承载 Docker headless bin `hope-agent-server` 的 `CARGO_PKG_VERSION`，必须随桌面版本同步——`--version` 与 `app_update` `current_version` 都读它；`ha-core` 不发布也不是 user-facing binary，但作为 workspace 共享 crate 跟着 bump 让整个产品版本一致。CI tag 构建前跑 `pnpm release:verify -- --tag vX.Y.Z` 校验上面五个来源 + `Cargo.lock` 一致。Updater 私钥严禁入仓
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
- **前台 idle guard 单一入口**：前台 turn 的忙/闲标记（`ChatSessionGuard` → `ACTIVE_CHAT_SESSIONS`，后台任务 / subagent 完成注入靠它「忙时排队、空闲再注入」）在 `run_chat_engine` 入口按 `ChatSource::holds_foreground_idle_guard()`（Desktop / Http / Channel）统一创建；ACP 直跑 `AssistantAgent::chat`、在其 turn 边界自建。`ParentInjection`（注入自身会自取消）/ `Subagent`（独立子会话）排除。**新增对话入口不得再各自手搓 per-shell guard**——Tauri 壳保留一个更早的 guard 仅为「用户发消息即取消在途注入」，靠引用计数与引擎 guard 安全重叠

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
- **会话级无痕（`sessions.incognito`）**：单一真相源；不注入 Memory / Active Memory / Awareness、跳过自动提取；**关闭即焚**——不进侧边栏列表 / 全局 FTS / Dashboard 统计；**与 Project / IM Channel 互斥**（前端灰化 + 后端入口防御）。**四旁路守卫红线（Epic E）**：`is_session_incognito` fail-closed 三态（行不存在按无痕兜底）；`ToolExecContext.incognito` 单点注入，门控大工具结果落盘（无痕走内存）与异步任务 args/spool（无痕占位 + 不 spool）；AllowAlways 经 `choose_scope` 强制内存 `Session` scope 绝不落盘；焚毁 `cleanup_watcher` 在 `session:purged`（区别于 `session:deleted`）清盘 `tool_results/` + job 行/spool；异步注入与在途回合在会话已删/已焚时跳过（防幽灵回合）。全貌见 [`session.md`](docs/architecture/session.md#四旁路守卫epic-e)

### Knowledge Base（知识空间）

详见 [`knowledge-base.md`](docs/architecture/knowledge-base.md)（实现 + 设计契约 D1–D14 决策账本 + 子系统全貌）。对外名「知识空间」（slogan：你的第二大脑），代码中性（模块 `knowledge/`、工具 `note_*`、作用域 `for_knowledge`）。**本节只列跨 PR 红线，实现细节 / 数据流 / 内部结构一律在架构文档**。

- **两类存储分明（D9）**：注册表 + 访问绑定落 `sessions.db`（真相源）；`note/chunk/link/tag` + FTS5 + vec0 落 `~/.hope-agent/knowledge/index.db`（纯可重建缓存，删了从 `.md` 全量重建）。**笔记 = 真实 `.md` 文件，唯一真相源**
- **访问默认 deny（D10 + WS8）**：唯一入口 `effective_kb_access`——incognito 归零 → IM 默认归零（除非账号级 `kbAccessOptIn` + 群聊 per-chat 确认 `kbAccessChats`，群内 `/kb`；查不到 / 不匹配 fail closed）→ `max(session_attach, project_attach)` → 滤 archived → 外部只读 root cap `read`。source / origin / channel 经 agent 真接线透传，**subagent 按 origin 血缘判定不洗权限**
- **两鉴权平面物理隔离（D10）**：owner 平面（HTTP / Tauri，`service.rs`）= 本机 / API key 信任，看全部 KB **不经 attach**；agent 平面（`note_*` 工具）走 `effective_kb_access`。`/api/knowledge/{kb}/files/*` 纯 owner、无 session 参数 / 无 fallback
- **作用域闭合 + 外部只读（D11 + WS7）**：所有读写经 `WorkspaceScope::for_knowledge`；**外部绑定 root 默认只读（桌面也拒），KB 级 `allow_external_writes` opt-in 才解锁外部写**；HTTP 写再叠 `allow_remote_writes`。**后台自主维护永不写外部**（无视 opt-in）
- **写盘原子 + stale-write guard**：所有笔记写经 `crate::platform::write_atomic`（temp+fsync+rename，**禁回退 `fs::write`**）；`expected_file_hash` 比**磁盘当前 raw BLAKE3**（不比索引 `content_hash`）；`note_patch` 走 `old/new` 唯一文本命中（0 / 多次都拒）。坐标系 D14 / resolve 确定性 #8 / chunk D12 / 图谱布局 J 细节见架构文档「写盘 / 坐标 / 解析 / 布局契约」节
- **检索 / embedding 独立旗舰（D7）**：chunk FTS5 + vec0 → RRF → MMR → 回 note，**独立 store 绝不折进 `recall_memory`**；知识空间有自己的 `knowledge_embedding` selector（与 `memory_embedding` 物理隔离、不寄生不回退）+ `knowledge_chunk` 参数；二者重 reindex 副作用故 **GUI-only 不进 `ha-settings`**（类比 `active_model`）。排序参数 `knowledge_search`（融合权重 / RRF-k / MMR-λ / 候选倍数，`search.rs::KnowledgeSearchConfig`，`search_notes` 读、`clamped()` 钳值）**纯查询期无 reindex → 正常 MEDIUM 设置**（GUI + `ha-settings`，GUI 带一键恢复默认）
- **会话感知访问单一入口**：`Agent::resolve_kb_access()`（`agent/mod.rs`）是 agent 侧唯一解析链（source/origin/channel + WS8 + project + `effective_kb_access`，按回合 memoize），系统提示段 / 被动召回 / 工具门控三处共用，不得各自重写。**只服务 schema/prompt/召回，绝不 gate 执行**（执行走 live `access_map`，同一 `resolve` 路径）
- **无 KB 不注入笔记工具**：`is_kb_scoped_tool`（`note_*` + `session_to_note`，**不含 `knowledge_recall`**）在 schema 构建后过滤——纯 UX / 省 token，**不是安全边界**（执行层兜底）。系统提示「# Knowledge Bases」段同理：`resolve_kb_access()` 为空整段省略，绝不广告会拒的库
- **读取即 untrusted**：`[[note]]` 注入（桥①）+ 被动相关笔记（桥③，`knowledge_passive_recall`，opt-in 默认关）一律套 `<untrusted_external_data>` 信封、受 `effective_kb_access` 约束、**永不提升为 system 指令**；incognito 零召回、IM 未 opt-in 零访问
- **`knowledge_recall` 工具（D7）**：一次查 memory + 笔记两 store，**两段独立排序不混排**，薄编排器**绝不折进 / 改动 `recall_memory`**；`Standard{default_deferred:true}` 经 `tool_search` 发现
- **外部 vault 实时同步（D6）**：`notify` watcher（per-KB 线程 + debounce）+ bind / 启动 reconcile（mtime 增量 + prune）
- **侧边栏 AI 对话面板**：对话 = `kind='knowledge'` 会话，**从主会话列表 / `/sessions` / 全局 FTS 隐藏**；锚定落 `knowledge_chat_threads`（D9，级联删）。复用主对话 `useChatStream`（新增可选 prop，主对话不传 = 行为不变）；当前文档每轮经 `getExtraAttachments` 以 `source:"quote"` 注入（cache-safe）。选区编辑两路：quote chip 进对话由 AI 改写 / 浮动 `QuickRewriteBar` 一次性
- **工具精简 `ToolScope::Knowledge`（与 D10 正交）**：仅 schema/prompt 可见性收窄到白名单（`note_*` + `knowledge_recall` + 记忆 + 框架基础），**绝不动 KB 访问**——访问仍由 `effective_kb_access` 单点裁决
- **精灵 / 灵感模式（`sprite/`，默认关）**：前端**多触发源**（编辑停顿 / 打开笔记 / 对话回复后 / 周期写作 / 大段粘贴，各 `triggers.*` 可配，仅在 AI 对话面板打开时挂）→ owner 命令 `kb_sprite_observe_cmd` → 后端统一节流（cooldown + 时限 + doc-hash 去重）+ side_query（persona 随 `proactive` 分主动/克制档）+ emit `sprite:casting`（LLM 调用期间，驱动猫咪「施法」光效）/ `sprite:suggestion`（紫色柔光气泡，不进消息流）→ 前端瞬态展示。**incognito 零精灵**（前后端双关卡）。非 agent 工具，设置三件套（MEDIUM）
- **自主维护（Layer 2，默认全关）**：后台扫内部 KB 产提案进 `sessions.db`（D9，级联删）；落地经 owner 平面（用户已批准故**绕 D10**，但写前重读磁盘 hash 守 stale-write + 跳外部只读）；`ha-settings` 归 **HIGH**
- **块级引用（仅 Obsidian `^block-id`，Logseq 不做）/ 原生大纲只读视图**：见架构文档；大纲红线**只读、永不替代 CM6 底座**
- **新增 KB 工具 / 端点**：工具走 `tools/note.rs` + `core_tools.rs` + `execution.rs`；Tauri / HTTP owner 薄壳调 `knowledge::service`；逻辑全在 ha-core（红线）

### 工具 & 审批

详见 [`permission-system.md`](docs/architecture/permission-system.md) / [`tool-system.md`](docs/architecture/tool-system.md) / [`browser.md`](docs/architecture/browser.md)（浏览器 8-action 表面 + 双 backend）。

- **统一权限引擎 v2**：所有调用走 `permission::engine::resolve_async()`，优先级 **Plan > Internal > YOLO > Protected/Dangerous > AllowAlways > Session 模式 preset > 兜底 Allow**
- **Session 模式三选一**：`default | smart | yolo`，`PermissionModeSwitcher` / `/permission` 切换；`AgentConfig.capabilities.default_session_permission_mode` 决定新会话初始 mode
- **Smart 模式忽略 `custom_approval_tools`**——UI 必须显式提示
- **保护路径 / 危险命令 / 编辑命令**：三独立列表，存 `~/.hope-agent/permission/*.json`；非 YOLO 模式强制弹窗（AllowAlways 按钮置灰），YOLO 只 `app_warn!` 不弹
- **Global YOLO**：CLI flag `--dangerously-skip-all-approvals` 与 `permission.global_yolo` OR 组合；判定入口 `security::dangerous::is_dangerous_skip_active()`，**与 Plan Mode 正交**
- **审批超时**：`approval_timeout_secs`（默认 300s，`0` 不限）+ `approval_timeout_action ∈ deny|proceed`。**strict 原因超时永不放行**（红线）：`proceed` 只对非 strict 生效；strict 原因（`forbids_allow_always`：保护路径 / 危险命令 / 高危 macOS 控制 / Plan-ask）超时强制 deny + `app_warn('permission','strict_timeout_deny')`。strict 判定单一真相源 `AskReason::forbids_allow_always`，`ApprovalReasonKind::is_strict()` 镜像（穷举单测断言一致）。超时另发统一 `approval:resolved`（`source=timeout_deny|timeout_proceed`）撤窗
- **审批授权来源审计**：每个后台 job 的 `background_jobs.approval_origin` 列记录授权方式（`ApprovalOrigin`：user / timeout_proceed / yolo / auto_approve / external_pre_approved / policy_allow），由审批闸单点算出写入 spawn ctx（exec + 非 exec 全覆盖）
- **无人值守审批 fail-closed**（[`permission::approval_surface`](crates/ha-core/src/permission/approval_surface.rs)）：审批阻塞前经单一入口 `check_and_request_approval` 预检 `evaluate_approval_surface(session_id)`——确证无人能批（cron `is_cron` / 无客户端 headless + 无 IM attach / ACP 无 capability / subagent 无父 surface）时按 `permission.unattended_approval_action ∈ deny(默认)|proceed` 处理：deny → `ToolRejection::denied_unattended` 即时拒绝(不再永久挂死)、proceed → 自动放行；emit `approval:unattended` 事件。**保守红线**：任何可能 surface(desktop 窗口 / web 客户端 / IM)即 Attended,唯 cron 例外。判 ACP 必须用 `is_acp()` 非 `ChatSource`(ACP 复用 Http)
- **Agent 工具开关**：`AgentConfig.capabilities.tools.allow/deny` 仅表示非 Core 工具的显式开 / 关覆盖；Core 工具不受影响。system_prompt / schemas / tool_search / 执行层统一走 `dispatch::resolve_tool_fate`
- **工具结果磁盘持久化**：> `toolResultDiskThreshold`（默认 50KB）写盘，上下文留 head+tail 预览
- **后台任务统一模型（Background Job，R1）**：异步工具子系统升格为统一后台任务模型——表/文件 `background_jobs`（`JobKind = Tool | Subagent | Group`，三类均已落地），**单一入口 `async_jobs::JobManager`** front 全部 spawn / cancel / list / replay / schedule（`spawn_explicit_job` 等收敛为其 `pub(crate)` Tool executor 内部，新增后台单元一律加 `JobManager` 方法而非平行 API）。模块名 `async_jobs/` 与 log category `"async_jobs"` 按「沿用血脉演进」保留（**诊断按 `category='async_jobs'` grep,DB 却是 `background_jobs`**）；EventBus 事件已统一 `job:*` 前缀（R3，见下）；`RuntimeTaskKind::AsyncJob` 内部枚举名不变；`progress` / `priority` / `attempt` 等列待对应 slice 消费再加（纯缓存 drop-rebuild，延后零成本）
- **异步 Tool 执行**：`exec` / `web_search` / `image_generate` 标 `async_capable=true`；落 `~/.hope-agent/background_jobs.db` + spool；`job_status` 多作业管理面（`action ∈ status|list|wait|cancel|result`，`tool_job_status(args, session_id)`）——`list` 枚举本会话在途、`wait{ids?,mode,timeout}` 短便利同步（clamp ≤10s，超时回 `still_running` 不长阻塞）、`cancel(id)` 跨进程取消。**长 fan-out 等齐的正道仍是注入而非 `wait`**（`batch_spawn` 的 Group 等齐后合并注入一轮，R5）
- **后台工具并发两层硬配额（R7.1，`async_jobs::slots`）**：全局 `async_tools.max_concurrent_jobs`（默认按核数 `clamp(cores-2,4,16)`）+ 每会话 `max_concurrent_jobs_per_session`（**默认按硬件取值 ≈ 全局 cap 的 3/4、band [3,12]、恒 < 全局**——固定值会在低核机上 ≥ 全局 cap 而 no-op,`0` = 不限）。`SlotManager::reserve_inner` 两层都要有空位才发 `SlotReservation`，否则**入队**（status `Queued`,非拒绝);调度器 `pick_fair_index` 在已达每会话上限的会话上**跳过**(防 head-of-line),RAII Drop 释放槽 + 唤醒调度器。**auto-background detach 占槽**靠 `slots::reserve_forced`（任务已在跑、不能排队/拒绝,只强制计数,可短暂超全局 **及每会话** cap——per-session cap 约束的是新后台任务的**准入**,不是已在跑的 auto-detach 任务数),经 `Arc<Mutex<Option<SlotReservation>>>` 随 worker 线程结束释放。**调度器/队列 per-process**（队列持不可持久化 live ctx,每进程一个 tier-agnostic 调度器,与 `replay_pending_jobs` 的 Primary-only 相反)。**per-kind 双域分治不进单一 struct**:tool 池在此,subagent 池在 `subagent::spawn`（per-session 拒绝,默认 8 `subagents.maxConcurrent`,R7.2),同轮安全工具走信号量 8（R7.3),结构类（depth/batch/turn）拒绝不排队——勿合并成一个 per-kind 配额表（speculative generality）
- **后台 job 重试（R7.4,`async_jobs::retry`）**：瞬时失败自动退避重投,**默认关闭(opt-in)**——纯策略 `decide(tool,attempt,error,cfg)` 仅对 `JobError::Failed`（干净 dispatch 错误、job token 未动）且**幂等可安全重跑工具**（`is_retry_eligible` 代码级白名单 = `web_search`/`web_fetch`,**非用户旋钮**）重投;`exec`/`image_generate`（副作用/非确定性）、`Cancelled`/`DeniedByUser`/`TimedOut` 永不重投。**默认 false**:eligible 多为**计费**供应商(`web_search`),重投会重计费,故交用户按需开启。`run_tool_once`（单次 dispatch+timeout+cancel,explicit + auto-bg 共用）+ `run_tool_with_retry`（loop + 可取消退避,job-level cancel 期间停重投;`max_attempts` 钳到 10;eligible 工具不得带 output_tail ring,debug_assert 守）。`max_job_secs` 是 **per-attempt** 预算(重投各一份)。配置 `async_tools.{retryEnabled,maxRetryAttempts}`（MEDIUM）。**新增 async_capable 工具若有副作用或计费,务必不要进 `is_retry_eligible`**
- **统一 `job:*` 事件命名空间（R3）**：所有后台任务生命周期走 `async_jobs::events` 发 `job:{created,updated,progress,completed}` + `job:mark_injected_failed`（替代旧 `async_tool_job:*`，破坏性 drop），kind-tagged（`tool` / `group`）+ `session_id`。`progress` 目前 Group 报 N/M 子完成。**`subagent` kind 沿用更丰富的 `subagent:*` 流不双发**；R4 面板合并两路 + `job_status list`。新增后台任务事件一律走 `events` helper，勿散落 `bus.emit`
- **后台 exec 命中审批点 park 为 `AwaitingApproval`（R8，`async_jobs::approval_bridge`）**：显式 `run_in_background` 的 exec **不再脱钩前同步审批**（`should_run_exec_reorder_gate` 仅 `AutoBackgroundEligible` 跑——保 ASYNC-2「审批不计入 `auto_background_secs`/`max_job_secs`」预算）——其命令 gate 落到后台 job 线程,attended 审批经 **thread-local 桥**翻转 job 行 `Running⇄AwaitingApproval`：`tools::approval` 定义 `BackgroundApprovalScope`/`BackgroundApprovalBridge`（**tools 零依赖 async_jobs**——runner 在 `run_job_to_completion` 注入闭包回调 job DB）,`check_and_request_approval` attended `rx.await` 前 `notify_bg_park`、`BgResumeGuard` Drop 兜底 resume（覆盖 resolve/timeout/**cancel-drop**)。批准续跑、拒绝/超时-deny **终态 `DeniedByUser`**（折进 `Failed` 状态,STOP 文本经注入保留)——**R8 验收据此 revise ASYNC-1「拒绝的 exec 不成 job」**(显式后台 exec 拒绝现成一条终态 job)。unattended/strict 仍 fail-closed deny（内层 gate 在 `rx.await` 前返回,**不 park**)。审批纯内存:**重启 parked job → `Interrupted`**（`list_running` active 集已含 `awaiting_approval`,无需新持久化）;**parked job 持槽不释放**（避免 resume 无空槽死锁,审批 `approval_timeout_secs` 兜底);cancel parked job → token trip 使 job `Cancelled` + `on_resume` 经 `dismiss_parked_job_approval` 拆孤儿审批弹窗（**remove-if-present 才 emit**,在 future 已 drop 后跑、不竞 `select!`);`set_approval_origin` 修正 spawn 时占位 origin(F6 审计)。**后台 subagent 内层审批只 block-and-wait、投影态 label 滞后**（子 session 跑、需事件监听非 thread-local)留 follow-up。`mark_cancelling`/`update_terminal` WHERE 已含 `awaiting_approval`,新增非终态状态须同步这些 active filter
- **运行中输出尾巴 `output_tail`（R3 ①）**：后台 `exec` job（**显式 `run_in_background` 与 auto-background 两路**）的 stdout/stderr 实时 tee 进进程本地有界 ring（`async_jobs::output_tail`，默认 8KB），`job_status(status)` 对 running job 返回 `output_tail`。**加法式**：仅 `ctx.output_tail_job_id` 为 Some（exec + 非 incognito）才走 tee，**前台同步 exec 不动**；显式路 ring 在 finalize / 队列取消 / 会话删清，auto-bg 路在 worker 内非 detach 终局（`next.is_none()`）清、detach 走 finalize 清；incognito 永不注册
- **后台任务面板 + 完成合并窗口（R4）**：owner-plane（host-trusted）读 `JobManager::list_session_snapshots(session)` / `get_job_snapshot(id)` → `BackgroundJobSnapshot`（camelCase 展示向,与 model-facing `job_status` JSON 物理分离,**Group 子投影折叠进 Group 行**）；Tauri `list_background_jobs` / `get_background_job` + HTTP `GET /api/sessions/{id}/background-jobs`、`GET /api/background-jobs/{id}`（Bearer 同 session 端点；owner 平面看全部不经 agent-scope,与 KB owner 平面一致）；前端镜像类型 `src/types/background-jobs.ts` + `useBackgroundJobs` 单订阅喂头部徽标 / 独立面板 / 工作台区块。**取消统一复用 `cancel_runtime_task(kind=async_job)`,不新增取消端点**。**完成注入合并窗口**：`async_tools.completionMergeWindowSecs`（默认 3,`0` 关）—— `finalize_job` 改走 `injection::enqueue_injection` 缓冲,同会话窗口内完成的多 tool job 合并一条 `<task-notification-batch>` 一轮注入（省计费 turn）。纯内存 live-path：崩溃则 terminal-but-uninjected,重启 `replay_pending_jobs` 各自补投（不丢、不合并）；Group(R5) 是预合并特例绕过；沿用 ghost-turn 闸 + 逐 job claim/release + on_injected 逐行恰好一次。完成桌面通知 `notification.notifyOnBackgroundJobComplete`（默认开,仅 `job:completed` 的 completed/failed/timed_out + 仅后台）
- **SSRF 统一策略**：出站 HTTP 必须走 `security::ssrf::check_url`；**新出站入口严禁自写 IP 校验**
- **文件 Diff 元数据**：`write` / `edit` / `apply_patch` / `read` 通过 `ToolExecContext.metadata_sink` 旁路传出 JSON；持久化到 `messages.tool_metadata` 列；前端右侧 `DiffPanel` 渲染（与 PlanPanel / CanvasPanel 视觉互斥）
- **工作台面板（Workspace）**：右侧互斥面板之一（`src/components/chat/workspace/`），三段聚合本会话任务进度 / 碰到的文件（读 + 改）/ URL 来源。**文件 / 来源走 `useWorkspaceArtifacts` 混合数据**：后端**读时聚合**（[`session::aggregate_session_artifacts`](crates/ha-core/src/session/artifacts.rs) 扫全会话 `tool_metadata` + `tool_result`/正文，`load_session_artifacts_cmd` / `GET /api/sessions/{id}/artifacts`，**只回摘要、无 before/after、每段最近 1000 封顶**给 `*Truncated` 标记）给完整历史 + count 真值；叠加当前轮 **live tail**（`useSessionFileChanges` / `useSessionUrlSources` 扫内存 `messages`，带结构化 diff、含流式中）按 `path` / `url` 合并（live 覆盖重叠项、未持久化的流式新增前插）。**聚合 dedup/排序规则 TS（live）+ Rust（后端）两份必须同步**，改一处改两处（注释互指）。窗口外（仅后端摘要、`diff=null`）文件无历史 diff，点击走「预览当前内容」；窗口内文件保留 `DiffPanel`。**无痕会话跳后端只用 live tail**（守「关闭即焚」）。输出 / 来源两段各定高内部滚动 + 滚到底自动增量渲染（[`useScrollPagedRender`](src/components/chat/workspace/useScrollPagedRender.ts)，无「加载更多」按钮）。任务进度从输入框上方移入此面板，输入框仅留 `WorkspaceStatusBar` 状态条；首次有内容自动展开、关闭后本会话不再自动弹（仿 BrowserPanel dismissed 模式）
- **文件操作统一**（详见 [`file-operations.md`](docs/architecture/file-operations.md)）：Markdown 链接 / 消息下挂文件 / 工作台产物文件三处**禁止各写 open/download**，统一走 `src/components/chat/files/`——纯策略 [`fileActions.ts`](src/lib/fileActions.ts)（`resolvePrimaryFileAction` / `resolveFileMenuActions`，按 `fileKind` × `supportsLocalFileOps()` 决议）+ `useFileActions`（消息树读 `fileActionsContext`，面板外用 overrides）+ `FileActionMenu`（右键 + ⋯）。行为矩阵：可预览类型（text/code/markdown/pdf/office/audio/video/image）点击=右侧 `FilePreviewPanel` 预览；其余本机=打开、远端=下载。新增可预览类型改 [`fileKind.ts`](src/lib/fileKind.ts) 的 `isPreviewableKind`。预览面板复用文件浏览器 `FilePreviewPane`（吃 `PreviewSource`，三适配器：project-fs / 绝对路径 / MediaItem）。**office 类型在 `FilePreviewPane` 内走前端富渲染**（[`files/office/`](src/components/chat/files/office/)：docx→docx-preview、xlsx→SheetJS、pptx→pptxviewjs，均懒加载、解析 `rawUrl` 取得的原始字节）；旧二进制 `.doc`/`.ppt` / 损坏 / 超大 / 渲染失败一律回退后端 `extractDoc` 文本提取（`OfficeTextFallback`，只在回退时才调后端，省一次提取）。**LLM 注入路径（`file_extract` 文本提取）不受影响**。**文件类型图标统一走 [`FileTypeIcon`](src/components/icons/FileTypeIcon.tsx)**（vscode-icons 彩色格式图标，`unplugin-icons` 构建期内联、仅打包所需、离线 CSP 安全；`FileMimeIcon` 是其 `(mime,name)` 薄适配）——新增可视化文件图标处复用它、按扩展名/MIME 扩 `EXT_ICON`，勿再用单色 lucide `File*`
- **preview-by-path 鉴权红线**：按绝对路径读取/提取/取流（Tauri `preview_read_text` / `preview_extract` + 客户端 `convertFileSrc`；HTTP `GET /api/sessions/{id}/files/{read,extract,by-path}`）。HTTP 三端点共用 [`authorized_canonical_file_path`](crates/ha-server/src/routes/sessions.rs)（**被会话 tool 消息引用 ∪ 落在会话工作目录内**，[`WorkspaceScope::contains`](crates/ha-core/src/filesystem/workspace.rs)），二者皆非的主机任意路径一律 403——**远端严禁放行任意主机路径**（= 远程任意文件读）。桌面信任本机。ha-core 侧 `read_text_abs` / `extract_abs`（[`filesystem/ops.rs`](crates/ha-core/src/filesystem/ops.rs)）不做 scope 容器，鉴权由 HTTP 边界负责

### Hooks

详见 [`hooks.md`](docs/architecture/hooks.md)（hooks 子系统单一真相源：28 事件矩阵 / 数据流 / 5 handler / 四层 scope / 安全 / 测试 / Roadmap）。

- **字段级对齐 Claude Code hooks 协议**；核心全在 `ha-core::hooks`（**零 Tauri 依赖**），desktop / server / ACP 共用
- **唯一入口 `HookDispatcher::dispatch(event, input)`** + `hooks::fire_*` 助手：内部封装 per-cwd scope 解析 / matcher 过滤 / 并发执行（catch_unwind 隔离）/ 去重 / 超时 / 聚合，调用方只读 `HookOutcome`；**严禁在业务代码里 match 具体 handler 类型**
- **28 事件**：24 个真触发（阻断型 `UserPromptSubmit`/`PreToolUse`/`PreCompact` + 21 个观察型），4 个协议保留（`WorktreeCreate`/`WorktreeRemove`/`TeammateIdle`/`InstructionsLoaded`——无对应概念，可配置不 dispatch）。`is_observation_only` 列表里的事件 `block` 决策降级为非阻断 + log
- **5 种 handler 全实现**：`command` / `http`（SSRF-gated）/ `mcp_tool` / `prompt`（side-query）/ `agent`（spawn 子 Agent）
- **四层 scope UNION**（无覆盖）：user（`config.json`）+ managed（`/etc/hope-agent/hooks.json`）编进全局 registry；project（`<工作目录>/.hope-agent/hooks.json`）+ local（`hooks.local.json`）按会话工作目录经 [`scopes::resolve_for_cwd`](crates/ha-core/src/hooks/scopes.rs) 合并（per-cwd 缓存 + mtime/generation 失效）。fire 路径统一走 `scopes::any_handlers_for(event, cwd)`，project-only hook 也能触发。**project/local 默认关**（`hooks_allow_project_scope` opt-in，默认 `false`——仓库 check-in 的 hooks 不因会话 cwd 指向就自动执行，供应链防护；Settings → Hooks 开启，`ha-settings` 只读）；`disable_all_hooks` 关所有 scope
- **配置走 config contract**：读 `cached_config().hooks`，user scope 写 `mutate_config(("hooks", source), …)`；`config:changed` 触发 `registry::reload_from_config`（user+managed 合并 + bump generation）。**`ha-settings` 技能只读 hooks**（写被 `BLOCKED_UPDATE_CATEGORIES` 拦截——可写=模型给自己装命令执行）
- **四入口统一 preflight**：Tauri / HTTP / IM / ACP 的 user message 持久化前过 [`agent::preflight::user_prompt_preflight`](crates/ha-core/src/agent/preflight.rs)（`UserPromptSubmit` 阻断点）；**新增 user message 入口必须走它**。block 的 prompt 不入会话/LLM 上下文，落一条 `event` 行
- **新增 hook 事件须埋点 + 测试**：阻断型构造 `HookInput` 调 `dispatch`，观察型走 `hooks::fire_*`；新事件须同步更新 `types.rs` 的 `common()`/`matcher_target()`/`is_observation_only()` 三处 match；审计统一 `category="hooks"`

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
- **激活入口**：`skill({name, args?})` 工具（`internal + always_load`）；斜杠 `/skillname args` 内联走 `[SYSTEM: ...]` + `display_text`（**当前未应用 `allowed-tools` / `check_requirements`**）；输入框 `@skill` 提及（markdown 链接 token `[@标签](#skill:<name>)`）由后端 send-time 注入到 `extra_system_context`（与 `[[note]]` 平行）
- **`@skill` 固定 allowlist + 链接 token（红线）**：`@skill` 只对**内置、固定**技能开放，非通用注入入口——`skills/mention.rs::AT_MENTIONABLE_SKILLS`（office 三件套 + `ha-browser` + `ha-mac-control`，后者 macOS-only）；`resolve_inline_skill_mentions` 扫 `[@…](#skill:<name>)` 链接 href、过 allowlist ∩ invocable ∩ OS，越界名静默跳过留原文。token 用 **markdown 链接**（标签本地化 + `#skill:` fragment href，不用 `skill://`——Streamdown 固定 sanitize 会剥自定义 scheme），输入框与历史**同一 token 渲染同一玫瑰粉 chip**（历史经 `MarkdownLink` 按 `#skill:` 派发 `SkillMentionChip`）。菜单走 `list_mentionable_skills`（Tauri / HTTP `GET /api/skills/mentionable`）；`enableSkillMention` 默认关、主对话 opt-in
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
- **后台 subagent 投影进 Background Job（R6，单向）**：用户委派的后台 subagent run（`!skip_parent_injection`——排除 plan/team/hook 内部 spawn；且 `!parent_incognito`）在 `spawn_subagent` 建一条 `background_jobs`（`kind=Subagent`、`subagent_run_id` FK）投影,与后台工具 job 共享 `job_status` list/status/cancel + 未来面板。**`subagent_runs` 是执行真相源,投影只承载 status/生命周期、绝不持有 run 正文(task/result/error)、绝不反写**。同步走单一 choke point `SessionDB::update_subagent_status`→`JobManager::sync_subagent_projection`（覆盖 run 生命周期 + 三处 kill fallback；terminal 冻结）；投影 `injected=true` **永不进工具注入/replay 路径**（subagent 自有 `inject_and_run_parent`，防双注入）；取消经 `cancel_job` kind=Subagent 分支路由到 `subagent::request_cancel_run`（注册表 cancel + DB-Killed 兜底，与 `kill` 工具同源），**不跑工具 job 的 hook/注入**。新增后台单元一律加 `JobManager` 方法,勿另起平行 API
- **`batch_spawn` = Group fan-out（R5，合并注入一轮）**：`batch_spawn` 建一条 `kind=Group` 协调行（`group_id` 列关联子投影；`args_json={"sealed":bool}`；`injected=true` 自发合并注入不进工具 replay），N 个子 subagent 携 `group_id`、**抑制个体注入**;全部子到终态时单赢 CAS（`claim_group_completion`，`Running→Completed`）认领并发**一条**合并 `<task-notification>`（**join-all-settle**:部分成功 + 各自终态,含 task/label;复用既有 `subagent_result` 前端 pill）。红线：**建 group 前预校验全部 task**（任一非法整体拒——否则已建子代理永不 seal → 漏交付）;`seal_group` 在 spawn 完成后标 sealed 并兜底完成检查;**取消 kind=Group 先标 group 终态再取消子 run**（无 flag 兜底同步标子 Killed 会同线程触发 join,group 须先终态使 CAS 落败）;投影插入失败的子 **ungroup 回退个体注入**（不丢结果）;incognito / jobs DB 未初始化不建 group（回退个体注入）;join 真相读 `subagent_runs`,group 行**绝不持有 run 正文**。重启 in-flight group 同 R6 标 Interrupted（子结果仍在 `subagent_runs`,合并注入不补发）
- Agent Team 模板 GUI 预配 + 模型按需发现；`TeamTemplateMember.description` 注入子 session 身份段
- Cron `delivery_targets`：final assistant text fan-out 到 IM；IM 会话内未显式传时自动取当前会话，显式 `[]` 关闭
- **`schedule_wakeup`（自我定时唤醒，`crate::wakeup`）≠ cron**：agent 发起的**一次性**「N 秒后把我叫回**当前会话**续跑」，到点经 `inject_and_run_parent`（复用 idle-gating + 取消 + 重试）注一条 `<wakeup>`+note 起新 parent turn。`wakeups.db` 持久 + 进程本地定时器；clamp `[10s,24h]`、每会话 pending ≤5（结构类③拒绝**不排队**）、`mark_fired` 仅注入落地置位（Abandoned 留 replay）、**replay Primary-only**（共享行，防 Secondary 双投）、incognito 仅内存、会话删/焚经 `cleanup_watcher::purge_for_session` 取消。**不与 cron 复用入口**（cron=用户配置周期 + 可独立会话/fan-out）

### IM Channel

详见 [`im-channel.md`](docs/architecture/im-channel.md)。

- 12 个插件，状态文件落 `~/.hope-agent/channels/`；入站媒体走 plug → worker → `Attachment` → `~/.hope-agent/attachments/{session_id}/`
- 工具审批通过 EventBus `approval_required` 监听，按 `supports_buttons` 走原生按钮或文本；`auto_approve_tools=true` 跳审批（opt-in）。**绕过审计**：`auto_approve_tools` 跳过引擎门时，若被跳过的调用本会命中 strict 原因（`forbids_allow_always`），跑一次 no-enforce 探测并 `app_warn('permission','auto_approve_bypass')`——纯审计不拦截（排除 `external_pre_approved` async 重入防重复告警）
- **多端审批一致性红线（Epic G）**：所有决议路径(submit / 超时 / 删会话 / eviction)emit `approval:resolved {requestId,sessionId,decision,source}` 统一撤窗(`ApprovalResolutionSource`:gui/http/im/session_deleted/timeout_deny/timeout_proceed/eviction);listener 收到即 `drop_pending_by_request_id` 清 `TEXT_PENDING`。**IM 应答 fail-closed**:按钮回调缺源直接拒(approval 不复用共享 `validate_callback_source_for_session` 的 `None→Ok`,那只留 ask_user)、文本回复 submit 前同样校验 session↔chat(handover 后旧 chat 拒)。**chat 接管**(eviction)在 notify 门前无条件枚举拒决该 session 全部 pending 审批 + `drop_pending_for_chat`
- **Auto-start 失败统一走 [`channel/start_watchdog.rs`](crates/ha-core/src/channel/start_watchdog.rs)**——退避 30s/60s/2m/5m，sweep 15s，user 操作永远胜过 watchdog；失败日志带 `classify_channel_error` 分类
- **流式预览 Transport 三选一**（[`worker/streaming.rs`](crates/ha-core/src/channel/worker/streaming.rs) `select_stream_preview_transport`）：`Draft (Telegram DM 专属) > Card (capabilities.supports_card_stream，目前仅飞书 cardkit) > Message (send_message+edit_message)`。Card / Draft 失败有降级（Card 创建期失败 → 切 Message；中后期 `update_card_element` 失败 → `broken=true`，收尾走 `send_message` 兜底）。新增飞书风格"无编辑标记"流式靠 `ChannelPlugin` 上 4 个 default-impl=`Err` 的 cardkit trait 方法（`create_card_stream` / `send_card_message` / `update_card_element` / `close_card_stream`）—— 仅飞书实现，11 个非飞书 channel 的 `capabilities.supports_card_stream=false` 走旧路径不变
- **`ImReplyMode` 三态对所有渠道生效**（`ChannelAccountConfig.settings.imReplyMode`，默认 `split`，[`channel/types.rs`](crates/ha-core/src/channel/types.rs)）：
  - `split`（默认）：每 round 的 narration + 媒体按时序作为独立消息发送；**流式渠道每 round 都是真打字机**——stream task 在 `tool_call → text_delta` 边界把当前 preview finalize 掉、按 transport 收尾、把该 round 媒体发完，再为下一 round 起一条全新 preview，每 round 用户都看到 typewriter；非流式渠道每条 narration 一次性
  - `final`：丢弃中间 round narration，只发最后 round 的 text + 末尾发所有媒体；不启用流式预览
  - `preview`：流式渠道用 preview transport 渲染合并文本（旧行为），跨 tool round 的相邻 narration 之间插入一个换行；非流式渠道降级为 `final`

  实现要点：
  - **stream task transport 由 mode 决定**——dispatcher 在 spawn stream task 之前算 `account.im_reply_mode()`，`Preview | Split` 都调 `select_stream_preview_transport`（`Split` + 非流式渠道返 `None` 时 stream task drain events、`finalized_rounds=0`），仅 `Final` 强制传 `None`
  - **round 边界 + 媒体归组**靠 `ChannelStreamSink::round_texts: RoundTextAccumulator<RoundOutput { text, medias }>`（[`chat_engine/types.rs`](crates/ha-core/src/chat_engine/types.rs)）。state machine：`text_delta → current.text`；`tool_call → close round`（idempotent，多 tool_call 同 round 只关一次）；`tool_result(media) → 挂到刚关闭的 round`
  - **stream task per-round finalize**（split + 流式渠道，[`worker/streaming.rs`](crates/ha-core/src/channel/worker/streaming.rs) `finalize_split_round`）：边界检测靠 `event.contains("\"type\":\"tool_call\"")` + `extract_text_delta`；按 transport 收尾——Message reset `preview_message_id` / Card `close_card_stream` / Draft `send_message` 把草稿落地——再从 `round_texts.completed[idx].medias` 取媒体走 `deliver_media_to_chat`。返给 dispatcher 的 `StreamPreviewOutcome.finalized_rounds` 记录已发 round 数
  - **dispatcher** 在 [`channel/worker/dispatcher.rs`](crates/ha-core/src/channel/worker/dispatcher.rs) 按 mode 调 `deliver_split` / `deliver_final_only` / `deliver_preview_merged` 三选一；`deliver_split` 跳过 `rounds[..finalized_rounds]`（stream task 已发），剩余 round 走 `send_message(text)` + `deliver_media`，最后 round 通过 `send_final_reply` 走 finalize 路径；`deliver_preview_merged` 与 stream task 共用 preview round 拼接规则，跨 tool round 补一个 `\n`，保证 live preview 与最终定稿一致
- **配置入口**：GUI（`EditAccountDialog` 三选项 Select，`preview` 选到非流式 channel 时 hint「will degrade to Final」）+ `/imreply [split|final|preview]` 斜杠命令
- **`ChannelStreamSink` 短路条件用 `contains` 不能用 `starts_with`**：`emit_tool_result` 走 `serde_json::json!({...})` + 默认 `BTreeMap`，键按字母序输出（`call_id` 永远在前），任何 anchor 在 `{"type":...` 的 fast-path 都不会触发。`media_items` / `tool_result` / `text_delta` / `tool_call` 检测都用 `event.contains(...)`，rarer-needle-first
- **`channel_conversations` 1:1 attach（双向）**：每个 (channel, account, chat, thread) 在任意时刻只关联一个 session（`uq_channel_conv_chat`），且每个 session 在任意时刻只能被一个 IM chat attach（`uq_channel_conv_session`）。新 chat 通过 `/session <id>` 或 handover 接管时，目标 session 上的旧 attach **物理 detach** 并通过 `channel:session_evicted` 事件发"会话被接管"系统消息——不再保留 observer 行。helper 入口 [`channel/db.rs`](crates/ha-core/src/channel/db.rs)：`attach_session` / `detach_session` / `update_session` / `get_conversation_by_session`，**不要直接写 `channel_conversations`**。
- **`source` 字段**：`inbound`（IM 入站新建）/ `attach`（`/session <id>` 显式接管）/ `handover`（GUI handover 或 `/handover` 推到该 chat）。
- **GUI ↔ IM live 流式镜像**：desktop / HTTP 触发的 turn 通过 [`chat_engine/im_mirror.rs`](crates/ha-core/src/chat_engine/im_mirror.rs) `attach_im_live_mirror` 把 `ChannelStreamSink` 注册到 [`SinkRegistry`](crates/ha-core/src/chat_engine/sink_registry.rs)，引擎 `emit_stream_event` 末尾的 fan-out hook 在每帧把 streaming event 转发到 IM 流式预览任务。turn 收尾走 `finalize_im_live_mirror`：drop SinkHandle → drain `RoundTextAccumulator` → 复用 dispatcher 的 `deliver_split` / `deliver_final_only` / `deliver_preview_merged`，按 IM account 的 `ImReplyMode`（`split / preview / final`）渲染——与 IM 入站 turn 完全对称。**两个通道独立走自己的发送通路**：GUI 永远走 Tauri IPC stream / HTTP `chat:stream_delta` 广播，不受 `imReplyMode` 影响；`imReplyMode` 仅决定 IM 端的呈现形态。错误 / 取消路径走 RAII drop，IM 端保留半截 preview，与入站 cancel 行为一致。`source ∈ {Subagent, ParentInjection, Channel, Cron}` 直接 no-op（IM 入站自己有完整流式管线，subagent/cron 不应外溢到 IM）。

- **新 slash 命令**：`/sessions`（picker 用户对话 session，过滤 cron / subagent / incognito）、`/session [<id>|exit]`（info / attach / detach）、`/projects`（picker）、`/handover <ch:acc:chat[:thread]>`（GUI 端推送，IM 不可见）、`/kb [on|off]`（WS8，群聊 per-chat 确认 KB 访问；DM 仅报状态——账号级 opt-in 在桌面 Settings；无 arg/`status` 报当前生效态）。`IM_DISABLED_COMMANDS` 仅含 `agent` / `handover`。
- **`channel:session_evicted` 事件**：`attach_session` / `update_session` 在 1:1 接管把旧 chat 物理 detach 之后，对每个被踢的 chat emit 一次此事件，payload `{ channelId, accountId, chatId, threadId, sessionId }`。[`channel/worker/eviction_watcher.rs`](crates/ha-core/src/channel/worker/eviction_watcher.rs) 订阅后调对应 plugin 的 `send_message` 发"this chat has been taken over by another endpoint"通知；`ChannelAccountConfig.notify_session_eviction`（默认 `true`）可静音。

### Dashboard / Recap / Learning

详见 [`dashboard.md`](docs/architecture/dashboard.md) / [`recap.md`](docs/architecture/recap.md)。

- `dashboard/insights.rs`：overview delta / cost trend / heatmap / health score / `query_insights` orchestrator
- Learning Tracker 落 `session.db.learning_events`，目前埋点：`skills::author` CRUD + `tool_recall_memory` 命中 + MCP tool 调用
- `/recap` 独立 `~/.hope-agent/recap/recap.db` 缓存按 `last_message_ts` 失效；`recap.analysisAgent` 与主对话 Agent 解耦

### 跨会话 / 全局

详见 [`session.md`](docs/architecture/session.md) / [`behavior-awareness.md`](docs/architecture/behavior-awareness.md) / [`ask-user.md`](docs/architecture/ask-user.md) / [`prompt-system.md`](docs/architecture/prompt-system.md)。

- **数据存储**：所有数据在 `~/.hope-agent/`，[`paths.rs`](crates/ha-core/src/paths.rs) 集中管理
- **统一日志**：前后端走 [`logging.rs`](crates/ha-core/src/logging.rs)（SQLite + 文本双写），API 请求体 `redact_sensitive` + 32KB 截断；agent 自主排查入口见 [`skills/ha-logs/SKILL.md`](skills/ha-logs/SKILL.md)（用 `exec` + `sqlite3 -readonly` 直查 `~/.hope-agent/{logs,sessions,background_jobs}.db`）
- **延迟工具加载**：opt-in `deferredTools.enabled`，只发核心 ~10 个 schema，其余通过 `tool_search` 发现；execution dispatch 不变
- **会话搜索**：FTS5 + `<mark>` 高亮 + XSS 防御（escape → 白名单反解）；`Cmd+F` 复用同一 `search_messages` + session_id 过滤
- **ask_user_question**：1–4 题结构化问答（单选/多选/输入）；pending 持久化 SQLite，App 重启 replay 断点续答；IM 按 `supports_buttons` 走按钮或文本
- **会话级工作目录**：`sessions.working_dir` 注入 system_prompt `# Working Directory` 段；**v1 只做 prompt 注入**，不改 `exec`/`read_file` 的 cwd 解析
- **桌面专属 markdown 路径链接**：仅 `is_desktop()` 注入 `MARKDOWN_PATH_LINKS_GUIDANCE`，要求 LLM 写 `[名](绝对路径)`；前端按 `isLocalPath()` + Transport 分流（Tauri 走 `open_directory`；HTTP/server 早返回禁用）。**例外**：anchor `title` 用 native HTML 不用 shadcn Tooltip（一条流式消息可能渲染上百个）

### 项目（Project）容器

详见 [`project.md`](docs/architecture/project.md)。

- **项目文件 = 工作目录里的真实文件**：上传文件直接落项目工作目录（无 `project_files` 表、无独立 `files/`/`extracted/`、无文本提取注入、无 `project_read_file` 工具）；模型靠 `# Working Directory` 段的顶层文件清单 + `read` 工具感知。**`project_files` / `ProjectFile` / `project_read_file` 已删，不要重新引入**
- 记忆优先级 Project > Agent > Global
- **工作目录合并（项目会话总有值）**：优先级 `session > project 显式 working_dir > 默认 workspace`；唯一入口 [`session/helpers.rs::effective_session_working_dir`](crates/ha-core/src/session/helpers.rs)（+ `effective_working_dir_for_meta`），**lazy ensure**——默认 workspace `~/.hope-agent/projects/{id}/workspace/` 在首次解析时 `ensure_dir_canonical` 创建并返回（不写进 DB，保持 `HA_DATA_DIR` 可迁移）。`project.working_dir` 留 NULL = 用默认 workspace。落盘解析走 [`project::resolve_project_dir`](crates/ha-core/src/project/files.rs)
- **文件浏览器作用域**：所有读写经 [`filesystem::WorkspaceScope`](crates/ha-core/src/filesystem/workspace.rs)（canonicalize + `starts_with` 失败闭合），`for_session` / `for_project` / `for_path` 三入口；**`for_path` 是只读 worktree 跳转**，后端把目标锚定到 base session/project 仓库的 worktree 列表（禁止跳到主机上任意 git repo），写操作经 `resolve_writable` 一律拒绝 path scope；ops 在 [`filesystem/ops.rs`](crates/ha-core/src/filesystem/ops.rs)。HTTP `/api/fs/*` 写端点受 `filesystem.allow_remote_writes`（默认 false）闸门，桌面 Tauri 不受限
- 删除级联（三步）：unassign sessions → 删 `projects` 行 → `rm -rf projects/{id}/`（含默认 workspace；用户显式选的外部目录不删）→ 删项目记忆（跨 db 单独执行）
- **IM 路由（无反向认领）**：项目不再认领 (channel, account)。要把 IM 中的会话归项目，从该 chat 内 `/project <id>`（或 picker）显式触发；`AssignProject` action 在 channel worker 内 UPDATE `sessions.project_id`，不再通过 channel→project 反查。**`Project.bound_channel` 已删除，不要重新引入**。

### Agent 解析链（默认 Agent）

7 级（首个非空胜出）：**显式参数 → `project.default_agent_id` → `topic.agent_id` → `group.agent_id` → `tg_channel.agent_id` → `channel_account.agent_id` → `AppConfig.default_agent_id` → 硬编码 `DEFAULT_AGENT_ID`（`"ha-main"`，定义在 [`agent_loader.rs`](crates/ha-core/src/agent_loader.rs)）**。统一入口 [`agent/resolver.rs::resolve_default_agent_id_full`](crates/ha-core/src/agent/resolver.rs)；无 IM 上下文的 desktop / HTTP 用 `resolve_default_agent_id` 包装（只传 project + channel_account）。**channel worker 不得自写解析链** —— Phase A5 已折叠到 resolver 单一真相源。

**遗留 `"default"` 自动重命名**：升级到使用 `"ha-main"` 的版本时，启动期 [`agent/migration.rs`](crates/ha-core/src/agent/migration.rs) 一次性把磁盘目录（`agents/default/` / `default-home/` / `plans/default/`）、`agents/*/agent.json` 里的 `subagents.allowedAgents` / `deniedAgents`、SQLite agent_id 列（sessions / team_members / teams / subagent_runs / projects / background_jobs / canvas_projects / logs）、`memories.scope_agent_id`（`scope_type='agent'` 的行）、`cron_jobs.payload_json` 内嵌的 agent_id 全部 rename 到 `"ha-main"`，再改写 `config.json`（`default_agent_id` / `recap.analysisAgent` / channel 各级 agent_id），落 sentinel `~/.hope-agent/.agent-id-renamed` 后续启动短路。每步独立 idempotent，崩溃可恢复；当 `agents/default/` 与 `agents/ha-main/` 同时存在（用户手动建过 ha-main）时迁移整体放弃，不写 sentinel、不动 DB / config，下次启动重试。**入口契约**：`init_runtime` 必须早于 `ensure_default_agent()`——后者会预创空 `agents/ha-main/` 模板，吞掉 rename。新增字面量 agent id 一律走 `crate::agent_loader::DEFAULT_AGENT_ID`（前端走 `@/types/tools` 的 `DEFAULT_AGENT_ID` / `isMainAgent`），不要重新引入 `"default"` 硬编码。

### 本地 LLM 助手

详见 [`local-model-loading.md`](docs/architecture/local-model-loading.md)。

- 后端锁 Ollama（OpenAI 兼容端点）；模型目录硬编码 [`local_llm/types.rs::model_catalog`](crates/ha-core/src/local_llm/types.rs)，按预算从大到小取首个 ≤ budget
- 预算：macOS 统一内存 50% / Win+Linux dGPU VRAM 50% 优先回落系统内存 50%，扣 1 GiB runtime
- App **不接管** Ollama 进程；安装走官方 `install.sh`（macOS+Linux），Windows 引导用户去官网
- 后台任务统一走 [`local_model_jobs.rs`](crates/ha-core/src/local_model_jobs.rs) + `~/.hope-agent/local_model_jobs.db`
- **Provider 写入 contract（强制）**：所有 Provider 列表与 `active_model` 写入必须走 [`provider/crud.rs`](crates/ha-core/src/provider/crud.rs) helper（`add_provider` / `update_provider` / `delete_provider` / `reorder_providers` / `set_active_model` / `add_and_activate_provider` / `add_many_providers` / `ensure_codex_provider_persisted`）；本地 LLM 安装走 `upsert_known_local_provider_model`。**禁止直接 `providers.push` / `retain` / 手写 `active_model`**
- **Known local backend catalog** 在 [`provider/local.rs`](crates/ha-core/src/provider/local.rs)（ollama / litellm / vllm / lm-studio / sglang）；前端"是否已配本地后端"必须消费 catalog，**禁止硬编码 regex**

### 自升级

详见 [`self-update.md`](docs/architecture/self-update.md)。

- **三档路径**：`Tauri` (desktop bundle 走 tauri-plugin-updater) / `PackageManager` (brew / scoop / aur / apt / dnf) / `SelfContained` (下载 bare binary → minisign 校验 → atomic swap → restart)；不可识别走 `ManualPrompt` 让用户在 `ask_user_question` 里选
- **Minisign pubkey 单一真相源**：[`ha-core/updater/keys.rs::MINISIGN_PUBKEY_BASE64`](crates/ha-core/src/updater/keys.rs) 与 `src-tauri/tauri.conf.json#plugins.updater.pubkey` 必须字符串相等。三重防线：启动期 `keys::assert_pubkey_matches_tauri_conf` panic / CI `lint.yml` 跑 `scripts/verify-updater-pubkey.mjs` / 本地 `.husky/pre-push` 同脚本拦截
- **`app_update` 工具**（`tools::app_update`，tier=`Core{Meta}`，`internal=false`，`async_capable=true`）：4 个 action `check | install | status | rollback`；`install` / `rollback` 在工具内部用 [`tools::ask_user_question::execute`](crates/ha-core/src/tools/ask_user_question.rs) 弹结构化 Yes/No 确认（不挪用 `AskReason::DangerousCommand`——语义不对，且需要承载升级 plan / 路径选择字段）
- **UpdaterBridge trait** ([`updater::UpdaterBridge`](crates/ha-core/src/updater/mod.rs)) 由 src-tauri 在 `setup.rs` 注册 (`crate::commands::update_bridge::register`)；ha-core 通过 `OnceLock` 反向调用，**严禁** ha-core 直接依赖 tauri-plugin-updater
- **Bare-binary release artifact**：`.github/workflows/release.yml` 每平台 build 后跑 `Bundle + sign bare binary` step，用同一 Minisign 私钥签 `tar.gz` (Unix) / `zip` (Windows)；`patch-manifest` job (`needs: build`) 合并 `bare_binary.platforms.<key>` 写回 `latest.json`
- **Binary swap 必须走 [`platform::atomic_replace_binary`](crates/ha-core/src/platform/mod.rs)**——Unix `rename(2)` 不影响在跑进程，Windows `MoveFileExW` 把 in-use binary rename-aside；**禁止 `fs::write` 直接覆盖运行中 binary**
- **自动更新统一配置 `AppConfig.auto_update`**（[`updater::AutoUpdateConfig`](crates/ha-core/src/updater/config.rs)，`ha-settings` 归 **HIGH**）：桌面（JS plugin-updater）与 headless（[`updater::auto_check`](crates/ha-core/src/updater/auto_check.rs) primary-gated loop，`!is_desktop()` 才起）共享同一份开关。后台只做**检查 + 静默预下载 + 校验**到 staging，**绝不自行 swap/restart**——安装仍走 `app_update install`（headless 用户审批）或桌面「更新并重启 / 仅更新」二选一，**桌面绝不无条件自动 relaunch**。命令 `get/set_auto_update_config`（Tauri + HTTP `/api/config/auto-update`）
- **下载健壮性红线**：自升级下载走 [`download::download_to`](crates/ha-core/src/updater/download.rs)（重试 + `Range` 断点续传）；`self_contained::install` swap 后**先 `--version` 冷烟自检、失败自动 `backup::most_recent()` 回滚**再重启；staging 经 [`staging::prune`](crates/ha-core/src/updater/staging.rs) GC，backup 树不被触及

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
- 新增 hook 事件：埋点（`dispatch` 或 `fire_*`）+ 同步 `types.rs` 三处 match（`common`/`matcher_target`/`is_observation_only`）+ 测试

## 设置（Settings）约定

所有用户可操作的配置必须同时具备 **GUI 入口** 和 **`ha-settings` 技能对应能力**，两者零偏差。新增/修改进入 `AppConfig` / `UserConfig` 且用户需要调整的字段时，**同一 PR 内三件事缺一不可**：

1. **GUI 控件**：[`src/components/settings/`](src/components/settings/) 对应面板，shadcn/ui + 三态保存按钮
2. **技能能力**：[`tools/settings.rs`](crates/ha-core/src/tools/settings.rs) 加读写分支 + 风险分级 + 副作用提示；同步更新 [`core_tools.rs`](crates/ha-core/src/tools/definitions/core_tools.rs) 的 `category` enum；含凭据需 read-only 的，加到 `BLOCKED_UPDATE_CATEGORIES` + `read_category` redact
3. **技能文档**：在 [`skills/ha-settings/SKILL.md`](skills/ha-settings/SKILL.md) 风险等级表登记

### 风险等级

- **LOW**：UI 偏好、显示配额（theme / language / notification / canvas 等）
- **MEDIUM**：行为调整，影响上下文 / 成本 / 输出质量（compact / memory_* / web_search / approval / multimodal / dreaming 等）
- **HIGH**：安全 / 网络暴露 / 全局键位 / 凭据 / 需要重启 / 权限规则 / 审批策略 / MCP 子系统级开关（proxy / embedding / shortcuts / server / skill_env / acp_control / `permission.global_yolo` / `smart_mode` / `mcp_global` / `protected_paths` / `dangerous_commands` / `unattended_approval` / `auto_update` 等）——技能在 `update_settings` 前**必须二次确认**

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
| 新增/删除子系统、架构文档、运行时 DB 或稳定 log category | [`skills/ha-self-diagnosis/references/`](skills/ha-self-diagnosis/references/)（`diagnostic-playbook.md` 子系统速查） |
| 新增/删除 Tauri 命令、HTTP 路由、`COMMAND_MAP` 条目 | [`api-reference.md`](docs/architecture/api-reference.md) 对应表格     |
| 功能变化导致 README 过时                            | `README.md` + `README.en.md`（同一 PR 双语同步）                      |
| 新增调研/对比分析                                   | `docs/research/` 新建调研文档                                         |
| 修改 README 任一语言版本                            | 同一 PR 同步另一语言（`README.md` ↔ `README.en.md`）                  |
| 新增/修改 Release Notes                             | 同一 PR 内中英双份（`docs/release-notes/vX.Y.Z.md` ↔ `vX.Y.Z.en.md`） |

- **AGENTS.md 是契约面**——只放跨 PR 必守的规则、红线、文件入口；**实现细节、内部数据结构、迁移逻辑、边角行为一律下沉到对应 architecture 文档**
- **架构文档强制**：子系统边界 / 数据流 / 持久化格式 / 跨模块 contract 改动须更新对应 `docs/architecture/`；新增架构级能力（新子系统 / 协议层）须同 PR 新建文档并登记到 `docs/README.md`
- **ha-self-diagnosis 索引同步**：新增 / 删除子系统、`docs/architecture/` 文档、`~/.hope-agent/` 运行时 DB（`paths.rs`）或稳定 log `category` 时，同步更新 [`skills/ha-self-diagnosis/references/`](skills/ha-self-diagnosis/references/) 的 `diagnostic-playbook.md`（Subsystem Reference：入口模块 / DB·config / log category / 故障 gotcha）。此处是 agent 自查与排障的 fallback 真相源，过时会直接误导诊断；新增的 log `category` 须为稳定字符串便于 grep
- **README 双语同步**：根目录 `README.md`（中文）+ `README.en.md`（英文），任一改动同次提交同步另一份
- **Release Notes 双语同步**：每版本 `vX.Y.Z.md` + `vX.Y.Z.en.md`，顶部互加 `简体中文 · English` 切换链接
- **CHANGELOG entry 单行**：每条 changelog 一句话讲用户感知 + `(#PR)` 引用，**不放**文件路径 / 数据结构 / 单测数 / 实现取舍——那些写 PR description 或 [`docs/architecture/`](docs/architecture/)。涉及契约 / 红线变更可加一行用户操作影响（如「首次启动自动迁移」），仍不展开实现。Release notes 可以稍长一段，但同样面向用户视角而不是实现叙事

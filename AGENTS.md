# Hope Agent

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，内置丰富 Provider 模板与预设模型，GUI 傻瓜式配置。三种运行模式：桌面 GUI（Tauri）、HTTP/WS 守护进程（`hope-agent server`）、ACP stdio（`hope-agent acp`）。

子系统设计与实现细节见 [`docs/architecture/`](docs/architecture/)；本文只列**影响每个 PR 的契约和红线**，不重复实现细节。

## 开发命令

```bash
pnpm tauri dev         # 启动开发模式（前端 + Tauri 热重载；beforeDevCommand 会先 dev:browser-host 备好 host）
pnpm dev               # 仅前端 Vite 开发服务器
pnpm dev:browser-host  # 仅重建 debug 版 ha-browser-host 并落到 dev 路径（改了 ha-browser-host 后手动刷新用）
pnpm tauri build       # 构建生产包
pnpm sync:version      # 以 package.json 为单一来源，同步各 Cargo.toml / tauri.conf.json / Cargo.lock 版本
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
Cargo.toml              Workspace 根（members: crates/ha-core, crates/ha-server, crates/ha-browser-host, src-tauri）
crates/
  ha-core/              核心业务逻辑（零 Tauri 依赖，纯 Rust 库）
  ha-server/            HTTP/WS 服务器（axum，REST API + WebSocket 流式推送）
  ha-browser-host/      浏览器后端辅助进程（native messaging broker / CDP host）
src-tauri/              Tauri 桌面 Shell（薄壳，调用 ha-core）
src/                    前端（React + TypeScript）
  components/           chat/ settings/ dashboard/ cron/ common/ ui/ 等
  lib/                  Transport 抽象层：transport.ts + transport-tauri.ts + transport-http.ts
  i18n/locales/         12 种语言翻译文件
skills/                 内置技能（meta / 编程方法论 vendor / 办公方法论原创）
docs/architecture/      子系统设计文档（跨 PR 必读单一真相源）
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
- **EventBus + 共享状态**：核心层用 `ha-core::EventBus` 替代 Tauri `APP_HANDLE`；Tauri 用 `State<AppState>`，server 用 `State<Arc<AppContext>>`
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

详见 [`memory.md`](docs/architecture/memory.md)；下一代 Dreaming（结构化 claim 层 / Deep resolver / Memory Profile / Context Pack 注入 / Lucid Review / 确定性评测）的完整架构见 [`dreaming.md`](docs/architecture/dreaming.md)。

- **优先级**：Project > Agent > Global，唯一入口 `effective_memory_budget(agent, global)`；预算只约束 system prompt 注入，`recall_memory` / `memory_get` 工具返回完整原文
- Active Memory / Awareness / User Profile 各作**独立 cache block** 注入，不作废静态前缀缓存
- **Dreaming Context Pack 注入**：高 salience 的 active claim 作 `## Pinned Memory` 段并入 system_prompt **静态 prefix**（复用首个 cache breakpoint，不新开动态 cache block）；claim 内容进 prefix 前必过 `sanitize_for_prompt`。Profile / Pinned 纳入 `effective_memory_budget` 同一预算池（Core > Pinned > Profile+legacy）
- **dedup 红线**：被 active managed claim 覆盖的 legacy memory 经单一来源 `covered_by_active_claim_memory_ids` 从 `# Memory` 段排除——**去重阈值必须对齐 Pinned 注入阈值 `PINNED_MIN_SALIENCE`（`context_pack.rs` 单一来源常量），dedup 永不比注入更激进**，否则中等 salience（`[0.5,0.7)`）claim 的影子记忆既被踢出 legacy、又够不到 Pinned = 无 prompt 出口；`user_pinned` link / `memories.pinned=1` 豁免
- **Active Memory v2**（`ActiveMemoryConfig.include_claims`，per-agent 走 agent.json 不进 `ha-settings`，默认关）把当前 turn 召回候选扩到 effective-active claim（过期 / superseded 不回灌）；incognito 全链归零
- **Lucid Review 用户纠错闭环**：唯一编排入口 `claims::review`——`update_claim`（PATCH 语义：edit / status / move scope / pin-unpin）+ `forget_claim`（archive / permanent）；decision-type 由 `resolve_update` 纯函数从 diff 派生。**红线**：① 每个用户操作落 `dreaming::record_user_action`（完成态 run + 单 decision）；② approve/edit/reject/expire/move_scope 写最高权重 `manual_correction` evidence，pin/unpin/flag 不写；③ content 变更后 `reembed_claim` 使下一轮召回反映新文本；④ forget archive 翻 `archived` + link 转 `managed` + 仅独管的 memory `pinned=0`，permanent 删 claim 图谱 + 仅独管的 orphan memory；⑤ 发 `memory:claim_changed`。owner 平面 `claim_update` / `claim_forget`（对齐 HTTP `PATCH /api/claims/{id}` / `POST /api/claims/{id}/forget`，本机 / API key 信任）**无 agent 工具面**——claim 纠错只对用户开放，模型不能自改
- **确定性评测**：`memory/dreaming/eval.rs` golden-fixture + `tests/fixtures/dreaming/*.json` + `tests/dreaming_eval.rs`（**无 LLM**，只测安全红线：作用域隔离 / 过期抑制 / 证据可追溯 / 冲突进待审 / legacy-sync 隐藏 / 证据 fail-closed）。**契约**：改动 claim 读路径 / effective-status / hidden-set / scope 过滤 / evidence 授权时，须加 fixture case 或保既有 fixture 绿
- **会话级无痕（`sessions.incognito`）**：单一真相源；不注入 Memory / Active Memory / Awareness、跳过自动提取；**关闭即焚**（不进侧边栏 / 全局 FTS / Dashboard 统计）；**与 Project / IM Channel 互斥**。四旁路守卫红线（Epic E，全貌见 [`session.md`](docs/architecture/session.md#四旁路守卫epic-e)）：`is_session_incognito` fail-closed 三态；大工具结果落盘走内存、异步任务 **占位不 spool**；AllowAlways 经 `choose_scope` 强制内存 `Session` scope 绝不落盘；`session:purged` 焚盘 `tool_results/` + job 行/spool；异步注入与在途回合在会话已删/已焚时跳过

### Knowledge Base（知识空间）

详见 [`knowledge-base.md`](docs/architecture/knowledge-base.md)（实现 + 设计契约 D1–D14 决策账本 + 子系统全貌）。对外名「知识空间」（slogan：你的第二大脑），代码中性（模块 `knowledge/`、工具 `note_*`、作用域 `for_knowledge`）。**本节只列跨 PR 红线，实现细节 / 数据流 / 内部结构一律在架构文档**。

- **两类存储分明（D9）**：注册表 + 访问绑定落 `sessions.db`（真相源）；`note/chunk/link/tag` + FTS5 + vec0 落 `~/.hope-agent/knowledge/index.db`（纯可重建缓存，删了从 `.md` 全量重建）。**笔记 = 真实 `.md` 文件，唯一真相源**
- **访问默认 deny（D10 + WS8）**：唯一入口 `effective_kb_access`——incognito 归零 → IM 默认归零（除非账号级 `kbAccessOptIn` + 群聊 per-chat 确认 `kbAccessChats`，群内 `/kb`；查不到 / 不匹配 fail closed）→ `max(session_attach, project_attach)` → 滤 archived → 外部只读 root cap `read`。source / origin / channel 经 agent 真接线透传，**subagent 按 origin 血缘判定不洗权限**
- **两鉴权平面物理隔离（D10）**：owner 平面（HTTP / Tauri，`service.rs`）= 本机 / API key 信任，看全部 KB **不经 attach**；agent 平面（`note_*` 工具）走 `effective_kb_access`。`/api/knowledge/{kb}/files/*` 纯 owner、无 session 参数 / 无 fallback
- **作用域闭合 + 外部只读（D11 + WS7）**：所有读写经 `WorkspaceScope::for_knowledge`；**外部绑定 root 默认只读（桌面也拒），KB 级 `allow_external_writes` opt-in 才解锁外部写**；HTTP 写再叠 `allow_remote_writes`。**后台自主维护永不写外部**（无视 opt-in）
- **写盘原子 + stale-write guard**：所有笔记写经 `crate::platform::write_atomic`（temp+fsync+rename，**禁回退 `fs::write`**）；`expected_file_hash` 比**磁盘当前 raw BLAKE3**（不比索引 `content_hash`）；`note_patch` 走 `old/new` 唯一文本命中（0 / 多次都拒）。坐标系 D14 / resolve 确定性 #8 / chunk D12 / 图谱布局 J 细节见架构文档「写盘 / 坐标 / 解析 / 布局契约」节
- **检索独立旗舰（D7）**：chunk FTS5 + vec0 → RRF → MMR → 回 note，**独立 store 绝不折进 `recall_memory`**；自有 `knowledge_embedding` selector（与 `memory_embedding` 物理隔离、不寄生不回退）；embedding / chunk 重 reindex 故 **GUI-only 不进 `ha-settings`**，纯查询期排序参数 `knowledge_search`（`search.rs::KnowledgeSearchConfig`，`clamped()` 钳值）走正常 MEDIUM
- **会话感知访问单一入口**：`Agent::resolve_kb_access()`（`agent/mod.rs`）是 agent 侧唯一解析链（按回合 memoize），prompt 段 / 被动召回 / 工具门控三处共用，不得各自重写。**只服务 schema/prompt/召回，绝不 gate 执行**（执行走 live `access_map`）
- **无 KB 不注入笔记工具**：`is_kb_scoped_tool`（`note_*` + `session_to_note`，**不含 `knowledge_recall`**）schema 构建后过滤——纯 UX / 省 token，**不是安全边界**（执行层兜底）；「# Knowledge Bases」prompt 段同理为空即省略
- **读取即 untrusted**：`[[note]]` 注入 + 被动召回（`knowledge_passive_recall`，默认关）一律套 `<untrusted_external_data>` 信封、受 `effective_kb_access` 约束、**永不提升为 system 指令**；incognito 零召回、IM 未 opt-in 零访问
- **`knowledge_recall` 工具（D7）**：一次查 memory + 笔记两 store，**两段独立排序不混排**，薄编排器**绝不折进 / 改动 `recall_memory`**
- **外部 vault 实时同步（D6）**：`notify` watcher（per-KB 线程 + debounce）+ bind / 启动 reconcile（mtime 增量 + prune）
- **侧边栏 AI 对话面板**：`kind='knowledge'` 会话，**从主会话列表 / `/sessions` / 全局 FTS 隐藏**；锚定落 `knowledge_chat_threads`（D9，级联删）。复用主对话 `useChatStream`（新增可选 prop，主对话不传 = 行为不变）
- **工具精简 `ToolScope::Knowledge`（与 D10 正交）**：仅 schema/prompt 可见性收窄到白名单，**绝不动 KB 访问**——访问仍由 `effective_kb_access` 单点裁决
- **精灵 / 灵感模式（crate 级 `sprite/`，默认关）**：前端多触发源 → owner 命令 `kb_sprite_observe_cmd` → 后端节流 + side_query → emit `sprite:casting` / `sprite:suggestion` → 前端瞬态展示。**incognito 零精灵**（前后端双关卡）。非 agent 工具，设置三件套（MEDIUM）
- **自主维护（Layer 2，默认全关）**：后台扫内部 KB 产提案进 `sessions.db`（D9，级联删）；落地经 owner 平面（用户已批准故**绕 D10**，但写前重读磁盘 hash 守 stale-write + 跳外部只读）；`ha-settings` 归 **HIGH**
- **块级引用（仅 Obsidian `^block-id`，Logseq 不做）/ 原生大纲只读视图**：见架构文档；大纲红线**只读、永不替代 CM6 底座**
- **新增 KB 工具 / 端点**：工具走 `tools/note.rs` + `core_tools.rs` + `execution.rs`；Tauri / HTTP owner 薄壳调 `knowledge::service`；逻辑全在 ha-core（红线）

### 设计空间（Design Space）

详见 [`design-space.md`](docs/architecture/design-space.md)（架构）。对外名「设计空间」（侧边栏「知识空间」正下方），代码标识 `design`（模块 `design/`、agent 工具 `design`、库 `design.db`、视图 `DesignView`）。**本节只列跨 PR 红线，实现细节一律在架构文档**。

- **编译只在后端 / 浏览器零编译（红线，精修自「轻量自包含 HTML」）**：产物 iframe 只加载**已编译落盘的静态产物**，**浏览器端零编译 / 零打包 / 零 JIT**（不引 in-browser Babel / esbuild-wasm / Tailwind JIT——这是旧版 `feat/atelier` 白屏卡顿被推倒重做的根因）。9 个静态 kind（web/mobile/deck/dashboard/poster/document/email/image/motion）+ audio 是纯自包含 HTML；**`component`（交互式 React）kind 走 `design::compile`（纯 Rust `oxc`，进程内编译 JSX/TSX→JS）在 ha-core 后端编译一次**，内联 vendored React 18 UMD（`design/assets/`，锁版本、零网络）+ 编译产物落成静态 `index.html`。**编译失败必降级为静态错误页 / 占位，绝不白屏 / 绝不后端 panic**。Component 编译产物 ≠ 源码故**不支持 oid 可视化微调**（微调仍只归 9 静态 kind）
- **产物墙非画布（红线）**：主编辑面是产物库缩略图墙 + 单产物稳定 iframe 预览（纯 CSS 缩放），**刻意不做无限画布**（旧版卡顿之源）
- **可视化微调确定性回写**：纯 HTML → 渲染期注入 `data-ds-oid` + 产出 oidmap（映射源码字节范围），`patch::apply_{style,text}_patch` 走单一命中 + `expected_hash`（磁盘 body BLAKE3）stale-write 守卫；预览态注入 dormant inspector bridge（收 `ds_activate` 才启用），**导出态 `editable=false` 不注入 bridge/oid**（干净产物）
- **文件即真相源**：产物 `index.html`/`source/`/`versions/`、设计系统 `SYSTEM.md`/`tokens.json` 都是磁盘真实文件；`design.db` 是可重建元数据注册表
- **内置设计系统两类**：6 套**原创原型语言**（`system.rs::builtins`）+ 一批**品牌风格参考**（`brands.rs` 种子 → `system::expand` 展开为完整 token 契约，对各品牌公开视觉语言的独立再诠释；`build_system_md` 对 `brand_ref=Some(..)` 渲染时**必附免责声明**、非官方）；token 编译注入产物 `:root`，产物 CSS 引用 `var(--ds-*)`
- **两鉴权平面**：owner 平面（Tauri/HTTP `service.rs`）本机/API key 信任、不经 access 检查；agent 平面（`design` 工具）；写盘走 `platform::write_atomic`，静态托管三闸（id 白名单 + `validate_safe_rest_path` + `contained_canonical`），iframe `sandbox="allow-scripts"`
- **设置三件套**：`AppConfig.design`（MEDIUM，含 `auto_critique` 成本项）+ `DesignSettingsPanel`（Tools 设置页 tab）+ `ha-settings` `design` category + SKILL.md 登记
- **新增 design 工具 action / 端点**：工具全在 `tools/design/mod.rs`（复用 `design::service`）；owner 薄壳 Tauri `commands/design.rs` + HTTP `routes/design.rs`，逻辑全在 ha-core（红线）

### 工具 & 审批

详见 [`permission-system.md`](docs/architecture/permission-system.md) / [`tool-system.md`](docs/architecture/tool-system.md) / [`sandbox.md`](docs/architecture/sandbox.md) / [`browser.md`](docs/architecture/browser.md)（浏览器 8-action 表面 + 双 backend）。

- **统一权限引擎 v2**：所有调用走 `permission::engine::resolve_async()`，优先级 **Plan > Internal > YOLO > Protected/Dangerous > AllowAlways > Sandbox soft allow > Session 模式 preset > 兜底 Allow**
- **Session 模式三选一**：`default | smart | yolo`，`PermissionModeSwitcher` / `/permission` 切换；`AgentConfig.capabilities.default_session_permission_mode` 决定新会话初始 mode
- **Smart 模式忽略 `custom_approval_tools`**——UI 必须显式提示
- **保护路径 / 危险命令 / 编辑命令**：三独立列表，存 `~/.hope-agent/permission/*.json`；非 YOLO 模式强制弹窗（AllowAlways 按钮置灰），YOLO 只 `app_warn!` 不弹
- **Global YOLO**：CLI flag `--dangerously-skip-all-approvals` 与 `permission.global_yolo` OR 组合；判定入口 `security::dangerous::is_dangerous_skip_active()`，**与 Plan Mode 正交**
- **审批超时**：`approval_timeout_secs`（默认 300s，`0` 不限）+ `approval_timeout_action ∈ deny|proceed`。**strict 原因超时永不放行**（红线）：`proceed` 只对非 strict 生效；strict 原因（`forbids_allow_always`：保护路径 / 危险命令 / 高危 macOS 控制 / raw CDP / Plan-ask）超时强制 deny + `app_warn('permission','strict_timeout_deny')`。strict 判定单一真相源 `AskReason::forbids_allow_always`，`ApprovalReasonKind::is_strict()` 镜像（穷举单测断言一致）。超时另发统一 `approval:resolved`（`source=timeout_deny|timeout_proceed`）撤窗
- **审批授权来源审计**：每个后台 job 的 `background_jobs.approval_origin` 列记录授权方式（`ApprovalOrigin`：user / timeout_proceed / unattended_proceed / yolo / auto_approve / external_pre_approved / policy_allow），由审批闸单点算出写入 spawn ctx（exec + 非 exec 全覆盖）
- **无人值守审批 fail-closed**（[`permission::approval_surface`](crates/ha-core/src/permission/approval_surface.rs)）：审批阻塞前经单一入口 `check_and_request_approval` 预检 `evaluate_approval_surface(session_id)`——确证无人能批（cron `is_cron` / 无客户端 headless + 无 IM attach / ACP 无 capability / subagent 无父 surface，**含 cron 起的 subagent → `Unattended(Cron)`，C03：子链 root 为 cron 时在 desktop 短路前判**）时按 `permission.unattended_approval_action ∈ deny(默认)|proceed` 处理：deny → `ToolRejection::denied_unattended` 即时拒绝(不再永久挂死)、proceed → 自动放行；emit `approval:unattended` 事件。**保守红线**：任何可能 surface(desktop 窗口 / web 客户端 / IM)即 Attended,唯 cron 例外。判 ACP 必须用 `is_acp()` 非 `ChatSource`(ACP 复用 Http)
- **浏览器扩展后端 + raw CDP 红线**（详见 [`browser.md`](docs/architecture/browser.md)）：可选 Chrome 扩展后端（MV3 + native messaging broker，`ExtensionBackend` 与隔离 `CdpBackend` 并列实现 `BrowserBackend`），由 `browser.backendPreference`（`extension_first` 默认 / `cdp_only` / `extension_only`）选择，驱动用户已登录的真实 Chrome（含全部 cookie / 会话）。**`control.raw_cdp` 是 strict**：`AskReason::BrowserRawCdp` ∈ `forbids_allow_always`（`ApprovalReasonKind::is_strict()` 镜像），引擎在 `allows_tool_call` **之前**拦截——AllowAlways 规则与 smart 自信都绕不过，**每次调用都审批、永无 Allow Always**。叠加方法黑名单（`BLOCKED_RAW_CDP_METHODS`：`Network.*Cookie*` / `Page.getCookies` 等，因 `Network.` 不在域前缀黑名单）+ 域前缀黑名单 + `Runtime.evaluate` / `callFunctionOn` 走 `security::ssrf::check_url` 扫描。**硬开关 `browser.extension.allow_raw_cdp`**（默认 `true`，置 `false` 在 `control_raw_cdp` 直接拒绝，agent 完全发不出 raw CDP）。`browser` settings category 归 **HIGH**（GUI BrowserPanel「高级」段 + `ha-settings` 可读写、无凭据子字段）
- **Agent 工具开关**：`AgentConfig.capabilities.tools.allow/deny` 仅表示非 Core 工具的显式开 / 关覆盖；Core 工具不受影响。system_prompt / schemas / tool_search / 执行层统一走 `dispatch::resolve_tool_fate`
- **工具结果磁盘持久化**：> `toolResultDiskThreshold`（默认 50KB）写盘，上下文留 head+tail 预览
- **后台任务统一模型**（详见 [`background-jobs.md`](docs/architecture/background-jobs.md)，本组只列红线）：表/文件 `background_jobs`（`JobKind = Tool | Subagent | Group`）+ **单一入口 `async_jobs::JobManager`**——新增后台单元一律加 `JobManager` 方法、**禁起平行 API**。命名分裂（历史契约勿改）：模块名 / log category 是 `async_jobs`，DB 是 `background_jobs`（诊断按 `category='async_jobs'` grep）；EventBus 走 `job:*`（kind-tagged + `session_id`，`subagent` kind 沿用更丰富的 `subagent:*` 不双发）；统一取消 `RuntimeTaskKind::AsyncJob`
- **异步 Tool 执行**：`exec` / `browser` / `web_search` / `image_generate` 标 `async_capable=true`；`job_status` 多作业面（`action ∈ status|list|wait|cancel|result`，`tool_job_status(args, session_id)`，`wait` clamp ≤10s 回 `still_running`）。**长 fan-out 等齐的正道是注入而非 `wait`**（Group 等齐后合并注入一轮）
- **两层硬配额（R7.1，`async_jobs::slots`）**：全局 `async_tools.max_concurrent_jobs`（默认 `clamp(cores-2,4,16)`，`0`=不限）+ 每会话 `max_concurrent_jobs_per_session`（默认 `(global×3/4).max(2)`、band [3,12]、恒 < 全局、`0`=不限）；`reserve_inner` 两层都要有空位才发 `SlotReservation`，否则**入队**（`Queued`，非拒绝），`pick_fair_index` 跳过已达每会话上限的会话；auto-bg detach 经 `reserve_forced` 强占槽可短暂超 cap。**调度器 / 队列 per-process**（队列持不可持久化 live ctx，与 `replay_pending_jobs` 的 Primary-only 相反）。**per-kind 双域分治勿合并**：tool 池在此，后台 subagent 池在 `subagent::queue`（R7.2，详见 [`subagent.md`](docs/architecture/subagent.md)），结构类（depth/batch/turn）拒绝不排队
- **后台 job 重试（R7.4，`async_jobs::retry`）**：默认关（opt-in）；纯策略 `decide()` 仅对 `JobError::Failed` + **代码级白名单 `is_retry_eligible` = `web_search`/`web_fetch`**（非用户旋钮）重投，`exec`/`image_generate` 及 `Cancelled`/`DeniedByUser`/`TimedOut` 永不重投；`max_job_secs` 是 **per-attempt** 预算、`max_attempts` 钳 10。**新增 async_capable 工具若有副作用或计费，务必不进 `is_retry_eligible`**
- **后台 exec 审批 park（R8，`async_jobs::approval_bridge`）**：显式后台 exec 命令 gate 落到 job 线程，经 thread-local 桥翻 `Running⇄AwaitingApproval`（桥定义在 `tools::approval`、**tools 零依赖 async_jobs**）；批准续跑、拒绝/超时-deny 终态 `DeniedByUser`、unattended/strict 仍 fail-closed deny **不 park**。**parked 持槽不释放**（防 resume 无空槽死锁）、**job 预算 timer 排除 parked 时长**（`parked_budget_extension`，守 ASYNC-2）、重启 parked→`Interrupted`。后台 subagent 内层审批改由 `approval_projection_watcher`（EventBus）补投影 label——**纯 label、绝不 gate 执行**（gotcha：`approval_required.session_id` snake vs `approval:resolved.sessionId` camel，两者都认）
- **运行中输出尾巴 `output_tail`（R3①，`async_jobs::output_tail`）**：后台 exec 的 stdout/stderr tee 进进程本地有界 ring，`job_status(status)` 返回。**加法式**：仅 `ctx.output_tail_job_id` 为 Some（exec + 非 incognito）才走，cap 起跑快照（读时钳 `[256,1MB]`，默认 8KB）；**incognito 永不注册**
- **完成注入合并窗口（R4）**：`async_tools.completionMergeWindowSecs`（默认 3，`0`=关）缓冲同会话多 tool job 合并一条 `<task-notification-batch>` 一轮注入。**恰好一次**：ghost-turn 闸 + 逐 job claim/release + `on_injected` 逐行恰好一次；崩溃靠 `replay_pending_jobs`（Primary-only）各自补投不合并；Group 预合并绕过。owner 面板读 `list_session_snapshots`→`BackgroundJobSnapshot`（Tauri `list_background_jobs` / HTTP `GET /api/sessions/{id}/background-jobs`、`/api/background-jobs/{id}`），取消复用 `cancel_runtime_task(kind=AsyncJob)`、不新增端点
- **配置 `AsyncToolsConfig`（`async_tools`，MEDIUM，GUI `save_async_tools_config`）**：默认值单一来源 `impl Default`，读时钳安全 band。**`0` 语义红线**：`max_concurrent_jobs` / `_per_session` 的 `0` = 真不限；其余 bounded-resource 旁钮（`output_tail_bytes` `[256,1MB]`、`max_queued_jobs` `[1,4096]`、`wakeup_max_delay_secs` `[10s,7d]`、`wakeup_max_pending_per_session` `[1,100]`）的 `0` 一律钳到地板、**绝非无限**
- **SSRF 统一策略**：出站 HTTP 必须走 `security::ssrf::check_url`；**新出站入口严禁自写 IP 校验**
- **文件 Diff 元数据**：`write` / `edit` / `apply_patch` / `read` 通过 `ToolExecContext.metadata_sink` 旁路传出 JSON；持久化到 `messages.tool_metadata` 列；前端右侧 `DiffPanel` 渲染（与 PlanPanel / CanvasPanel 视觉互斥）
- **工作台面板（Workspace）**：右侧互斥面板（`src/components/chat/workspace/`），聚合任务进度 / 文件（读+改）/ URL 来源。**`useWorkspaceArtifacts` 混合数据**：后端读时聚合（[`session::aggregate_session_artifacts`](crates/ha-core/src/session/artifacts.rs)，只回摘要、每段最近 1000 封顶）+ 当前轮 live tail（带结构化 diff）。**红线**：聚合 dedup/排序规则 TS（live）+ Rust（后端）两份必须同步（注释互指）；**无痕会话跳后端、只用 live tail**（守关闭即焚）
- **文件操作统一**（详见 [`file-operations.md`](docs/architecture/file-operations.md)）：Markdown 链接 / 下挂文件 / 工作台产物三处**禁止各写 open/download**，统一走 `src/components/chat/files/` 的纯策略 [`fileActions.ts`](src/lib/fileActions.ts)（按 `fileKind` × `supportsLocalFileOps()` 决议）。矩阵：可预览类型→右侧 `FilePreviewPane` 预览、其余本机=打开 / 远端=下载（新增可预览类型改 [`fileKind.ts`](src/lib/fileKind.ts) 的 `isPreviewableKind`）。office 走前端富渲染、失败回退后端 `extractDoc`（**LLM 注入路径 `file_extract` 不受影响**）；文件图标统一走 [`FileTypeIcon`](src/components/icons/FileTypeIcon.tsx)
- **preview-by-path 鉴权红线**：按绝对路径读取/提取/取流（Tauri `preview_read_text` / `preview_extract`；HTTP `GET /api/sessions/{id}/files/{read,extract,by-path}`）。HTTP 三端点共用 [`authorized_canonical_file_path`](crates/ha-server/src/routes/sessions.rs)（**被会话 tool 消息引用 ∪ 落在会话工作目录内**），二者皆非的主机任意路径一律 403——**远端严禁放行任意主机路径**（= 远程任意文件读）；桌面信任本机

### Hooks

详见 [`hooks.md`](docs/architecture/hooks.md)（hooks 子系统单一真相源：28 事件矩阵 / 数据流 / 5 handler / 四层 scope / 安全 / 测试 / Roadmap）。

- **字段级对齐 Claude Code hooks 协议**；核心全在 `ha-core::hooks`（**零 Tauri 依赖**），desktop / server / ACP 共用
- **唯一入口 `HookDispatcher::dispatch(event, input)`** + `hooks::fire_*` 助手（内部封装 scope 解析 / matcher / 并发隔离 / 去重 / 超时 / 聚合）；**严禁在业务代码里 match 具体 handler 类型**
- **28 事件**：24 真触发（阻断型 `UserPromptSubmit`/`PreToolUse`/`PreCompact` + 21 观察型）+ 4 协议保留（无对应概念、不 dispatch）。`is_observation_only` 事件的 `block` 决策降级为非阻断 + log
- **5 种 handler 全实现**：`command` / `http`（SSRF-gated）/ `mcp_tool` / `prompt`（side-query）/ `agent`（spawn 子 Agent）
- **四层 scope UNION**（无覆盖）：user + managed 编进全局 registry；project + local 按会话工作目录经 [`scopes::resolve_for_cwd`](crates/ha-core/src/hooks/scopes.rs) 合并。**project/local 默认关**（`hooks_allow_project_scope` opt-in，默认 `false`——供应链防护，`ha-settings` 只读）；`disable_all_hooks` 关所有 scope
- **配置走 config contract**：读 `cached_config().hooks`，user scope 写 `mutate_config(("hooks", source), …)`。**`ha-settings` 技能只读 hooks**（写被 `BLOCKED_UPDATE_CATEGORIES` 拦截——可写 = 模型给自己装命令执行）
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
- 命名空间 `mcp__<server>__<tool>`；工具默认 eager 注入，单个 server opt-in `deferred_tools=true`（默认 `false`）才改走 `tool_search` 发现
- OAuth 2.1 + PKCE 自实现（不用 `rmcp::auth_client`）；凭据 0600 落 `~/.hope-agent/credentials/mcp/{id}.json`
- **配置读写 contract**：读 `cached_config().mcp_servers`；写 `mutate_config(("mcp.<op>", source), ...)`，`op ∈ add|update|remove|reorder|global|import`
- handshake 401/403 → `ServerState::NeedsAuth`（避免 watchdog 死循环）

### Subagent / Team / Cron

详见 [`subagent.md`](docs/architecture/subagent.md) / [`agent-team.md`](docs/architecture/agent-team.md) / [`cron.md`](docs/architecture/cron.md)。

- `subagent(action="spawn_and_wait")` 前台等待 `foreground_timeout`（默认 30s），超时自动转后台
- **后台 subagent 投影进 Background Job（R6，单向）**：用户委派的后台 subagent run（排除 plan/team/hook 内部 spawn 与 incognito）在 `spawn_subagent` 建一条 `kind=Subagent` 投影，共享 `job_status` / 面板 / 取消。**`subagent_runs` 是执行真相源，投影绝不持有正文、绝不反写**；同步走单一 choke point `SessionDB::update_subagent_status`；取消路由到 `subagent::request_cancel_run`（**不跑工具 job 的 hook/注入**）。详见 [`background-jobs.md`](docs/architecture/background-jobs.md)
- **`batch_spawn` = Group fan-out（R5，合并注入一轮）**：`kind=Group` 协调行关联 N 个子投影、抑制个体注入，全部子终态时单赢 CAS 发**一条**合并 `<task-notification>`（join-all-settle）。红线：**建 group 前预校验全部 task**（任一非法整体拒，否则漏交付）；**取消先标 group 终态再取消子 run**；group 行**绝不持有 run 正文**（join 真相读 `subagent_runs`）。详见 [`background-jobs.md`](docs/architecture/background-jobs.md)
- Agent Team 模板 GUI 预配 + 模型按需发现；`TeamTemplateMember.description` 注入子 session 身份段
- Cron `delivery_targets`：final assistant text fan-out 到 IM；IM 会话内未显式传时自动取当前会话，显式 `[]` 关闭
- **Cron 时区（红线）**：`CronSchedule::Cron.timezone` 是 IANA 名、**真正生效**——`compute_next_cron` / 日历 `compute_occurrences` 同口径经 `schedule::parse_timezone`（`chrono_tz::Tz`）按该时区墙钟解释、DST-aware，再转 UTC 落库（`cron` 0.13 `after<Z:TimeZone>` 泛型）。**DST 秋退红线（C01）**：`compute_next_cron` 取 `.find(|dt| *dt > *after)` 而非裸 `.next()`——fall-back ambiguous 墙钟（如 01:30 出现两次）下 `cron` 下一个本地 occurrence 换算回 UTC 可能早于 `after`，裸 `.next()` 把过去时刻写进 `next_run_at` 叠加 `get_due_jobs(next_run_at<=now)` → 约 30 分钟窗口内每 tick 重复触发（已实测复现）；跳过非严格未来 occurrence 与 At/Every 的 `> after` 契约一致。校验单一真相源 `parse_timezone` / `validate_timezone`（`cron::validate_timezone` re-export），`parse_schedule` 创建/更新期非法名 `bail!`，**禁止静默回退 UTC**（静默回退正是旧 bug 隐形之因）。`At`/`Every` 无时区（时间戳自带 offset）。`CronDB::open` backfill 把 null 时区 Cron 行回填宿主时区（`iana-time-zone`）+ 重算 `next_run_at`，幂等、宿主不可检测则 no-op；**`cron_meta` sentinel `tz_backfill_done` 门控真·一次性（跑过即短路、不再每 boot 全表扫）——红线：`None` 时区双重语义（迁移前 legacy vs `parse_schedule`「Omit for UTC」故意 UTC），无 sentinel 则每次启动回填会把升级后新建的故意-UTC 任务静默改成宿主时区**；宿主不可检测时不写 sentinel（下次重试）。fire 时**非空但解析失败**的时区名回退 UTC 前 `app_warn`（不再静默；空/缺省仍是静默 UTC 默认）。前端 `CronJobForm` 仅 cron 类型显示 IANA 选择器、默认浏览器时区
- **Cron 对话集中展示 + 未读聚合**：cron 运行会话（`is_cron=1`）**不再进主侧边栏会话列表 / 搜索**——`list_sessions_paged_for_sidebar` 加 `exclude_cron` 谓词（`list_sessions_paged` 通用查询不受影响）、前端 `SessionList` 去 cron Tab + 搜索结果滤 cron。改在 cron 面板（`CronCalendarView` 第三模式「历史」，与任务详情页的「运行历史」同名）的 master-detail 里整体按时间倒序展示：左栏跨 job 时间线 `cron_run_timeline`（**cron.db 与 sessions.db 两库不可 JOIN，在 ha-core `cron::timeline` 装配**：`CronDB::list_run_timeline` 取 run 行 + `SessionDB::cron_session_read_state` 批量补 title/unread，session 被 purge 回退 job_name/0），右栏复用主聊天 `MessageList` + `parseSessionMessages` 只读渲染（`CronSessionViewer`，无 ChatInput）。侧边栏 cron icon 未读角标走 `cron_unread_total`（`is_cron=1` 的未读 assistant 聚合）+ 前端 `useCronUnreadStore`（监听 `cron:run_completed` / `cron:unread_changed` 刷新），一键清除 `cron_mark_all_read`（mark 全部 cron 会话已读 + emit `cron:unread_changed`）。**点开单条对话不自动标已读**，已读仅靠显式「全部已读」；Dock 角标不计入 cron 未读。视图模式 `localStorage` 持久化（`cron_view_mode`）
- **Cron 投递白名单（红线）**：`delivery_targets` 的 `(channel,account,chat,thread)` 必须命中 `channel_conversations`（`ChannelDB::conversation_exists`，与 `list_channel_targets` 同源）。创建/更新期对模型**显式提供**的未命中目标 `bail!` 拒绝（从当前会话推断出的目标可信、不校验）；投递期 `deliver_results` 投递前再查一次、未命中或 channel_db 不可用 **fail-closed 跳过 + warn**（`deliver_injection_for_session` 委托它自动继承）。白名单即边界，投递路径不叠加 SSRF。防 prompt 注入把携账号身份的周期投递变定向外泄通道
- **Cron 投递健壮性（§8）**：`deliver_results` 返回 `DeliveryReport`（attempted/succeeded/failed/skipped），白名单之上叠四项——① 每 target send 超时/报错按 500ms 指数退避**重投至多 3 次**（IM 不计费故默认开 + 固定次数、**非用户旋钮**，与 `async_jobs::retry` 的 config-gated 计费工具区分；语义 **at-least-once**，重投极少重复一条胜过静默丢结果）；② `cron_run_logs.delivery_status`（`run_log_status()` 派生 `None`/`delivered`/`partial`/`failed`，success 路插入后回写 / failure 路经 `record_failure` 新参带入，GUI run-log 列表展示）；③ 账号删除致目标失效 → `CronDeliveryTarget.stale` 经 **`apply_delivery_target_stale_flags` 单锁 read-modify-write 按 `account_id` 翻转（绝不走 `update_job` 重校验整条 schedule；绝不用 claim 时快照整列覆盖——长任务执行期间用户改投递目标不被回写覆盖）** 持久化，删账号入口 `mark_account_delivery_targets_stale`（幂等、每 job 走同一原子方法）eager 标记，GUI 标红；④ per-job `prefix_delivery_with_name`（opt-in 默认关，成功投递加 `[Cron] {name}` 前缀，job 级字段**不走设置三件套**）。删账号前 owner 平面 `cron_jobs_referencing_account`（Tauri / HTTP `GET /api/cron/jobs-referencing-account/{id}`）反向扫描提醒受影响任务
- **Cron 崩溃/取消/恢复一致性（§9）**：① **取消不丢 / 不误判（C4，红线）**——纯函数 `classify_cron_terminal(result, was_cancelled)` 裁决终态（穷举单测）。**cron 引擎跑 `abort_on_cancel=false`，取消中断返回 `Ok("")` 不抛 `Err`**：`Ok` 空串 + cancelled → Cancelled（不投空消息 / 不推进排程）；非空 `Ok` → Success（含晚到取消不丢已完成结果）；`Err && was_cancelled` → Cancelled（仅防御）；其它 `Err` → Failure。**绝不可改回 naïve「`Ok` 一律 success」**（会把取消中断当成功投空消息）。② **claim↔register 窗口（C7，红线）**——`cancel::register(&job.id, &claimed_at)` 提前到 claim 后 / 任何 await 前，RAII `CancelRegistrationGuard`（持 `claimed_at`）全路径清理；`cancel.rs` 的 `CANCELS` live flag 与 `PENDING_CANCELS` 占位**全部按 `claimed_at` run-keyed**——`CANCELS` 值为 `(claimed_at, flag)`，**live 分支与占位分支都比对 `claimed_at`**，`remove(job_id, claimed_at)` 亦然。`cancel_running_job` 读 `running_at` 与 `cancel()` 间有 TOCTOU：循环任务可能 A 跑完、以同 `job_id` 重 claim 成 B，**裸 job_id 命中会误翻 B 的 flag 取消 B**（旧实现只补了占位分支、live 分支裸命中即翻 = 半个洞）；run-key 后不匹配返回 `false`（目标 run 已逝、无可取消），`register` 只 drain 本 run 占位。③ **跨进程取消（C5）**——注册表进程本地、cron 仅 Primary 跑，跨实例取消回落 job-timeout 兜底（**不引入持久 `cancel_requested` 列**）；**C09**：`cancel` 占位分支加 `is_primary()` 门（内层 `cancel_with_pending(allow_pending)`，单测传 true），非 Primary 取消无 live flag 的 run 返回 **false** 不留泄漏占位、不骗 UI「已取消」。④ **崩溃留痕（D2）**——run 起跑即 `add_running_run_log`（`status='running'`/`finished_at=NULL`），终态走 `finalize_run_log` 单次 UPDATE（**绝不再为成功/失败/取消各 INSERT**）；**开 run_log 失败 → `run_log_id=None`（不 `unwrap_or(0)`），四条终态路径统一经 `finalize_or_insert_run_log`（`Some` UPDATE / `None` INSERT），否则 `UPDATE WHERE id=0` 匹配 0 行 → 整条审计行静默丢失**（no-session 早退同走 None→INSERT）；`recover_orphaned_runs` 由此真正生效，同进程 panic 由 `RunningMarkerGuard` 兜底 finalize。⑤ **Primary 可观测（C6）**——每 tick UPSERT `cron_meta.scheduler_heartbeat`，启动时陈旧（≥300s）`app_warn`；Primary 崩溃非丢任务（catch-up 补跑）故不做接管。⑥ **超时协作取消（审查修复 + 复核 C02）**——per-run 超时不硬 drop：先置 `cancel_flag` 再给 `CRON_TIMEOUT_CANCEL_GRACE_SECS`（5s 有界）让引擎收尾，终态判定纯函数 `compute_was_cancelled(timed_out, user_cancelled_pre_timeout, flag)`（超时路径忽略自设 grace flag，**但超时触发前用户已取消则归 Cancelled，C08**；非超时路径任何 flag=用户取消）。**宽限期结果经纯函数 `resolve_after_timeout_grace`（现收 `user_cancelled_pre_timeout`）判定（C02，红线）：未取消且期内跑完的非空 Ok 采纳为 Success（投递、不计禁用），否则（空 Ok / Err / 未完成）才归 Failure(timeout)；超时触发前用户已取消则丢弃宽限产出、归 Cancelled（C08 优先于 C02，合入前 /code-review #4）**——否则踩线完成的真实产出被丢、误投 timeout 失败、连续踩线 `max_failures` 次静默禁用本能跑完的健康任务。⑦ **infra 失败不计禁用（审查修复，红线）**——`record_failure` 的 `count_toward_disable=false`（session 创建失败这类 turn 未起跑的基础设施错误）走 `reschedule_without_failure`：推进 `next_run_at`、不 bump `consecutive_failures`、不自动禁用，否则几次瞬时 DB 抖动会禁用健康任务。
- **Cron 可观测性 + 日历精度（§10）**：① 零输出不掩盖——`classify_cron_terminal` 增 `Empty`（非取消的空 `Ok`）→ run_log `status='empty'` + **跳投递（不发空消息）** + 非失败推进排程（**recurring 走 `reschedule_without_failure` 不重置 `consecutive_failures`，C07**——否则偶发空输出抹掉失败计数让病态任务永不禁用；At-Empty 仍终态化 Completed）；`deliver_results` 加空 Success 守卫覆盖 G2 注入。**通知面 emit `status="empty"`（审查修复 #5，不再借 `"success"`）**，前端弹中性 `notification.cronEmpty`（**仅一次性 `At`；循环任务 empty 强制 `notify=false` 不每轮弹，`notify_empty = notify_on_complete && At`，合入前 /code-review #14**——「健康即静默」的监控任务否则每轮刷屏）；**`cancelled` 同样独立分支弹 `notification.cronCancelled`（#6），不落 `cronError`**。② error run 的 `cron:run_completed` 携 `failure_reason`（前端通知附原因）。③ **日历匹配改前向（审查修复 #7，替代旧 `min(±2min, 间隔/2)` 自适应窗口）**——`match_run_logs_to_occurrences` 把每条 log 归到「不晚于 `started_at` 的最近 occurrence」+ 60s 反向 skew 容差；旧对称窗口对秒级 cron 表达式（`validate_cron_expression` 无最小间隔）会压到 tick 延迟以下、**丢掉真跑过的日历圆点**。④ `find_job_by_session` 改 `ORDER BY id DESC`（确定性 tiebreak，防 G2 misroute）。⑤ `mark_missed_at_jobs` 保留 `LIKE '%"type":"at"%'` + 单测锁 serde tag。⑥ **dashboard 成功率不被新状态污染（#3）**——成功率分母改为 decided（`success + failed`）、`total_runs` 排除在途 `running`；`running`/`empty`/`cancelled` 既非成功也非失败，不进分母（`dashboard/{insights,queries}.rs` + 前端 `TaskSection` 同步）。**延后**：错过槽位 `skipped` run_log（心跳已覆盖宕机可观测）
- **Cron delete 审批（红线）**：`manage_cron action=delete` 是唯一对接权限引擎 v2 的 action（其余维持 internal 免审），delete 分支以 `is_internal=false` 调 `resolve_tool_permission`，引擎 `check_cron_delete`（落 `resolve_soft_approval_layer`，YOLO 短路 + AllowAlways 累加器之后）发**非 strict** `AskReason::CronDelete`：Default 弹 / Smart judge 自决 / YOLO·global-yolo 免 / 无人值守 fail-closed。非 strict 只约束 timeout/unattended 轴（不进 `forbids_allow_always`）；**AllowAlways 刻意抑制**——`gate_cron_delete` 强制 `allow_always_forbidden=true` + 前端 `barsAllowAlways` 禁按钮，因 `manage_cron` allowlist matcher 只按 `action` 不含 `id`，持久化即「删任意任务」id 无关常驻授权且 `allows_tool_call` 先于本门命中。`ApprovalReasonKind::CronDelete` + 前端 `ApprovalDialog.tsx` union + 12 语言 `approval.reasons.cron_delete` 三处同步（一致性单测锁后端两者）。不做 creator 作用域隔离。**删运行中任务先取消（C15）**：`delete_job` 删前对在途 run run-keyed `super::cancel::cancel`（按 `running_at` 比对，不误伤循环任务后续 run），止其白跑完 + 投递到已删任务；在途 run_log 随 `ON DELETE CASCADE` 删（用户主动删，审计行丢失可接受），终态写 no-op 命中已删行。三处 owner delete 入口统一走 `cron::delete_job_and_sessions`，连带清理该 job 的 cron 运行会话（摘出侧栏后否则既不可达又永久 orphan），细节见 [`cron.md`](docs/architecture/cron.md)
- **Cron 运行身份 `ChatSource::Cron`（红线）**：cron 执行的 `run_chat_engine` turn 背专属 `ChatSource::Cron`（不再复用 `Channel`），语义=「后台、非交互、但 owner-internal 的顶层会话」：`holds_foreground_idle_guard` / `fires_user_lifecycle_hooks` / `tracks_seq` 全真（后台注入须让位、顶层会话照常起 hook、拿并发流守卫），`broadcasts_to_user_ui` 假（不上主 bus，结果走 `delivery_targets`），`active_counts` 不计（与 Subagent/ParentInjection 同属后台）。**KB 访问关键**：`kb_access_source` 映射到 `KbAccessSource::Cron`（`is_im()==false`）→ `effective_kb_access` 不触发 `im_lineage_denied` → cron 走 owner 的 `max(session,project)` 路径，`note_*`/`[[note]]`/`knowledge_recall` 正常可用（旧版背 `Channel`→`Im`→WS8 无 `channel_kb_context` 一律拒，定时任务静默零 KB，本项修复）。**incognito 仍归零**（短路在 IM 门前）；cron 起的 subagent 继承 `origin_source=Cron` 不被 WS8 拒（executor 传 `origin_source:None`，引擎按 source 派生）；owner KB 读与 `delivery_targets` 投递是两道独立门（投递边界仍由 §1 白名单守）。新增 `ChatSource` variant 必须同步 `stream_seq.rs` 全部语义方法 + `active_counts` 穷举 match + `kb_access_source` 映射
- **Cron per-job 权限/沙箱覆盖 + 意图感知 Smart（红线）**：`CronJob.{permission_mode_override,sandbox_mode_override}: Option<{SessionMode,SandboxMode}>`（job 级字段，**不走设置三件套**，与 `job_timeout_secs` 同类）。`None`=跟随 Agent 默认；非空时 executor 经现成 `update_session_{permission,sandbox}_mode` 回写会话行（**单一真相源**，不碰权限引擎 / 不改无人值守 fail-closed）。**owner 平面专属**：GUI 表单 / Tauri / HTTP 可设，**模型面 `manage_cron` 工具恒 `None`、不进 schema、且 `update` 拒绝任何带 owner 覆盖的 job**（否则注入模型可改写一个 `permission=yolo` job 的 prompt 重置提权——`manage_cron_schema_never_exposes_*` 单测锁 schema、update 分支 `bail!` 锁改写）。**沙箱写入/预检全 fail-closed（不 fail-open）**：① 沙箱 override 写入失败即 fail-closed 终止本次运行（exec 读同一会话行，写丢=裸跑 host）；权限 override 写失败仅 `app_warn`（退回 Agent 默认=更严，安全）；② 预检读 `get_session_sandbox_mode`，**读错回退到 expected（override 或 Agent 默认）而非 `Off`**；③ 有效沙箱 `enabled()` 则 `ensure_sandbox_available()`，失败 run_log `error` + return、**绝不回落宿主机**，但 `count_toward_disable=false`（turn 未跑、无副作用，与 `no_session` 同档——否则瞬时 Docker 抖动 / 根本不调 exec 的任务会被误自动禁用）。**意图感知 Smart（无人值守专属）**：executor 经 `permission::task_intent`（session-keyed，RAII `TaskIntentGuard` 清）记录 cron prompt 为「意图」；`execution.rs` 仅在 **Smart 会话**经 `evaluate_approval_surface`（**单一真相源**，覆盖 cron / cron 血缘 subagent / headless / acp）派生 `ResolveContext.unattended` 并取意图 → `resolve_async` 透传 `judge::JudgeContext` → judge 放行与意图一致的删除/外发、拒越界/被注入的。**红线**：judge 只升降**非 strict** 的 Ask（strict `forbids_allow_always` 在 judge 前已拦、永不放行）；意图经 `<task_intent>` 信封结构隔离、明示「仅作范围参考、不自授权」（防意图自述「全部已授权」击穿）；意图（用户所写=可信）对比 args（模型所发=可能被注入）；**非 unattended/非 Smart 会话 `unattended=false`/`task_intent=None` → judge prompt 与 cache key 与改动前逐字节一致，普通对话 smart 行为零变化**（穷举单测锁）。外发仍叠 cron `delivery_targets` 白名单（§1）。**已知限制**：cron 血缘 subagent 与跨 turn 后台 job 的意图按会话 id 查不到（退化为保守的无意图无人值守框架，安全不越权）
- **Cron 并发配额 slot-before-claim（红线）**：`CronConfig.max_concurrent`（`AppConfig.cron`，默认 5，`0`=不限，MEDIUM）给调度器全局并发上限——每个 cron 运行是完整 agent turn，齐发会打满机器 / 触发限流。调度器（catch-up + tick 共用 `dispatch_due_jobs`）**先抢 slot 再 claim**：因 `claim_scheduled_job_for_execution` 副作用是推进 `next_run_at`，必须先 `count_running()`（`COUNT(running_at NOT NULL)`，并发计数单一真相源，覆盖 scheduled/catch-up/手动 run-now 三路）算 `available_slots(max, running)`（纯函数 `saturating_sub`），至多 claim `available` 个，到顶 `break` 让**剩余到期任务保持 `next_run_at` 不变下个 tick 重试**（不跳不丢）。手动 `run now` 绕过上限但 `running_at` 计入占用；`count_running` 失败 fail-closed 跳过本 pass。**run-now Primary-only + 正交（C10/C12a，红线）**：`execute_job_public`（三入口单 chokepoint）顶部 `is_primary()` 门——Secondary 永不跑 cron（避免被 Primary 启动 recover/clear 误清 + recurring 双 claim）；owner 三入口（Tauri `cron_run_now` / HTTP `POST /api/cron/jobs/{id}/run` / `manage_cron action=run_now` 工具）在 spawn 前各自前置 `is_primary()`，非 Primary 返错而非假成功（`{scheduled:true}` / "Triggered immediate execution"）（合入前 /code-review #3 + Codex 复核 P2 补齐工具路径，否则 Secondary 上 run-now 报成功却永不执行）；`ClaimedCronJob.immediate` 让 run-now 与调度/禁用正交——只记 run_log + 投递,**绝不动 status/schedule/consecutive_failures**(不复活 disabled、失败不 bump/不禁用计划任务)。**槽释放时序（合入前 /code-review #6）**：scheduled run 在 `deliver_results` 前即 `clear_running` 释放槽（`next_run_at` 已推进、不会重 claim），挂死/限流的投递目标不占 cap slot 阻塞其它任务；run-now（immediate）保槽穿过投递（next_run_at 未推进，早清会被中途重 claim）。三件套齐全（GUI 在设置页「定时任务」分区 `CronSettingsPanel`——cron 面板头部齿轮按钮经 `onOpenSettings("cron")` 深链进入；`ha-settings` `cron` category + SKILL.md），dedicated `get/save_cron_config`（Tauri + HTTP `/api/config/cron`）
- **Cron 失败处理 §5（可配 timeout / 分类 / 自动禁用通知）**：① `CronConfig.job_timeout_secs`（默认 **600**，`effective_job_timeout_secs()` 钳 `[30,7200]`，**`0` 钳地板非无限**——卡死运行只能靠超时释放槽）；**per-job 覆盖（C19）`CronJob.job_timeout_secs: Option<u64>`**（job 级字段不走设置三件套，经同款 `clamp_cron_job_timeout_secs` 钳）非空时优先于全局——让 legit 长任务声明自己预算而不抬全局，卡死任务仍超时+N 次禁用。与 `max_concurrent` 同属 `cron` category（同一 CronConfig，ha-settings 自动覆盖；GUI 在设置页「定时任务」分区，**save 必带全三字段**否则 serde 默认会重置其它）。② `cron::failure::CronFailureClass{Timeout/Configuration/Transient}` 纯函数分类，**只做诊断**（run-log status + 通知 reason，timeout→status=`timeout`），**刻意不改 `max_failures` 禁用策略**（防误分类过早禁用）。③ `update_after_run` 返回 `bool`（失败触顶翻 disabled 时 true，**`max_failures==0` = 不限/永不自动禁用**——判定加 `max_failures>0` 守卫对齐 `max_concurrent` 的 0-语义，否则 `>= 0` 恒真致模型/HTTP 传 0 首次失败即禁用，C26）→ `record_failure` 发**一次性** `emit_cron_disabled_event`：复用 `cron:run_completed` 但**强制 `notify=true`** + `auto_disabled`/`consecutive_failures`/`failure_reason`，前端 `useChatSession` 弹专属「已禁用」通知。`notification.cronDisabled*` + `cronReason.*` 12 语
- **Cron 排程校验单一真相源 §6（红线）**：`schedule::validate_schedule(&CronSchedule)` 是「这条排程是否合法」的唯一裁决——At timestamp 可解析、Every `interval_ms ∈ [MIN_EVERY_INTERVAL_MS(60000，1 分钟地板), i64::MAX]`（**上限红线 C13**：超 i64::MAX 在 `as i64`/`try_from` 溢出→`compute_next_run` 返 None→落成 active+next_run=NULL 永不触发/永不回收僵尸，`mark_missed` 只管 At）、Cron expr 合法 + 非空 timezone 是已知 IANA 名（空 / 空白 = UTC 不校验）。三入口统一调用:① 持久化 chokepoint `CronDB::add_job` / `update_job`（**关键**——owner 平面 Tauri `cron_create/update_job` + HTTP `create/update_job` 直接把前端构造的 `CronSchedule` 喂 add/update，此前只校验 Cron expr+tz，At 垃圾时间戳 / Every `interval_ms=0` 永不触发的死任务能绕过）② 模型工具路径 `parse_schedule`（提取 JSON 字段 + 归一化后委托 `validate_schedule`，不再各自内联值校验）。`validate_cron_expression` / `validate_timezone` 仍是 timezone/expr 级原语,被 `validate_schedule` 复用。**`update_job` 系统字段 DB 为准（C04，红线）**：`status` / `next_run_at` / `consecutive_failures` 从 live 行读、**不取 caller 快照**——编辑字段不丢在途退避偏移、不把系统在快照之后改的状态（如自动禁用）复活回 active；仅排程真变且 Active 才重算 `next_run_at`，仅 Active 编成过去 `At` 才 → `missed`，终态/暂停绝不复活
- **Cron At 补跑/终态 §7（红线，依赖 §4）**：`CronConfig.at_grace_secs`（默认 300，`effective_at_grace_secs()` 仅上限钳 7 天，**`0` 保留=严格不补跑**，与 timeout 的 `0`-floor 不同；同 `cron` category，GUI 在设置页「定时任务」分区）。`mark_missed_at_jobs(grace_secs)`（调度器传入，**启动恢复期与每个 tick 都必须在该轮 catch-up/dispatch 之前**跑，合入前 /code-review #5——不再仅启动一次，故 within-grace 抢不到 slot 的 `At` 累计逾期超 grace 后会被后续 tick 终态化 `missed`、不无限滞留 active）按 `cutoff=now-grace`：`At` 逾期 `< cutoff` → `missed`；`∈[cutoff,now]` → 保持 active 让 catch-up 经 §4 `dispatch_due_jobs` slot-aware 补跑；`next_run_at IS NULL` 的 active `At`（claim 后崩溃僵尸 / 过去时间戳创建）一并 → `missed`（**一次性可能已产副作用,标 missed 不重跑**，side-effect 安全）。`recover_orphaned_runs` 只修 run_logs、不碰 job 行,故僵尸终态靠 `mark_missed_at_jobs` 的 NULL 分支。**取消的一次性 `At`（审查修复 #11）**：`record_cancelled` 对 `At` 调 `terminalize_one_shot_completed`（`status='completed'`+`next_run_at=NULL`）即刻终态,不再留 `active`+NULL 僵尸等下次重启被 `mark_missed_at_jobs` 收（循环任务保持 active 按排程触发）。**失败/超时的一次性 `At`（复核 Sweep#1，红线）**：`update_after_run` 失败分支对 `At` 终态化 `missed`（`next_run_at=NULL`）、**不退避重试**——其 turn 可能已产生副作用（发邮件/下单），重投会重复副作用最多 `1+max_failures` 次；仅 infra 失败（turn 未起跑、无副作用）经 `reschedule_without_failure` 重试。recurring 仍走退避/自动禁用。**暂停不被成功跑复活（合入前 /code-review #2，红线）**：`update_after_run` 成功分支 UPDATE 加 `AND status='active'`——mid-run 被 `toggle_job` 暂停（不取消在途 run）的循环任务，该次运行成功完成不再被静默改回 `active`（对齐失败-禁用分支 `status != 'disabled'` 守卫；run-now 经 `immediate` 跳过本路径）
- **`schedule_wakeup`（自我定时唤醒，`crate::wakeup`）≠ cron**：agent 发起的**一次性**「N 秒后把我叫回当前会话续跑」，到点经 `inject_and_run_parent` 注一条 `<wakeup>` 起新 parent turn。`wakeups.db` 持久 + 进程本地定时器；clamp `[10s, async_tools.wakeup_max_delay_secs]`（默认 24h，10s 下限固定不可配）、每会话 pending ≤ `wakeup_max_pending_per_session`（默认 5）、**replay Primary-only**（防 Secondary 双投）、incognito 仅内存、会话删/焚经 `wakeup::purge_for_session`（由 `session::cleanup_watcher` 调用）取消。**不与 cron 复用入口**

### IM Channel

详见 [`im-channel.md`](docs/architecture/im-channel.md)。

- 12 个插件，状态文件落 `~/.hope-agent/channels/`；入站媒体走 plug → worker → `Attachment` → `~/.hope-agent/attachments/{session_id}/`
- 工具审批通过 EventBus `approval_required` 监听，按 `supports_buttons` 走原生按钮或文本；`auto_approve_tools=true` 跳审批（opt-in）。**绕过审计**：`auto_approve_tools` 跳过引擎门时，若被跳过的调用本会命中 strict 原因（`forbids_allow_always`），跑一次 no-enforce 探测并 `app_warn('permission','auto_approve_bypass')`——纯审计不拦截（排除 `external_pre_approved` async 重入防重复告警）
- **多端审批一致性红线（Epic G）**：所有决议路径(submit / 超时 / 删会话 / eviction)emit `approval:resolved {requestId,sessionId,decision,source}` 统一撤窗(`ApprovalResolutionSource`:gui/http/im/session_deleted/timeout_deny/timeout_proceed/eviction);listener 收到即 `drop_pending_by_request_id` 清 `TEXT_PENDING`。**IM 应答 fail-closed**:按钮回调缺源直接拒(approval 不复用共享 `validate_callback_source_for_session` 的 `None→Ok`,那只留 ask_user)、文本回复 submit 前同样校验 session↔chat(handover 后旧 chat 拒)。**chat 接管**(eviction)在 notify 门前无条件枚举拒决该 session 全部 pending 审批 + `drop_pending_for_chat`
- **Auto-start 失败统一走 [`channel/start_watchdog.rs`](crates/ha-core/src/channel/start_watchdog.rs)**——退避 30s/60s/120s/240s（cap 5m），sweep 15s，user 操作永远胜过 watchdog；失败日志带 `classify_channel_error` 分类
- **流式预览 Transport 三选一**（[`worker/streaming.rs`](crates/ha-core/src/channel/worker/streaming.rs) `select_stream_preview_transport`）：`Draft (Telegram DM 专属) > Card (仅飞书 cardkit) > Message (send+edit)`，各级有降级。新增 cardkit 风格靠 `ChannelPlugin` 上 4 个 default-impl=`Err` 的 trait 方法，仅飞书实现（11 个非飞书 channel `supports_card_stream=false` 走旧路径）
- **`ImReplyMode` 三态对所有渠道生效**（`ChannelAccountConfig.settings.imReplyMode`，默认 `split`，[`channel/types.rs`](crates/ha-core/src/channel/types.rs)）：`split` 每 round narration + 媒体按时序独立消息（流式渠道每 round 真打字机）；`final` 只发最后 round text + 末尾发媒体、不启流式；`preview` 流式渠道渲染合并文本、非流式降级 `final`。实现（round 边界 / per-round finalize / dispatcher `deliver_split`·`deliver_final_only`·`deliver_preview_merged` 三路）见架构文档
- **配置入口**：GUI `EditAccountDialog` 三选项 + `/imreply [split|final|preview]` 斜杠命令
- **`ChannelStreamSink` 短路条件用 `contains` 不能用 `starts_with`**：`emit_tool_result` 走 `serde_json::json!({...})` + 默认 `BTreeMap`，键按字母序输出（`call_id` 永远在前），任何 anchor 在 `{"type":...` 的 fast-path 都不会触发。`media_items` / `tool_result` / `text_delta` / `tool_call` 检测都用 `event.contains(...)`，rarer-needle-first
- **`channel_conversations` 1:1 attach（双向）**：每个 (channel, account, chat, thread) 在任意时刻只关联一个 session（`uq_channel_conv_chat`），且每个 session 在任意时刻只能被一个 IM chat attach（`uq_channel_conv_session`）。新 chat 通过 `/session <id>` 或 handover 接管时，目标 session 上的旧 attach **物理 detach** 并通过 `channel:session_evicted` 事件发"会话被接管"系统消息——不再保留 observer 行。helper 入口 [`channel/db.rs`](crates/ha-core/src/channel/db.rs)：`attach_session` / `detach_session` / `update_session` / `get_conversation_by_session`，**不要直接写 `channel_conversations`**。
- **`source` 字段**：`inbound`（IM 入站新建）/ `attach`（`/session <id>` 显式接管）/ `handover`（GUI handover 或 `/handover` 推到该 chat）。
- **GUI ↔ IM live 流式镜像**：desktop / HTTP turn 经 [`chat_engine/im_mirror.rs`](crates/ha-core/src/chat_engine/im_mirror.rs) `attach_im_live_mirror` 注册 `ChannelStreamSink` 到 [`SinkRegistry`](crates/ha-core/src/chat_engine/sink_registry.rs)，收尾 `finalize_im_live_mirror` 按 `ImReplyMode` 渲染——与入站对称。**两通道独立**：GUI 永远走 Tauri / HTTP 流、不受 `imReplyMode` 影响；`imReplyMode` 仅决定 IM 端形态。引擎自动 attach 对 `source ∈ {Subagent, ParentInjection, Channel, Cron}` no-op
- **后台完成注入回投 IM（G1/G2/G3）**：后台 job / subagent / group 完成的 `ParentInjection` 由 [`subagent::injection::inject_and_run_parent`](crates/ha-core/src/subagent/injection.rs) **自行 attach 注入镜像并在同一 future 内 await finalize**（注入跑短命 runtime，`spawn(finalize)` 会被腰斩）；cron 会话经 `cron::delivery::deliver_injection_for_session` 下发 `delivery_targets`。注入空闲门超时不丢弃，重排队进 `PENDING_INJECTIONS` 待空闲重试（group 合并注入不再永久丢失）

- **新 slash 命令**：`/sessions`（picker 用户对话 session，过滤 cron / subagent / incognito）、`/session [<id>|exit]`（info / attach / detach）、`/projects`（picker）、`/handover <ch:acc:chat[:thread]>`（GUI 端推送，IM 不可见）、`/kb [on|off]`（WS8，群聊 per-chat 确认 KB 访问；DM 仅报状态——账号级 opt-in 在桌面 Settings；无 arg/`status` 报当前生效态）。`IM_DISABLED_COMMANDS` 仅含 `agent` / `handover`。
- **`channel:session_evicted` 事件**：`attach_session` / `update_session` 在 1:1 接管把旧 chat 物理 detach 之后，对每个被踢的 chat emit 一次此事件，payload `{ channelId, accountId, chatId, threadId, sessionId }`。[`channel/worker/eviction_watcher.rs`](crates/ha-core/src/channel/worker/eviction_watcher.rs) 订阅后调对应 plugin 的 `send_message` 发"this chat has been taken over by another endpoint"通知；`ChannelAccountConfig.notify_session_eviction`（默认 `true`）可静音。

### Dashboard / Recap / Learning

详见 [`dashboard.md`](docs/architecture/dashboard.md) / [`recap.md`](docs/architecture/recap.md)。

- `dashboard/insights.rs`：overview delta / cost trend / heatmap / health score / `query_insights` orchestrator
- **模型用量总账**：所有会触发模型推理 / one-shot / side_query / summarize / embedding / STT / judge / web_search / image_generation / provider_test 的新调用入口，必须通过 [`model_usage.rs`](crates/ha-core/src/model_usage.rs) 写入 `session.db.model_usage_events`；Dashboard token / cost 总量以该表为准。Provider 原始 usage 返回多少就记录多少，未返回 token 的本地模型 / STT / embedding / 生图只记录调用次数与耗时，**禁止用字符估算冒充准确 token**。无痕会话不得入账。
- Learning Tracker 落 `session.db.learning_events`，目前埋点：`skills::author` CRUD + `tool_recall_memory` 命中 + MCP tool 调用
- `/recap` 独立 `~/.hope-agent/recap/recap.db` 缓存按 `last_message_ts` 失效；`recap.analysisAgent` 与主对话 Agent 解耦

### 跨会话 / 全局

详见 [`session.md`](docs/architecture/session.md) / [`behavior-awareness.md`](docs/architecture/behavior-awareness.md) / [`ask-user.md`](docs/architecture/ask-user.md) / [`prompt-system.md`](docs/architecture/prompt-system.md)。

- **数据存储**：所有数据在 `~/.hope-agent/`，[`paths.rs`](crates/ha-core/src/paths.rs) 集中管理
- **统一日志**：前后端走 [`logging/mod.rs`](crates/ha-core/src/logging/mod.rs)（SQLite + 文本双写），API 请求体 `redact_sensitive` + 32KB 截断；agent 自主排查入口见 [`skills/ha-logs/SKILL.md`](skills/ha-logs/SKILL.md)（用 `exec` + `sqlite3 -readonly` 直查 `~/.hope-agent/{logs,sessions,background_jobs}.db`）
- **延迟工具加载**：opt-in `deferredTools.enabled`，只发核心 ~10 个 schema，其余通过 `tool_search` 发现；execution dispatch 不变
- **会话搜索**：FTS5 + `<mark>` 高亮 + XSS 防御（escape → 白名单反解）；`Cmd+F` 复用同一 `search_messages` + session_id 过滤
- **ask_user_question**：1–4 题结构化问答（单选/多选/输入）；pending 持久化 SQLite，App 重启 replay 断点续答；IM 按 `supports_buttons` 走按钮或文本
- **会话级工作目录**：`sessions.working_dir` 注入 system_prompt `# Working Directory` 段，并作为 `exec` 实际 cwd（`execution.rs::default_cwd()`）与 `read` 工具相对路径解析的首选根
- **桌面专属 markdown 路径链接**：仅 `is_desktop()` 注入 `MARKDOWN_PATH_LINKS_GUIDANCE`，要求 LLM 写 `[名](绝对路径)`；前端按 `localPathFromHref()` + Transport 分流（Tauri 走 `open_directory`；HTTP/server 早返回禁用）。**例外**：anchor `title` 用 native HTML 不用 shadcn Tooltip（一条流式消息可能渲染上百个）

### 项目（Project）容器

详见 [`project.md`](docs/architecture/project.md)。

- **项目文件 = 工作目录里的真实文件**：上传文件直接落项目工作目录（无 `project_files` 表、无独立 `files/`/`extracted/`、无文本提取注入、无 `project_read_file` 工具）；模型靠 `# Working Directory` 段的顶层文件清单 + `read` 工具感知。**`project_files` / `ProjectFile` / `project_read_file` 已删，不要重新引入**
- **项目会话懒创建（desktop / HTTP 交互入口）**：进项目「新建对话」**不预先 `create_session_cmd` 落库**，停在草稿态，首条消息经 `chat` 的 `projectId` 走 `create_session_with_project` 才落库。`project_id` 与 `incognito` 互斥（后端强制 off）。**仅交互入口懒创建**（IM / cron / subagent 仍 eager）；前端 `effectiveProjectId` 是「当前项目」单一来源。**进项目入口不得再 `create_session_cmd` 预建**
- 记忆优先级 Project > Agent > Global
- **工作目录合并（项目会话总有值）**：优先级 `session > project 显式 working_dir > 默认 workspace`；唯一入口 [`session/helpers.rs::effective_session_working_dir`](crates/ha-core/src/session/helpers.rs)，**lazy ensure**（默认 workspace `~/.hope-agent/projects/{id}/workspace/` 首次解析时创建、不写 DB）。`project.working_dir` 留 NULL = 用默认 workspace
- **文件浏览器作用域**：所有读写经 [`filesystem::WorkspaceScope`](crates/ha-core/src/filesystem/workspace.rs)（canonicalize + 失败闭合），`for_session` / `for_project` / `for_path` 三入口；**`for_path` 是只读 worktree 跳转**（写操作一律拒）。HTTP `/api/fs/*` 写端点受 `filesystem.allow_remote_writes`（默认 false）闸门，桌面 Tauri 不受限
- 删除级联（三步）：unassign sessions → 删 `projects` 行 → `rm -rf projects/{id}/`（含默认 workspace；用户显式选的外部目录不删）→ 删项目记忆（跨 db 单独执行）
- **IM 路由（无反向认领）**：项目不再认领 (channel, account)。要把 IM 中的会话归项目，从该 chat 内 `/project <id>`（或 picker）显式触发；`AssignProject` action 在 channel worker 内 UPDATE `sessions.project_id`，不再通过 channel→project 反查。**`Project.bound_channel` 已删除，不要重新引入**。

### Agent 解析链（默认 Agent）

7 级（首个非空胜出）：**显式参数 → `project.default_agent_id` → `topic.agent_id` → `group.agent_id` → `tg_channel.agent_id` → `channel_account.agent_id` → `AppConfig.default_agent_id` → 硬编码 `DEFAULT_AGENT_ID`（`"ha-main"`，定义在 [`agent_loader.rs`](crates/ha-core/src/agent_loader.rs)）**。统一入口 [`agent/resolver.rs::resolve_default_agent_id_full`](crates/ha-core/src/agent/resolver.rs)；无 IM 上下文的 desktop / HTTP 用 `resolve_default_agent_id` 包装（只传 project + channel_account）。**channel worker 不得自写解析链** —— Phase A5 已折叠到 resolver 单一真相源。

**遗留 `"default"` 自动重命名**：升级到使用 `"ha-main"` 的版本时，启动期 [`agent/migration.rs`](crates/ha-core/src/agent/migration.rs) 一次性把磁盘目录（`agents/default/` / `default-home/` / `plans/default/`）、`agents/*/agent.json` 里的 `subagents.allowedAgents` / `deniedAgents`、SQLite agent_id 列（sessions / team_members / teams / subagent_runs / projects / background_jobs / canvas_projects / logs）、`memories.scope_agent_id`（`scope_type='agent'` 的行）、`cron_jobs.payload_json` 内嵌的 agent_id 全部 rename 到 `"ha-main"`，再改写 `config.json`（`default_agent_id` / `recap.analysisAgent` / channel 各级 agent_id），落 sentinel `~/.hope-agent/.agent-id-renamed` 后续启动短路。每步独立 idempotent，崩溃可恢复；当 `agents/default/` 与 `agents/ha-main/` 同时存在（用户手动建过 ha-main）时迁移整体放弃，不写 sentinel、不动 DB / config，下次启动重试。**入口契约**：`init_runtime` 必须早于 `ensure_default_agent()`——后者会预创空 `agents/ha-main/` 模板，吞掉 rename。新增字面量 agent id 一律走 `crate::agent_loader::DEFAULT_AGENT_ID`（前端走 `@/types/tools` 的 `DEFAULT_AGENT_ID` / `isMainAgent`），不要重新引入 `"default"` 硬编码。

### 本地 LLM 助手

详见 [`local-model-loading.md`](docs/architecture/local-model-loading.md)。

- 后端锁 Ollama（OpenAI 兼容端点）；模型目录硬编码 [`local_llm/types.rs::model_catalog`](crates/ha-core/src/local_llm/types.rs)，按预算从大到小取首个 ≤ budget
- 预算：macOS 统一内存 60% / Win+Linux dGPU VRAM 60% 优先回落系统内存 60%（`RECOMMENDATION_BUDGET_PERCENT=60`），扣 1 GiB runtime
- App **不接管** Ollama 进程；安装走官方 `install.sh`（macOS+Linux），Windows 引导用户去官网
- 后台任务统一走 [`local_model_jobs.rs`](crates/ha-core/src/local_model_jobs.rs) + `~/.hope-agent/local_model_jobs.db`
- **Provider 写入 contract（强制）**：所有 Provider 列表与 `active_model` 写入必须走 [`provider/crud.rs`](crates/ha-core/src/provider/crud.rs) helper（`add_provider` / `update_provider` / `delete_provider` / `reorder_providers` / `set_active_model` / `add_and_activate_provider` / `add_many_providers` / `ensure_codex_provider_persisted`）；本地 LLM 安装走 `upsert_known_local_provider_model`。**禁止直接 `providers.push` / `retain` / 手写 `active_model`**
- **Known local backend catalog** 在 [`provider/local.rs`](crates/ha-core/src/provider/local.rs)（ollama / litellm / vllm / lm-studio / sglang）；前端"是否已配本地后端"必须消费 catalog，**禁止硬编码 regex**

### 自升级

详见 [`self-update.md`](docs/architecture/self-update.md)。

- **三档路径**：`Tauri` (desktop bundle 走 tauri-plugin-updater) / `PackageManager` (brew / scoop / aur / apt / dnf) / `SelfContained` (下载 bare binary → minisign 校验 → atomic swap → restart)；不可识别走 `ManualPrompt` 让用户在 `ask_user_question` 里选
- **Minisign pubkey 单一真相源**：[`ha-core/updater/keys.rs::MINISIGN_PUBKEY_BASE64`](crates/ha-core/src/updater/keys.rs) 与 `src-tauri/tauri.conf.json#plugins.updater.pubkey` 必须字符串相等。三重防线：启动期 `keys::assert_pubkey_matches_tauri_conf` panic / CI `lint.yml` 跑 `scripts/verify-updater-pubkey.mjs` / 本地 `.husky/pre-push` 同脚本拦截
- **`app_update` 工具**（`tools::app_update`，`async_capable=true`）：4 个 action `check | install | status | rollback`；`install` / `rollback` 工具内用 `ask_user_question` 弹结构化 Yes/No 确认
- **UpdaterBridge trait**（[`updater::UpdaterBridge`](crates/ha-core/src/updater/mod.rs)）由 src-tauri 注册、ha-core 经 `OnceLock` 反向调用，**严禁 ha-core 直接依赖 tauri-plugin-updater**
- **Bare-binary release artifact**：`release.yml` 每平台用同一 Minisign 私钥签 `tar.gz`(Unix)/`zip`(Windows)，`patch-manifest` job 合并写回 `latest.json`
- **Binary swap 必须走 [`platform::atomic_replace_binary`](crates/ha-core/src/platform/mod.rs)**（Unix `rename(2)` / Windows `MoveFileExW` rename-aside）——**禁止 `fs::write` 直接覆盖运行中 binary**
- **自动更新统一配置 `AppConfig.auto_update`**（[`updater::AutoUpdateConfig`](crates/ha-core/src/updater/config.rs)，`ha-settings` 归 **HIGH**）：桌面与 headless 共享开关；后台只做**检查 + 静默预下载 + 校验**到 staging，**绝不自行 swap/restart**——安装走 `app_update install`（headless 审批）或桌面二选一，**桌面绝不无条件自动 relaunch**
- **下载健壮性红线**：下载走 [`download::download_to`](crates/ha-core/src/updater/download.rs)（重试 + `Range` 续传）；`self_contained::install` swap 后**先 `--version` 冷烟自检、失败自动回滚**再重启

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

- 新功能放 `crates/ha-core/` 单独模块；Tauri 命令在 `src-tauri/src/lib.rs` 注册，HTTP 路由在 `crates/ha-server/src/lib.rs`（`build_router_with_cors`）注册、handler 在 `routes/*.rs`
- 内部用 `anyhow::Result`；Tauri 命令边界用 `Result<T, CmdError>`（[`error.rs`](src-tauri/src/commands/error.rs)），`?` 直接传 `anyhow::Error`，不要 `.map_err(|e| e.to_string())`；HTTP 路由按 axum 习惯返 `Result<Json<T>, (StatusCode, String)>`
- 异步命令加 `async`，不要 `block_on`
- **禁止 `log` crate 宏**，必须用 `app_info!` / `app_warn!` / `app_error!` / `app_debug!`（[`logging/mod.rs`](crates/ha-core/src/logging/mod.rs)）。例外：`lib.rs::run()` 中 AppLogger 初始化前 + `main.rs` panic 恢复
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
- 新增 HTTP 端点须在 `crates/ha-server/src/lib.rs`（`build_router_with_cors`）注册
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
- **HIGH**：安全 / 网络暴露 / 全局键位 / 凭据 / 需要重启 / 权限规则 / 审批策略 / MCP 子系统级开关（proxy / shortcuts / server / skill_env / acp_control / `permission.global_yolo` / `smart_mode` / `mcp_global` / `protected_paths` / `dangerous_commands` / `unattended_approval` / `auto_update` 等）——技能在 `update_settings` 前**必须二次确认**（注：`embedding` 已改为只读，见下节「强制留 GUI 的例外」）

### 强制留 GUI 的例外（read-only via skill）

五类不进 `update_settings`（凭据安全 + 运行时稳定性）：**Provider 列表与 API Key**、**IM Channel 账号（`channels`）**、**MCP 服务器配置（`mcp_servers`）**、**`active_model` / `fallback_models` 写入**、**embedding 模型选择（`embedding`）——携 API Key + 重 reembed 副作用，写入走 Settings → Memory（`embedding_models` + `memory_embedding` owner 命令）**。`get_settings` 仍可读但敏感字段 redact（`channels.accounts[*].credentials/settings`、`mcp_servers.env/headers/oauth`、`embedding.apiKey`；embedding 读经 `resolve_memory_embedding_config` 解析真实启用模型）。

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

技术文档索引见 [`docs/README.md`](docs/README.md)（`docs/architecture/` 架构）。

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

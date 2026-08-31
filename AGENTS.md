# Hope Agent

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，内置 Provider 模板与预设模型，GUI 傻瓜式配置。三种运行模式：桌面 GUI（Tauri）、HTTP/WS 守护进程（`hope-agent server`）、ACP stdio（`hope-agent acp`）。技术栈见 `package.json` / `Cargo.toml`。

**本文只放跨 PR 必守的红线、同步契约与唯一入口**——实现细节、数据结构、迁移逻辑、边角行为一律在 [docs/architecture/](docs/architecture/)（索引 [docs/README.md](docs/README.md)）。加内容前先问：删掉它会让 agent 犯错吗？不会就别加。前端 / UI 风格规范见 [src/AGENTS.md](src/AGENTS.md)（`src/` 嵌套 AGENTS.md，改前端时自动生效）。

## 安全红线

- **API Key / OAuth Token 禁止出现在任何日志中**
- **Server Owner Token 只许走 Bearer header 或同源登录 body，禁止进 URL**；同源浏览器换 HttpOnly Cookie，跨源 WebSocket / iframe 只用短时、scope 受限票据，资源票据入口必须保持只读 allowlist
- `tauri.conf.json` CSP 不要放行外部域名
- OAuth token 在 `~/.hope-agent/credentials/auth.json`，登出时必须 `clear_token()`
- OAuth 保存只走安全目录与 `write_secure_file_outcome`，异步写经 `save_token_async`；写入未发布不得报登录成功，已发布的令牌轮换不得按旧凭据重试

## 提交前检查（强制）

[`.husky/pre-push`](.husky/pre-push) push 时自动跑全套门禁，与 CI required check 一一对应、改一边同步另一边；Agent 勿重跑。clippy / cargo test 覆盖 `ha-base` + `ha-agent-loop` + `ha-agent-runtime` + `ha-memory` + `ha-goal` + `ha-workflow` + `ha-config-schema` + `ha-core` + `ha-acp` + `ha-browser` + `ha-channel` + `ha-cron` + `ha-dash` + `ha-design` + `ha-eval-runtime` + `ha-improve` + `ha-knowledge` + `ha-mac` + `ha-local-llm` + `ha-mcp` + `ha-media` + `ha-pet` + `ha-skills` + `ha-updater` + `ha-vcs` + `ha-weather` + `ha-server`，`src-tauri` 不在门禁内、须 `--workspace` 自查。

- **开发中只单点验证**（`cargo check -p <crate>` / `pnpm typecheck`）；跑 clippy / cargo test / pnpm {test,lint} 须先问用户等回复，例外限跨 crate / 多文件收尾，跑前说明
- **应急跳过**：`HA_SKIP_PREPUSH=1`（限纯 `.md` / 弱网）/ `HA_SKIP_PREPUSH_TEST=1`（只跳 cargo test）。禁止 `--no-verify`（会绕过 GPG 等钩子）
- **i18n 无 CI 兜底**：当次改动涉及的 key 提交时须全语言齐全（存量缺失不强制），`node scripts/sync-i18n.mjs --check` 自查
- **评测不进 CI / PR / pre-push**：完整专项评测只本地显式跑（`hope-agent-eval`），默认 `cargo test` 只留快速契约测试；GitHub CI 不构建 ha-eval、不跑评测 smoke。详见 [capability-eval](docs/architecture/agent/capability-eval.md)

## 分支与发布

- 外部 Actions 固定完整提交摘要；R2 密钥只在读写步骤环境中可见，rclone 先验固定摘要再执行；守卫在 `scripts/check-workflow-supply-chain.mjs`（pre-push / CI 同源）

`main` 开发下个 minor，已发布 minor 各有 `release/vX.Y` 维护分支；跨分支只许 cherry-pick、禁 merge（否则未发布功能漏进维护分支）。

- 改 workflow job 名 / matrix 须 `gh api` 同步 ruleset `main-branch-protection` 的 `required_status_checks`；`lint.yml` / `rust.yml` `merge_group: checks_requested` 不可删，否则 Merge Queue 无 required checks
- **评测 GitHub workflow 当前暂停**：仓库无 capability-eval.yml / model-campaign.yml，release.yml 不校验 / 附加 eval evidence；deterministic 与真实模型证据链仍物理分离（policy 各一份 `evals/policy/release.json` / `evals/live/policy/release.json`），恢复远端评测须配置 PR 显式启用，不能只放回旧 workflow
- **真实模型评测仅本地 App / CLI**：隔离 `config.json` 禁存 Provider Key，只用合成 / 授权脱敏数据、禁个人生产账号与真实用户数据；当前不配置受保护 Runner / GitHub Provider secrets / 自动 Campaign / 签名发布证据，恢复后 Provider-only 防火墙才是网络边界（环境变量只作部署证明）

详见 [release-process](docs/release-process.md) / [capability-eval](docs/architecture/agent/capability-eval.md) / [live-model-evaluation](docs/architecture/agent/live-model-evaluation.md)

## 设置约定

用户可调配置须同时有 GUI 入口与 `ha-settings` 能力；新增/改 `AppConfig`/`UserConfig` 可调字段**同一 PR 三处缺一不可**：① `src/components/settings/` 面板；② `crates/ha-core/src/tools/settings.rs` 读写分支 + `SETTINGS_CATEGORY_RISKS` 风险级 + `core_tools.rs` `category` enum，**携密只读项还须加 `BLOCKED_UPDATE_CATEGORIES` + `read_category` redact（只加读＝凭据可写）**；③ `skills/ha-settings/SKILL.md` 风险表。

- **风险级单一登记 + 守卫**：`SETTINGS_CATEGORY_RISKS` 是风险级唯一来源（schema 由它派生）；HIGH / read_only 集合有 golden 测试钉住（改任一项即 fail 逼复核），`risk_level()` 撞未登记 category 触 `debug_assert`——别让 HIGH（安全/凭据/权限，全表见 SKILL.md）静默降级 medium 丢**写前二次确认**。
- **只读例外双理由（红线）**：凭据安全**或**运行时稳定性——`active_model`/`fallback_models` 不携密、无重副作用仍恒 GUI-only（须与 provider 状态/agent 重建协同），**别当误挡解封**；Provider 列表与 API Key 更严：无 category、禁新增入口。
- **凭据必脱敏（红线）**：带凭据新字段须接入 `redact_*_value`（否则 LLM 拿 history 当 leak 通道）；只覆盖非空串（保住「未设」vs「已清空」）。
- **读写 contract（红线）**：读 `cached_config()`、写 `mutate_config((category, source), …)`；禁 `Mutex<AppConfig>` / `load_config()`+`save_config()` 克隆-改-存。详见 [config-system](docs/architecture/infra/config-system.md)。
- **STT 默认参数统一合并**：batch 只在 `failover_transcribe_batch`、streaming 只在 `SttSessionManager::start` 合并 `stt.default_options`，请求的非空 / `Some` 字段优先；新增转写入口不得各自复制合并逻辑或绕过这两个边界。Azure Speech 的 `language` 缺失须在联网前 fail closed。

## 易错提醒（新增即同步）

Tauri 命令 → `invoke_handler!`；HTTP 端点 → `build_router_with_cors`；两者任一改动 → [api-reference](docs/architecture/system/api-reference.md)。Rust 依赖变更先 `cargo check --workspace`。改 `tauri.conf.json` 主窗口字段须同步 `tauri.windows/linux.conf.json`（平台 conf 对 `app.windows` **数组整体替换**，漏同步即该平台静默丢字段——Windows 曾因此丢 `center`）。

## 编码规范

前端 / UI 见 [src/AGENTS.md](src/AGENTS.md)（`src/` 嵌套 AGENTS.md，改前端时自动生效）。

### 文档（中文为主）

- **新增或修改中文文档时统一使用中文主术语**：标题、正文、表格和 Mermaid 可见标签都以中文为主；技术概念首次出现可写成“中文（English）”，后续只用中文。品牌、协议名、标准缩写，以及代码中的类型、函数、字段、枚举值、配置键、命令和路径可以保留英文；代码标识须用反引号。禁止无必要的中英夹写或给同一概念反复换译名，例如首次定义“权威会话历史（canonical history）”“请求投影视图（request projection）”后，后文只写中文。中英文用户指南仍须保持语义对齐。

### 后端（Rust）

- **阻塞 IO 红线**：async 里 SQLite/config 写必经 [`run_blocking`](crates/ha-base/src/blocking.rs)/`SessionDB::run`/`mutate_config_async`，禁 inline / `block_on`（[process-model](docs/architecture/system/process-model.md) Layer C′）
- **禁 `log` crate 宏**，用 `app_info!` 系列（例外见 [logging](docs/architecture/infra/logging.md)）
- **核心业务路径必须埋点**，带最小复现上下文；`category`/`source` 命名稳定便于 grep
- **禁字节索引切片字符串**，用 `crate::truncate_utf8`
- 错误：内部 `anyhow::Result`，Tauri 边界 `Result<T, CmdError>` 直接 `?`，禁 `.map_err(|e| e.to_string())`（[backend-separation](docs/architecture/system/backend-separation.md)）
- **跨平台原语统一进 [`platform/`](crates/ha-base/src/platform/)**，走 `crate::platform::xxx()`（[platform](docs/architecture/infra/platform.md)）

## 架构契约

子系统细节在对应 `docs/architecture/<name>.md`；本节只列跨 PR 契约与红线。

### 分层 & 运行模式

详见 [backend-separation](docs/architecture/system/backend-separation.md) / [process-model](docs/architecture/system/process-model.md) / [transport-modes](docs/architecture/system/transport-modes.md)；版本发布见 [release-process](docs/release-process.md)。

- **分层 Crate**：`ha-base`（基础设施：paths / logging / platform / security / permissions / terminal，**不得依赖任何 ha-\* 业务 crate**）← `ha-config-schema`（`AppConfig` 全部 wire 类型，**零行为逻辑**——需要子系统服务的行为一律留 ha-core，禁把子系统依赖拖进 schema）← `ha-core`（kernel）← 特征 crate（依赖 ha-core、壳层 `wire()` 装配）← `ha-server` / `src-tauri`（薄壳）。**「零 Tauri 依赖」适用于 ha-base / ha-config-schema / ha-core 与全部特征 crate**；事件走 `ha-core::EventBus`，核心层禁用 `APP_HANDLE`。新增 `AppConfig` 可达类型进 ha-config-schema、ha-core 原地 `pub use` 再导出（既有 `crate::config::…` 路径不变）
- **正式 turn 唯一准入是 `TurnKernel`**：Desktop / HTTP / ACP / Channel / Cron / Subagent / ParentInjection / SessionTool / Eval 全部构造封闭 `TurnRequest` + 来源专用 `TurnSubmission`，禁直接构造 `ChatEngineParams`、解析生产 model chain、调用旧 chat engine / `AssistantAgent::chat`。`ha-agent-runtime` 只消费 kernel 生成的 `AdmittedTurn`，拥有主 engine、四 Provider、one-shot、Hope round/tool driver 与 vision bridge；`ha-agent-loop` 是零 Hope/网络/DB 依赖的纯状态机
- **Goal / Workflow / Memory 同样「机器上浮、裁决留核」**：`ha-goal` 拥有 runner/policy/handler，`ha-workflow` 拥有 preview/QuickJS/typed-result/handler，`ha-memory` 拥有 recall/extract/embedding/external-provider/reembed/Dreaming；wire/status、`SessionDB` 类型化 ledger、permission/Stop/Eval/incognito/access verdict 与 required ports 留 `ha-core`。这四个 feature 禁裸连接/直接写 `sessions`/`messages`，旧实现不得回流；由 `scripts/check-agent-kernel-boundaries.mjs`（pre-push + CI）守卫
- **kernel 的 `sessions.db` 写连接不对特征 crate 开放（红线）**：`SessionDB::with_conn_internal` 恒 `pub(crate)`，特征 crate 一律走类型化方法；只读聚合自开 `SQLITE_OPEN_READ_ONLY` 连接（如 ha-dash）。拿到裸句柄即可绕过 kernel 对 `sessions` / `messages` 的不变量与事务边界，故**判据是零裸连接 / 零直接 SQL**——不是「不写 sessions/messages」（建会话、写 user message、开 chat turn 照样合法），而是对这两张表的每次写都经 kernel 类型化方法。**新组照此办理**：先算不动点（碰连接的、及其调用链全留 kernel），别拿通用 `with_conn` 当过渡；测试 fixture 走 `with_conn_for_test`（生产构建不存在）。分刀边界与迁移史详见 [backend-separation](docs/architecture/system/backend-separation.md)
- **工具契约层 `tool_defs` 与分发层 `tools` 单向**：`TOOL_*` 常量 / `ToolDefinition` 家族 / `ToolExecContext` / `ToolScope` / `ToolRejection` 定义在 `ha-core/src/tool_defs/`，kernel 一律 `crate::tool_defs::…` 引用；**生产代码绝不依赖 `tools::dispatch` / `registry` / adapter**，需要分发层行为的方法改 extension trait 挂分发侧（须显式 import，不在门面 glob 内）。**由 `scripts/analyze-crate-deps.mjs` 守卫**（pre-push + lint.yml，生产边零容忍、`--tests` 覆盖登记测试边）。详见 [backend-separation](docs/architecture/system/backend-separation.md)
- **`slash_commands` 是装配层，入向必须为零**：契约物（命令表 / wire 类型 / parser / fuzzy / 转录落库 / 选择器渲染）在 kernel `slash_defs/`，分发（dispatch / IM 菜单 / 技能参数元数据）经 `slash_hooks` 三槽在 `init_runtime` 注册，`slash_commands/` 只留 handler。特征 crate 禁 `use ha_core::slash_commands::…`，只准 `ha_core::slash_defs::…` / `ha_core::slash_hooks::…`；shells 与 kernel `app_init` 同源豁免。**`scripts/analyze-crate-deps.mjs` 双重守卫**（kernel 内部入边清零 + 跨 crate 禁引装配层，pre-push + lint.yml 强制）
- **特征分组图必须保持无环（红线）**：`scripts/analyze-crate-deps.mjs` 断言守卫（重新成环即非零退出，pre-push + CI，`--tests` 模式覆盖纯测试边构成的环）。兄弟间**单向**边合法（只约束拆分顺序），成环才拒——环内成员无法先于破环单独成 crate，而随手加的特征间反向边编译期完全无感。新增特征间引用前先跑一次脚本。详见 [backend-separation](docs/architecture/system/backend-separation.md)
- **Learning 埋点发布面在 kernel `learning_events.rs`**：生产者遍布 kernel / skills / knowledge / ha-mcp 四层，发布面留 dashboard 会让它们（含已拆出的特征 crate）反向依赖 ha-dash。`dashboard::learning` 只做只读聚合、保留原路径再导出；新增事件种类由生产者侧声明，dashboard 无需预先认识
- **ha-base 需要上层数据时留注册钩子、绝不反向依赖**：钩子在 `init_runtime()` 早期注册，冲突处理按语义各异（如 `security::dangerous::register_config_flag_source` 被顶替即 panic——它控制全局审批跳过，来源不可被替）。`ha-core` 以 `pub use ha_base::*` 全量再导出，故 `crate::paths::…` / `ha_core::platform::…` / `app_info!` 等既有路径**全部不变**
- **模块跨 crate 搬迁必须同步 [`.github/CODEOWNERS`](.github/CODEOWNERS)**：路径失配不报错，只会让安全代码悄悄失去强制评审
- **Transport**：**新 invoke 必须同时实现 Tauri + HTTP 两套适配**（[`transport.ts`](src/lib/transport.ts)）；新 HTTP 端点默认经 Bearer 鉴权
- **版本单一来源 `package.json`**：只走 `pnpm version` 同步，禁止手改任一 Cargo.toml / tauri.conf.json；**Updater 私钥严禁入仓**
- **模式判定**用 `ha_core::runtime_role()` / `is_desktop()`，别给共享函数加 mode 参数

### 工具 & 审批

详见 [docs/architecture/](docs/architecture/)：permission-system/tool-system/sandbox/browser/background-jobs/media-generation/file-operations。

- 工具调用唯一入口 `permission::engine::resolve_async()`；Smart 不消费 `custom_approval_tools`，UI 须提示。
- strict 永不自动放行：超时/无人值守 `proceed` 强制 deny；判定源 `AskReason::forbids_allow_always`，`ApprovalReasonKind::is_strict()` 须镜像。
- 无人值守 fail-closed：`check_and_request_approval` 预检 `evaluate_approval_surface`，`permission.unattended_approval_action` 默认 deny；可能 surface 即 Attended，唯 cron（含其血缘 subagent）例外——**判 cron 看 live turn 的 `ChatSource::Cron`（运行会话已是 `is_cron=0` 的普通会话），禁用 display-only 的 `origin`**；判 ACP 用 `is_acp()` 非 `ChatSource`（复用 Http）。
- `control.raw_cdp` strict：每调用必审批、永无 Allow Always（规则/smart 均绕不过）；方法/域黑名单 + SSRF 扫描 + 硬开关 `browser.extension.allowRawCdp=false` 三道执行层防御勿削弱。
- 出站 HTTP 必走 `security::ssrf::check_url`，新入口严禁自写 IP 校验。
- 可见性与执行层兜底走 `dispatch::resolve_tool_fate`（`tools.allow/deny` 只覆盖非 Core）。
- 结构化副输出唯一通道：`ToolExecContext.metadata_sink`→`messages.tool_metadata`→工作台；新工具禁自开旁路。
- 后台单元唯一入口 `async_jobs::JobManager`，禁平行 API；命名分裂勿改：模块/log `async_jobs`、DB `background_jobs`、事件 `job:*`；审批 park 桥在 `tools::approval`（tools 零依赖 async_jobs）。
- 双域勿合并：tool 池 `async_jobs::slots`，后台 subagent 池 `subagent::queue`；资源类（槽满）入队非拒绝，结构类（depth/batch/turn）硬拒不排队；parked 持槽不释放（否则 resume 无空槽死锁）、预算 timer 排除 parked 时长；`approval_projection_watcher` 只补 label、绝不 gate 执行。
- 重试白名单代码级：`is_retry_eligible` 仅 `web_search`/`web_fetch`；新 `BackgroundPolicy::GenericJob` 工具有副作用/计费就别加。
- `AsyncToolsConfig` 的 `0`：仅 `max_concurrent_jobs`/`_per_session` 真不限，其余 bounded-resource 旁钮钳到地板、绝非无限（`completion_merge_window_secs` 的 `0`=关，不在此列）。
- incognito：`output_tail` 永不注册；工作台聚合跳后端、只用 live tail。
- 图/音生成必走 `media_gen::execute_image`/`execute_audio`，禁各写 provider 循环；凭据只 owner UI 可写。
- **托管二进制供应链**：Chrome for Testing / FFmpeg 只读各自随包 manifest 的不可变 URL、精确大小、SHA-256、来源与许可证；必须先验摘要与冒烟再原子提升，失败保留上一份已验证版本，禁恢复 rolling `latest` 或仅凭 HTTPS/marker 放行。
- **沙箱镜像必须内容寻址**：默认 Debian slim 由 `sandbox-image-manifest.json` 固定多架构 digest，所有自定义 `SandboxConfig.image` 也须为 `name@sha256:<64 位摘要>`；旧裸默认 tag 只准迁移到内置 digest，禁止自动拉取其它可变 tag。Docker 部署 `isolated` 双层 fail-closed 与 non-root/read-only/network-none/cap-drop/资源上限不可削弱。
- 工作台聚合 dedup/排序 TS 与 Rust（`session::aggregate_session_artifacts`）两份须同步。
- 文件打开/下载/预览走 `useFileResource`；新可预览类型改 `src/lib/fileKind.ts` `isPreviewableKind`。
- preview-by-path：HTTP 三端点共用 `authorized_canonical_file_path`（tool 消息引用 ∪ 会话工作目录内），其余 403（远端严禁任意主机路径）；桌面信任本机。
- **Docker 部署执行沙箱只允许 `isolated`**：`HA_DEPLOYMENT=docker` 时工作区经有界副本 + Archive API 进入匿名 volume；`standard` / `workspace` / `trusted` 在预检与执行层双重 fail closed，禁止把父容器路径当宿主 daemon 路径。数据根、其祖先与 credentials 不得作为归档源；取消 / timeout 必须覆盖副本与归档准备全过程。

### Memory

详见 [memory](docs/architecture/core/memory.md)；Dreaming（claim 层 / Deep resolver / Lucid Review / 确定性评测）见 [dreaming](docs/architecture/core/dreaming.md)。

- **预算唯一入口 `effective_memory_budget`**（Project > Agent > Global）：turn-dependent 内容（Recall / Profile / Awareness）走 `CoreMemorySnapshot` 之后的动态 block，**不得重拼 Core system string**、不得因项目主题正文变化改稳定前缀（否则每轮废 prompt cache）
- **默认不静态注入**：仅完整 V1 rollback 或 `compatibility.legacyStaticMemory=true` 才恢复 `## Pinned Memory` Context Pack；其 claim 进 prompt 前须 `sanitize_for_prompt`（**与动态召回信封是两条独立义务**），legacy dedup 阈值须对齐注入阈值 `PINNED_MIN_SALIENCE`、**dedup 永不比注入更激进**（否则中等 salience claim 两头落空）
- **自动召回默认关**（`memory.recall.enabled`，Deep Recall 独立默认关）：关闭时只自动用 Core，**工具面不得 gate 在此开关**（模型仍可按需调 Memory tools）。开启后**过期 / superseded / archived / needs_review 不回灌**。旧 per-agent `ActiveMemoryConfig` 仅一个 minor 兼容 / rollback，**不得迁成全局同意**
- **自动流程永不硬改用户记忆**：Deep Resolver 冲突只在高置信写 `needs_review`、**永不自动 supersede**；低置信 / 未知 relation / LLM 失败均 no-op
- **外部 Memory 兼容门不得静默联网**：配置读取/预检零网络，仅 owner 显式测试连接时探测版本/能力；Graphiti `<0.28.2`、Supermemory 自托管 `<0.0.8`、OpenViking `<0.4.15`、Honcho 自托管 `<3.0.12` 全部阻断，未知版本只许 `PullOnly`，发送本地记忆的策略 fail-closed
- **纠错唯一入口 `claims::review`**：**无 agent 工具面**，只对用户开放、模型不能自改；**改 content 必 `reembed_claim`**，否则下轮召回仍命中旧文本
- **注入即 untrusted**：召回文本套 `<untrusted_external_data>`，项目索引注入前 XML escape，claim / 图谱文本进 prompt 前 sanitize
- **fail closed**：全局 / agent memory off、incognito、非项目会话在 schema 与执行层双归零。`sessions.incognito` 是无痕单一真相源（不注入 Memory / Awareness、跳过自动提取、关闭即焚，**与 Project / IM Channel 互斥**，四旁路守卫见 [session](docs/architecture/core/session.md#四旁路守卫epic-e)）。项目记忆读写拒 symlink 与 canonical escape、变更持项目级 OS 独占锁、更新 / 删除须带上次 `read` 的 BLAKE3 `expectedFileHash`（陈旧写 fail closed）
- **确定性评测刻意不进默认 Cargo test**：`memory/dreaming/eval.rs` + `evals/suites/memory-dreaming/fixtures/` **无 LLM**，只由 `hope-agent-eval` 跑（进 cargo test 或加 LLM 判分即破坏确定性）
- **改这些须同步**：claim 读路径 / effective-status / hidden-set / scope 过滤 / evidence 授权 → 加 fixture + 提 suite version + 追加 `evals/version-lock.json` key（已有 `id@version` 不可覆写，CI 强制 append-only）；Deep Resolver 分组 / 基数 / 决策映射 → `auto_resolver_graph_planning` fixture；检索 SQL / RRF / trigram → 跑 `pnpm memory:benchmark`
- **嵌入用途是向量契约**：新增 / 更新 / 重嵌显式用 `Document`，检索用 `Query`，只有相似度 / 聚类用 `Symmetric`；禁按单条/批量数量推断。用途与 provider 前缀/task 语义必须进签名及缓存键，旧签名 fail-closed 后只经可取消、幂等重嵌迁移，禁止原地重解释
- **Retrieval Planner**：`role=injected/selected` 是既成 prompt 事实，跨源只能 canonical-dedup / 裁剪 `candidate/considered`，**不得重排或丢弃已注入 ref**
- **新增 Goal / Workflow / Async / Agent 执行边界**须传播 `EvalRunContext` 身份并在终态关闭 guard；`evals/live/version-lock.json` 同样 append-only，manifest 禁 shell

### Subagent / Team / Cron

详见 [subagent](docs/architecture/agent/subagent.md) / [agent-team](docs/architecture/agent/agent-team.md) / [cron](docs/architecture/infra/cron.md) / [background-jobs](docs/architecture/agent/background-jobs.md)。

- **后台 subagent / Group 投影单向**：`subagent_runs` 为真相源，投影不持正文、不反写，排除 plan/team/hook 内部 spawn 与 incognito（durable 表，守关闭即焚）；同步只走 `SessionDB::update_subagent_status`，取消走 `subagent::request_cancel_run`（刻意不跑工具 job 的 hook/注入，勿并入统一取消）。`batch_spawn` 建 group 前预校验全部 task（否则漏交付），取消先标 group 终态再取消子 run
- **Subagent continuation 不抢在途回投**：续跑事务只准 suppress 尚未 claim 的 `pending` parent delivery；遇 `injecting` / `injecting_no_replay` 必须 fail closed，禁靠进程内 cancel 与跨进程 injector 竞速。显式消费 active delivery 只持久记录 consume request，由 claim owner 收尾为 `suppressed`；Primary 启动仅重置未消费的普通 `injecting`，no-replay arm 在无 owner 终止证明时保持 fail-closed、绝不自动 terminalize
- **Subagent Provider / 回投恢复不可失活**：Provider 整链失败的外层重试留在同一 child session，须可被 Stop 取消并向 parent 注入恢复状态；`subagent_result_deliveries.requested_at` 是 durable 回投退避真相，Primary 的 5s replay sweep 是运行期活性保证，禁止退化成只在启动 / Continue 时扫一次
- `TeamTemplateMember.description` 注入子 session 身份段
- **Cron 投递白名单**：`delivery_targets` 须命中 `channel_conversations`——模型显式给的未命中目标创建期 `bail!`，投递期再查、未命中或 DB 不可用 fail-closed 跳过。白名单即边界（刻意不叠 SSRF）
- **Cron delete 审批**：`manage_cron action=delete` 唯一非 internal action，刻意抑制 AllowAlways——matcher 只按 `action` 不含 `id`，持久化即「删任意任务」常驻授权。owner 三入口走 `cron::delete_job_and_legacy_sessions`：逻辑删除 Task，保留 run logs 与全部普通 / legacy Session；新增审批原因同步 `ApprovalReasonKind` + `ApprovalDialog.tsx` union + 全语言文案
- **Cron owner-only 覆盖**：`permission_mode_override` / `sandbox_mode_override` 仅 owner 可设，`manage_cron` 恒 `None`、不进 schema、`update` 拒带覆盖的 job（否则注入可排 `permission=yolo` 提权）。沙箱与权限 override 写失败**均 fail-closed 终止本次运行**：沙箱写丢=裸跑 host；权限写丢=按 agent 默认跑，agent 默认可能**比 override 更宽松**（owner 收紧场景 agent `yolo` → override `smart` 常见），静默回退即隐性提权，故两侧对称——**别拉回不对称**；预检读错回退 expected 而非 `Off`（防 `.unwrap_or(Off)`）；`ensure_sandbox_available_for_mode()` 失败即终止、不回落宿主机
- **Cron 排程与时区**：`schedule::validate_schedule` 为合法性唯一裁决（owner/模型共用），非法 IANA 时区 `bail!`、禁止静默回退 UTC；`compute_next_cron` 用 `.find(|dt| *dt > *after)` 非裸 `.next()`（否则 DST 秋退写入过去时刻 → 每 tick 重触发）；时区 backfill 经 `cron_meta` sentinel `tz_backfill_done` 真·一次性（形似性能优化，删掉即把故意-UTC 任务静默改成宿主时区）；`update_job` 系统字段以 DB live 为准、不取 caller 快照
- **Cron Primary-only + slot-before-claim**：执行与 run-now 三入口前置 `is_primary()`（非 Primary 返错不假成功）；调度器先 `count_running()`（并发计数单一真相源，失败 fail-closed 跳过本 pass）抢槽再 claim——claim 会推进 `next_run_at`，反序即静默丢一轮
- **启动清理必须带 owner 界（红线）**：`clear_stale_running` / `recover_orphaned_runs` 恒按 `running_owner` / `started_owner != CronDB::owner_token`（`NULL` 视为「不是本进程」）判断。`start_scheduler` 起独立 OS 线程后立即返回，启动清理与 `app_init` 下一行的事件 watcher **并发**——去掉这条界即回到「watcher 派出的合法在途任务被当遗留清掉 → 周期 tick 重新 claim → 副作用跑两遍」。**别改回调用顺序保证**（那条从未成立），**也别改回时间界**（`Utc::now()` 不受 Rust happens-before 约束，系统时间回拨即误清）。owner token 与墙上时钟解耦，回拨也不误清
- **`at_grace_secs` 的 `0` 是 async_tools 规则的例外**：`0`=严格不补跑、只钳上限不钳地板，勿套用「bounded-resource 旁钮 `0` 一律钳地板、绝非无限」。`save_cron_config` 替换整个 `CronConfig`——新增字段须同步各 save 调用点，漏传即被 serde 默认静默重置
- `CronFailureClass` 只做诊断、刻意不改 `max_failures` 禁用策略（防误分类过早禁用）
- **`ChatSource::Cron`**：`kb_access_source` 映射 `KbAccessSource::Cron`（非 IM → owner KB）、incognito 归零；新增 variant 须同步 `stream_seq.rs` 语义方法 + `active_counts` 穷举 match + `kb_access_source` 映射
- Cron 终态语义（取消不误判 / 空输出不掩盖 / `At` 失败不重试 / infra 失败不计禁用 / 暂停不复活）互锁，改 `classify_cron_terminal` / `update_after_run` / `mark_missed_at_jobs` 前必读 cron.md；新增 run_log status 须同步 `dashboard/{insights,queries}.rs` 成功率口径 + 前端 `TaskSection` / `cronHelpers`
- **`schedule_wakeup` ≠ cron、不复用入口**：replay 仅 Primary（防双投）、incognito 仅内存、会话删经 `wakeup::purge_for_session` 取消

### LLM 主对话

详见 [provider-system](docs/architecture/core/provider-system.md) / [failover](docs/architecture/agent/failover.md) / [side-query](docs/architecture/agent/side-query.md) / [agent-config](docs/architecture/core/agent-config.md) / [automation-model](docs/architecture/core/automation-model.md)

- spawn / shell / automation 触发的正式多轮 turn 一律构造来源专用 `TurnSubmission` 进入 `TurnKernel`；禁止自包 Provider / round / `on_delta` 循环
- Codex 不参与 failover profile 轮换（OAuth 无 profile，executor 按 `api_type` 强制关，caller 传 true 也无效）
- 视觉桥 `crates/ha-agent-runtime/src/vision_bridge.rs`：`function_models.vision` opt-in、未配=关（回退占位符、不自动挑选）。只改 `api_messages` 副本、绝不改 `conversation_history`（就地改=永久丢图）；只扫 user/tool、跳 assistant（改写毁 tool 调用）；转录套 `<untrusted_external_data>`、绝不作 system 指令；绝不在 side_query 触发（防递归）；incognito 走 per-turn 缓存、绝不写全局
- 后台一次性 LLM 调用走 `automation::run` / `run_vision` + `function_models.automation`，同类消费者勿另写形状。例外：Memory Extract 与 Compact 摘要刻意不接入（签名不支持链式循环；Compact 属 fail-fast 关键路径），只加 `model_override`，勿迁移

### Chat Engine & Streaming

详见 [chat-engine](docs/architecture/core/chat-engine.md)；未读口径见 [session](docs/architecture/core/session.md)。

- **侧聊是父会话拥有的一等隐藏会话**：恒用 `SessionKind::Side` + 独立 `TurnKernel` / 消息账本 / 审批与流状态，`forked_from_session_id` 记录归属，禁止伪装成普通 sidebar session 或复用主会话执行态；创建时只复制稳定历史并标 `messages.is_side_snapshot=1`，Dashboard 消息统计必须排除快照而保留之后的新活动。永久删除主会话须级联其全部侧聊，项目与工作目录始终按侧会话自己的持久化元数据解析。
- **未读单一来源**：普通未读计**会话数**，资格只走 `regular_session_scope_sql` / `regular_unread_exists_sql`，禁止分页求和；Scheduled 是同一 regular watermark 的过滤投影（读普通会话即清 Scheduled 角标），IM Channel 仍是独立域、与普通未读互不清除，新专属对话空间须用独立 `SessionKind`
- **Bundled HTTP UI 只作观察者**：非 incognito 主对话由服务端持有执行；页面、WebSocket 或反向代理断开不得取消 turn，前端须以 durable `turnId` 重连终态；会话删除导致 turn 404 时须终止本地等待并释放轮询 / 订阅
- **Stop 须自证「还会不会有终态事件」**：`StopChatResult`（Tauri 与 `POST /api/chat/stop` 同一形状，禁各造）里 `terminal_event_pending` / `completion_sealed` / `latched` 三者全 false = 本次 Stop 一个事件都不会发（陈旧终态 turn、无活跃 turn 的 session-only Stop），调用方必须自行收敛、禁空等。Stop **只报本次做了什么**，权威状态恒由 `get_session_stream_state` 唯一提供，别在 Stop 里再算一份。前端收敛须双读 + 1.5s 确认 + 无 request owner；轮询兜底两次权威读都 `admissionActive=false` 时**必须无视 turn id 不匹配**（本地那个才是陈旧的，否则 15s 轮询永久 bail、只能重启），但不得越过 request-owner 守卫；Stop 按钮全窗口去重（targeted pause 每调一次新建一条回执并把上一条谎标 `resumed_at`）
- **API-Round 分组**：新 Provider adapter 须经 `push_and_stamp` 标 `_oc_round`（否则压缩切割拆散 tool_use / tool_result 配对），请求体构建前统一 `prepare_messages_for_api()` 剥离元数据
- **前台 idle guard 单一入口**：`ha-agent-runtime` 的共享 engine 按 TurnKernel 封印的 `ChatSource::holds_foreground_idle_guard()` 统一建 `ChatSessionGuard`（ACP 同路，无例外），新增对话入口不得手搓 per-shell guard
- **Typed `@file` 绑定不跨工作区漂移**：composer 草稿须按会话或 lazy project 隔离；文件选择的 provenance 必须绑定选择时的有效 workspace root，发送时 IncomingTurnWire 与 typed attachment 共用同一 root，项目 / 会话 / 工作目录变化后须失效而不得把相对路径重定向到另一工作区
- **Stop / Continue 是持久世代围栏**：Stop receipt 事务须 `Immediate`，先落 `session_autonomy_pauses` 再收敛 foreground stream / Goal / Workflow / Subagent / Wakeup；每次 Stop 新建 generation，Continue 必须 exact `pause_id`。用户输入“继续”由模型经 eager `session_continue` 解锁，owner Tauri / HTTP 也不得绕过精确回执；model-facing Continue 还须证明 foreground provenance 且 turn admission epoch 不早于当前 Stop，旧前台 turn 禁替用户解锁。Global Stop 必须从共享 `chat_stream_runs` 枚举别进程 Desktop / HTTP / IM / ACP；incognito 绝不为此落 stream/session 身份，只比较 session-free `runtime_control_epochs.global_stop`。Global receipt 必须标记发布它的 epoch，admission 后迟落的同代 receipt 不得误杀新的 foreground turn，targeted/下一代 Stop 仍须胜出。每个进程只取消自己持有的 foreground / injection / subagent / workflow runtime，须按 immutable run id / lineage/global epoch 轮询共享 Stop generation，禁 Secondary 直接 terminalize 别进程 runner，且快速 Continue 不得藏掉旧 Stop；Secondary Continue 只在同一 CAS 发布 durable replay request，wakeup / workflow runtime 必须由 Primary 定时认领，禁在 Secondary 本地假恢复或消费后无 handoff；新增自主执行边界须接入暂停、重放与重启 fence

### 桌面宠物（Pet）

详见 [pet](docs/architecture/core/pet.md)。

- **主对话投影边界**：只接入显式携带第一方 `ChatUiSurface` 的主动多轮主对话；side query、automation、compact、Memory、Cron、IM、ACP、subagent 与后台 job 等额外 LLM 请求不得接入。Pet 点击气泡只发 typed navigation，**不得提前清未读**；必须由目标消息列表真实加载并渲染后的 read receipt 推进 watermark
- **宠物包自动化导入唯一入口**：本机走 `hope-agent pet preview` → 用户确认 `packageHash` → `hope-agent pet import --expected-package-hash`，远程走 Bearer-auth HTTP preview / commit；来源域名不决定资格，但所有入口仍须走 `ha-pet` 的统一校验与原子安装，禁止技能或壳层直接写宠物目录、静默安装，或在用户仅请求导入时顺带启用 overlay。用户明确请求“导入并启用”时，commit 成功后只许以返回的 `petRef` 独立走 `hope-agent pet activate` / desktop-only `POST /api/pets/activate`；两者必须把选择 + enabled 原子交给当前 Tauri 进程驱动窗口生命周期，禁用 `enableAfterImport` 偷渡或离线改配置假成功

### 上下文压缩

5 层渐进式 + `ContextEngine` / `CompactionProvider` 可插拔；阈值、TTL 节流、反应式微压缩、Tier 3 文件恢复详见 [context-compact](docs/architecture/core/context-compact.md)。

### Knowledge Base（知识空间）

详见 [knowledge-base](docs/architecture/core/knowledge-base.md)。

- **两类存储**：笔记 `.md` = 唯一真相源；注册表 + **访问绑定**落 `sessions.db`；`index.db` 仅可重建缓存，**权限绝不落其中**（重建即静默重置授权）
- **访问默认 deny**：唯一裁决 `effective_kb_access`（incognito / IM 未 opt-in 归零；subagent 按 origin 血缘不洗权限）；owner 平面不经 attach，agent 平面（`note_*`）必过
- **agent 侧唯一解析链**：`Agent::resolve_kb_access()`，prompt 段 / 被动召回 / 工具门控共用，**不得重写**；**只服务 schema/prompt/召回，绝不 gate 执行**（执行走 live `access_map`）。`is_kb_scoped_tool` / `ToolScope::Knowledge` 仅收窄 schema 可见性，**非安全边界**
- **写入三闸**：`WorkspaceScope::for_knowledge`（外部 root 只读、**桌面也拒**（刻意反「桌面不受限」通例），须 `allow_external_writes`；HTTP 再叠 `allow_remote_writes`；**后台维护永不写外部**）→ `platform::write_atomic`（**禁回退 `fs::write`**）→ `expected_file_hash` 比磁盘 raw BLAKE3（**非索引 `content_hash`**）
- **检索独立**：笔记 store **绝不折进 `recall_memory`**（`knowledge_recall` 两段不混排）；`knowledge_embedding` 与 `memory_embedding` 物理隔离、**不寄生不回退**；embedding / chunk 重 reindex 故 **GUI-only 不进 `ha-settings`**（设置三件套例外）
- **Knowledge 确定性评测不进默认测试**：chunk / parser / FTS / 聚合 / KB 隔离 / evidence 坐标变更须同步 `knowledge-retrieval-evidence` fixture、提升 suite version，并向 `evals/version-lock.json` 追加新 key；只由显式 `hope-agent-eval` 跑，禁 LLM / 网络
- **读取即 untrusted**：`[[note]]` 与 `knowledge_passive_recall` 套 `<untrusted_external_data>` 信封，**永不升为 system 指令**；incognito 零召回 / 零精灵
- **接线**：会话独立 `SessionKind::Knowledge`（主列表 / `/sessions` / 全局 FTS 隐藏，与 design 同谓词）；**新增 KB 工具须同步 `ha-knowledge` 的 `tools/note.rs`（handler）+ `tools/mod.rs::note_dispatch_entries`（dispatch 条目，经 `wire()` 的 `register_external_tools`）+ kernel `core_tools.rs`（schema，名字常量与 `ToolDefinition` 是纯契约、恒留 kernel）**

### 设计空间（Design Space）

详见 [design-space](docs/architecture/infra/design-space.md)。**新增 action / 端点：工具进 `crates/ha-design/src/tool_design/mod.rs`，Tauri / HTTP 薄壳只调 `design::service`，逻辑全在 ha-design 特征 crate**。

- **浏览器零编译**：iframe 只载后端编译落盘的静态产物（`component` 经 `design::compile`）；**禁 in-browser Babel / esbuild-wasm / Tailwind JIT**（曾因此白屏卡顿）；编译失败降错误页，**不白屏 / 不 panic**。**刻意不做无限画布**（同一卡顿根因）
- **回写确定性**：磁盘即真相源，`design.db` 仅可重建注册表；微调回写单一命中 + `expected_hash` stale-write 守卫，写盘**一律** `platform::write_atomic`。**component 编译产物 ≠ 源码故无 oid 微调**，仅 `supports_oid_edit` kind（非 image/audio/component）可 `edit_element`
- **边界**：owner（`service.rs`，本机 / API key 信任，**刻意不经 access 检查**）与 agent `design` 工具两平面隔离；iframe 恒 `sandbox="allow-scripts"`；`ToolScope::Design` 仅收窄 schema、**非安全边界**；**incognito 零设计**（fail-closed）；`SessionKind::Design` **与 knowledge 同谓词从主侧栏 / `/sessions` / 全局 FTS 隐藏**，新增专属空间**必须**同步该谓词
- **小改必须就地精改**（实测曾抹空整页）：`get_artifact` → `edit_element(oid)`，**绝不整段 `update_artifact` 重造、绝不 web_fetch 读产物**

### Agent 控制平面 / 通用场景

详见 [docs/architecture/](docs/architecture/)：goal/workflow/loop/context-retrieval/domain-{workflow,quality,eval}。

- **控制面归位**：`/goal` 目标+完成标准 · `/mode` 强度 · `/workflow` 一次性可恢复可审批脚本 · `/task` 进度 · `/loop` 重复触发（复用 Cron，不另起 scheduler） · `/worktree` 隔离 coding。禁持续触发伪装 workflow、一次性脚本伪装 loop。
- **领域模块不扩权、不执行**：`domain_workflow_templates` 只述交付契约，不给连接器权限；`preview_domain_workflow` 只出 draft/preview，不建 run 不执行、不碰连接器、不发/改外部系统。Domain Learning 复用 Coding Improvement 同一 proposal queue（禁平行队列），preview → apply → 用户显式 promotion 才落生产，禁直改生产模板/connector 策略/eval fixture。
- **复核评测**：Domain Quality、`domain_eval_runs`、`evaluate_domain_quality_gate` 确定性只读：不调 LLM、不写状态、不碰连接器、不发送/发布、不自动学成正式规则。证据走 `domain_evidence_items` + Goal link，禁冒充 diff/validation/file evidence；与 `coding_eval_runs` 不混用，coding release gate 不代替 domain quality gate。
- **crate 边界**：机器（提案生成 / 蒸馏 / 预览 / 落盘 / 提升、fixture 与 campaign 跑批、质量复核、四道闸与 soak 报表）在 `ha-improve`，**台账（三个模块的 `impl SessionDB` 纯 SQL 层 + wire 类型 + 行映射）恒留 kernel** 同名模块；kernel → ha-improve 只经 `improve_hooks` 单槽（工作流终态记 coding retro，未装配 `Ok(None)`）。新增 owner 入口只加壳层薄封装，**别把 SQL 写进 ha-improve**
- **两处 fail closed**：只有 `requestedAction` 命中 approval gate 或 `highRiskAction=true` 才 `needs_user`（否则模板带 gate 即阻塞普通复核），缺确认阻塞 Goal；incognito 下 preview/evidence/quality/eval 拒绝或返空只读，不落 durable。

### Hooks

详见 [hooks](docs/architecture/agent/hooks.md)（单一真相源，字段级对齐 Claude Code hooks 协议）。**硬验收 = 5 个套件**（协议面 `hooks_compat` / 字段名 `hooks_compat_payload` / 可阻断 `hooks_compat_blocking` / 输出面 `hooks_compat_output` / Stop 再驱动 `hooks_stop_continue`），跑法见 hooks.md §14；只跑 `hooks_compat` 只覆盖 1/5。

- **唯一入口 `HookDispatcher::dispatch` / `hooks::fire_*`**；调用方只读 `HookOutcome`，严禁 match handler 类型
- **新 user message 入口须过 `agent::preflight::user_prompt_preflight`**（`UserPromptSubmit` 阻断点），并把交给 `active_turn::try_acquire` 的**同一个** `turn_id` 填进 `PreflightArgs`；**不 acquire 的入口**（如 ACP）传自铸 id 或 `""`——`""` 恒等于「省略 `prompt_id`」，绝不回落注册表(否则会把同会话另一轮的 id 盖上来)；新 hook 事件须埋点 + 测试 + 同步 `types.rs` 三处 match（`common`/`matcher_target`/`is_observation_only`）——**漏登记 `is_observation_only` 则新观察事件意外可阻断**
- **project/local scope 按工作区与内容授权**：执行前须同时命中 canonical cwd 与 project/local Hook 文件 BLAKE3；路径别名、symlink、移动、内容变化均 fail closed。旧 `hooks_allow_project_scope` 仅反序列化兼容、执行层忽略且不得自动迁移（否则信任所有未来 cwd）；授权只许 GUI/owner 传输入口，`ha-settings` 对 hooks 只读（可写 = 模型自装命令执行）。command Hook 子进程先 `env_clear()`，只继承最小运行环境 + `allowedEnvVars` 逐名声明，合成 `HOPE_*` / `CLAUDE_*` 最后覆盖；禁恢复全宿主环境继承

### Plan Mode

详见 [plan-mode](docs/architecture/agent/plan-mode.md)。

- **进入永远由用户拍板**：模型只能经 `enter_plan_mode` Yes/No 审批，**不能自己转 state**
- **plan = 设计契约（执行期不改），task = 唯一进度真相**
- **执行层兜底**：`resolve_tool_permission` 必须查 live plan state，防 mid-turn 进 plan 后剩余工具绕过

### Skill 系统

详见 [skill-system](docs/architecture/agent/skill-system.md)（优先级/激活入口/`allowed-tools` gap/`skills::author` 原语）。

- **内置技能编译期嵌入二进制**（`skills/embedded.rs`）：禁止往构建产物单独拷 `skills/`
- **`@skill` 固定 allowlist**：非通用注入入口，单一来源 `skills::mention::AT_MENTIONABLE_SKILLS`
- **`skills::author` 写正文三路径（create/update/patch）全过 `security_scan`，命中即 bail 不降级**；自动**创建**默认落 draft 待用户确认，但 `promotion:"auto"` 直接写 Active；**`patch` 就地改已存在技能——目标 Active 时即刻对模型生效，不落 draft、不经确认**
- **crate 边界**：机器（解包 / 发现解析 / 创作 / auto-review / 提及 / fork / 命令面 / `skill` 工具）在 `ha-skills`，**契约 `skills/types.rs` + 台账 `skills/activation.rs` + 纯谓词 `skills/{requirements,prompt,slash}.rs` 恒留 kernel**（slash 命令表与 system prompt 直接用，条件激活台账被 `tools::execution` / `system_prompt` / `cleanup_watcher` 三处读写）；kernel → ha-skills 只经 `skills_hooks` 九槽。**`rerun-if-changed=../../skills` 在 `crates/ha-skills/build.rs`**——它与 `#[folder]` 必须同 crate，分开即 warm-target release 静默 ship 旧技能集

### MCP 客户端

**配置读写**：读 `cached_config().mcp_servers`，写 `mutate_config(("mcp.<op>", source), …)`；网络 transport 与 OAuth 全路径出站过 SSRF 门，凭据 0600 落 `credentials/mcp/`。详见 [mcp](docs/architecture/integration/mcp.md)。

### 平台 MCP 服务器（`hope-agent mcp`）

**红线**：共享 host `ha-core/src/mcp_server/`（`ToolProvider` 注册表），不做子系统专属 server；默认只读、`--allow-writes` 才注册写集且 host 层双保险再拦；**恒不暴露**写代码仓库 / deploy / share / delete / export 类工具；stdio interop 经 `acquire_or_secondary_for` 恒**被动 Secondary**，永不争 Primary。详见 [mcp-server](docs/architecture/integration/mcp-server.md)。

### IM Channel

详见 [im-channel](docs/architecture/integration/im-channel.md)。

- **审批一致性 + fail-closed（红线）**：所有决议路径（submit/超时/删会话/eviction）必须 emit `approval:resolved` 统一撤窗；approval / ask_user 捕获并复验 exact `InteractiveAttachIdentity`，按钮回调缺源即拒（**不复用 ask_user 的 `None→Ok`**）、文本 submit 前校验完整 route；chat 接管在 notify 门**前**只拒决 identity 已失效的旧 pending，禁误拒 replacement attach 新请求；`auto_approve_tools`（opt-in）跳门时命中 strict 须 `app_warn('permission','auto_approve_bypass')`——**纯审计不拦截**
- **事件匹配用 `contains` 不用 `starts_with`（红线）**：`emit_tool_result` 的 `json!`+`BTreeMap` 键按**字母序**排（`call_id` 恒首位），锚 `{"type":...` 的 fast-path **永不触发**
- **`channel_conversations` 双向 1:1（红线）**：一 chat ↔ 一 session，接管即物理 detach 旧 attach + emit `channel:session_evicted`；读写一律走 [`channel/db.rs`](crates/ha-core/src/channel/db.rs) helper，**禁止直接写表**
- **注入回投须在同一 future 内 await finalize**：`inject_and_run_parent` 自驱动镜像（注入跑短命 runtime，`spawn(finalize)` 会被腰斩）；空闲门超时**不丢弃**，重排队进 `PENDING_INJECTIONS`
- **单一入口勿另起**：流式预览选路走 `select_stream_preview_transport`，新卡片风格靠 `ChannelPlugin` default=`Err` trait 方法扩展；assistant / mirror / catch-up / eviction / startup 的 provider 写统一经 `worker/provider_lane.rs` 按物理 target 预留顺序，实际 future 交给进程生命周期 executor，禁 request runtime 取消已接受 mutation；attach catch-up 必须先 `prepare_attach_catchup` 预留 lane，再且只能消费 `AttachCatchupReservation::attach`，由同一 DB 临界区完成 active generation + message watermark capture 与 durable attach，禁 plain attach 后 capture（极短 turn 会漏投或 live/static 双发）；exact generation 的 provider terminal 须保留有界 `Completed` tombstone，只有零 mutation / attach moved / ParentInjection `Confirmed` abort 可 release，禁 Guard Drop 直接忘记已投终态；新内容每次写前 live-check attach，旧 handle 的 abort / close 只做 lane-only cleanup；Card 未确认 update 不得 close；native mutation 只有能证明平台零送达的拒绝才准降级，timeout / 断连 / 5xx / 未知码一律 `Ambiguous` 禁补发；cosmetic typing/loading 必有有界超时、不得阻塞主回复；auto-start 失败重试走 [`channel/start_watchdog.rs`](crates/ha-channel/src/channel/start_watchdog.rs)（**user 操作永远胜过 watchdog**），勿自写退避

### 跨会话 / 全局

详见 [`docs/architecture/`](docs/architecture/)：session / ask-user / prompt-system / behavior-awareness / help-center

- 数据在 `~/.hope-agent/`，新路径走 `paths.rs`；日志走 `logging/mod.rs`，请求体必经 `redact_sensitive`
- 唯一结构化问答入口 `ask_user_question`：富输入 / 风格卡只能扩展它（答案仍走 `selected[]`），绝不 fork
- `sessions.working_dir` 三用：`# Working Directory` 段 + `exec` cwd + `read` 相对根，非纯 prompt 提示
- 手册单一来源 `docs/user-guide/`（rust-embed）：禁复制正文 / 拷进产物；中英同 PR 对齐（CI `check-docs-parity`）。例外：Dockerfile rust 阶段 `COPY docs/user-guide` 是编译期 embed 依赖，须保留
- markdown 路径链接仅桌面：`is_desktop()` 才注入 `MARKDOWN_PATH_LINKS_GUIDANCE`；其 `[名](绝对路径)` 格式与前端 `localPathFromHref()` 是同步契约；非桌面靠 `supportsLocalFileOps()` 关入口 + `/api/desktop/open-directory` 返 no-op（**不是**早返回禁用）。例外：anchor `title` 用原生 HTML 非 shadcn Tooltip（一条消息上百个）

### 项目（Project）容器

详见 [project](docs/architecture/core/project.md)。

- **已删勿引入**：`project_files`/`ProjectFile`/`project_read_file`（项目文件=工作目录真实文件）、`Project.bound_channel`（IM 无反向认领，归属靠 chat 内 `/project <id>`）
- **交互入口懒创建**：进项目「新建对话」不得 `create_session_cmd` 预建，首条消息经 `chat` 的 `projectId` 落库；`project_id` 与 `incognito` 互斥（**后端强制 incognito off**）；IM/cron/subagent 仍 eager
- **两个唯一入口**：工作目录 `session::effective_session_working_dir`（session > project > 默认 workspace）；文件读写 `filesystem::WorkspaceScope`（失败闭合，`for_path` 只读，HTTP 写受 `filesystem.allow_remote_writes` 默认 false）
- **多源文件夹身份不可漂移**：`projects.linked_dirs_json` 顺序即 `project_folder` scope 稳定索引；会话 cwd 覆盖项目主目录时，主目录作为末尾虚拟源根继续进入 prompt / 工具 allowlist / 文件浏览器，索引为 `linked_dirs.len()` 且仅 session scope 可解析，live 项目主目录路径不匹配即 fail closed
- **AGENTS.md 缺失可保留**：添加已有目录时用户可取消创建，之后只读检查、启动迁移和未改工作目录的元数据更新均不得补建；读取返回 `exists`，保存必须回传 `expectedExists` + raw BLAKE3，存在状态或内容任一变化都 stale-write fail closed
- **删除级联**：`rm -rf projects/{id}/` **绝不波及用户显式选的外部 working_dir**；跨 db 项目记忆单独删、启动期 reconciler 兜底

### Agent 解析链（默认 Agent）

详见 [agent-config](docs/architecture/core/agent-config.md)。

- **7 级链唯一入口 [`agent/resolver.rs::resolve_default_agent_id_full`](crates/ha-core/src/agent/resolver.rs)**：顺序固定、首个非空胜出；channel worker 与新会话入口不得自写解析链
- **禁止裸字面量 agent id / 重新引入 `"default"`**：走 [`agent_loader::DEFAULT_AGENT_ID`](crates/ha-core/src/agent_loader.rs)（当前值 `"ha-main"`；前端走 `@/types/tools` 同名常量 + `isMainAgent`）
- **启动序**：`init_runtime`（含 legacy `"default"`→`"ha-main"` 一次性迁移）**必须**早于 `ensure_default_agent()`，否则预创空 `agents/ha-main/` 模板会吞掉 rename（迁移整体放弃且静默）

### 自升级

详见 [self-update](docs/architecture/infra/self-update.md)。红线：

- **下载产物必须验签**：更新下载走 `ha_updater::download::download_to`，落地 / swap 前必过 Minisign `signature::verify_bytes`
- **pubkey 两处必须相等**：`ha_updater::keys::MINISIGN_PUBKEY_BASE64` ↔ `tauri.conf.json#plugins.updater.pubkey`（启动 panic / CI / pre-push 三重校验）
- **manifest endpoints 两处逐项逐序相等**：`ha_updater::manifest::UPDATE_MANIFEST_URLS` ↔ `tauri.conf.json#plugins.updater.endpoints`（`verify-updater-endpoints.mjs`，CI + pre-push，另校验镜像域名 ↔ `mirror-release-r2.yml` 的 `PUBLIC_BASE`）。R2 镜像恒排第一——**是可达性不是延迟**（部分用户访问不了 github.com）；**刻意维持「首个成功者胜」**、不做比版本取新，否则桌面与 headless 会对「当前哪个版本」分歧
- **镜像 manifest 是派生物、GitHub 那份权威**：`mirror-release-r2.yml` 只改 URL 与 `notes` 链接，`signature` 原样复制**绝不重算**（验签用编译期嵌入 pubkey，故污染镜像装不进恶意二进制，最坏只能拒服务/谎报版本）；必须**先回抓校验全部对象再写 `latest.json`**，顺序反了可变 manifest 就会指向不存在的字节
- **可变面只许当前稳定版写（红线）**：`download/latest/` 与 `download/latest.json` 全局共享，给非 latest / prerelease 写＝降级广播（R2 是 endpoint[0] 且首个成功者胜，全体客户端从此看不到真新版）——回填旧 tag 与发 prerelease 都会触发本 workflow，故推进可变面须过 `PROMOTE` 门控；不可变 `download/<tag>/` 永远照写。`latest/` 别名**禁带 immutable 头**（文件名每版复用）、可变上传须 `--ignore-times`（同名同大小会被静默跳过）；`checkout` 的 `ref:` **必须显式**指默认分支（`release.published` 下 `github.ref` 是 tag，裸 checkout 会取 tag，历史版本从此不可回填）；`update-linux-repo.yml` 整桶 pull 的 `--include` 过滤是 load-bearing，新增本 job 顶层路径须同步
- **换 binary 只走 `platform::atomic_replace_binary`**（禁 `fs::write` 覆盖运行中 binary）；swap 后冷烟自检失败自动回滚
- **安装 / 重启必经用户确认**：`auto_update` 后台只检查 + 预下载 staging，`app_update` 的 `install` / `rollback` 弹 `ask_user_question`，**桌面绝不无条件 relaunch**
- **ha-updater 不依赖 tauri-plugin-updater**（自升级独立 crate），桌面路径经 `ha_updater::UpdaterBridge` 反向注册
- **壳层装配契约**：每个调 `ha_core::init_runtime` 的二进制（src-tauri / hope-agent-server / ha-eval runner）必须先调 [`ha_server::wire_features()`](crates/ha-server/src/lib.rs)——**单一来源**，按序调 `ha_updater::wire()` + 各特征 crate 的 `wire()`。**别在 shell 里内联 `wire()` 序列**（漏改任一处即静默丢 handler：有 schema 无 handler，启动期只剩 `registry_freeze` warn 兜底）。新增特征 crate = 改 `wire_features()` 一处 + 三个壳的 `Cargo.toml` 加 path dep

### Dashboard / Recap / Learning

详见 [dashboard](docs/architecture/infra/dashboard.md) / [recap](docs/architecture/infra/recap.md)。

- **用量总账（红线）**：新增任何触发模型推理 / embedding / STT / judge / `web_search` / 生图生音 / `provider_test` / vision 的入口必须经 [`model_usage.rs`](crates/ha-core/src/model_usage.rs) 入账（无痕不记，**cron / subagent / 后台维护照记**），**禁止字符估算冒充 token**；新增 `KIND_*` 须同步 `DashboardFilter.USAGE_KIND_VALUES` + `dashboard.usageKind.*` 全部语言
- **大盘只读、不伪造因果（红线）**：ha-dash 的 `dashboard/control_plane.rs` 是 Goal / Workflow / Loop / Task / Plan 聚合唯一入口；无可靠外键前禁止按 session 拼因果漏斗，零分母返 `null`，Goal / Workflow / Loop / Task / attention 排除 incognito / Cron / 子会话

### 本地 LLM 助手

详见 [local-model-loading](docs/architecture/core/local-model-loading.md)。

- Ollama 自动脚本安装当前关闭、所有平台引导手工下载；恢复须证明固定版本、大小、摘要及二阶段下载完整性，不能只验证脚本入口

- 后端锁 Ollama（OpenAI 兼容端点），**App 不接管其进程**；模型目录与硬件预算算法见 `crates/ha-local-llm/src/local_llm/types.rs::model_catalog` / `RECOMMENDATION_BUDGET_PERCENT`（任务台账仍在 kernel `local_model_jobs`）
- **Provider 写入 contract**：Provider 列表与 `active_model` 一切写入走 [`provider/crud.rs`](crates/ha-core/src/provider/crud.rs) helper（本地安装走 `upsert_known_local_provider_model`），**禁止 `providers.push` / `retain` / 手写 `active_model`**
- **本地后端判定消费 catalog**（[`provider/local.rs`](crates/ha-core/src/provider/local.rs)），**禁止硬编码 regex**

## 项目结构

二十六 crate workspace：`ha-base`（基础设施底层，**不依赖任何 ha-\* 业务 crate**）/ `ha-config-schema`（`AppConfig` wire 类型闭包，**只依赖 ha-base 与叶子 crate、零行为逻辑**）/ `ha-core`（核心业务，**零 Tauri 依赖**）/ 特征 crate `ha-acp`·`ha-browser`·`ha-channel`·`ha-cron`·`ha-dash`·`ha-design`·`ha-eval-runtime`·`ha-improve`·`ha-knowledge`·`ha-local-llm`·`ha-mac`·`ha-mcp`·`ha-media`·`ha-pet`·`ha-skills`·`ha-updater`·`ha-vcs`·`ha-weather`（依赖 ha-core，壳层 `wire()` 装配，**同守零 Tauri 红线**；`ha-eval-runtime` 是唯一无 `wire()` 者——kernel 对它零引用）/ `ha-server`（axum HTTP·WS）/ `ha-browser-host`（浏览器辅助进程）/ `ha-eval-spec`（评测协议，**不依赖 ha-core**）/ `ha-eval`（评测 CLI）＋ `src-tauri/`（桌面薄壳），`src/` 前端，`skills/` 内置技能，`evals/` 评测资产。

## 开发命令

```bash
pnpm desktop                          # 交互选择下面四种桌面开发模式
pnpm dev:desktop                      # 默认桌面开发（不构建 Browser Host / Eval Sidecar）
pnpm dev:desktop:browser              # Chrome 插件联调（仅构建 Browser Host）
pnpm dev:desktop:eval                 # 评测功能开发（仅构建 Eval Sidecar）
pnpm dev:desktop:full                 # 完整桌面能力（构建两者）
pnpm tauri dev                        # 兼容旧入口（等价于完整能力，构建两者）
node scripts/sync-i18n.mjs --check    # 翻译缺失（--apply 补齐）
cargo run -p ha-eval --locked -- validate   # 评测资产校验
```

其余脚本读 `package.json` scripts；CLI / Docker / 评测子命令见 [cli](docs/architecture/system/cli.md) / [docker](docs/deployment/docker.md) / [capability-eval](docs/architecture/agent/capability-eval.md)。

## 文档维护

索引 [`docs/README.md`](docs/README.md)。**AGENTS.md 只放跨 PR 红线与入口**，细节下沉 `docs/architecture/`。

同 PR 同步：功能/命令/模块增删 → `CHANGELOG.md` + `AGENTS.md`；技术栈/架构/规范/契约 → AGENTS.md；子系统边界/数据流/持久化/跨模块 contract → architecture 文档，新增架构级能力新建文档 + 登记索引；Tauri 命令/HTTP 路由/`COMMAND_MAP` 增删 → `docs/architecture/system/api-reference.md`；子系统/架构文档/运行时 DB/稳定 log `category` 增删 → `skills/ha-self-diagnosis/references/diagnostic-playbook.md`；README/release notes 任一语言 → 同步 .en.md。

**CHANGELOG 单行**：用户视角一句 + `(#PR)`，不写实现；契约/红线可加一行用户影响。

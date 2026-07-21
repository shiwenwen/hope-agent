# Hope Agent

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，内置 Provider 模板与预设模型，GUI 傻瓜式配置。三种运行模式：桌面 GUI（Tauri）、HTTP/WS 守护进程（`hope-agent server`）、ACP stdio（`hope-agent acp`）。技术栈见 `package.json` / `Cargo.toml`。

**本文只放跨 PR 必守的红线、同步契约与唯一入口**——实现细节、数据结构、迁移逻辑、边角行为一律在 [docs/architecture/](docs/architecture/)（索引 [docs/README.md](docs/README.md)）。加内容前先问：删掉它会让 agent 犯错吗？不会就别加。前端 / UI 风格规范见 [src/AGENTS.md](src/AGENTS.md)（`src/` 嵌套 AGENTS.md，改前端时自动生效）。

## 安全红线

- **API Key / OAuth Token 禁止出现在任何日志中**
- `tauri.conf.json` CSP 不要放行外部域名
- OAuth token 在 `~/.hope-agent/credentials/auth.json`，登出时必须 `clear_token()`

## 提交前检查（强制）

[`.husky/pre-push`](.husky/pre-push) push 时自动跑全套门禁，与 CI required check 一一对应、改一边同步另一边；Agent 勿重跑。clippy / cargo test 只覆盖 `ha-core` + `ha-server`，`src-tauri` 不在门禁内、须 `--workspace` 自查。

- **开发中只单点验证**（`cargo check -p <crate>` / `pnpm typecheck`）；跑 clippy / cargo test / pnpm {test,lint} 须先问用户等回复，例外限跨 crate / 多文件收尾，跑前说明
- **应急跳过**：`HA_SKIP_PREPUSH=1`（限纯 `.md` / 弱网）/ `HA_SKIP_PREPUSH_TEST=1`（只跳 cargo test）。禁止 `--no-verify`（会绕过 GPG 等钩子）
- **i18n 无 CI 兜底**：当次改动涉及的 key 提交时须全语言齐全（存量缺失不强制），`node scripts/sync-i18n.mjs --check` 自查
- **评测不进 CI / PR / pre-push**：完整专项评测只本地显式跑（`hope-agent-eval`），默认 `cargo test` 只留快速契约测试；GitHub CI 不构建 ha-eval、不跑评测 smoke。详见 [capability-eval](docs/architecture/capability-eval.md)

## 分支与发布

`main` 开发下个 minor，已发布 minor 各有 `release/vX.Y` 维护分支；跨分支只许 cherry-pick、禁 merge（否则未发布功能漏进维护分支）。

- 改 workflow job 名 / matrix 须 `gh api` 同步 ruleset `main-branch-protection` 的 `required_status_checks`；`lint.yml` / `rust.yml` `merge_group: checks_requested` 不可删，否则 Merge Queue 无 required checks
- **评测 GitHub workflow 当前暂停**：仓库无 capability-eval.yml / model-campaign.yml，release.yml 不校验 / 附加 eval evidence；deterministic 与真实模型证据链仍物理分离（policy 各一份 `evals/policy/release.json` / `evals/live/policy/release.json`），恢复远端评测须配置 PR 显式启用，不能只放回旧 workflow
- **真实模型评测仅本地 App / CLI**：隔离 `config.json` 禁存 Provider Key，只用合成 / 授权脱敏数据、禁个人生产账号与真实用户数据；当前不配置受保护 Runner / GitHub Provider secrets / 自动 Campaign / 签名发布证据，恢复后 Provider-only 防火墙才是网络边界（环境变量只作部署证明）

详见 [release-process](docs/release-process.md) / [capability-eval](docs/architecture/capability-eval.md) / [live-model-evaluation](docs/architecture/live-model-evaluation.md)

## 设置约定

用户可调配置须同时有 GUI 入口与 `ha-settings` 能力；新增/改 `AppConfig`/`UserConfig` 可调字段**同一 PR 三处缺一不可**：① `src/components/settings/` 面板；② `crates/ha-core/src/tools/settings.rs` 读写分支 + `SETTINGS_CATEGORY_RISKS` 风险级 + `core_tools.rs` `category` enum，**携密只读项还须加 `BLOCKED_UPDATE_CATEGORIES` + `read_category` redact（只加读＝凭据可写）**；③ `skills/ha-settings/SKILL.md` 风险表。

- **漏登记风险级不报错**：`risk_level()` 静默回落 `medium`，HIGH（安全/凭据/权限，全表见 SKILL.md）失去**写前二次确认**。
- **只读例外双理由（红线）**：凭据安全**或**运行时稳定性——`active_model`/`fallback_models` 不携密、无重副作用仍恒 GUI-only（须与 provider 状态/agent 重建协同），**别当误挡解封**；Provider 列表与 API Key 更严：无 category、禁新增入口。
- **凭据必脱敏（红线）**：带凭据新字段须接入 `redact_*_value`（否则 LLM 拿 history 当 leak 通道）；只覆盖非空串（保住「未设」vs「已清空」）。
- **读写 contract（红线）**：读 `cached_config()`、写 `mutate_config((category, source), …)`；禁 `Mutex<AppConfig>` / `load_config()`+`save_config()` 克隆-改-存。详见 [config-system](docs/architecture/config-system.md)。

## 易错提醒（新增即同步）

Tauri 命令 → `invoke_handler!`；HTTP 端点 → `build_router_with_cors`；两者任一改动 → [api-reference](docs/architecture/api-reference.md)。Rust 依赖变更先 `cargo check --workspace`。

## 编码规范

前端 / UI 见 [src/AGENTS.md](src/AGENTS.md)（`src/` 嵌套 AGENTS.md，改前端时自动生效）。

### 后端（Rust）

- **阻塞 IO 红线**：async 里 SQLite/config 写必经 [`run_blocking`](crates/ha-core/src/blocking.rs)/`SessionDB::run`/`mutate_config_async`，禁 inline / `block_on`（[process-model](docs/architecture/process-model.md) Layer C′）
- **禁 `log` crate 宏**，用 `app_info!` 系列（例外见 [logging](docs/architecture/logging.md)）
- **核心业务路径必须埋点**，带最小复现上下文；`category`/`source` 命名稳定便于 grep
- **禁字节索引切片字符串**，用 `crate::truncate_utf8`
- 错误：内部 `anyhow::Result`，Tauri 边界 `Result<T, CmdError>` 直接 `?`，禁 `.map_err(|e| e.to_string())`（[backend-separation](docs/architecture/backend-separation.md)）
- **跨平台原语统一进 [`platform/`](crates/ha-core/src/platform/)**，走 `crate::platform::xxx()`（[platform](docs/architecture/platform.md)）

## 架构契约

子系统细节在对应 `docs/architecture/<name>.md`；本节只列跨 PR 契约与红线。

### 分层 & 运行模式

详见 [backend-separation](docs/architecture/backend-separation.md) / [process-model](docs/architecture/process-model.md) / [transport-modes](docs/architecture/transport-modes.md)；版本发布见 [release-process](docs/release-process.md)。

- **三 Crate**：业务全进 `ha-core`（**零 Tauri 依赖**），`ha-server` / `src-tauri` 只做薄壳；事件走 `ha-core::EventBus`，核心层禁用 `APP_HANDLE`
- **Transport**：**新 invoke 必须同时实现 Tauri + HTTP 两套适配**（[`transport.ts`](src/lib/transport.ts)）；新 HTTP 端点默认经 Bearer 鉴权
- **版本单一来源 `package.json`**：只走 `pnpm version` 同步，禁止手改任一 Cargo.toml / tauri.conf.json；**Updater 私钥严禁入仓**
- **模式判定**用 `ha_core::runtime_role()` / `is_desktop()`，别给共享函数加 mode 参数

### 工具 & 审批

详见 [docs/architecture/](docs/architecture/)：permission-system/tool-system/sandbox/browser/background-jobs/media-generation/file-operations。

- 工具调用唯一入口 `permission::engine::resolve_async()`；Smart 不消费 `custom_approval_tools`，UI 须提示。
- strict 永不自动放行：超时/无人值守 `proceed` 强制 deny；判定源 `AskReason::forbids_allow_always`，`ApprovalReasonKind::is_strict()` 须镜像。
- 无人值守 fail-closed：`check_and_request_approval` 预检 `evaluate_approval_surface`，`permission.unattended_approval_action` 默认 deny；可能 surface 即 Attended，唯 cron（含其血缘 subagent）例外；判 ACP 用 `is_acp()` 非 `ChatSource`（复用 Http）。
- `control.raw_cdp` strict：每调用必审批、永无 Allow Always（规则/smart 均绕不过）；方法/域黑名单 + SSRF 扫描 + 硬开关 `browser.extension.allowRawCdp=false` 三道执行层防御勿削弱。
- 出站 HTTP 必走 `security::ssrf::check_url`，新入口严禁自写 IP 校验。
- 可见性与执行层兜底走 `dispatch::resolve_tool_fate`（`tools.allow/deny` 只覆盖非 Core）。
- 结构化副输出唯一通道：`ToolExecContext.metadata_sink`→`messages.tool_metadata`→工作台；新工具禁自开旁路。
- 后台单元唯一入口 `async_jobs::JobManager`，禁平行 API；命名分裂勿改：模块/log `async_jobs`、DB `background_jobs`、事件 `job:*`；审批 park 桥在 `tools::approval`（tools 零依赖 async_jobs）。
- 双域勿合并：tool 池 `async_jobs::slots`，后台 subagent 池 `subagent::queue`；资源类（槽满）入队非拒绝，结构类（depth/batch/turn）硬拒不排队；parked 持槽不释放（否则 resume 无空槽死锁）、预算 timer 排除 parked 时长；`approval_projection_watcher` 只补 label、绝不 gate 执行。
- 重试白名单代码级：`is_retry_eligible` 仅 `web_search`/`web_fetch`；新 async_capable 工具有副作用/计费就别加。
- `AsyncToolsConfig` 的 `0`：仅 `max_concurrent_jobs`/`_per_session` 真不限，其余 bounded-resource 旁钮钳到地板、绝非无限（`completion_merge_window_secs` 的 `0`=关，不在此列）。
- incognito：`output_tail` 永不注册；工作台聚合跳后端、只用 live tail。
- 图/音生成必走 `media_gen::execute_image`/`execute_audio`，禁各写 provider 循环；凭据只 owner UI 可写。
- 工作台聚合 dedup/排序 TS 与 Rust（`session::aggregate_session_artifacts`）两份须同步。
- 文件打开/下载/预览走 `useFileResource`；新可预览类型改 `src/lib/fileKind.ts` `isPreviewableKind`。
- preview-by-path：HTTP 三端点共用 `authorized_canonical_file_path`（tool 消息引用 ∪ 会话工作目录内），其余 403（远端严禁任意主机路径）；桌面信任本机。

### Memory

详见 [memory](docs/architecture/memory.md)；Dreaming（claim 层 / Deep resolver / Lucid Review / 确定性评测）见 [dreaming](docs/architecture/dreaming.md)。

- **预算唯一入口 `effective_memory_budget(agent, global)`**（Project > Agent > Global）：只约束 Core 静态注入与 V1 rollback legacy 段，`recall_memory` / `memory_get` 回原文。`CoreMemorySnapshot` 是会话固定 prefix，turn-dependent 内容（Recall / Profile / Awareness）走其后动态 block，**不得重拼 Core system string**；项目主题正文变化不得改稳定前缀（否则每轮废 prompt cache）
- **默认不静态注入**：仅完整 V1 rollback 或 `compatibility.legacyStaticMemory=true` 恢复 `## Pinned Memory` Context Pack——其 claim 进 prompt 前须 `sanitize_for_prompt`（**与动态召回信封是两条独立义务**）、与 Profile / legacy 共预算；legacy dedup 阈值须对齐注入阈值 `PINNED_MIN_SALIENCE`（`context_pack.rs` 单一来源），**dedup 永不比注入更激进**，否则中等 salience claim 两头落空、无 prompt 出口
- **自动召回默认关**（`memory.recall.enabled`，Deep Recall 独立默认关）：关闭时只自动用 Core，**工具面不得 gate 在此开关**（模型仍可按需调 Memory tools）。开启后**过期 / superseded / archived / needs_review 不回灌**。旧 per-agent `ActiveMemoryConfig` 仅一个 minor 兼容 / rollback，**不得迁成全局同意**
- **自动流程永不硬改用户记忆**：Deep Resolver 冲突只在高置信写 `needs_review`、**永不自动 supersede**；低置信 / 未知 relation / LLM 失败均 no-op
- **纠错唯一入口 `claims::review`**：**无 agent 工具面**，只对用户开放、模型不能自改；**改 content 必 `reembed_claim`**，否则下轮召回仍命中旧文本
- **注入即 untrusted**：召回文本套 `<untrusted_external_data>`，项目索引注入前 XML escape，claim / 图谱文本进 prompt 前 sanitize
- **fail closed**：全局 / agent memory off、incognito、非项目会话在 schema 与执行层双归零。`sessions.incognito` 是无痕单一真相源（不注入 Memory / Awareness、跳过自动提取、关闭即焚，**与 Project / IM Channel 互斥**，四旁路守卫见 [session](docs/architecture/session.md#四旁路守卫epic-e)）。项目记忆读写拒 symlink 与 canonical escape、变更持项目级 OS 独占锁、更新 / 删除须带上次 `read` 的 BLAKE3 `expectedFileHash`（陈旧写 fail closed）
- **确定性评测刻意不进默认 Cargo test**：`memory/dreaming/eval.rs` + `evals/suites/memory-dreaming/fixtures/` **无 LLM**，只由 `hope-agent-eval` 跑（进 cargo test 或加 LLM 判分即破坏确定性）
- **改这些须同步**：claim 读路径 / effective-status / hidden-set / scope 过滤 / evidence 授权 → 加 fixture + 提 suite version + 追加 `evals/version-lock.json` key（已有 `id@version` 不可覆写，CI 强制 append-only）；Deep Resolver 分组 / 基数 / 决策映射 → `auto_resolver_graph_planning` fixture；检索 SQL / RRF / trigram → 跑 `pnpm memory:benchmark`
- **Retrieval Planner**：`role=injected/selected` 是既成 prompt 事实，跨源只能 canonical-dedup / 裁剪 `candidate/considered`，**不得重排或丢弃已注入 ref**
- **新增 Goal / Workflow / Async / Agent 执行边界**须传播 `EvalRunContext` 身份并在终态关闭 guard；`evals/live/version-lock.json` 同样 append-only，manifest 禁 shell

### Subagent / Team / Cron

详见 [subagent](docs/architecture/subagent.md) / [agent-team](docs/architecture/agent-team.md) / [cron](docs/architecture/cron.md) / [background-jobs](docs/architecture/background-jobs.md)。

- **后台 subagent / Group 投影单向**：`subagent_runs` 为真相源，投影不持正文、不反写，排除 plan/team/hook 内部 spawn 与 incognito（durable 表，守关闭即焚）；同步只走 `SessionDB::update_subagent_status`，取消走 `subagent::request_cancel_run`（刻意不跑工具 job 的 hook/注入，勿并入统一取消）。`batch_spawn` 建 group 前预校验全部 task（否则漏交付），取消先标 group 终态再取消子 run
- `TeamTemplateMember.description` 注入子 session 身份段
- **Cron 投递白名单**：`delivery_targets` 须命中 `channel_conversations`——模型显式给的未命中目标创建期 `bail!`，投递期再查、未命中或 DB 不可用 fail-closed 跳过。白名单即边界（刻意不叠 SSRF）
- **Cron delete 审批**：`manage_cron action=delete` 唯一非 internal action，刻意抑制 AllowAlways——matcher 只按 `action` 不含 `id`，持久化即「删任意任务」常驻授权。owner 三入口走 `cron::delete_job_and_sessions`；新增审批原因同步 `ApprovalReasonKind` + `ApprovalDialog.tsx` union + 全语言文案
- **Cron owner-only 覆盖**：`permission_mode_override` / `sandbox_mode_override` 仅 owner 可设，`manage_cron` 恒 `None`、不进 schema、`update` 拒带覆盖的 job（否则注入可排 `permission=yolo` 提权）。沙箱 fail-closed：override 写失败即终止本次运行（写丢=裸跑 host），权限 override 写失败仅 warn（退回更严）——不对称刻意，勿拉平；预检读错回退 expected 而非 `Off`（防 `.unwrap_or(Off)`）；`ensure_sandbox_available()` 失败即终止、不回落宿主机
- **Cron 排程与时区**：`schedule::validate_schedule` 为合法性唯一裁决（owner/模型共用），非法 IANA 时区 `bail!`、禁止静默回退 UTC；`compute_next_cron` 用 `.find(|dt| *dt > *after)` 非裸 `.next()`（否则 DST 秋退写入过去时刻 → 每 tick 重触发）；时区 backfill 经 `cron_meta` sentinel `tz_backfill_done` 真·一次性（形似性能优化，删掉即把故意-UTC 任务静默改成宿主时区）；`update_job` 系统字段以 DB live 为准、不取 caller 快照
- **Cron Primary-only + slot-before-claim**：执行与 run-now 三入口前置 `is_primary()`（非 Primary 返错不假成功）；调度器先 `count_running()`（并发计数单一真相源，失败 fail-closed 跳过本 pass）抢槽再 claim——claim 会推进 `next_run_at`，反序即静默丢一轮
- **`at_grace_secs` 的 `0` 是 async_tools 规则的例外**：`0`=严格不补跑、只钳上限不钳地板，勿套用「bounded-resource 旁钮 `0` 一律钳地板、绝非无限」。`save_cron_config` 替换整个 `CronConfig`——新增字段须同步各 save 调用点，漏传即被 serde 默认静默重置
- `CronFailureClass` 只做诊断、刻意不改 `max_failures` 禁用策略（防误分类过早禁用）
- **`ChatSource::Cron`**：`kb_access_source` 映射 `KbAccessSource::Cron`（非 IM → owner KB）、incognito 归零；新增 variant 须同步 `stream_seq.rs` 语义方法 + `active_counts` 穷举 match + `kb_access_source` 映射
- Cron 终态语义（取消不误判 / 空输出不掩盖 / `At` 失败不重试 / infra 失败不计禁用 / 暂停不复活）互锁，改 `classify_cron_terminal` / `update_after_run` / `mark_missed_at_jobs` 前必读 cron.md；新增 run_log status 须同步 `dashboard/{insights,queries}.rs` 成功率口径 + 前端 `TaskSection` / `cronHelpers`
- **`schedule_wakeup` ≠ cron、不复用入口**：replay 仅 Primary（防双投）、incognito 仅内存、会话删经 `wakeup::purge_for_session` 取消

### LLM 主对话

详见 [provider-system](docs/architecture/provider-system.md) / [failover](docs/architecture/failover.md) / [side-query](docs/architecture/side-query.md) / [agent-config](docs/architecture/agent-config.md) / [automation-model](docs/architecture/automation-model.md)

- spawn tool loop 的 chat 走 `chat_engine::run_chat_engine`，禁止自包 `on_delta`
- Codex 不参与 failover profile 轮换（OAuth 无 profile，executor 按 `api_type` 强制关，caller 传 true 也无效）
- 视觉桥 `agent/vision_bridge.rs`：`function_models.vision` opt-in、未配=关（回退占位符、不自动挑选）。只改 `api_messages` 副本、绝不改 `conversation_history`（就地改=永久丢图）；只扫 user/tool、跳 assistant（改写毁 tool 调用）；转录套 `<untrusted_external_data>`、绝不作 system 指令；绝不在 side_query 触发（防递归）；incognito 走 per-turn 缓存、绝不写全局
- 后台一次性 LLM 调用走 `automation::run` / `run_vision` + `function_models.automation`，同类消费者勿另写形状。例外：Memory Extract 与 Compact 摘要刻意不接入（签名不支持链式循环；Compact 属 fail-fast 关键路径），只加 `model_override`，勿迁移

### Chat Engine & Streaming

详见 [chat-engine](docs/architecture/chat-engine.md)；未读口径见 [session](docs/architecture/session.md)。

- **未读单一来源**：普通未读计**会话数**，资格只走 `regular_session_scope_sql` / `regular_unread_exists_sql`，禁止分页求和；Regular / Cron / IM 三域互不清除，新专属对话空间须用独立 `SessionKind`
- **API-Round 分组**：新 Provider adapter 须经 `push_and_stamp` 标 `_oc_round`（否则压缩切割拆散 tool_use / tool_result 配对），请求体构建前统一 `prepare_messages_for_api()` 剥离元数据
- **前台 idle guard 单一入口**：`run_chat_engine` 按 `ChatSource::holds_foreground_idle_guard()` 统一建 `ChatSessionGuard`（ACP 自建），新增对话入口不得手搓 per-shell guard

### 上下文压缩

5 层渐进式 + `ContextEngine` / `CompactionProvider` 可插拔；阈值、TTL 节流、反应式微压缩、Tier 3 文件恢复详见 [context-compact](docs/architecture/context-compact.md)。

### Knowledge Base（知识空间）

详见 [knowledge-base](docs/architecture/knowledge-base.md)。

- **两类存储（D9）**：笔记 `.md` = 唯一真相源；注册表 + **访问绑定**落 `sessions.db`；`index.db` 仅可重建缓存，**权限绝不落其中**（重建即静默重置授权）
- **访问默认 deny**：唯一裁决 `effective_kb_access`（incognito / IM 未 opt-in 归零；subagent 按 origin 血缘不洗权限）；owner 平面不经 attach，agent 平面（`note_*`）必过
- **agent 侧唯一解析链**：`Agent::resolve_kb_access()`，prompt 段 / 被动召回 / 工具门控共用，**不得重写**；**只服务 schema/prompt/召回，绝不 gate 执行**（执行走 live `access_map`）。`is_kb_scoped_tool` / `ToolScope::Knowledge` 仅收窄 schema 可见性，**非安全边界**
- **写入三闸**：`WorkspaceScope::for_knowledge`（外部 root 只读、**桌面也拒**（刻意反「桌面不受限」通例），须 `allow_external_writes`；HTTP 再叠 `allow_remote_writes`；**后台维护永不写外部**）→ `platform::write_atomic`（**禁回退 `fs::write`**）→ `expected_file_hash` 比磁盘 raw BLAKE3（**非索引 `content_hash`**）
- **检索独立**：笔记 store **绝不折进 `recall_memory`**（`knowledge_recall` 两段不混排）；`knowledge_embedding` 与 `memory_embedding` 物理隔离、**不寄生不回退**；embedding / chunk 重 reindex 故 **GUI-only 不进 `ha-settings`**（设置三件套例外）
- **读取即 untrusted**：`[[note]]` 与 `knowledge_passive_recall` 套 `<untrusted_external_data>` 信封，**永不升为 system 指令**；incognito 零召回 / 零精灵
- **接线**：会话独立 `SessionKind::Knowledge`（主列表 / `/sessions` / 全局 FTS 隐藏，与 design 同谓词）；**新增 KB 工具须同步 `tools/note.rs` + `core_tools.rs`（schema）+ `execution.rs`（dispatch）**

### 设计空间（Design Space）

详见 [design-space](docs/architecture/design-space.md)。**新增 action / 端点：工具进 `tools/design/mod.rs`，Tauri / HTTP 薄壳只调 `design::service`，逻辑全在 ha-core**。

- **浏览器零编译**：iframe 只载后端编译落盘的静态产物（`component` 经 `design::compile`）；**禁 in-browser Babel / esbuild-wasm / Tailwind JIT**（旧版 `feat/atelier` 白屏卡顿根因）；编译失败降错误页，**不白屏 / 不 panic**。**刻意不做无限画布**（同一卡顿根因）
- **回写确定性**：磁盘即真相源，`design.db` 仅可重建注册表；微调回写单一命中 + `expected_hash` stale-write 守卫，写盘**一律** `platform::write_atomic`。**component 编译产物 ≠ 源码故无 oid 微调**，仅 `supports_oid_edit` kind（非 image/audio/component）可 `edit_element`
- **边界**：owner（`service.rs`，本机 / API key 信任，**刻意不经 access 检查**）与 agent `design` 工具两平面隔离；iframe 恒 `sandbox="allow-scripts"`；`ToolScope::Design` 仅收窄 schema、**非安全边界**；**incognito 零设计**（fail-closed）；`SessionKind::Design` **与 knowledge 同谓词从主侧栏 / `/sessions` / 全局 FTS 隐藏**，新增专属空间**必须**同步该谓词
- **小改必须就地精改**（实测曾抹空整页）：`get_artifact` → `edit_element(oid)`，**绝不整段 `update_artifact` 重造、绝不 web_fetch 读产物**

### Agent 控制平面 / 通用场景

详见 [docs/architecture/](docs/architecture/)：goal/workflow/loop/context-retrieval/domain-{workflow,quality,eval}。

- **控制面归位**：`/goal` 目标+完成标准 · `/mode` 强度 · `/workflow` 一次性可恢复可审批脚本 · `/task` 进度 · `/loop` 重复触发（复用 Cron，不另起 scheduler） · `/worktree` 隔离 coding。禁持续触发伪装 workflow、一次性脚本伪装 loop。
- **领域模块不扩权、不执行**：`domain_workflow_templates` 只述交付契约，不给连接器权限；`preview_domain_workflow` 只出 draft/preview，不建 run 不执行、不碰连接器、不发/改外部系统。Domain Learning 复用 Coding Improvement 同一 proposal queue（禁平行队列），preview → apply → 用户显式 promotion 才落生产，禁直改生产模板/connector 策略/eval fixture。
- **复核评测**：Domain Quality、`domain_eval_runs`、`evaluate_domain_quality_gate` 确定性只读：不调 LLM、不写状态、不碰连接器、不发送/发布、不自动学成正式规则。证据走 `domain_evidence_items` + Goal link，禁冒充 diff/validation/file evidence；与 `coding_eval_runs` 不混用，coding release gate 不代替 domain quality gate。
- **两处 fail closed**：只有 `requestedAction` 命中 approval gate 或 `highRiskAction=true` 才 `needs_user`（否则模板带 gate 即阻塞普通复核），缺确认阻塞 Goal；incognito 下 preview/evidence/quality/eval 拒绝或返空只读，不落 durable。

### Hooks

详见 [hooks](docs/architecture/hooks.md)（单一真相源，字段级对齐 Claude Code hooks 协议，`hooks_compat.rs` 硬验收）。

- **唯一入口 `HookDispatcher::dispatch` / `hooks::fire_*`**；调用方只读 `HookOutcome`，严禁 match handler 类型
- **新 user message 入口须过 `agent::preflight::user_prompt_preflight`**（`UserPromptSubmit` 阻断点）；新 hook 事件须埋点 + 测试 + 同步 `types.rs` 三处 match（`common`/`matcher_target`/`is_observation_only`）——**漏登记 `is_observation_only` 则新观察事件意外可阻断**
- **project/local scope 默认关**（`hooks_allow_project_scope`，供应链防护：开启即信任所有未来 cwd）；`ha-settings` 对 hooks 只读，可写 = 模型自装命令执行

### Plan Mode

详见 [plan-mode](docs/architecture/plan-mode.md)。

- **进入永远由用户拍板**：模型只能经 `enter_plan_mode` Yes/No 审批，**不能自己转 state**
- **plan = 设计契约（执行期不改），task = 唯一进度真相**
- **执行层兜底**：`resolve_tool_permission` 必须查 live plan state，防 mid-turn 进 plan 后剩余工具绕过

### Skill 系统

详见 [skill-system](docs/architecture/skill-system.md)（优先级/激活入口/`allowed-tools` gap/`skills::author` 原语）。

- **内置技能编译期嵌入二进制**（`skills/embedded.rs`）：禁止往构建产物单独拷 `skills/`
- **`@skill` 固定 allowlist**：非通用注入入口，单一来源 `skills::mention::AT_MENTIONABLE_SKILLS`
- **`skills::author` 写正文三路径（create/update/patch）全过 `security_scan`，命中即 bail 不降级**；自动**创建**默认落 draft 待用户确认，但 `promotion:"auto"` 直接写 Active；**`patch` 就地改已存在技能——目标 Active 时即刻对模型生效，不落 draft、不经确认**

### MCP 客户端

**配置读写**：读 `cached_config().mcp_servers`，写 `mutate_config(("mcp.<op>", source), …)`；网络 transport 与 OAuth 全路径出站过 SSRF 门，凭据 0600 落 `credentials/mcp/`。详见 [mcp](docs/architecture/mcp.md)。

### 平台 MCP 服务器（`hope-agent mcp`）

**红线**：共享 host `ha-core/src/mcp_server/`（`ToolProvider` 注册表），不做子系统专属 server；默认只读、`--allow-writes` 才注册写集且 host 层双保险再拦；**恒不暴露**写代码仓库 / deploy / share / delete / export 类工具；stdio interop 经 `acquire_or_secondary_for` 恒**被动 Secondary**，永不争 Primary。详见 [mcp-server](docs/architecture/mcp-server.md)。

### IM Channel

详见 [im-channel](docs/architecture/im-channel.md)。

- **审批一致性 + fail-closed（红线）**：所有决议路径（submit/超时/删会话/eviction）必须 emit `approval:resolved` 统一撤窗；按钮回调缺源即拒（**不复用 ask_user 的 `None→Ok`**）、文本回复 submit 前校验 session↔chat、chat 接管在 notify 门**前**拒决该 session 全部 pending；`auto_approve_tools`（opt-in）跳门时命中 strict 须 `app_warn('permission','auto_approve_bypass')`——**纯审计不拦截**
- **事件匹配用 `contains` 不用 `starts_with`（红线）**：`emit_tool_result` 的 `json!`+`BTreeMap` 键按**字母序**排（`call_id` 恒首位），锚 `{"type":...` 的 fast-path **永不触发**
- **`channel_conversations` 双向 1:1（红线）**：一 chat ↔ 一 session，接管即物理 detach 旧 attach + emit `channel:session_evicted`；读写一律走 [`channel/db.rs`](crates/ha-core/src/channel/db.rs) helper，**禁止直接写表**
- **注入回投须在同一 future 内 await finalize**：`inject_and_run_parent` 自驱动镜像（注入跑短命 runtime，`spawn(finalize)` 会被腰斩）；空闲门超时**不丢弃**，重排队进 `PENDING_INJECTIONS`
- **单一入口勿另起**：流式预览选路走 `select_stream_preview_transport`，新卡片风格靠 `ChannelPlugin` default=`Err` trait 方法扩展；auto-start 失败重试走 [`channel/start_watchdog.rs`](crates/ha-core/src/channel/start_watchdog.rs)（**user 操作永远胜过 watchdog**），勿自写退避

### 跨会话 / 全局

详见 [`docs/architecture/`](docs/architecture/)：session / ask-user / prompt-system / behavior-awareness / help-center

- 数据在 `~/.hope-agent/`，新路径走 `paths.rs`；日志走 `logging/mod.rs`，请求体必经 `redact_sensitive`
- 唯一结构化问答入口 `ask_user_question`：富输入 / 风格卡只能扩展它（答案仍走 `selected[]`），绝不 fork
- `sessions.working_dir` 三用：`# Working Directory` 段 + `exec` cwd + `read` 相对根，非纯 prompt 提示
- 手册单一来源 `docs/user-guide/`（rust-embed）：禁复制正文 / 拷进产物；中英同 PR 对齐（CI `check-docs-parity`）。例外：Dockerfile rust 阶段 `COPY docs/user-guide` 是编译期 embed 依赖，须保留
- markdown 路径链接仅桌面：`is_desktop()` 才注入 `MARKDOWN_PATH_LINKS_GUIDANCE`；其 `[名](绝对路径)` 格式与前端 `localPathFromHref()` 是同步契约；非桌面靠 `supportsLocalFileOps()` 关入口 + `/api/desktop/open-directory` 返 no-op（**不是**早返回禁用）。例外：anchor `title` 用原生 HTML 非 shadcn Tooltip（一条消息上百个）

### 项目（Project）容器

详见 [project](docs/architecture/project.md)。

- **已删勿引入**：`project_files`/`ProjectFile`/`project_read_file`（项目文件=工作目录真实文件）、`Project.bound_channel`（IM 无反向认领，归属靠 chat 内 `/project <id>`）
- **交互入口懒创建**：进项目「新建对话」不得 `create_session_cmd` 预建，首条消息经 `chat` 的 `projectId` 落库；`project_id` 与 `incognito` 互斥（**后端强制 incognito off**）；IM/cron/subagent 仍 eager
- **两个唯一入口**：工作目录 `session::effective_session_working_dir`（session > project > 默认 workspace）；文件读写 `filesystem::WorkspaceScope`（失败闭合，`for_path` 只读，HTTP 写受 `filesystem.allow_remote_writes` 默认 false）
- **删除级联**：`rm -rf projects/{id}/` **绝不波及用户显式选的外部 working_dir**；跨 db 项目记忆单独删、启动期 reconciler 兜底

### Agent 解析链（默认 Agent）

详见 [agent-config](docs/architecture/agent-config.md)。

- **7 级链唯一入口 [`agent/resolver.rs::resolve_default_agent_id_full`](crates/ha-core/src/agent/resolver.rs)**：顺序固定、首个非空胜出；channel worker 与新会话入口不得自写解析链
- **禁止裸字面量 agent id / 重新引入 `"default"`**：走 [`agent_loader::DEFAULT_AGENT_ID`](crates/ha-core/src/agent_loader.rs)（当前值 `"ha-main"`；前端走 `@/types/tools` 同名常量 + `isMainAgent`）
- **启动序**：`init_runtime`（含 legacy `"default"`→`"ha-main"` 一次性迁移）**必须**早于 `ensure_default_agent()`，否则预创空 `agents/ha-main/` 模板会吞掉 rename（迁移整体放弃且静默）

### 自升级

详见 [self-update](docs/architecture/self-update.md)。红线：

- **下载产物必须验签**：更新下载走 `updater::download::download_to`，落地 / swap 前必过 Minisign `signature::verify_bytes`
- **pubkey 两处必须相等**：`updater::keys::MINISIGN_PUBKEY_BASE64` ↔ `tauri.conf.json#plugins.updater.pubkey`（启动 panic / CI / pre-push 三重校验）
- **换 binary 只走 `platform::atomic_replace_binary`**（禁 `fs::write` 覆盖运行中 binary）；swap 后冷烟自检失败自动回滚
- **安装 / 重启必经用户确认**：`auto_update` 后台只检查 + 预下载 staging，`app_update` 的 `install` / `rollback` 弹 `ask_user_question`，**桌面绝不无条件 relaunch**
- **ha-core 不依赖 tauri-plugin-updater**，桌面路径经 `updater::UpdaterBridge` 反向注册

### Dashboard / Recap / Learning

详见 [dashboard](docs/architecture/dashboard.md) / [recap](docs/architecture/recap.md)。

- **用量总账（红线）**：新增任何触发模型推理 / embedding / STT / judge / `web_search` / 生图生音 / `provider_test` / vision 的入口必须经 [`model_usage.rs`](crates/ha-core/src/model_usage.rs) 入账（无痕不记，**cron / subagent / 后台维护照记**），**禁止字符估算冒充 token**；新增 `KIND_*` 须同步 `DashboardFilter.USAGE_KIND_VALUES` + `dashboard.usageKind.*` 全部语言
- **大盘只读、不伪造因果（红线）**：`dashboard/control_plane.rs` 是 Goal / Workflow / Loop / Task / Plan 聚合唯一入口；无可靠外键前禁止按 session 拼因果漏斗，零分母返 `null`，Goal / Workflow / Loop / Task / attention 排除 incognito / Cron / 子会话

### 本地 LLM 助手

详见 [local-model-loading](docs/architecture/local-model-loading.md)。

- 后端锁 Ollama（OpenAI 兼容端点），**App 不接管其进程**；模型目录与硬件预算算法见 `local_llm/types.rs::model_catalog` / `RECOMMENDATION_BUDGET_PERCENT`
- **Provider 写入 contract**：Provider 列表与 `active_model` 一切写入走 [`provider/crud.rs`](crates/ha-core/src/provider/crud.rs) helper（本地安装走 `upsert_known_local_provider_model`），**禁止 `providers.push` / `retain` / 手写 `active_model`**
- **本地后端判定消费 catalog**（[`provider/local.rs`](crates/ha-core/src/provider/local.rs)），**禁止硬编码 regex**

## 项目结构

六 crate workspace：`ha-core`（核心业务，**零 Tauri 依赖**）/ `ha-server`（axum HTTP·WS）/ `ha-browser-host`（浏览器辅助进程）/ `ha-eval-spec`（评测协议，**不依赖 ha-core**）/ `ha-eval`（评测 CLI）＋ `src-tauri/`（桌面薄壳），`src/` 前端，`skills/` 内置技能，`evals/` 评测资产。

## 开发命令

```bash
pnpm tauri dev                        # 开发（改 ha-browser-host 后先 pnpm dev:browser-host）
node scripts/sync-i18n.mjs --check    # 翻译缺失（--apply 补齐）
cargo run -p ha-eval --locked -- validate   # 评测资产校验
```

其余脚本读 `package.json` scripts；CLI / Docker / 评测子命令见 [cli](docs/architecture/cli.md) / [docker](docs/deployment/docker.md) / [capability-eval](docs/architecture/capability-eval.md)。

## 文档维护

索引 [`docs/README.md`](docs/README.md)。**AGENTS.md 只放跨 PR 红线与入口**，细节下沉 `docs/architecture/`。

同 PR 同步：功能/命令/模块增删 → `CHANGELOG.md` + `AGENTS.md`；技术栈/架构/规范/契约 → AGENTS.md；子系统边界/数据流/持久化/跨模块 contract → architecture 文档，新增架构级能力新建文档 + 登记索引；Tauri 命令/HTTP 路由/`COMMAND_MAP` 增删 → `docs/architecture/api-reference.md`；子系统/架构文档/运行时 DB/稳定 log `category` 增删 → `skills/ha-self-diagnosis/references/diagnostic-playbook.md`；README/release notes 任一语言 → 同步 .en.md。

**CHANGELOG 单行**：用户视角一句 + `(#PR)`，不写实现；契约/红线可加一行用户影响。

**规划归档**：调研/roadmap 归外部 iCloud `HopeAI/Hope Agent/Plans/`，**仓库内任何路径不留已完成 roadmap**；落地后须把设计决策同步回架构文档。

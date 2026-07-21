# Dreaming 子系统架构

## 概述

Dreaming 是 Hope Agent 的**离线记忆固化**子系统：在应用空闲、定时或手动触发时，于聊天热路径之外把对话痕迹整理成可审计、可追溯、可纠正的**长期心智**。它分两代能力共存：

- **Light（一代，固化）**：扫描近期 `memories`，用一次 LLM `side_query` 评分提名，把高价值条目 pin 起来并写一篇「Dream Diary」叙事。
- **结构化 claim 层（下一代）**：把记忆从平铺的 `memories` 列表升级为结构化 `memory_claims`（主谓宾三元组 + 证据 + 作用域 + 生命周期），叠加确定性过期 / 冲突 resolver、Memory Profile 合成、Context Pack 注入、以及面向用户的 **Lucid Review** 纠错闭环。

核心设计取向：**离线、可审计、用户掌控、不污染热路径**。所有 consolidation 异步进行，聊天侧只消费预先生成的注入素材；每个系统决策落 `dreaming_decisions` 审计行；用户随时可 approve / edit / reject / forget 任何 claim；无痕会话「关闭即焚」，永不进入任何长期存储。

> 本文是 Dreaming 子系统的单一真相源。记忆召回引擎、Embedding 配置、自动提取等记忆系统通用机制见 [`memory.md`](memory.md)；本文只覆盖 Dreaming 独有的固化 / claim / 注入 / 纠错 / 评测链路。Tauri 命令 ↔ HTTP 路由的完整对照见 [`api-reference.md`](api-reference.md)。

### 设计目标

- 把「记得一堆句子」升级为「维护一组带来源、带时效、带置信度的结构化事实」。
- 每条 active claim 都能追溯到至少一条证据；每个系统动作都有审计记录。
- 时效与冲突可治理：过期自动抑制、冲突进待审而非自动覆盖。
- 控制权归用户：整理结果可被用户纠正，纠正即权威（最高置信度）。

### 非目标（红线）

| 非目标 | 原因 |
|---|---|
| 自动硬删用户记忆 | 删除须显式确认；自动流程只做 `superseded` / `expired` / `archived` / `needs_review` 标记 |
| 阻塞聊天热路径 | 所有 consolidation 异步；聊天只消费预生成的 Context Pack |
| 一次性重写 memory backend | 旧 `memories` 表继续可用，claim 层以双轨增量引入 |
| 无痕会话进入长期记忆 | 「关闭即焚」：不入候选、不入证据、不入 profile、不入统计 |
| 模型自改 `MEMORY.md` / 自改自己的 claim | `MEMORY.md` 是用户 / Core Memory；claim 纠错是纯 owner 平面，无 agent 工具面 |
| evidence 原文直接成为指令 | 证据默认不进 system prompt；文件 / 工具输出须经提取 + sanitize 才能成为 claim |
| 跨 scope 污染 | Project A 的事实不进 Project B；Agent 间独立 |

## 数据模型

所有表与旧 `memories` 同住 `memory.db`（同一 SQLite 文件、同一事务边界，保证 claim + evidence 原子写入）。时间戳统一用**固定宽度 RFC3339 毫秒 + `Z`**（`crate::util::now_rfc3339`），使 `valid_until < now` 之类的字符串比较在词序上单调。

分三组：**Claim 三表**（事实 + 证据 + 双轨同步）、**运行与审计表**（dreaming 运行协调 + 决策日志）、**Profile 快照表**。

### Claim 三表

#### `memory_claims`（结构化事实，schema 见 `sqlite/backend.rs`）

| 列 | 类型 | 含义 |
|---|---|---|
| `id` | TEXT PK | UUID |
| `scope_type` / `scope_id` | TEXT | `global`（id 为 NULL）/ `agent` / `project` |
| `claim_type` | TEXT | `user_profile` / `preference` / `project_fact` / `standing_rule` / `reference` / `task_pattern` |
| `subject` / `predicate` / `object` | TEXT | 结构化三元组（如 `user` / `prefers` / `Chinese`）|
| `content` | TEXT | 人类可读陈述（保留来源语言）|
| `tags_json` | TEXT | JSON 数组 |
| `confidence` | REAL | [0,1]，由 `evidence_class` baseline 推导（默认 0.5）|
| `confidence_source` | TEXT | `derived` / `llm_adjusted` / `user_confirmed` |
| `salience` | REAL | [0,1] 长期价值度，决定注入优先级（默认 0.5）|
| `status` | TEXT | `active` / `superseded` / `expired` / `archived` / `needs_review` |
| `valid_from` / `valid_until` | TEXT | 可选生效区间（RFC3339）；**过期判定核心**见下 |
| `supersedes_claim_id` | TEXT | 指向被本 claim 替代的旧 claim |
| `source_run_id` | TEXT | 生成 / 更新本 claim 的 `dreaming_runs.id` |
| `embedding_signature` | TEXT | 向量签名；为 NULL 时该行从 vec0 KNN 脱落、降级 FTS-only |
| `created_at` / `updated_at` | TEXT | RFC3339 |

索引：`idx_memory_claims_scope(scope_type,scope_id)` / `_status` / `_type` / `_spo(subject,predicate)` / `_updated(updated_at DESC)`。

**effective-status（关键不变量）**：持久化 `status` 恒为底值；读取时由 `claims::write::effective_status(status, valid_until, now)` 派生——`status='active'` 且 `valid_until` 非空且 `valid_until < now`（词序比较）则视为 `expired`，不改存储行。**所有读路径**（list / search / pinned / Context Pack / V2 Fast Recall / legacy Active Memory / linked legacy 隐藏）统一走它，过期 claim 永不进 prompt。`is_injectable_status` 仅 `active` 为真。

**FTS5 / 向量旁车**：`memory_claims_fts(content, subject, object)` 外部内容索引，由触发器自动同步、首次从既有行 rebuild；`memory_claims_vec` 是 vec0 虚表，仅在配置 embedding 时懒创建，缺失容错。检索复用混合引擎（FTS5 + vec0 + RRF），但**独立存储**，与记忆主表互不串扰。

#### `memory_evidence`（来源证据）

每条 active claim ≥ 1 条证据（红线：active claim 必可追溯）。`FK(claim_id) → memory_claims ON DELETE CASCADE`，索引 `idx_memory_evidence_claim(claim_id)`。

- `evidence_class`（**闭包 6 值**，决定 confidence baseline，写入时未知值规整为 `assistant_inferred`）：

  | evidence_class | baseline |
  |---|---|
  | `manual_correction` | 1.00 |
  | `user_confirmed` | 0.95 |
  | `explicit_user_statement` | 0.85 |
  | `project_artifact_fact` | 0.75 |
  | `assistant_inferred` | 0.45（默认）|
  | `behavioral_pattern` | 0.35 |

  baseline 由 `claims::write::confidence_baseline` 单一来源给出（有确定性单测）；LLM 只输出标签、不直接定数值。
- `source_type`：`session_message` / `memory` / `file` / `tool_result` / `url` / `recap_facet` / `manual`，配 `source_id` / `session_id` / `message_id` / `file_path` / `url` 锚点。
- `quote`：短摘录（`QUOTE_MAX_CHARS=400` 码点、`logging::redact_sensitive` 脱敏）；`redaction_status ∈ redacted | raw_allowed | anchor_only`（`raw_allowed` 仅限用户自述）。`weight` 默认 1.0（用户纠错证据恒 1.0、最高优先级）。
- **证据不等于 prompt**：system prompt 默认只注入 claim 的 `content`，**不注入 evidence quote**；展开 quote 必经后端授权（见安全章）。

#### `memory_claim_links`（claim ↔ 旧 memory 双轨同步）

`PK(claim_id, memory_id)`，双 FK 级联，索引 `idx_memory_claim_links_memory(memory_id)`。`sync_mode` 三态决定旧记忆的注入是否受 claim 状态牵动：

- `managed`：claim 失活（superseded/expired/archived）时，关联的旧 memory 停止注入（由读时 hidden-set 覆盖）。
- `user_pinned`：用户手动 pin，**不**自动 unpin；claim 状态变化只生成 `needs_review`。
- `detached`：claim 状态永不影响该 memory（backfill 默认用它，保证「不改变现有注入」）。

### 运行与审计表

> 这组表把 Light 一代的进程内 `LAST_REPORT` 快照升级为**可跨进程协调、重启存活、可审计**的持久层。

- **`dreaming_runs`**：每轮运行元数据。`trigger`（`idle` / `cron` / `manual` / `user_correction`）、`phase`（`light` / `deep` / `profile` / `user`）、`status`（`running` / `completed` / `failed` / `skipped`）、`owner_instance_id` + `heartbeat_at` + `lease_expires_at`（跨进程租约）、计数（scanned / nominated / promoted / decision / duration_ms）、`diary_path`。索引 `_started(started_at DESC)` / `_status`。
- **`dreaming_locks`**：跨进程租约。`lock_key`（形如 `light:global`）单行持锁，`lease_expires_at < now` 视为可抢占。进程内另有 `AtomicBool DREAMING_RUNNING` 防同进程重入；二者叠加形成串行保护。
- **`dreaming_decisions`**：机器可读决策流，`FK(run_id) → dreaming_runs ON DELETE CASCADE`，索引 `_run(run_id)`。`decision_type` 按来源分族：Light 写 `promote`（target=memory）；Deep resolver 写 `expire` / `merge` / `needs_review`（target=claim）；Lucid Review 写用户纠错族（`approve` / `reject` / `edit` / `move_scope` / `pin` / `unpin` / `flag` / `forget` / `forget_permanent`，target=claim）。`before_json` / `after_json` 存状态快照供 diff 展示。
- **`dreaming_watermarks`**（`PK(scope_key, source_type)`）/ **`dreaming_pending_sources`**：扫描水位与高频源捕获队列，作为跨进程协调基础设施持久化；当前 Light 扫描走固定时间窗口，水位驱动的增量扫描为预留能力。

### `memory_profile_snapshots`（Profile 快照）

每个作用域一份可注入的 Markdown 档案摘要，按 `version` 单调分层（`UNIQUE(scope_type, scope_id, version)`，`scope_id` 用 `''` 而非 NULL 以避免 UNIQUE 把多个 NULL 视为不同值）。`source_run_id` 追溯生成运行。新版本优先注入；无快照或 Profile 合成未启用时，回退到 legacy `profile`-tagged 记忆渲染（避免「## User Profile」空白）。

### ER 概览

```mermaid
erDiagram
    dreaming_runs ||--o{ dreaming_decisions : "1:N"
    dreaming_runs ||--o{ dreaming_locks : "1:N"
    dreaming_runs ||--|| memory_profile_snapshots : "1:1 source_run_id"
    memory_claims ||--o{ memory_evidence : "1:N ON DELETE CASCADE"
    memory_claims ||--o{ memory_claim_links : "1:N"
    memory_claim_links }o--|| memories : "N:1 legacy 双轨"
```

## Pipeline

### 触发与并发保护

| 触发 | 时机 | 默认 |
|---|---|---|
| **Idle** | 应用空闲达阈值（Guardian 心跳 60s 检测，Primary-only） | `idleTrigger.enabled=true`，`idleMinutes=30` |
| **Cron** | 6 字段 cron 表达式（监听 `config:changed` 重排） | `cronTrigger.enabled=false`，`cronExpr="0 0 3 * * *"` |
| **Manual** | Dashboard「Run now」/ owner 命令 | `manualEnabled=true` |

idle / cron 自动周期依次跑 **Light 固化 → 保守自动 Deep sweep → Profile 合成**（后两者各受 `deepResolver.*` / `profileSynthesis.enabled` 门控）。自动 Deep sweep 默认执行确定性过期和最多 8 组 graph-first LLM 分类，但它没有“选一个事实覆盖另一个”的权限：高置信冲突只进待审，近重复只有在高置信且图谱 alias 或词法相似度再次佐证时才合并，任何低置信 / 未知 / 失败都 no-op。**`autoSupersede` 不是配置项**——`DeepResolverConfig` 里根本没有这个字段，它只是 `ResolverPreflightReport` 与 auto sweep 审计 `scope_json` 里硬写的常量 `false`，用来对外声明「自动流程无覆盖权限」。完整人工 Deep resolver 仍经 `dreaming_run_resolver` 触发。Dashboard 在触发前可通过 owner-only `dreaming_resolver_preflight` / `GET /api/dreaming/resolver/preflight` 做只读预检：统计 active claim、确定性过期候选、冲突候选组、自动阈值、LLM 分组调用上限与配置阻塞原因；预检不调用 LLM、不写 claim、不创建 run。Memory Health 也输出同一组 Deep Resolver backlog 指标，供复制诊断和支持排障；Settings → Memory → Overview 的 Health 卡片必须显示 backlog / blocked / clear 状态，但 backlog 只作为 `info` issue，不改变 health status。手动 resolver 与自动 sweep 都必须在长期记忆总开关关闭时 fail-closed skip，不能继续写 claim 状态。

两道串行锁：进程内 `AtomicBool DREAMING_RUNNING`（`try_claim` 失败即 skip）+ 跨进程 `dreaming_locks` 租约（被他进程持有则 skip，高频源可入 `dreaming_pending_sources` 队列）。Primary 启动时 `recover_stale_*` 把过期 `running` 行标 `failed`、删过期锁、回收超期 `claimed` 源；每日 retention 复跑并 GC。

### 三类可运行周期

Dreaming 以三个独立可运行的 cycle 落地（均写 durable run + decision）：

1. **Light 固化**（`pipeline.rs`）：
   - **Scanner**（`scanner.rs`，`spawn_blocking`）：取近 `scopeDays` 天未 pin 的 `memories`（超取后客户端过滤到 `candidateLimit`）；为每条挂证据指针，**incognito 会话 fail-closed 剔除**。
   - **Narrative**（`narrative.rs`，经 `crate::automation::run` 一次性 LLM 调用）：渲染候选 → 要求 JSON 信封 `{promotions:[{id,score,title,rationale}], diary}`；超时 `narrativeTimeoutSecs`；按 `promotion.minScore`（0.75）/ `maxPromote`（5）过滤排序。模型链解析（`resolve_dreaming_chain`，Deep Resolver 复用同一函数）：`modelOverride`（`ModelChain`）→ deprecated `narrativeModel`（`provider:model`，惰性解析）→ `function_models.automation` 全局默认链 → 聊天全局模型；真跨模型降级，详见 [模型 vs Agent 统一配置](automation-model.md)。
   - **Promotion**（`promotion.rs`，`spawn_blocking`）：对存活且未 pin 的记忆 `toggle_pin(true)`。
   - **Diary + Finalize**：写 `~/.hope-agent/memory/dreams/*.md`；`finish_run` + 每条 promotion 写 `promote` 决策。
2. **Deep Resolver**（`resolver.rs`，`dreaming_run_resolver`）：
   - **Preflight**（`resolver_preflight`）：只读读取 active claims，输出 `ResolverPreflightReport`（`canRunManual`、`expiredCandidateCount`、`conflictGroupCount`、`groupsToAnalyze`、`blockingReasons` 等）；不调用 LLM、不改状态、不落审计 run，供 Dashboard 禁用按钮和解释“这次运行会做什么”。
   - **确定性过期**：扫所有 active claim，`valid_until < now` → `Expire` 决策（纯字符串比较，无 LLM）。
   - **统一分组**：按 `(scope_type, scope_id, claim_type, subject, predicate)` 分组，只取 >1 成员且 ≥2 种不同归一化 object 的组；过期候选在分类前剔除。claim 内容、图谱邻边和 LLM rationale 都按 untrusted data 处理，进 prompt 前 sanitize，落审计前脱敏并限长。
   - **自动 graph-first sweep**：已知多值谓词（`uses` / `likes` / `works_on` 等）直接 graph-noop，避免把合法并存事实误判为冲突；其余组携带 alias 连通、对象 degree、邻边、证据数量 / 人工证据 / 最高权重和有效期信号，最多分析 `autoResolveMaxGroups`（默认 8，钳 `[1,20]`）。只有 `confidence >= autoResolveMinConfidence`（默认 0.92）才可写状态：冲突仅 `NeedsReview`；duplicates 还须 alias 连通或词法相似度 ≥ `autoMergeSimilarity`（默认 0.84）才 `Merge`；永不产生 `Supersede`。每组调用经统一 `automation` 模型链，usage operation 为 `dreaming.resolver.auto`，便于单独观察后台治理 token 成本。
   - **手动完整分析**：每轮最多 `MAX_RESOLVER_GROUPS=50` 组，各经 `classify_group` 发一次 `automation::run`（与自动 sweep 同一条 `automation` 模型链，非 `side_query` 模块）。LLM 回 `duplicates → Merge`（保留最高置信 + 最新者，存档另一方）/ `conflict → NeedsReview` / `independent → no_op`，同样**绝不自动 supersede**。usage operation 为 `dreaming.resolver.manual`；自动与手动运行都持跨进程 Deep lease，并把 graph-noop / LLM-noop / 截断 / 失败写入 durable run note 与事件。
3. **Profile 合成**（`profile.rs`，`dreaming_run_profile`，受 `profileSynthesis.enabled` 门控、默认开）：按 scope 取 active claim、按 `confidence × salience` 排序取前 `maxLinesPerScope`（12，排除 `reference` 类）、规则式渲染 Markdown bullet。Idle/Cron 走规则式零 LLM；Manual 额外对每 scope 发一次 `side_query` 重写求流畅（只压缩重组、不创作）。写入 `memory_profile_snapshots`（version=MAX+1）。

```mermaid
graph TD
    T["触发<br/>idle / cron / manual"] --> A1{"进程内锁<br/>DREAMING_RUNNING"}
    A1 -->|占用| SK[skip]
    A1 -->|获得| A2{"跨进程租约<br/>dreaming_locks"}
    A2 -->|被持有| PQ["enqueue pending + skip"]
    A2 -->|获得| R["create_run (running)"]
    R --> L["Light: scan → narrative → promote → diary"]
    L --> F["finish_run + promote 决策"]
    F --> AD{"deepResolver<br/>auto enabled?"}
    AD -->|是| DS["Auto Deep: expire + graph-first bounded LLM"]
    AD -->|否| P{"profileSynthesis<br/>.enabled?"}
    DS --> P
    P -->|是| PF["Profile 合成 → snapshot"]
    R -.手动.-> D["Manual Deep: expire + 完整冲突 needs_review/merge"]
```

### Deep Resolver 自动裁决红线

自动 sweep 与手动 resolver 共用同一套纯函数管线（分组 → 图谱信号 → LLM 裁决 → 映射 → apply）。下列不变量是安全边界而非调优项，改动时须同步 [确定性评测](#确定性评测golden-fixtures) 的 `auto_resolver_graph_planning` fixture：

1. **永不自动 supersede（结构性保证）**：`ResolverDecisionType` 只有 `Expire` / `Merge` / `NeedsReview` 三个 variant，**没有 `Supersede`**——「用一个事实覆盖另一个」在类型层就不可表达。`map_verdict_to_decisions` 对 `conflict` 一律给全组成员 `NeedsReview`；`Merge` 也不改写幸存者，`claims::merge_claims` 只把 drop 方标 `archived`（active-gated，keep 与 drop 都必须仍是 `active`，否则整笔 no-op 且不动 evidence），随后把它的 evidence 改挂到 keep 方。auto sweep 的审计 `scope_json` 与 run note 都硬写 `autoSupersede: false` / `auto_supersede=false`。
2. **已知多值谓词先 graph-noop**：`plan_auto_resolution_groups` 逐组算 `graph_group_signals`，`predicate_cardinality == MultiValued` 的组直接进 `graph_noop_group_ids`、**不发 LLM**，避免把合法并存的事实误判成冲突。基数判定 `predicate_cardinality` 先过 `normalize_predicate`（小写 + 非 ASCII 字母数字折成 `_` + 压缩分段）再查表：`MULTI_VALUED_PREDICATES`（`uses` / `likes` / `works_on` 等）允许精确、前缀、后缀三种命中（故 `uses_package_manager` 判 MultiValued），`SINGLE_VALUED_PREDICATES`（`timezone` / `preferred_theme` / `email` 等）只允许精确或后缀命中；两表都不中则 `Unknown`。**`Unknown` 仍进 LLM 组**——graph-noop 只对确知多值的谓词生效，不是「不认识就跳过」。
3. **自动冲突只写 `needs_review`**：`map_auto_verdict_to_decisions` 先以 `verdict.confidence < cfg.auto_min_confidence()`（`auto_resolve_min_confidence`，默认 0.92、读时钳 `[0.75,0.99]`、非有限值回落默认）整组 no-op；通过后 `conflict` 复用 `map_verdict_to_decisions` 得到全员 `NeedsReview`，只在 rationale 前缀补 `auto resolver confidence=…`。
4. **自动 near-duplicate merge 须二次佐证**：`duplicates` 除高置信外还要 `cfg.auto_merge_near_duplicates` 为真，且 `auto_duplicate_is_corroborated` 通过——要么 `signals.alias_connected`（组内全部归一化 object 经 `ALIAS_PREDICATES`（`alias_of` / `same_as` / `equivalent_to` / `aka`）的同 scope 边彼此连通），要么**每一条** drop→keep 的 `content` 或 `object` 词法相似度（`lexical_similarity`，token 集 Jaccard）≥ `cfg.auto_merge_similarity_threshold()`（`auto_merge_similarity`，默认 0.84、读时钳 `[0.70,0.98]`）。任一条不满足即整组返回空 = no-op，**绝不「大部分像就合」**。
5. **低置信 / 未知 relation / LLM 失败均 no-op**：`parse_verdict` 只接受 `duplicates` / `conflict` / `independent` 三种 relation，其余（含 JSON 提取或反序列化失败）返 `None`，`confidence` 非有限值丢弃、其余钳 `[0,1]`（缺失按 0.0 参与阈值判定，故必然低置信 no-op）；`classify_group` 的 `automation::run` 报错同样返 `None`。`None` 与 `independent` 都不产生任何 decision，只累加 `llm_noop_groups` / `llm_failed` 写进 run note；`resolve_dreaming_chain` 解析不出模型链时整轮不发 LLM、标 `llm_failed`。仅当 `llm_failed` 且 applied 总数为 0 时 run 才落 `failed`。
6. **有界 + untrusted**：LLM 组数在 `plan_auto_resolution_groups` 内按 `group_cap.clamp(1,20)` 截断并置 `truncated`（**graph-noop 组不占额度、不被截断**）；手动路径另受 `MAX_RESOLVER_GROUPS=50` 约束。**进 prompt 的两类文本都是 untrusted**：claim 行由 `render_group` 对 `object` / `content` 逐条 `sanitize_for_prompt`，图谱邻边由 `graph_group_signals` 同样逐条 sanitize 且 `neighboring_edges` 上限 12 条；落库 rationale 经 `bounded_rationale`（`redact_sensitive` + 折叠空白 + 截 512 码点）。

## Claim 写路径与 Backfill

### 双写 + canonicalize（`claims::write` / `claims::store`）

自动提取（[`memory_extract.rs`](../../crates/ha-core/src/memory_extract.rs)，受 `extractClaims` 开关控制、**默认开**——claim 与 facts 同一次 side_query 抽取，无额外 LLM 调用）在写旧 `MemoryEntry` 的同时，把 `ClaimCandidate` 经 `write_candidate` 双写：

- **作用域固定为提取上下文的 `default_scope`**（会话 / 提取 API 参数），**不信任 LLM 的 scope hint**（防跨项目路由）。
- **规则式去重**：粗筛 `(scope_type, scope_id, claim_type, subject, predicate)` + Rust 侧 `normalize_object`（折叠空白 + 小写）精确比对；命中 active claim 则合并证据、更新 `updated_at`，否则建新 claim + ≥1 证据。
- `confidence` 由 `evidence_class` baseline 推导；`valid_until` 经 `normalize_valid_until` 规整（完整 RFC3339 / 裸日期 / 带时区都转 UTC+Z；无法解析 → `None`，绝不静默过期）。
- **重嵌时序**：内容变更后 `apply_claim_fields` 置 `embedding_signature=NULL`，随后 `reembed_claim`（须先 drop 写连接——`embed_and_index_claim` 内部再取写锁，writer Mutex 不可重入）。signature 清空使旧向量在自愈前从 KNN 脱落、降级 FTS-only（严格优于返回语义过时匹配）。

### Backfill（旧 memory → claim，`claims::backfill`）

把存量 `memories` 确定性映射为 claim（规则式、无 LLM），在不改变当前注入的前提下让老用户进入 claim 世界：

- 规则映射（`MemoryType → claim_type / subject / predicate / evidence_class`）。
- **低风险自动激活**：仅 pinned 的 `User` / `Feedback` → `active`，其余 → `needs_review`。
- **链接恒 `detached`**：claim 状态永不牵动旧 memory 注入（红线：backfill 不改现有 prompt）。
- `plan_backfill` 干跑（精确计数 + ≤200 行预览）/ `apply_backfill` 实跑（**重新扫描、不信任 plan 列表**，memory 已消失 / 已 link → skip，幂等）。

## 读 API 与注入

### 读 API（`claims::store`）

`list_claims`（scope / status / claim_type 过滤，limit 钳 [1,500]，按 `updated_at DESC`）、`search_claims`（FTS5 + 可选 vec0，RRF 融合，返 effective-active + scope 过滤）、`list_pinned_claims`（effective-active 且 `salience >= min_salience`，按 salience/confidence/updated 排序）、`get_claim`（claim + evidence + links）。状态过滤与 effective-status 对齐：`active` 过滤并入 `valid_until >= now`，`expired` 过滤匹配 `status='expired' OR (active AND valid_until<now)`。

### Prompt / 召回接入（`dreaming::context_pack` / `sqlite::prompt` / `agent::active_memory` / `system_prompt::build`）

Memory UX v2 默认只把会话固定的 Global / Agent / Project `CoreMemorySnapshot` 放入稳定 system prefix；Dreaming claim、Profile Snapshot 与 legacy SQLite memory 都不默认常驻。它们分成一条默认动态路径和一条显式兼容静态路径：

1. **Relevant Claims（V2 默认动态路径）**：query-dependent，**不进静态 prefix**（否则每轮作废 prompt cache）。自动动态召回默认关闭；用户显式开启 `memory.recall.enabled` 且 `memory.recall.includeClaims=true`（后者默认 true）后，V2 Fast Recall 才按 Project→Agent→Global 搜索 effective-active claims，执行实时 scope / status 过滤，并与 legacy memory、Profile、Procedure、Graph 候选交给统一 Retrieval Planner 做确定性融合和 token 裁剪；入选内容只进入本轮动态 Memory Context suffix。可选 Deep Recall 仅对已有 Fast shortlist 做 bounded rerank / distill。`shortlist_claim_candidates` 是 V2 与 legacy 共用的底层候选读取函数；`ActiveMemoryConfig.include_claims` 只控制旧 per-agent 兼容 / V1 rollback 链，完整回滚时才恢复由 LLM 选择单句 `## Active Memory` 的旧产品机制。
2. **Profile（V2 默认动态路径）**：Profile 仍由 Dreaming 离线合成和持久化，但只有自动召回已明确开启、当前 query 命中 profile intent 且候选通过实时资格与预算裁剪时，才作为动态候选参与本轮回答；快照变化不得改写当前会话的稳定 Core prefix。
3. **Pinned Claims / Profile（legacy 静态兼容路径）**：只有完整 V1 rollback 或显式 `compatibility.legacyStaticMemory=true` 才调用 `build_context_pack`，取高 salience（`>= PINNED_MIN_SALIENCE = 0.7`）active claim 渲染 `## Pinned Memory`，并用 `render_snapshot_section` 注入 `## User Profile`。每行必须经 `sanitize_for_prompt` + 截断；无 Profile 快照时才回退 legacy profile 段。
4. **legacy 单一来源 dedup**：只在上述兼容静态路径生效。`covered_by_active_claim_memory_ids`（`hidden_claim_linked` 的正向镜像）把被 active managed claim（salience ≥ 0.7、未过期）覆盖的 legacy memory 从 SQLite 段排除，避免同一事实双份注入。**三道豁免**：`user_pinned` link / `memories.pinned=1` / 非 managed link。**去重阈值与 Pinned 注入阈值对齐（同读 `PINNED_MIN_SALIENCE`）**——低于阈值的 claim 影子继续走 legacy 兜底，绝不丢事实。

V2 动态候选受 `memory.recall.maxSelected/maxTokens` 约束，不占用或改写 Core 静态预算；legacy 静态兼容路径才按 **Core > Pinned >（Profile + legacy）** 共享 `effective_memory_budget`。**Provider 注入合约**：Core 保持 canonical 稳定 prefix，Recall/Profile 等 turn-dependent 内容追加为独立 dynamic suffix；provider adapter 只按能力渲染多 cache block / 单 system / 弱 cache 形态，不能把动态段反向拼进稳定 fingerprint。

## Lucid Review 用户纠错闭环

把整理结果的控制权交给用户。**纯 owner 平面**（Tauri 命令 / HTTP 路由，API key / 本机信任），**无 agent 工具面**——模型不能自改自己的记忆。GUI 走可复用的 `ClaimReviewActions`（Dashboard Needs Review 队列 + Settings `ClaimsBetaView` 详情共用）。唯一编排入口 `claims::review`：

- **`update_claim(ClaimUpdate)`**（PATCH 语义，逐字段可选）：`resolve_update`（纯函数、无 DB、穷举单测）从「当前行 vs 请求」diff 派生主 decision-type，优先级 **status > scope > pin > edit**：
  - status：`active`→`approve` / `archived`→`reject` / `expired`→`expire`（标记过时）/ `needs_review`→`flag`
  - scope：`move_scope`（global 清空 scope_id）
  - pin/unpin：salience 越过 / 退到 `PINNED_MIN_SALIENCE`（pin=0.95 / unpin=0.5）
  - edit：content / subject / predicate / object / tags
  - approve / edit 视为用户确认：置 `confidence_source=user_confirmed`，confidence 提到 0.95（若更低）
- **`forget_claim(claim_id, permanent, note)`**：`permanent=false`（默认）翻 `archived` + 停止 linked legacy 注入，**保留 evidence 作审计**；`permanent=true` 硬删 claim 图谱（claim + evidence + link + vec0）+ 仅其独管的 orphan memory，连 evidence 一起删。

底层原语在 `claims::store`：`claim_edit_state`（读 diff 基线）/ `apply_claim_fields`（**any→any 状态**，区别于 resolver 的 active-gated `set_claim_status`）/ `reembed_claim` / `add_correction_evidence` / `forget_claim`。

红线与副作用：

- **审计**：每个用户动作经 `dreaming::record_user_action` 落一条 `trigger=user_correction` / `phase=user` / `status=completed` 的运行 + 单 decision（`before_json` / `after_json` **完整字段快照**——对称且覆盖三元组 / tags / confidence，可重建整次变更），在 Dashboard 运行历史与流水线 run 并列。审计 best-effort：纠错已成功，审计写失败只 `app_warn!` 不回滚。
- **证据门**：`approve` / `edit` / `reject` / `expire` / `move_scope` 写一条 `weight=1.0`、`raw_allowed` 的纠错证据（approve 用 `user_confirmed`、其余 `manual_correction`）；`pin` / `unpin` / `flag` / `noop` 不写（非事实修正）。
- **事件**：`memory:claim_changed`（每次）+ `memory:review_required`（flag 时）经 EventBus 发出，Dashboard 据此实时刷新。

## 安全与隐私

### 无痕会话零泄漏（fail-closed）

红线：无痕会话内容禁止进入 claim / evidence / profile / 统计。短路点遍布全链：

- **提取**：`memory_extract` 的 `extract_after_turn` / `flush_before_compact` / 空闲提取入口先查 `is_session_incognito`，真则直接返回。
- **扫描 / 证据**：`scanner` 的证据构造 fail-closed——session 元数据缺失 / 已删 / `incognito=true` 都视为不可见，只挂 memory ref、不挂 session ref；`evidence.rs` 的 `evidence_quote` 双门（无 `message_id` 拒、session 不可见拒），即便 quote 已脱敏也永不展开。
- **注入**：incognito 会话整段记忆注入跳过、改注入显式「Incognito Session」指令；Context Pack / Active Memory 同被短路。
- **手动写入工具**：`save_memory` / `update_core_memory` 工具入口同样 fail-closed——`ToolExecContext.incognito` 为真直接拒，与提取路径对称。否则模型可在无痕会话里手动落库 / 改 `MEMORY.md`，绕过「关闭即焚」。

### Scope 隔离

提取时 `resolve_extract_scope` 把会话归到 `Project{id}`（有项目）否则降到 `Agent{id}`（不跳 Global）；读注入时 Context Pack 按显式 scope 列表取并 union，**绝不让 Project A 的事实进 Project B 的 prompt**。

### 来源脱敏与 Prompt 注入防护

- **脱敏**：evidence quote 经 `logging::redact_sensitive`（`sk-*` / `-----BEGIN` / JWT 等）+ 码点截断（不按字节，保护多字节字符）。展开 quote 必经后端授权（验 session 非 incognito + message 存在），前端不可绕过。
- **Sanitize**：`sanitize_for_prompt` 检测 13 种注入模式（`ignore previous instructions` / `system prompt:` / `<|im_start|>` 等），命中替换为 `[Content filtered: ...]`，否则转义特殊分隔符；Pinned Claims 与 legacy 段逐行都过它。
- **证据不升格指令**：只有经提取（`COMBINED_EXTRACT_PROMPT` 验证）的 claim `content` 进 prompt；`file` / `tool_result` / `url` 证据原文不直接成为 standing instruction。

## 配置

`DreamingConfig` 持久化于 `AppConfig.dreaming`（camelCase），完整字段与默认：

| 字段 | 默认 | 含义 |
|---|---|---|
| `enabled` | `true` | 主开关，关闭则所有触发 no-op |
| `idleTrigger.{enabled, idleMinutes}` | `true` / `30` | 空闲触发 |
| `cronTrigger.{enabled, cronExpr}` | `false` / `"0 0 3 * * *"` | 定时触发（6 字段 cron）|
| `manualEnabled` | `true` | Dashboard「Run now」|
| `promotion.{minScore, maxPromote}` | `0.75` / `5` | Light 提名阈值 / 单轮上限 |
| `scopeDays` | `1` | 扫描窗口（天）|
| `candidateLimit` | `50` | 单轮候选上限 |
| `narrativeMaxTokens` | `2048` | narrative side_query token 预算 |
| `narrativeTimeoutSecs` | `60` | narrative 超时 |
| `modelOverride` | `null` | 专用模型链 `ModelChain`；null = 落 `function_models.automation` → 聊天全局模型。deprecated `narrativeModel`（`provider:model`）仍惰性兼容 |
| `profileSynthesis.{enabled, maxLinesPerScope}` | `true` / `12` | Profile 合成 / 每 scope 行数上限 |
| `deepResolver.autoExpireOnLightCycle` | `true` | Light 后自动执行确定性过期 |
| `deepResolver.autoResolveOnLightCycle` | `true` | Light 后自动执行保守 graph-first 分类 |
| `deepResolver.autoResolveMaxGroups` | `8` | 单轮自动 LLM 分组上限，读时钳 `[1,20]` |
| `deepResolver.autoResolveMinConfidence` | `0.92` | 自动状态变更最低置信度，读时钳 `[0.75,0.99]` |
| `deepResolver.autoMergeNearDuplicates` | `true` | 允许自动合并被二次佐证的近重复；关闭后冲突仍可进待审 |
| `deepResolver.autoMergeSimilarity` | `0.84` | 无 alias 边时的词法佐证阈值，读时钳 `[0.70,0.98]` |

GUI 在「设置 → 记忆 → Dreaming」（`DreamingPanel`，含 idle 倒计时与 cron 可视化编辑器）；`ha-settings` 技能可读写同一字段集（风险等级 MEDIUM，登记于 [`skills/ha-settings/SKILL.md`](../../skills/ha-settings/SKILL.md)），二者零偏差。Dreaming 负责 claim 的生成与整理，不等于允许在每轮对话中自动召回：V2 是否自动检索由全局 `memory.recall.enabled`（默认关闭）控制，是否纳入 claim 由 `memory.recall.includeClaims`（默认开启、但仅在自动召回有效时生效）控制。`ActiveMemoryConfig.include_claims`（per-agent，默认关）仅保留给旧兼容 / V1 rollback 链。

## API / UI 表面

owner 平面命令（Tauri ↔ HTTP 一一对应，**完整签名与语义见 [`api-reference.md`](api-reference.md)**，本表不重复）：

- **Claim 读 / 纠错**：`claim_list` / `claim_get` / `claim_update`（PATCH，`id` 走 path）/ `claim_forget`。
- **Backfill**：`memory_backfill_plan` / `memory_backfill_apply`。
- **运行**：`dreaming_run_now`（Light）/ `dreaming_run_resolver`（Deep）/ `dreaming_run_profile`（Profile）。
- **状态 / 只读**：`dreaming_list_runs` / `dreaming_get_run` / `dreaming_is_running` / `dreaming_last_report` / `dreaming_idle_status` / `dreaming_resolver_preflight` / `dreaming_list_profile_snapshots` / `dreaming_list_diaries` / `dreaming_read_diary`（路径遍历防护）/ `dreaming_evidence_quote`（incognito 归零）。
- **配置**：`get_dreaming_config` / `save_dreaming_config`。

EventBus 事件：`dreaming:cycle_started` / `dreaming:cycle_complete`（payload 含 `runId` / `phase` / `trigger`）、`memory:claim_changed`、`memory:review_required`。

UI：

- **Dashboard → Dreaming Center**（`dashboard/dreaming/`）：运行历史（含 decision + evidence 展开）、Needs Review 队列（`NeedsReviewQueue` + 逐 claim `ClaimReviewActions`）、手动运行按钮（`dreaming_is_running` 时禁用）、idle 倒计时。
- **Settings → 记忆面板**（`settings/memory-panel/`）：`ClaimsBetaView`（claim 列表 + 详情 + backfill 计划 / 应用）、`ProfileSnapshotView`（每 scope 最新快照 + 手动合成）、`DreamingPanel`（全配置）。

## 确定性评测（Golden Fixtures）

Dreaming 靠离线 eval 守红线、不靠感觉。三层避免 LLM 非确定性让 CI 变脆：

| 层级 | 内容 | 默认 CI |
|---|---|---|
| Deterministic | scope 过滤 / 过期抑制 / 证据可追溯 / 冲突进待审 / legacy-sync 隐藏 / 证据 fail-closed | weekly / release 专项评测 |
| Golden LLM fixtures | claim 抽取 / profile 合成 / 冲突 rationale（固定模型或 mock）| 手动 / nightly |
| Human review set | 真实样本匿名后人工标注 precision/recall | release 前抽样 |

已落地 **deterministic 层**：

- [`memory/dreaming/eval.rs`](../../crates/ha-core/src/memory/dreaming/eval.rs)——fixture 类型 + `load_fixtures()` + `evaluate(backend, fixture)`，经**公共 API 跑真实读路径**（播种 → list / get / 注入候选 / evidence_quote 断言），不重写被测逻辑。
- [`evals/suites/memory-dreaming/fixtures/*.json`](../../evals/suites/memory-dreaming/fixtures/)——9 个 canonical fixture，覆盖基础 claim 红线及 `auto_expire_planning` / `auto_resolver_graph_planning`；`valid_until` 用固定 token 保持与时钟无关。
- [`ha-eval`](../../crates/ha-eval/)——每个 fixture 在独立子进程和 claim store 中运行，产出可审计 case evidence；不编入默认 Cargo test。

### 同步契约

- **claim 读路径**：改动 claim 读路径 / effective-status / hidden-set / scope 过滤 / evidence 授权等安全红线时，须在 fixtures 加 case 或保既有绿。
- **Deep Resolver 规划（`auto_resolver_graph_planning`）**：这条 fixture 是自动 sweep「哪些组进 LLM、哪些直接 graph-noop、何时算截断」的锁。`checks.auto_resolver_graph_plan` 播种 claim 后**直接调纯函数** `plan_auto_resolution_groups(scoped, expiring, group_cap)`（`expiring` 取自 `plan_auto_expiration_sweep`，**不发 LLM**），断言 `llm_group_ids` / `graph_noop_group_ids` / `truncated` 三者。当前 fixture 锁住的行为：单值谓词 `preferred_theme`、`timezone` 两组进 LLM；多值谓词 `uses_package_manager` 组 graph-noop；已过期的 `timezone` 成员先被确定性过期摘走、不进候选图（故该组只剩两成员）；`group_cap=1` 时 LLM 组截到一组且 `truncated=true`，而 graph-noop 组不受 cap 影响。

  **改动下列任一符号，必须同步更新本 fixture 或保既有绿**：

  | 面 | 符号 |
  |---|---|
  | 分组 | `group_conflicts` 的分组键 `(scope_type, scope_id, claim_type, subject, predicate)`、「>1 成员且 ≥2 种不同 `claims::normalize_object`」准入、`expiring` 剔除；`plan_auto_resolution_groups` 的截断语义与 `group_cap.clamp(1,20)` |
  | 基数规则 | `MULTI_VALUED_PREDICATES` / `SINGLE_VALUED_PREDICATES` / `ALIAS_PREDICATES` 三张词表、`normalize_predicate` 归一化、`predicate_cardinality` 的精确 / 前缀 / 后缀匹配规则、`graph_group_signals` 的 `alias_connected` 连通判定 |
  | 自动决策映射 | `map_auto_verdict_to_decisions` 的 relation→decision 映射与置信门、`auto_duplicate_is_corroborated` 的佐证条件、`parse_verdict` 接受的 relation 集合与 confidence 钳位、`ResolverDecisionType` 的 variant 集合 |

  **覆盖边界要清楚**：fixture 无 LLM，只能直接锁住「分组 / 基数」两面；「自动决策映射」由 `resolver.rs` 的穷举单测把关（`graph_planner_skips_known_multi_value_predicates` / `automatic_conflicts_require_high_confidence_and_only_route_to_review` / `automatic_duplicate_merge_requires_graph_or_lexical_corroboration` / `verdict_parser_rejects_unknown_relations_and_clamps_confidence` / `automatic_group_planning_is_bounded_and_reports_truncation`），映射改动若波及入选组则同时反映到本 fixture。两者须一并保绿——单测绿而 fixture 未跑不算满足本契约。

## 与现有子系统的关系

- **[`memory_extract`](../../crates/ha-core/src/memory_extract.rs)**：claim 双写的上游 hook，消费 `add_with_dedup` 三态补 link。
- **Fast/Deep Recall / Context Pack**（见 [`memory.md`](memory.md)）：claim 注入的承载层；V2 Retrieval Planner 在用户显式允许自动召回后融合结构化 claim，legacy Active Memory 仅作为兼容 / 回滚路径保留。
- **[Recap](recap.md) / [Awareness](behavior-awareness.md)**：与 Dreaming 同为离线 / 动态注入子系统，各自独立 store，互不折叠。
- **[Side Query](side-query.md)**：Light narrative / Deep 冲突 / Profile 重写都走它，复用主对话 prefix 命中 cache。
- **Session / Evidence 生命周期**：incognito 会话证据永不写入；常规会话删除 / 压缩后 evidence 退化为 `anchor_only`（留锚点、清 quote），claim 仍可保留（≥1 证据锚点）。
- **Project 生命周期**：删除项目时级联清理该 scope 的 **claim 图谱**（claim + evidence + link + vec0 + profile snapshot），与 legacy memory 一并清（`claims::delete_claims_for_scope`）。FK 在 `memory.db` 上未开，故显式 teardown；避免删项目后孤儿 claim 残留在列表 / Lucid Review。

## 关键源文件

| 路径 | 职责 |
|---|---|
| [`memory/dreaming/pipeline.rs`](../../crates/ha-core/src/memory/dreaming/pipeline.rs) | Light 周期编排 |
| [`memory/dreaming/{scanner,narrative,promotion,scoring}.rs`](../../crates/ha-core/src/memory/dreaming/) | Light 各阶段（一代）|
| [`memory/dreaming/{triggers,cron_loop}.rs`](../../crates/ha-core/src/memory/dreaming/) | idle / cron / manual 触发 + 跨进程协调 |
| [`memory/dreaming/store.rs`](../../crates/ha-core/src/memory/dreaming/store.rs) | durable run / 决策日志 / profile 快照 / `record_user_action` |
| [`memory/dreaming/resolver.rs`](../../crates/ha-core/src/memory/dreaming/resolver.rs) | Deep 确定性过期 + 冲突分析 |
| [`memory/dreaming/profile.rs`](../../crates/ha-core/src/memory/dreaming/profile.rs) | Memory Profile 合成 |
| [`memory/dreaming/context_pack.rs`](../../crates/ha-core/src/memory/dreaming/context_pack.rs) | legacy / V1 rollback 的 Pinned Claims 静态兼容注入 |
| [`memory/dreaming/evidence.rs`](../../crates/ha-core/src/memory/dreaming/evidence.rs) | 证据 quote 授权读取（fail-closed）|
| [`memory/dreaming/{config,types,eval}.rs`](../../crates/ha-core/src/memory/dreaming/) | 配置 / 共享类型 / 确定性评测调度 |
| [`memory/claims/store.rs`](../../crates/ha-core/src/memory/claims/store.rs) | claim schema + 读 API + 纠错原语 + effective-status |
| [`memory/claims/write.rs`](../../crates/ha-core/src/memory/claims/write.rs) | 双写 + canonicalize + confidence baseline + 归一化 |
| [`memory/claims/backfill.rs`](../../crates/ha-core/src/memory/claims/backfill.rs) | 旧 memory → claim 回填（dry-run / apply）|
| [`memory/claims/review.rs`](../../crates/ha-core/src/memory/claims/review.rs) | Lucid Review：update_claim / forget_claim |
| [`memory/sqlite/{backend,trait_impl,prompt}.rs`](../../crates/ha-core/src/memory/sqlite/) | schema DDL / hidden-set / sanitize + 快照渲染 |
| [`agent/active_memory.rs`](../../crates/ha-core/src/agent/active_memory.rs) | V2 Fast/Deep Recall 执行与 legacy Active Memory 兼容链（含 claim 候选）|
| [`src/components/dashboard/dreaming/`](../../src/components/dashboard/dreaming/) · [`settings/memory-panel/`](../../src/components/settings/memory-panel/) | Dashboard Dreaming Center + Settings 记忆面板 |
| [`crates/ha-eval`](../../crates/ha-eval/) · [`evals/suites/memory-dreaming/`](../../evals/suites/memory-dreaming/) | 独立确定性评测 + 9 golden fixtures |

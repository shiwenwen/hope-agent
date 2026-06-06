# 下一代 Dreaming 计划：从离线固化到可审计长期心智

> 目标：把现有 Dreaming Light 从“扫描近期 memory → LLM 打分 → pin 高分项 → 写 Dream Diary”升级为一套**本地优先、证据可追溯、会纠错、按 Project / Agent / Global 分层治理、可评测**的长期记忆系统。
>
> 这不是推倒重来。现有 [`memory/dreaming/`](../../crates/ha-core/src/memory/dreaming/) 是正确的第一阶段：离线、不阻塞聊天、低成本、可观察。下一代是在它之上增加结构化 claim、来源证据、冲突/过期治理、Memory Profile、用户审核和评测闭环。

## 0. 背景

### 0.1 现状

Hope Agent 已经具备 Dreaming 的核心底座：

- `memory_extract.rs`：对话后自动提取事实、偏好、项目上下文、reference；支持 `profile` 反思条目。
- `memory/dreaming/`：空闲 / cron / 手动触发，扫描近期 memory，让 LLM 提名值得长期固化的候选，并将其 `pinned=true`。
- `Dream Diary`：每轮写 markdown，Dashboard 可回看。
- `Active Memory`：每轮 user turn 前主动召回相关记忆，作为独立 cache block 注入。
- `Recap` / `Awareness` / `Context Compact`：提供长窗口复盘、跨会话状态、压缩前记忆 flush。

当前 Dreaming 的定位仍是 **Light phase**：

- 候选主要来自近期 `memories` 表，而不是直接理解全部会话、工具产物和项目演化。
- 结果主要是 `pinned` 这个布尔值，缺少结构化“为什么信、来自哪、是否过期、和谁冲突”。
- Dream Diary 是人类可读，但机器后续复用和审核能力有限。
- 还没有“Memory Summary / Profile”式的可见用户画像，也没有系统化评测。

### 0.2 下一步要赢在哪里

我们不只追平“后台整理记忆”。Hope Agent 应该做得更适合 agent：

| 维度 | 下一代目标 |
|---|---|
| 本地优先 | 记忆、证据、决策日志默认在 `~/.hope-agent/`，可用本地模型跑低敏 consolidation |
| Scope 精准 | Project > Agent > Global 三层原生治理，避免不同项目人格/上下文污染 |
| 证据可追溯 | 每条长期记忆都有来源 message / session / file / tool / URL anchor |
| 会纠错 | 识别新旧事实冲突、时间过期、用户纠正；新事实 supersede 旧事实 |
| 可审核 | 用户能看“它认为它知道什么”，能 approve / reject / edit / forget / mark outdated |
| 可行动 | 不只记住偏好，还能发现可复用 skill、cron、project TODO、风险和待确认问题 |
| 可评测 | 有 golden fixtures 和指标，避免记忆系统越聪明越玄学 |

## 1. 非目标

| # | 不做 | 原因 |
|---|---|---|
| N1 | 首版自动硬删除用户记忆 | 记忆删除应显式确认；自动流程只做 `superseded` / `archived` / `needs_review` |
| N2 | 让 Dreaming 阻塞聊天热路径 | 所有深层 consolidation 继续异步，聊天最多消费已准备好的 context pack |
| N3 | 一次性重写 memory backend | 现有 `memories` 表和 tools 继续可用；新 schema 以兼容层增量引入 |
| N4 | 把无痕会话纳入任何长期记忆 | 保持“关闭即焚”：不入候选、不入 evidence、不入 profile、不入统计 |
| N5 | 让模型自行修改 `memory.md` | `memory.md` 是用户/agent core memory；Dreaming 可建议 patch，但默认不自动写 |

## 2. 总体设计

下一代 Dreaming 拆成四类产物：

1. **Memory Claims**：结构化长期事实。例：“用户希望中文回复”“项目 X 使用 Tauri 2 + React 19”“某 API key 轮换方案已废弃”。
2. **Evidence Graph**：每个 claim 的来源证据，包括 session/message/file/tool/url，支持追溯和反驳。
3. **Memory Profile**：面向用户和 prompt 的可读摘要，按 Global / Agent / Project 分层。
4. **Dream Decisions**：每轮 Dreaming 的机器可读决策日志，记录 promote / supersede / archive / needs_review / no-op。

现有 `memories` 继续作为兼容层：

- `save_memory` / `recall_memory` / `memory_get` 仍读写现有 memory。
- Dreaming v2 可以把高置信 claim 同步为 `MemoryEntry`，并沿用 `pinned` 的 prompt 优先级。
- 未来 UI 逐步从“平铺 memory 列表”扩展到“Claim + Evidence + MemoryEntry”混合视图。

兼容层必须有**双向同步协议**，否则会出现“过期 claim 已被过滤，但旧 pinned memory 仍进入 prompt”的幽灵记忆：

- Claim 自动创建 legacy memory 时必须写 `memory_claim_links`。
- Claim 状态从 `active` 变为 `superseded` / `expired` / `archived` 时，关联 legacy memory 默认 `pinned=false`，并从 prompt candidates 中隐藏；用户手动保留除外。
- 兼容层判断用 `effective_status`，不是只看持久化 `status`：`valid_until < now()` 的 claim 即使 `status=active`，也按 `expired` 处理；linked legacy memory 在 prompt candidate 读取时实时跳过，`sync_mode=user_pinned` 除外（用户手动保留只生成 `needs_review`，不自动隐藏）。
- Legacy memory 被用户手动编辑 / pin / delete 时，反向写一条 `dreaming_decisions`，并标记相关 claim 需要重算。
- `recall_memory` 仍可返回 archived/superseded 旧内容，但必须显示状态；system prompt 注入只取 active。

现有 `memory_extract.rs` 的反省式 `profile` 记忆与新 Memory Profile 不能并行成为两套 prompt 来源。演进规则：

- Phase 2 继续保留 `COMBINED_EXTRACT_PROMPT` 的 `profile` 分支，但它只作为 `claim_type=user_profile` 的输入来源。
- Phase 4 起，`## User Profile` 由 `memory_profile_snapshots` 生成；legacy profile-tagged memories 不再直接渲染成独立 profile 段，只作为 claims/evidence 的兼容来源。
- 只要 Profile Snapshot 不存在、过期或 Deep/Profile synthesis 未启用，就 fallback 到现有 profile-tagged memories；不能因为 Deep 默认关闭导致 `## User Profile` 空白。
- Profile synthesis 不完全绑 Deep：idle/light 基于 active user_profile claims **规则式拼接**生成轻量 snapshot（不额外烧 side_query，守住 Light 单轮 1 次成本约束）；LLM 综合重写与冲突解释留给 Deep。

## 3. 数据模型

所有新表默认落在 **memory backend 同一个 SQLite 数据库**，不新开 `dreaming.db`。原因：

- `memory_claim_links.memory_id` 需要和 legacy `memories.id` 做事务一致的同步。
- Backfill、claim 双写、prompt candidate 过滤都需要在同一 transaction 内完成。
- migration 跟随 `memory/sqlite/backend.rs`，不要放到 `recap.db` 或 session db。

所有带 `scope_type/scope_id`、`agent:<id>`、`source_run_id/scope_json` 的字段，必须登记到 [`agent/migration.rs`](../../crates/ha-core/src/agent/migration.rs) 的 agent-id rename 迁移清单。新增表时要同步写 migration 测试，防止 `default` → `ha-main` 这类 rename 后 claim scope 悬空。

### 3.1 新表：`dreaming_runs`

持久化每次运行，不再只保留进程内 `LAST_REPORT`。

```sql
CREATE TABLE dreaming_runs (
  id TEXT PRIMARY KEY,
  trigger TEXT NOT NULL,              -- idle | cron | manual | post_turn | compact_flush | memory_import | user_correction
  phase TEXT NOT NULL,                -- light | deep
  status TEXT NOT NULL,               -- running | completed | failed | skipped
  owner_instance_id TEXT,              -- process/runtime instance that owns the lease
  heartbeat_at TEXT,
  lease_expires_at TEXT,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  scope_json TEXT NOT NULL,           -- scanned scopes, windows, watermarks
  scanned_count INTEGER NOT NULL DEFAULT 0,
  decision_count INTEGER NOT NULL DEFAULT 0,
  promoted_count INTEGER NOT NULL DEFAULT 0,
  note TEXT
);
```

### 3.1.1 新表：`dreaming_locks`

`DREAMING_RUNNING: AtomicBool` 只能挡住同进程重入；desktop / server / ACP 多进程场景需要 SQLite lease。

```sql
CREATE TABLE dreaming_locks (
  lock_key TEXT PRIMARY KEY,           -- e.g. light:global, deep:project:<id>
  run_id TEXT NOT NULL,
  owner_instance_id TEXT NOT NULL,
  heartbeat_at TEXT NOT NULL,
  lease_expires_at TEXT NOT NULL
);
```

Acquire 规则：

1. 在一个 SQLite transaction 中读 `dreaming_locks.lock_key`。
2. 不存在或 `lease_expires_at < now()` 时抢占并写新 lease。
3. 未过期则 skip，不排队等待。
4. 运行中每 N 秒 heartbeat；进程崩溃后下一轮可接管 expired lease。
5. run 结束时删除 lock；删除失败不影响结果，靠过期兜底。

### 3.1.2 新表：`dreaming_watermarks`

跨 run 保存扫描进度，不能只放在 `dreaming_runs.scope_json` 的单次快照里。

```sql
CREATE TABLE dreaming_watermarks (
  scope_key TEXT NOT NULL,             -- global | agent:<id> | project:<id>
  source_type TEXT NOT NULL,           -- session_messages | memories | tool_metadata | recap_facets
  last_source_id TEXT,
  last_source_ts TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (scope_key, source_type)
);
```

### 3.1.3 新表：`dreaming_pending_sources`

高频 Light source capture 需要持久队列，避免 Deep lock 正在运行时直接丢候选。

```sql
CREATE TABLE dreaming_pending_sources (
  id TEXT PRIMARY KEY,
  scope_key TEXT NOT NULL,
  source_type TEXT NOT NULL,
  source_id TEXT NOT NULL,
  source_ts TEXT,
  payload_json TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending', -- pending | claimed | processed | skipped
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX idx_dreaming_pending_sources_scope_status
  ON dreaming_pending_sources(scope_key, status, created_at);
```

### 3.2 新表：`memory_claims`

结构化长期记忆主表。

```sql
CREATE TABLE memory_claims (
  id TEXT PRIMARY KEY,
  scope_type TEXT NOT NULL,           -- global | agent | project
  scope_id TEXT,
  claim_type TEXT NOT NULL,           -- user_profile | preference | project_fact | standing_rule | reference | task_pattern
  subject TEXT NOT NULL,              -- user | agent:<id> | project:<id> | tool:<name>
  predicate TEXT NOT NULL,            -- prefers | uses | works_on | avoid | completed | deprecated
  object TEXT NOT NULL,
  content TEXT NOT NULL,              -- human-readable sentence, same language as source when possible
  tags_json TEXT NOT NULL DEFAULT '[]',
  confidence REAL NOT NULL DEFAULT 0.5,
  confidence_source TEXT NOT NULL DEFAULT 'derived', -- derived | llm_adjusted | user_confirmed
  salience REAL NOT NULL DEFAULT 0.5,
  freshness_policy_json TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'active', -- active | superseded | expired | archived | needs_review
  valid_from TEXT,
  valid_until TEXT,
  supersedes_claim_id TEXT,
  source_run_id TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

设计要点：

- `confidence`：证据强度，主要由 `evidence_class` 推导。用户明确说的 > 用户确认 > assistant 推断 > 间接行为。
- `salience`：长期有用程度。用于 prompt 预算和 profile 摘要。
- `confidence_source`：记录置信度来源。默认由 `evidence_class` 推导，LLM 只能建议调整，不能单独决定自动降级。
- `freshness_policy_json`：保存衰减参数；实时 freshness 在读取/排序时根据 `valid_from` / `valid_until` / half-life 计算，不存静态衰减值。
- `status`：自动流程默认不删除，先降级或标记待审核。

`evidence_class` 与 `source_type` 是两条轴：

| 字段 | 含义 | 示例 |
|---|---|---|
| `source_type` | 物理来源 | `session_message` / `file` / `tool_result` / `manual` |
| `evidence_class` | 认知强度（封闭枚举，见下表） | `manual_correction` / `user_confirmed` / `explicit_user_statement` / `project_artifact_fact` / `assistant_inferred` / `behavioral_pattern` |

置信度基线由 `evidence_class` 决定，`source_type=session_message` 不能自动等同高置信，因为它可能是用户明说，也可能是 assistant 推断。首版建议区间：

| evidence_class | confidence baseline |
|---|---|
| `manual_correction` | 1.00 |
| `user_confirmed` | 0.95 |
| `explicit_user_statement` | 0.85 |
| `project_artifact_fact` | 0.75 |
| `assistant_inferred` | 0.45 |
| `behavioral_pattern` | 0.35 |

以上 6 个为 `evidence_class` 的**全部合法取值**（evidence 表 `evidence_class` 列只接受这 6 值）；映射由 deterministic test 覆盖，LLM 只输出 `evidence_class` 标签，不参与 confidence 数值产出。

Embedding 策略：

- Claim canonicalize 默认必须支持 FTS-only，因为现有 memory embedding 可能关闭。
- 若复用 `memory_embedding`，则只在 active embedding 可用且签名匹配时使用 claim embedding；关闭时降级为 `scope + claim_type + subject + predicate + normalized_object` 规则匹配。
- 首版不新增独立 claim embedding selector，避免配置爆炸；若后续要独立 selector，应仿照 knowledge embedding 单独设计，而不是隐式寄生 memory embedding。

### 3.3 新表：`memory_evidence`

每条 claim 的来源证据。

```sql
CREATE TABLE memory_evidence (
  id TEXT PRIMARY KEY,
  claim_id TEXT NOT NULL,
  source_type TEXT NOT NULL,          -- session_message | memory | file | tool_result | url | recap_facet | manual
  evidence_class TEXT NOT NULL DEFAULT 'assistant_inferred',
  source_id TEXT NOT NULL,
  session_id TEXT,
  message_id TEXT,
  file_path TEXT,
  url TEXT,
  quote TEXT,                         -- 短摘录，严格限长
  redaction_status TEXT NOT NULL DEFAULT 'redacted', -- redacted | raw_allowed | anchor_only
  access_scope_json TEXT NOT NULL DEFAULT '{}',
  weight REAL NOT NULL DEFAULT 1.0,
  created_at TEXT NOT NULL,
  FOREIGN KEY (claim_id) REFERENCES memory_claims(id) ON DELETE CASCADE
);
```

证据原则：

- 不保存无痕会话证据。
- `quote` 做限长和敏感信息脱敏；必要时只存 source anchor。
- HTTP/server 读取证据引用文件时仍走 preview-by-path 鉴权红线。
- UI 默认展示 source 摘要，不直接展开原文 quote；展开前按 claim scope + session/file 授权二次过滤。
- Evidence 可以证明 claim，但不等于 prompt 内容；system prompt 默认不注入 evidence quote。

### 3.3.1 新表：`memory_claim_links`

记录 claim 与 legacy memory 的同步关系，避免双轨期间状态漂移。

```sql
CREATE TABLE memory_claim_links (
  claim_id TEXT NOT NULL,
  memory_id INTEGER NOT NULL,
  sync_mode TEXT NOT NULL DEFAULT 'managed', -- managed | user_pinned | detached
  last_synced_claim_status TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (claim_id, memory_id),
  FOREIGN KEY (claim_id) REFERENCES memory_claims(id) ON DELETE CASCADE,
  FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
);
```

同步规则：

- `managed`：Dreaming 可按 claim 状态自动 pin/unpin/hide legacy memory。
- `user_pinned`：用户手动 pin 的 memory 不被自动 unpin，只生成 `needs_review`。
- `detached`：用户明确解除关联后，claim 不再影响该 memory。

### 3.4 新表：`dreaming_decisions`

让 Dream Diary 从纯 markdown 变成可审计事件流。

```sql
CREATE TABLE dreaming_decisions (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  decision_type TEXT NOT NULL,        -- promote | merge | supersede | expire | archive | needs_review | no_op
  target_type TEXT NOT NULL,          -- memory | claim | profile | task | skill_suggestion
  target_id TEXT,
  score REAL,
  rationale TEXT NOT NULL,
  before_json TEXT,
  after_json TEXT,
  created_at TEXT NOT NULL
);
```

### 3.5 新表：`memory_profile_snapshots`

保存可展示、可注入的摘要快照。

```sql
CREATE TABLE memory_profile_snapshots (
  id TEXT PRIMARY KEY,
  scope_type TEXT NOT NULL,
  scope_id TEXT,
  version INTEGER NOT NULL,
  body_md TEXT NOT NULL,
  source_run_id TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE(scope_type, scope_id, version)
);
```

Snapshot version 在 transaction 中用 `MAX(version)+1` 分配；并发冲突时重试，避免 idle/light 与 Deep 同时生成同 scope snapshot 时覆盖。

## 4. Pipeline 设计

### 4.1 触发器

在现有 `Idle / Cron / Manual` 上扩展：

| trigger | phase | 目的 |
|---|---|
| `post_turn` | `light` | 成功 turn 后，满足阈值时做小窗口 claim extraction |
| `idle` | `light` / `deep` | 空闲时做轻量固化；用户开启 Deep 后做长窗口 consolidation |
| `cron` | `deep` | 夜间长窗口整理 |
| `manual` | `light` / `deep` | Dashboard 手动运行 |
| `compact_flush` | `light` | Tier 3 压缩前后补证据，防信息丢失 |
| `memory_import` | `light` | 批量导入后统一去重、合并、重建 profile |
| `user_correction` | `light` | 用户编辑/拒绝/标过期后立即重算相关 claim |

所有触发统一走 durable run claim：

- 进程内 `AtomicBool` 继续保留，避免同进程重入。
- 新增 SQLite lease，避免 desktop/server 多进程同时跑 Deep。
- ACP minimal 不启动 idle/cron loop，但手动 API 可以运行。
- Lock key 以 `phase + scope` 命名：Light 可按 scope 并发，Deep 默认全局串行，避免长窗口 consolidation 互相覆盖。
- Idle 是否触发 Deep 由独立配置控制（如 `dreaming.deepIdle.enabled`），默认关闭；未开启时 idle 只跑 Light。

触发器接线点：

| 触发 | 接线点 |
|---|---|
| `post_turn` | `chat_engine` 成功收尾后，复用现有 `schedule_memory_extraction_after_turn` 阈值 |
| `compact_flush` | `memory_extract::flush_before_compact` 成功保存后追加 claim candidate |
| `user_correction` | `memory_claim_update` / `memory_claim_forget` / legacy memory pin-edit-delete API |
| `memory_import` | `memory_import` 完成后写 backfill job |

高频 Light 触发不能因为 Deep 正在跑而丢数据：

- Light claim extraction 写 pending queue / watermark，Deep lock 未释放时只跳过 consolidation，不跳过 source capture。
- Deep 结束后补跑同 scope 的 pending Light consolidation。
- `skip` 只代表本次 consolidation 没执行，不代表候选 source 被丢弃。
- `dreaming_pending_sources` 的 `processed/skipped` 行按 retention 策略清理，默认保留 30 天；`claimed` 超过 lease 后回到 `pending`。

### 4.2 阶段 A：Source Scanner

候选来源按成本分层：

| 来源 | 用途 |
|---|---|
| `memories` | 兼容当前 Light pipeline，最快 |
| `session messages since watermark` | 从对话直接提取遗漏事实 |
| `messages.tool_metadata` / `session::aggregate_session_artifacts` | 把文件、URL、工具产物纳入证据 |
| `recap facets` | 长窗口主题、项目进展、周期性行为 |
| `memory profile snapshots` | 和旧 summary 对比，找变化 |
| `user corrections` | 权重最高，触发冲突重算 |

Scanner 规则：

- 跳过 incognito session。
- Project session 只进入对应 Project scope，同时可抽少量 Global 用户偏好。
- 每个 source chunk 必须携带 source anchor，后续 claim 不允许无来源。
- 使用 watermark，避免每次全库重扫。

### 4.3 阶段 B：Claim Extraction

把当前 `memory_extract` 的 JSON array 升级为 claim 候选：

```json
{
  "claims": [
    {
      "claimType": "preference",
      "subject": "user",
      "predicate": "prefers_response_language",
      "object": "Chinese",
      "content": "用户希望用中文回复。",
      "scope": {"type": "global"},
      "evidenceClass": "explicit_user_statement",
      "salience": 0.85,
      "temporal": {"validFrom": null, "validUntil": null},
      "evidenceRefs": ["message:..."],
      "tags": ["language", "response-style"]
    }
  ],
  "uncertainties": [
    {
      "question": "这个偏好是全局偏好还是仅限当前项目？",
      "evidenceRefs": ["message:..."]
    }
  ]
}
```

实现策略：

- Light 只处理最近对话和 memory，单次 side_query。
- Deep 分 chunk 提取，再 reduce 合并，避免单 prompt 过大。
- 低置信候选只进 `needs_review`，不自动注入。
- LLM 输出 `evidenceClass` / salience / temporal hints；最终 `confidence` 由 `evidence_class` baseline + 用户确认 / 手动 correction 状态推导。

### 4.4 阶段 C：Canonicalization / Merge

对候选 claim 找已有等价项：

1. 先按 `scope_type + claim_type + subject + predicate` 粗筛。
2. 再用 FTS / embedding / normalized object 做相似匹配。
3. LLM 只处理模棱两可的 top-k 候选，避免每条都烧模型。

Light 也必须做规则式粗治理，不能等 Deep 默认开启后再去重：

- `scope_type + scope_id + claim_type + subject + predicate + normalized_object` 精确命中时直接 merge evidence。
- `valid_until` 已过期的 claim 在读取时不注入 prompt，即使 Deep 尚未运行。
- 同一 source chunk 重复提取时用 evidence/source hash 去重。
- LLM canonicalize / split / rationale 留给 Deep，但基础去重在 Light 完成。

合并动作：

| 动作 | 含义 |
|---|---|
| `merge_evidence` | 内容相同，增加证据和更新时间 |
| `refine_content` | 新证据让表达更准确，但不改变事实 |
| `supersede` | 新事实替换旧事实 |
| `split` | 原 claim 太宽，拆成多个精确 claim |
| `needs_review` | 证据冲突或 scope 不确定，交给用户 |

### 4.5 阶段 D：Temporal / Conflict Resolver

这是下一代 Dreaming 的核心差异：不仅“记住”，还要“知道什么时候不该再信”。

规则优先级：

1. 用户手动 correction / edit / forget 最高。
2. 更新、更明确的用户陈述 > 旧陈述。
3. Project scope 内事实不自动覆盖 Global / 其他 Project。
4. 时间性表达必须设置 `valid_until`，或写入 `freshness_policy_json` 衰减参数（读取时计算，不存静态值）。
5. assistant 自己推断的 profile 低于用户明确表达。

典型处理：

- “我下周去新加坡”到 `valid_until` 后读取层 freshness 归零、默认不进 prompt，Deep run 再判断是否已完成或归档。
- “这个项目改用 Bun，不用 pnpm” supersede 旧项目 claim，但不影响全局“默认用 pnpm”。
- “别再记这个”直接将相关 claim 标 `archived`，并写 manual evidence。

Freshness 读取时计算：

- `valid_until < now()` 时读取层 freshness 视为 0，默认不进 prompt。
- `effective_status(claim)` 在所有 prompt candidate、linked legacy memory、Active Memory v2 召回路径中统一使用；`status=active` 但 `valid_until` 已过期时视为 `expired`。
- 热路径必须在 SQL 层完成 effective filter：prompt candidates 通过 JOIN `memory_claim_links` + `memory_claims.valid_until/status` 过滤 linked legacy memory，避免 N+1 逐条查 claim；JOIN 逻辑必须尊重 `memory_claim_links.sync_mode='user_pinned'` 的豁免，不自动隐藏这类 legacy memory，只把对应 claim/link 推入 review queue。
- 无明确期限的 claim 使用 `freshness_policy_json.half_life_days` 计算排序分，不定期写回。
- `pinned` / `standing_rule` / `user_confirmed` 可以配置为 evergreen，但仍可被用户 correction supersede。

### 4.6 阶段 E：Promotion / Demotion

现有 `pinned=true` 保留，但不再是唯一结果。

| 输出 | 行为 |
|---|---|
| `promote_claim` | 高置信 + 高 salience，进入 prompt 候选和 Memory Profile |
| `pin_memory` | 同步到现有 `MemoryEntry.pinned=true`，兼容旧 UI |
| `demote_memory` | 过期或低价值，取消 pinned，但不删除 |
| `supersede_claim` | 旧 claim 不再注入，保留审计 |
| `needs_review` | Dashboard 提醒用户确认 |

### 4.7 阶段 F：Memory Profile Synthesis

每个 scope 生成一份短摘要：

- Global：用户长期偏好、沟通方式、跨项目稳定事实。
- Agent：这个 Agent 对用户的工作方式、输出偏好、常用工具。
- Project：项目栈、架构约束、当前目标、重要决策、废弃方案。

Profile 不是自由发挥，必须引用 active claims：

```markdown
## User Profile

- 用户偏好中文回复，喜欢直接指出问题和下一步。
- 用户关心 Hope Agent 的本地化、可控性和 agent-native 能力。

## Project Profile

- Hope Agent 是 Tauri 2 + React 19 + Rust 桌面应用，核心逻辑在 `ha-core`。
- 新 invoke 必须同时实现 Tauri 与 HTTP transport。
```

### 4.8 阶段 G：Context Pack

聊天热路径不直接跑 Deep。它消费 Dreaming 预先生成的 context pack：

```rust
pub struct MemoryContextPack {
    pub scope_profile_md: String,
    pub pinned_claims_md: String,
    pub relevant_claims_md: String,
    pub warnings_md: String,
    pub source_digest: Vec<SourceRef>,
}
```

注入顺序：

1. `memory.md` Core Memory
2. Memory Profile Snapshot
3. Pinned Claims
4. Relevant Claims
5. 未被 claim 覆盖的 legacy SQLite memory（双轨期）
6. Active Memory 当前 turn 召回
7. Awareness suffix

这些都应继续作为独立 cache block，避免动态记忆刷新打爆静态 prompt cache。

注入唯一来源与预算（双轨期关键，消歧 §2 的「claim 同步为 pinned memory」与本段的独立注入）：

- **单一来源**：被 active claim 覆盖（存在 `memory_claim_links`）的 legacy memory 从第 5 段排除，只走 Pinned / Relevant Claims 段，避免同一事实重复注入、占双份预算。
- 未被任何 claim 覆盖的 legacy memory（双轨期占多数）继续走第 5 段，不因 Context Pack 引入而丢失。
- §2 的「claim 同步为 `MemoryEntry.pinned`」只服务旧 UI 的列表展示与优先级排序，**不构成第二条 prompt 注入路径**——prompt 注入以 Context Pack 为准。
- Profile / Claims 新段纳入现有 `effective_memory_budget` 同一预算池，按 Core Memory > Profile > Pinned Claims > Relevant Claims > 未覆盖 legacy memory 的优先级裁剪，不另开预算绕过 4 级体系。

### 4.9 Provider 注入合约

Context Pack 不能假设所有 provider 都支持任意数量的 system block。实现上分两层：

1. Core 层统一产出 `MemoryContextPack`，不关心 provider。
2. Provider adapter 层按能力降级：

| Provider 能力 | 注入方式 |
|---|---|
| 多 system block / cache block | Core Memory、Profile、Claims、Active Memory、Awareness 分块注入 |
| 单 system prompt | 按固定顺序拼接成一个 `# Memory` 段，保留 heading 边界 |
| prompt cache 支持弱 | 只注入 Profile + top pinned claims，Active Memory 继续短 suffix |

`OneShotRequest` / streaming adapter 需要从单独的 `awareness_suffix`、`active_memory_suffix` 演进到 `dynamic_context_blocks: Vec<ContextBlock>`，但 provider adapter 必须保留旧字段 fallback，避免一次性改穿所有 provider。

## 5. UI / UX

### 5.1 Dashboard → Dreaming Center

把当前 Dreaming Tab 从“日记列表 + Run now”升级为控制台：

| 区域 | 内容 |
|---|---|
| Status | 最近一次 run、下次 idle/cron、候选数、决策数、耗时、错误 |
| What changed | 本轮新增 / 合并 / 过期 / 冲突 / 待审核 |
| Memory Profile | Global / Agent / Project 三层摘要 |
| Needs review | 低置信、冲突、scope 不确定的候选 |
| Sources | 可展开来源：session、message、file、tool、URL |
| Controls | Run Light / Run Deep / pause triggers / export diary |

职责边界：

- Settings → Memory → Dreaming 仍负责配置：enabled、idle、cron、threshold、模型、预算。
- Dashboard → Dreaming Center 负责运行历史、review queue、Profile、sources 和手动 run。
- 两边都可显示状态条，但只有 Settings 修改配置，避免同一开关出现在两个地方。

### 5.2 Memory Detail

用户点开一条记忆或 claim 后看到：

- 当前内容和 scope。
- 置信度、重要度、新鲜度、状态。
- 来源证据列表。
- 被它 supersede 的旧记忆。
- 操作：Approve、Edit、Reject、Forget、Move scope、Mark outdated、Pin/Unpin。

### 5.3 用户纠错闭环

任何纠错都写成最高优先级 evidence：

- “不是这样” → 对相关 claim 标 `needs_review`，触发 `user_correction` run。
- “以后都用中文” → 写 Global preference claim。
- “这个只对本项目有效” → move 到 Project scope。
- “忘掉这个” → archive claim + 关联 memory，不硬删 evidence，除非用户选择永久删除。

## 6. API / Transport

新增 API 必须同时有 Tauri + HTTP 适配。

兼容原则：

- 保留现有 `dreaming_run_now` Tauri/transport 命令，作为 `dreaming_run { phase: "light", trigger: "manual" }` 的 alias。
- HTTP `POST /api/dreaming/run` 可以扩展 request body；空 body 继续等价于当前 manual light run。
- 新 API 优先 additive，不破坏 Dashboard 现有 Dreaming Tab。
- 现有 `dreaming_last_report` / `dreaming_idle_status` / `dreaming_is_running` / `dreaming_list_diaries` / `dreaming_read_diary` 保留至少一个 minor；Dashboard 迁移到 run API 后再标 deprecated。

| 命令 | HTTP | 用途 |
|---|---|---|
| `dreaming_run` | `POST /api/dreaming/run` | 扩展 body：`phase`, `scope`, `dryRun` |
| `dreaming_list_runs` | `GET /api/dreaming/runs` | 列运行历史 |
| `dreaming_get_run` | `GET /api/dreaming/runs/{id}` | 读 run + decisions |
| `memory_claim_list` | `GET /api/memory/claims` | 按 scope/status/search 过滤 |
| `memory_claim_get` | `GET /api/memory/claims/{id}` | 读 claim + evidence |
| `memory_claim_update` | `PATCH /api/memory/claims/{id}` | edit / status / scope |
| `memory_claim_forget` | `POST /api/memory/claims/{id}/forget` | archive 或永久删除 |
| `memory_profile_get` | `GET /api/memory/profile` | 当前 scope profile |
| `memory_profile_regenerate` | `POST /api/memory/profile/regenerate` | 手动重建 summary |

事件：

| EventBus | 用途 |
|---|---|
| `dreaming:cycle_started` | Dashboard 显示 running |
| `dreaming:cycle_progress` | 分阶段进度 |
| `dreaming:cycle_complete` | 保留现有事件，payload 扩展 |
| `memory:claim_changed` | Memory Profile / Memory list 局部刷新 |
| `memory:review_required` | Dashboard badge |

## 7. 与现有子系统的关系

### 7.1 `memory_extract.rs`

短期保留现有提取逻辑。新增 `extract_claim_candidates`：

- 旧路径继续写 `MemoryEntry`。
- 新路径写 `memory_claims` + `memory_evidence`。
- 配置开关允许双写，灰度稳定后再把 Dreaming 迁到 claim-first。
- Claim 双写必须消费 `add_with_dedup` 三态：`Created { id }` 关联新 memory id；`Updated { id }` 关联被合并的既有 id；`Duplicate { existing_id }` 关联 existing id，并只补 evidence/link，不重复创建 claim。

### 7.2 `memory/dreaming/`

建议扩展目录：

```text
memory/dreaming/
  pipeline.rs          # orchestration
  scanner.rs           # source collection
  extraction.rs        # claim extraction
  canonicalize.rs      # merge / dedup
  resolver.rs          # temporal + conflict
  profile.rs           # profile synthesis
  decisions.rs         # audit log
  context_pack.rs      # prompt-ready blocks
  eval.rs              # fixtures runner
```

### 7.3 `recap`

Deep run 使用 recap facet 作为长窗口 source，避免每次重读全部 session。

- `recap` 负责“过去发生了什么”的报告。
- `dreaming` 负责“其中哪些改变了长期记忆”的治理。

### 7.4 `awareness`

Awareness 仍偏“最近其他会话在干什么”。Dreaming Profile 偏长期稳定心智。

- Awareness suffix：短期、动态、可能每轮变。
- Memory Profile：长期、慢变、可审核。

### 7.5 `Active Memory`

Active Memory v2 不只从 `memories` 搜索，也从 active claims 搜索。

- 候选：Project claims → Agent claims → Global claims → legacy memories。
- 所有 claim 候选先过 `effective_status` 与 `valid_until` 过滤，避免过期 claim 从 Active Memory 绕回 prompt。
- LLM 选择：返回 1 到 3 条相关 claim，而不是单句自由总结。
- 输出带 source digest，便于回答时提示“基于已保存的项目记忆”。
- 现有 Active Memory 默认关闭，Phase 5 的收益默认只通过被动 Context Pack 体现；是否默认开启 Active Memory v2 需要独立产品决策。

### 7.6 `Context Compact`

Tier 3 compact 前的 `flush_before_compact` 应写 claim candidates，并把压缩前 message refs 作为 evidence。

### 7.7 Session / Evidence 生命周期

Evidence 引用的 session、message、file 可能被删除或压缩。策略：

- Session 删除：关联 `memory_evidence` 转为 `redaction_status=anchor_only`，清空 `quote`，保留 `source_type/session_id` 作为历史锚点；用户选择永久删除时才级联删 evidence。
- Message 因 compact 不再在上下文中：evidence 仍可保留 message id + short quote；quote 已脱敏限长，不依赖原 message 继续存在。
- 文件不存在或越权：evidence 仍显示 source 摘要，但不可展开；不能算作可展开证据。
- 验收中的“active claim 都能追溯来源”至少要求存在一个非 incognito source anchor；可展开 quote 不是必要条件。

## 8. 安全与隐私

### 8.1 Incognito

必须维持现有契约：

- 不提取 memory。
- 不进入 Dreaming scanner。
- 不写 evidence。
- 不进入 Memory Profile。
- 不出现在 Dashboard 统计。

### 8.2 Scope 隔离

默认规则：

- Project session 中的项目事实写 Project scope。
- 用户偏好只有在明显跨项目时才写 Global。
- Agent 行为偏好写 Agent scope，除非用户明确说“所有 Agent 都这样”。
- 移动 scope 必须保留决策日志。

### 8.3 Source Redaction

证据摘录必须限长并脱敏：

- API key、token、cookie、private key 不进 quote。
- 文件路径证据只存 path anchor；HTTP 读取继续走会话/工作目录授权。
- 用户选择 Forget 时，默认 archive；永久删除需要二次确认。
- 跨 scope 查看 evidence 时只展示 source type / timestamp / title；原文 quote 需进入对应 Project/session 上下文后再展开。
- `access_scope_json` 记录 evidence 允许展示的 scope，不能只靠前端过滤。

### 8.4 Prompt Injection 防护

注入 prompt 的 claim content 继续走 `sanitize_for_prompt`。

新增规则：

- evidence quote 默认不进 system prompt。
- `source_type=file/tool_result/url` 的内容不能直接提升为 standing instruction，必须经过 claim extraction 和安全过滤。

## 9. 评测

下一代 Dreaming 必须有离线 eval，不靠感觉。

### 9.1 Golden Fixtures

建立 fixture：

```text
crates/ha-core/tests/fixtures/dreaming/
  user_preferences.json
  project_scope_isolation.json
  temporal_supersede.json
  conflict_resolution.json
  incognito_exclusion.json
  source_evidence.json
```

每个 fixture 包含：

- input sessions / memories / files / tool metadata
- expected claims
- expected statuses
- expected profile summary snippets
- forbidden outputs

### 9.2 指标

| 指标 | 目标 |
|---|---|
| Claim precision | 自动 active claim 中错误率 < 3% |
| Recall relevance | 当前 turn 相关 claim 命中率高于 legacy Active Memory |
| Stale suppression | 已过期事实不进入 prompt |
| Scope leakage | Project A 事实不进入 Project B |
| Evidence coverage | active claim 至少 1 条 evidence |
| Cost | Light 单轮 1 次 side_query；Deep 可配置上限 |
| Latency | 聊天热路径不等待 Deep；Active Memory timeout 可控 |

### 9.3 评测分层

评测分三层，避免 LLM 非确定性让 CI 变脆：

| 层级 | 内容 | 是否进默认 CI |
|---|---|---|
| Deterministic | schema migration、scope filter、incognito exclusion、resolver 状态机、legacy sync | 是 |
| Golden LLM fixtures | claim extraction、profile synthesis、conflict rationale；固定模型或 mock response | 手动 / nightly |
| Human review set | 真实用户样本匿名化后人工标注 precision/recall | release 前抽样 |

`Claim precision < 3%` 只对 human-labeled active claims 统计；deterministic tests 负责守住安全红线。

### 9.4 回归测试

开发期默认单点验证：

- Rust schema / resolver 改动：`cargo check -p ha-core`
- 前端 UI 改动：`pnpm typecheck`
- 需要跑 clippy / cargo test / pnpm test / pnpm lint 时按仓库规则先问用户，除非跨模块收尾。

## 10. 分阶段实施计划

### Phase 0：把 Light 做稳

目标：不改变用户体验，补基础审计和耐久状态。

- 新增 `dreaming_runs` / `dreaming_locks` / `dreaming_decisions`。
- 新增 `dreaming_watermarks` / `dreaming_pending_sources`，保存跨 run 扫描进度和待处理 source。
- Pending source retention + claimed lease recovery。
- `run_cycle` 写 durable run，不只靠进程内 `LAST_REPORT`。
- `DreamReport` 增加 `runId`。
- Dashboard 从 `last_report_snapshot` 迁移到 run API。
- 现有 Dream Diary 继续写。

验收：

- 手动 / idle / cron run 都有 durable 记录。
- 多进程同时触发同一个 lock key 时只有一个 run 执行，其余 skip。
- Deep lock 存在时，Light source capture 进入 pending queue，不丢候选。
- `processed/skipped/claimed` pending source 都能按策略清理或恢复。
- Dashboard 重启后仍能显示最近一次 run。

### Phase 1：Evidence Layer

目标：现有 memory promotion 有来源。

- `memory_extract` 保存 `source_session_id` 已有；补 message-level anchors。
- Dreaming scanner 输出 candidate source refs。
- PromotionRecord 增加 evidence refs。
- Diary markdown 中保留机器可读 evidence 注释。
- Evidence 展示默认只出 source 摘要，展开 quote 走后端授权。

验收：

- 每条 promoted memory 能追到 session/message 或 memory id。
- incognito source 不进入 evidence。

### Phase 2：Claim Schema 双写

目标：在不破坏旧 memory 的前提下开始结构化。

- 新增 `memory_claims` / `memory_evidence` / `memory_claim_links` migration。
- 新表落 memory backend 同库，并加入 agent-id rename migration。
- `memory_extract` 增加 claim extraction prompt，和 legacy memory 双写。
- Light 写入时做规则式粗去重，不等 Deep 才治理。
- 双写消费 `add_with_dedup` 三态，`Updated/Duplicate` 也能正确建立 `memory_claim_links`。
- Claim list/get API + Tauri/HTTP transport。
- Memory 管理页隐藏开关显示 Claims beta。
- Claim 状态变化会同步关联 legacy memory 的 prompt visibility。

验收：

- 新对话能产生 legacy memory + claim。
- Claim 有 scope、confidence、salience、evidence。
- Superseded claim 不会通过 linked legacy memory 继续注入 prompt。
- Embedding disabled 时 canonicalize 仍能 FTS/rule-only 工作。
- `evidence_class` 到 confidence baseline 的映射有 deterministic test。

### Phase 2.5：Existing Memory Backfill

目标：老用户已有 memory 也进入 claim 世界，但不制造惊吓。

- 从现有 `memories` 批量生成 claim candidates。
- `pinned` memory 默认高 salience，但不自动覆盖新 claim。
- 无 source_session_id 的 memory 生成 `source_type=memory` evidence。
- 首次 backfill 支持 dry-run，结果进 review queue；低风险偏好可自动 active。

验收：

- 旧 memory 能生成 claims 和 links。
- Backfill 不改变用户现有 prompt 注入，除非用户确认或规则判定低风险。

### Phase 3：Deep Consolidation

目标：真正超越 Light：合并、冲突、过期。

- 新增 `canonicalize.rs`。
- 新增 `resolver.rs`。
- Deep scanner 使用 session watermark + recap facets。
- 支持 `supersede` / `expired` / `needs_review`。
- 只处理 Light 无法确定的合并、冲突、split 和长窗口过期，不承担基础去重。
- 不自动硬删。

验收：

- fixture 中“旧事实被新事实替换”通过。
- Project scope 隔离通过。

### Phase 4：Memory Profile

目标：用户能看见“它认为它知道什么”。

- 新增 `memory_profile_snapshots`。
- Profile synthesis 只基于 effective active claims；idle/light 规则式拼接生成轻量 snapshot（不烧 side_query），Deep 负责 LLM 长窗口重写。
- Snapshot version 有唯一约束和并发重试。
- 现有 profile-tagged memories 只作为 claim/evidence 来源，不再直接生成第二套 `## User Profile`。
- Snapshot 不存在、过期或 Profile synthesis 关闭时，继续 fallback 到 legacy profile-tagged memories。
- Settings / Dashboard 展示 Global / Agent / Project profile。
- 用户可 edit / reject profile 下钻到 claim。

验收：

- Profile 可解释，每条摘要可追到 claims。
- Deep 关闭时 `## User Profile` 仍有 legacy fallback，不会空白。
- 用户 edit 后生成 correction evidence。

### Phase 5：Context Pack + Active Memory v2

目标：让整理结果真正提升聊天质量。

- `context_pack.rs` 生成 prompt-ready blocks。
- Provider adapter 增加 `dynamic_context_blocks` 合约和单 prompt fallback。
- `system_prompt::build_memory_section` 接入 Profile + Claims。
- Active Memory 候选源扩展到 claims。
- claim 与 linked legacy memory 单一来源去重（被覆盖的 legacy memory 不在 SQLite memory 段重复注入），Profile / Claims 新段纳入 `effective_memory_budget` 预算池。
- Provider adapter 继续独立 cache block 注入。

验收：

- 相关项目事实比 legacy memory 更稳定进入 prompt。
- 过期 claim 不注入。
- 被 active claim 覆盖的 linked legacy memory 不在 legacy 段重复注入；Profile / Claims 段受 `effective_memory_budget` 约束。

### Phase 6：Lucid Review UI

目标：把控制权交给用户。

- Dashboard Dreaming Center。
- Needs Review 队列。
- Claim detail 证据抽屉。
- Approve / Edit / Reject / Forget / Move scope。

验收：

- 用户能修正错误记忆，修正会影响下一轮 prompt。
- 所有用户操作有 decision log。

### Phase 7：Agentic Dreaming

目标：不仅记住，还主动沉淀可复用能力。

候选输出：

- Skill suggestion：多次重复操作 → 建议生成 skill draft。
- Cron suggestion：周期性任务 → 建议创建 cron。
- Project risk：多次失败/冲突 → 标记项目风险。
- TODO carryover：用户明确未完成事项 → 放入 project/task context，而不是普通 memory。

约束：

- 只建议，不自动创建 skill/cron/task。
- 走 `ask_user_question` 或 Dashboard review。

验收：

- 至少一个 fixture 能从重复行为中生成 skill suggestion。
- 用户拒绝后不会反复提示同一 suggestion。

## 11. 验收总标准

下一代 Dreaming 进入默认开启前必须满足：

- 任意 active claim 都能追溯来源。
- 用户可以关闭、暂停、手动运行、查看、纠正、忘记。
- 无痕会话零泄漏。
- Project scope 不互串。
- 新表参与 agent-id rename migration。
- memory backend embedding 关闭时仍可运行 Light claim 去重。
- 已 superseded / expired 的 claim 不进入 prompt。
- 已 superseded / expired 的 linked legacy memory 不会以 pinned 形式绕回 prompt。
- `valid_until` 读取时过期会同步影响 linked legacy memory 和 Active Memory v2 召回。
- Context Pack 启用后，被 active claim 覆盖的 linked legacy memory 只走 Claims 段、不在 legacy 段重复注入（单一来源）。
- `sync_mode=user_pinned` 的 linked memory 不会被 effective_status 自动隐藏，但必须进入 review queue。
- Profile Snapshot 不存在或 Deep 关闭时，legacy profile fallback 保持可用。
- Evidence 展开经过后端授权，不能只靠前端隐藏。
- 多进程 Deep run 有 lease 保护，崩溃后可恢复。
- Pending source 有 retention/lease recovery，不无限增长、不永久卡 claimed。
- 深层整理失败不影响聊天。
- 至少 6 类 golden fixture 通过。
- Dashboard 能解释“本轮改了什么、为什么改、来自哪里”。

## 12. 开放问题

| 问题 | 倾向 |
|---|---|
| Claims 是否替代 MemoryEntry？ | 不替代，至少两个 minor 版本双轨运行 |
| 是否允许自动 unpin？ | 允许，但只在高置信 supersede/expired 时；否则 needs_review |
| 是否默认跑 Deep？ | 初期只手动 / cron opt-in；Light 默认 |
| 本地模型是否默认用于 Deep？ | 可选优先，不强制默认；claim JSON 必须 schema 校验 + 重试，结构化失败则降级到云端/配置模型或 needs_review |
| Evidence quote 保存多长？ | 默认 300-500 chars，敏感信息脱敏 |
| 用户 Forget 是否物理删除 evidence？ | 默认 archive；提供永久删除高级操作 |
| `memory_claim_links` 是否允许多对多？ | 允许；一个 broad memory 可拆多个 claim，一个 claim 可合并多条 memory |
| Deep lock key 粒度 | Light 按 scope 并发；Deep 初期全局串行，后续再按 Project 并发 |

## 13. 首批 PR 建议

1. `dreaming_runs` + `dreaming_locks` + `dreaming_watermarks` + `dreaming_pending_sources` + `dreaming_decisions` durable audit + pending GC。
2. Promotion evidence refs + Diary 机器可读注释 + evidence 授权读取。
3. Claim schema migration + `memory_claim_links` + agent rename migration + claim list/get API。
4. `memory_extract` claim 双写 + `add_with_dedup` 三态 link + Light rule-only dedup + linked legacy memory 同步协议。
5. Existing memory backfill dry-run + review queue。
6. Deep resolver MVP：merge / supersede / expired。
7. Memory Profile snapshot + legacy profile-tagged memory fallback 下线 + Dashboard read-only view。
8. Context Pack provider 合约 + Active Memory v2 从 claims 召回。
9. Review UI 和用户纠错闭环。
10. Deterministic eval + golden fixtures。

## 14. 一句话定位

现有 Dreaming 是“睡一觉，把最近值得记住的东西 pin 住”。下一代 Dreaming 要变成“有证据、有层级、有自我纠错、有用户审阅、有评测的本地长期心智系统”。

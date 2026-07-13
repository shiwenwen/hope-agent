# Memory UX v2：核心记忆、动态召回与学习控制改造路线

> 状态：实施中 RFC  
> 目标分支：`feat/memory-ux-v2`  
> 基线提交：`3e31b2fb1 feat(memory): add progressive project auto memory`  
> 创建日期：2026-07-13  
> 完成后归档：将稳定契约合并回 `docs/architecture/memory.md`、`dreaming.md`、`prompt-system.md`、`project.md` 和 `agent-config.md`，本路线文档转入外部 Plans 归档。

## 1. 执行摘要

Hope Agent 已具备成熟的长期记忆底层：Global / Agent / Project 三级作用域、Markdown Core Memory、SQLite、FTS5、trigram、vec0、RRF/MMR、自动提取、Claims / Evidence、Profile、Dreaming、Procedure、Graph、外部 Provider、审计、备份和可解释 trace。

当前主要问题不是能力不足，而是产品心智和运行时边界没有收敛：

- Global / Agent 旧版小写 `memory.md`、Profile、Pinned Claims 和大量 SQLite 记忆共同进入静态 system prompt。
- Project Auto Memory 已使用 `MEMORY.md` 索引 + topic files 渐进加载，但尚未与 Global / Agent 和统一 Memory Budget 合并。
- Active Memory、Memory Selection、Recall Summary、Procedure / Graph recall 分别存在，普通用户很难理解它们之间的关系。
- “是否使用已有记忆”和“是否从当前对话学习”没有彻底分开。
- system prompt 虽然每轮重建，但其静态记忆输入可能在后台更新后变化，造成 prompt cache 前缀失效。

Memory UX v2 将用户可见能力收敛为三个概念：

```text
始终记住      Markdown Core Memory      会话级稳定前缀
相关时想起    Long-term Memory Store    每轮动态后缀
从对话中学习  Learning Pipeline         只控制新记忆产生
```

底层现有能力全部保留。改造的重点是重新划分注入位置、统一调度、简化配置和 GUI，不删除任何记忆资产或安全边界。

## 2. 目标、非目标与硬性不变量

### 2.1 产品目标

1. 普通用户只需理解“始终记住”“相关时想起”“从对话中学习”。
2. 静态记忆短小、稳定、可审计，长期积累不会无限推高首轮上下文。
3. 动态记忆在需要时高质量召回，无关回合不注入。
4. 默认召回不依赖额外 LLM，避免逐轮增加延迟、token 和失败面。
5. 学习、使用和深度召回是三个正交控制面。
6. Global / Agent / Project 使用同一套渐进式 Markdown 结构与预算协议。
7. Prompt Cache 稳定前缀在会话内保持字节级一致。
8. 用户始终可以查看、修改、移动 scope、提升、降级和忘记记忆。

### 2.2 技术目标

- 标准空会话的 Core Memory 静态注入目标不超过 1,600 token，硬上限默认 2,400 token。
- 动态召回默认最多 5 条、合计不超过 800 token。
- 简单寒暄、确认、感谢类输入动态召回为 0 条。
- 至少 90% 的普通回合不产生额外记忆 LLM 调用。
- 本地动态召回 P95 不超过 100ms；超过预算 fail-soft，不阻塞主回答。
- 后台学习、Dreaming、Profile 更新不改变进行中会话的 stable prompt fingerprint。
- 热缓存下 cacheable stable tokens 的 cache read ratio 目标不低于 80%。
- 所有 Provider 使用同一上层语义，差异只存在于 wire-level cache breakpoint 和消息形态。

### 2.3 非目标

- 不重写 SQLite / FTS / vec0 / Claims / Dreaming 的底层存储引擎。
- 不删除 legacy memories、Profile snapshots、Claims、Evidence、Episodes、Procedures 或 Graph 数据。
- 不把 Markdown Memory 变成权限或安全策略。强制规则仍属于 Agent instructions、AGENTS.md、Permission Engine 或 Hooks。
- 不让外部 Memory Provider 成为本地执行的强依赖。
- 不在第一阶段追求自动修改 Core Memory 的完全自治。
- 不把 Prompt Cache 误认为减少上下文窗口占用；它只减少重复计算、成本和 TTFT。

### 2.4 硬性不变量

```text
MemoryVisible(turn)
  = CoreMemorySnapshot(session)
  + DynamicRecall(turn)
  + ExplicitConversationMemory(turn)

DynamicRecall(turn) <= EffectiveEligibleMemory(session, turn)

Incognito => CoreMemorySnapshot = empty
          && DynamicRecall = empty
          && Learning = disabled

ReviewFirstCandidate.status != approved
  => candidate must not be visible to any agent prompt path
```

其他不可破坏的契约：

- Scope 优先级和冲突覆盖为 Project > Agent > Global。
- 实时权限、Incognito、Project 绑定、Agent shared 配置、Memory master switch 在每轮执行前重新裁决。
- Snapshot 只冻结内容，不冻结权限；权限撤销必须立即让该层从下一轮消失。
- 所有动态文本进入 prompt 前继续执行 `sanitize_for_prompt` 和 untrusted-data 包装。
- `recall_memory` / `memory_get` 仍可在通过权限和 scope 检查后返回完整原文。
- Owner 平面管理能力不因 Agent memory off 而消失。

## 3. 统一术语与用户心智

| 用户术语 | 内部术语 | 是否固定进入 Prompt | 主要存储 |
|---|---|---:|---|
| 始终记住 | Core Memory | 是，会话级快照 | `MEMORY.md` + `topics/*.md` |
| 相关时想起 | Dynamic Recall | 否，按 turn 选择 | SQLite / Claims / Profile / Procedure / Graph |
| 从对话中学习 | Learning Pipeline | 不直接注入 | candidates / memories / claims / evidence |
| 重要 | Recall Boost | 否，只提升召回权重 | pinned/salience/priority metadata |
| 始终记住这条 | Promote to Core | 是，刷新后生效 | Core Memory repository |
| 深度召回 | LLM Rerank / Distill | 否，默认关闭 | side query 临时结果 |

必须在 UI、文档和日志里停止混用下列概念：

- `pinned` 不再天然意味着静态注入。
- `active memory` 不再表示整个动态记忆体系，只保留为兼容字段或内部迁移名。
- `memory learning mode` 不再承担 memory master switch 的职责。
- `profile snapshot` 是动态召回来源，不是默认静态人格块。

## 4. 目标运行时架构

```text
                          ┌──────────────────────────┐
Global / Agent / Project  │ CoreMemoryRepository     │
MEMORY.md + topics ──────▶│ index + topic + revision │
                          └────────────┬─────────────┘
                                       │ session start / reload / compact
                                       ▼
                          ┌──────────────────────────┐
                          │ CoreMemorySnapshot       │
                          │ stable, token-bounded    │
                          └────────────┬─────────────┘
                                       │ stable prefix
                                       ▼
User turn ──▶ Intent Gate ──▶ Hybrid Retrieval ──▶ Deterministic Ranker
                 │                   │                       │
                 │ skip              │ memories/claims/...  │ ambiguous only
                 ▼                   ▼                       ▼
              no recall       DynamicRecallPack       optional LLM rerank
                                      │
                                      │ dynamic suffix after stable boundary
                                      ▼
                                Provider Request

Conversation completion ──▶ Learning Gate ──▶ Candidate / Review / Store
                                                   │
                                                   └─▶ Promote-to-Core proposal
```

新增统一编排对象：

```rust
pub struct MemoryContextPlan {
    pub core: CoreMemorySnapshot,
    pub recall: Option<DynamicRecallPack>,
    pub explicit_turn_updates: Vec<ExplicitMemoryNotice>,
    pub manifest: MemoryContextManifest,
}
```

职责边界：

- `CoreMemoryRepository`：文件路径、索引、topic、原子写、revision、迁移。
- `CoreMemorySnapshot`：会话冻结内容、scope、token、hash、加载时间。
- `MemoryRecallPlanner`：意图门控、候选融合、预算、排序、可选深度召回。
- `MemoryLearningPolicy`：学习模式、scope 路由、审核和提升建议。
- `MemoryContextManifest`：只记录 hash、长度、token、来源、延迟和裁决原因，不记录敏感原文。

## 5. Core Memory：三层渐进式 Markdown

### 5.1 文件布局

```text
~/.hope-agent/memory/
  MEMORY.md
  topics/
    identity.md
    preferences.md

~/.hope-agent/agents/{agent_id}/memory/
  MEMORY.md
  topics/
    workflow.md
    communication.md

~/.hope-agent/projects/{project_id}/memory/
  MEMORY.md
  topics/
    architecture.md
    commands.md
```

`MEMORY.md` 是三个 scope 的最终 canonical 文件名。现有小写文件不能通过
简单的 case-only rename 或“首次写入覆盖”迁移，必须由
`CoreMemoryRepository` 统一解析：

| Scope | Legacy 路径 | V2 canonical 路径 |
|---|---|---|
| Global | `~/.hope-agent/memory.md` | `~/.hope-agent/memory/MEMORY.md` |
| Agent | `~/.hope-agent/agents/{id}/memory.md` | `~/.hope-agent/agents/{id}/memory/MEMORY.md` |
| Project | 无已发布 legacy 路径 | `~/.hope-agent/projects/{id}/memory/MEMORY.md` |

Project Auto Memory 当前大写路径保持不变，仅把实现移动到通用 repository。
Global / Agent 按“先兼容、后收口”的迁移协议切换到大写；任何 Tauri、HTTP、
Agent loader、tool、import、backup 或 restore 入口都不得自行拼接文件名。

### 5.1.1 小写到大写的兼容迁移

迁移由 ha-core 的 `CoreMemoryPathResolver` 和 `CoreMemoryMigrationManifest`
单点负责。Resolver 必须枚举目录中的真实文件名，不能只依赖
`Path::exists()` 判断大小写，因为 macOS 默认大小写不敏感，而 Linux / Server
通常大小写敏感。

每个 scope 的磁盘状态按以下矩阵处理：

| 状态 | 读取 | 写入 | 处理 |
|---|---|---|---|
| 两者都不存在 | 空 | canonical 大写 | 创建新结构 |
| 只有 legacy 小写 | 小写 | 进入迁移事务 | 备份、复制、校验、建立双写基线 |
| 只有 canonical 大写 | 大写 | 大写 | 正常 V2 |
| 两者都存在且 hash 相同 | 大写 | 双写兼容 | 记录同步 revision |
| 两者都存在且仅小写相对 manifest 变化 | 小写 | 同步到大写后双写 | 兼容旧版本写入 |
| 两者都存在且仅大写相对 manifest 变化 | 大写 | 同步到小写后双写 | 兼容新版本写入 |
| 两者都变化且内容不同 | 最近一次已同步版本 | 暂停自动覆盖 | 标记 conflict，要求用户选择/合并 |

迁移事务：

1. 获取 scope 级跨进程 advisory lock。
2. 重新读取 legacy / canonical / manifest，避免 TOCTOU。
3. 为 legacy 创建带 timestamp 和 BLAKE3 hash 的迁移备份。
4. 将内容原子写入 canonical 大写路径并 fsync。
5. 回读 canonical，验证 raw BLAKE3 与源内容一致。
6. 在兼容窗口内把 legacy 保留为同步镜像。
7. 最后原子写入 migration manifest；manifest 是两份内容上次同步 hash 的真相源。
8. 任一步失败都保留 legacy 和备份，不得删除唯一可读副本。

兼容窗口至少持续一个 minor 版本：

- V2 的所有 Core Memory 写入在同一锁内依次写 canonical、legacy mirror、manifest。
- canonical 写成功但 mirror/manifest 失败时，以 canonical 为本次提交结果，Health 标记
  `legacy_mirror_stale` 并在下次启动修复；不得反向覆盖已成功的新内容。
- 用户降级到旧版本后，旧版本仍可读取和写入小写文件；重新升级时 resolver 根据
  manifest 识别“仅 legacy 变化”并安全同步回大写。
- 两边都在上次同步后发生变化时绝不按 mtime 猜测，也不静默合并；Memory Center
  显示 diff/merge 入口，Agent runtime 暂用 manifest 对应的最后已同步版本。
- 兼容窗口结束后停止双写，但小写只读 fallback 和迁移备份读取能力至少再保留一个
  minor；最终写入真相源只剩大写 `MEMORY.md`。

迁移 manifest 建议存放：

```text
~/.hope-agent/memory/migrations/core-memory-v2.json
```

只记录 scope、legacy/canonical path、hash、revision、同步时间和状态，不记录记忆原文。

### 5.2 `MEMORY.md` 内容契约

`MEMORY.md` 是短索引，不是无限增长的事实列表。推荐格式：

```markdown
# Core Memory

- 偏好使用中文、先给结论，再给必要细节。
- 修改代码时优先做针对性检查，不主动重复跑全套门禁。

## Topics

- [沟通偏好](topics/communication.md)：语气、输出结构和解释深度
- [开发工作流](topics/workflow.md)：分支、测试和提交习惯
```

约束：

- 索引每条只表达一个稳定事实或指向一个 topic。
- 临时任务状态、一次性错误、短期计划不得进入 Core Memory。
- 密钥、token、认证头、私人原文证据不得进入 Core Memory。
- 自动学习默认只能生成提升建议，不能静默覆盖用户维护的索引。
- 显式“始终记住”可直接写入；发生冲突时进入 Review，而不是覆盖旧事实。

### 5.3 Token Budget

新增配置：

```json
{
  "coreMemory": {
    "totalTokens": 1600,
    "hardMaxTokens": 2400,
    "globalTokens": 350,
    "agentTokens": 450,
    "projectTokens": 650,
    "protocolTokens": 150,
    "topicReadMaxTokens": 800
  }
}
```

预算规则：

1. 使用 Provider-aware token estimator；不可用时使用带 10% 余量的上界。
2. 每层先获得自己的保底额度，未使用额度进入共享池。
3. 共享池按 Project > Agent > Global 分配。
4. 发生截断时按完整 Markdown 条目裁剪，不从 UTF-8 字符或链接中间截断。
5. 标题、scope 协议和冲突优先级说明计入 `protocolTokens`。
6. 当前 Project 的 200 行 / 25KB 保留为文件读取安全上限，不再表示 Prompt 注入预算。

### 5.4 Session Snapshot

新增 `CoreMemorySnapshot`：

```rust
pub struct CoreMemorySnapshot {
    pub global: Option<CoreLayerSnapshot>,
    pub agent: Option<CoreLayerSnapshot>,
    pub project: Option<CoreLayerSnapshot>,
    pub rendered: String,
    pub estimated_tokens: u32,
    pub fingerprint: String,
    pub created_at: String,
}
```

生命周期：

- Session 首次 turn 创建。
- 普通 API round 复用，不重新读磁盘。
- `/clear`、Tier 3 compact、新 session、显式 reload 重新创建。
- 用户在当前会话显式写入 Core Memory 时：
  - 磁盘立即持久化；
  - 当前 turn 通过动态 `ExplicitMemoryNotice` 告知模型；
  - 静态 snapshot 默认不变；
  - 下次 reload/compact/session 生效。
- 权限、Incognito 或 Project 解绑变化时，不得继续复用不再 eligible 的 layer。

### 5.5 统一工具与兼容入口

新增 canonical 工具 `core_memory`：

```text
list(scope)
read(scope, path?, offset?, limit?)
search(scope, query)
write(scope, name, description, content, expected_hash?)
delete(scope, path, expected_hash?)
promote(memory_id|claim_id, scope, topic?)
reload(session_id)
```

兼容策略：

- `update_core_memory` 保留并映射到 `core_memory.write`。
- `project_memory` 保留并映射到 `scope=project`。
- 权限、审批、审计和 hook 记录 canonical action，同时保留原工具名来源。
- Project scope 必须从 live session 解析；不得接受模型伪造 project id 绕过绑定。

## 6. Dynamic Recall：长期记忆按需进入

### 6.1 退出静态 Prompt 的来源

下列来源不再默认进入 `build_memory_section`：

- legacy SQLite memories；
- Profile Snapshot；
- 自动 high-salience / pinned Claims；
- Episodes；
- Procedures；
- Graph edges / neighborhood；
- 外部 Provider 同步的长期条目。

这些数据继续作为 `MemoryRecallPlanner` 的候选源。用户显式提升为 Core 的内容才进入稳定 prefix。

### 6.2 意图门控

召回前先做零 LLM 的确定性 gate：

`Skip` 示例：

- 寒暄：hi、你好、早上好；
- 简单确认：好的、继续、可以；
- 感谢或结束语；
- 只依赖当前消息即可完成的短指令；
- Incognito、memory off、agent memory disabled。

`Recall` 示例：

- “按我平时的方式……”；
- “上次我们怎么处理的……”；
- 涉及用户偏好、个人背景、项目历史、已做决策；
- 当前 Project 中的架构、命令、故障历史；
- query 包含候选记忆的稀疏精确词或实体。

Gate 输出必须进入 manifest：`skip_reason`、`intent`、`query_terms_hash`，不保存原 query。

### 6.3 候选检索

默认并行查询：

- legacy memory：FTS5 + trigram + vec0；
- claims：FTS5 + trigram + vec0，只取 effective-active；
- profile：按 scope 生成候选行，不整块注入；
- procedure：仅 procedure intent；
- graph：以已命中的实体为中心 bounded expand；
- external provider：仅在配置允许且本地 deadline 内返回时参与。

默认参数：

```json
{
  "candidateLimitPerSource": 8,
  "candidateLimitTotal": 24,
  "maxSelected": 5,
  "maxTokens": 800,
  "retrievalTimeoutMs": 100,
  "graphExpansionMaxEdges": 6
}
```

### 6.4 确定性排序

推荐 score：

```text
finalScore =
  0.34 * retrievalScore
  + 0.18 * scopeScore
  + 0.14 * intentScore
  + 0.12 * confidence
  + 0.10 * salience
  + 0.06 * recency
  + 0.06 * explicitPriority
  - duplicatePenalty
  - contradictionPenalty
```

要求：

- Project > Agent > Global 是 scope 权重和冲突覆盖，不得仅依赖列表加载顺序。
- 稀疏精确命中不得被默认高权重向量结果挤出。
- 同一 canonical fact 的 memory / managed claim / profile 投影必须去重。
- 用户纠正和 manual correction evidence 优先于自动提取。
- 过期、superseded、archived、needs_review 不进入可注入集合。
- 排序 tie-break 必须稳定：scope、source rank、id。

### 6.5 深度召回

当前 Active Memory 的 LLM side query 调整为可选 `deepRecall`：

- 默认关闭。
- 只在以下条件之一满足时运行：
  - 用户显式选择深度召回；
  - top candidates 分差低于 ambiguity threshold；
  - 用户询问模糊历史且确定性检索得到多个冲突候选；
  - 需要将多条记忆压缩成单一 bounded insight。
- 沿用 side-query cache prefix，输出仍受 timeout 和 max chars 限制。
- LLM 失败时退化为确定性 Top-K，不退化为全量静态注入。

### 6.6 Dynamic Recall Pack

```rust
pub struct DynamicRecallPack {
    pub rendered: String,
    pub selected: Vec<UsedMemoryRef>,
    pub considered_count: usize,
    pub estimated_tokens: u32,
    pub latency_ms: u64,
    pub mode: RecallMode, // fast | deep | cached
}
```

Provider 注入契约：

- 作为独立 dynamic suffix，位于稳定 system/core prefix 之后。
- Anthropic：使用独立动态 system block，不改变前序 cache breakpoint。
- OpenAI Responses / Chat：保持固定 stable items，相关记忆追加到尾部。
- Codex：按已探测 wire capability 渲染，不假设 Responses 特性。
- Failover 只携带规范化 pack 和 refs，由新 Provider 重新渲染。

## 7. Learning：使用与生成正交

### 7.1 顶层控制

```json
{
  "memory": {
    "enabled": true,
    "recall": {
      "enabled": true,
      "mode": "fast"
    },
    "learning": {
      "mode": "smart"
    },
    "deepRecall": {
      "enabled": false
    }
  }
}
```

语义：

- `memory.enabled=false`：Agent 平面读取、召回、学习和 Memory tools 全部关闭；Owner 管理面保留。
- `recall.enabled=false`：不做动态召回，但仍可使用 Core Memory。
- `learning.mode=manual`：不自动产生候选；显式保存仍可用。
- `deepRecall.enabled=false`：不调用额外 LLM，不影响 fast recall。

### 7.2 学习模式

| 模式 | 自动提取 | 写入动态库 | 进入 Review | 自动改 Core |
|---|---:|---:|---:|---:|
| `smart` 推荐 | 是 | 高置信、低风险 | 冲突、敏感、scope 不确定 | 否，只提议 |
| `review_first` | 是 | 批准后 | 全部 | 否 |
| `manual` | 否 | 仅显式保存 | 视显式操作而定 | 仅显式操作 |

Memory Off 不再伪装成学习模式，它是独立 master switch。

### 7.3 Per-session 控制

Session 新增：

```text
use_memories: inherit | allow | deny
contribute_to_memories: inherit | allow | deny
```

- `use_memories=deny`：该会话不加载 Core，不做动态召回，但不删除已有数据。
- `contribute_to_memories=deny`：该会话不参与自动提取、Dreaming source、Profile synthesis。
- 两者互不影响。
- Incognito 强制两者 deny，且不可被 per-session override 放宽。

### 7.4 Scope 路由

```text
project session + project fact       -> Project
project session + universal user fact -> Agent/Global candidate, usually Review
non-project + agent-specific habit   -> Agent
non-project + universal preference   -> Agent by default, Global promotion requires confidence/user action
non-project + project-like fact      -> Unassigned, never Agent static fallback
```

新增 `unassigned_memory_candidates` 或等价 pending scope：

- 保存内容、来源 session、建议 scope、置信度和理由。
- 不参与 Prompt 和普通召回。
- 用户可选择 Project / Agent / Global 或删除。
- 若之后同一 session 被绑定到 Project，可提供批量归属建议，但不得静默迁移已批准的 Agent/Global 数据。

### 7.5 Core 提升

动态记忆满足下列条件时可产生提升建议：

- 用户明确说“始终记住”；
- 同一事实在多个独立会话被重复确认；
- 长期稳定偏好，高 confidence + high salience；
- 用户将动态记忆手动标记为“始终记住”。

自动流程不能提升：

- 临时任务状态；
- 未确认推断；
- 冲突事实；
- 外部不可信文本；
- secret / credential / auth material；
- 仅单次工具输出或网页内容。

## 8. Prompt 与 Cache 契约

### 8.1 PromptEnvelope

Memory 在统一 PromptEnvelope 中的位置：

```text
stableCore
  identity / safety / tool protocol
  agent instructions
  CoreMemorySnapshot (Global -> Agent -> Project)

sessionStatic
  project rules / working directory contract

turnDynamic
  permission / awareness
  DynamicRecallPack
  procedure / knowledge recall
  task / hook reminders
```

`DynamicRecallPack` 不得拼回 stable system string 后再计算 stable fingerprint。

### 8.2 Cache key

建议稳定 key：

```text
provider + model + agent + promptContractVersion + projectFingerprint
```

- 不把 turnDynamic、recall ids、profile version 或当前 history hash放入 key。
- Incognito 使用 session 随机隔离 key。
- Core snapshot 内容变化自然改变请求前缀；key 只用于稳定路由，不代替 exact-prefix matching。
- 后端不支持 `prompt_cache_key` 时按现有负能力缓存和单次重试契约降级。

### 8.3 Manifest

`MemoryContextManifest` 字段：

```text
session_id_hash
core_snapshot_fingerprint
core global/agent/project tokens + bytes + revision
recall enabled/mode/intent/skip_reason
candidate counts per source
selected count + tokens
retrieval/deep-recall latency
learning mode + session contribute policy
scope rejection counts
stable prefix fingerprint
dynamic suffix fingerprint
```

只记录长度、hash、枚举和计数，不记录原文、query、embedding 或 evidence quote。

## 9. GUI / UX 信息架构

### 9.1 Memory Overview 普通模式

只显示三张主卡：

1. **始终记住**
   - Global / Agent / Project 三个 scope。
   - 显示 token 用量、索引条数、topic 数、最后更新时间。
   - 操作：查看、编辑、添加、重新加载、提升建议。

2. **相关时想起**
   - 主开关。
   - 文案明确：“默认使用本地快速检索，不额外调用模型。”
   - 最近召回：命中条数、耗时、来源 scope。
   - 深度召回作为次级开关，明确延迟/token 影响，默认关闭。

3. **从对话中学习**
   - 智能学习 / 先审核 / 仅手动。
   - 展示待审核数量、未归属数量、最近学习时间。

### 9.2 Advanced Memory Engine

折叠保留：

- Embedding Provider；
- Hybrid Search / RRF / MMR / temporal decay；
- Claims / Evidence；
- Dreaming / Resolver / Profile synthesis；
- Procedure / Graph；
- External Providers；
- Backup / Health / Repair；
- token、candidate、timeout 等细粒度预算。

高级配置继续可用，但普通用户不需要理解其依赖关系。

### 9.3 回答下方 Memory Trace

默认摘要：

```text
已使用记忆：核心 3 层 · 相关记忆 2 条
```

展开后显示：

- 内容预览；
- Global / Agent / Project scope；
- `core` / `recalled` / `candidate`；
- 为什么命中；
- 编辑、忘记、移动 scope、提升为 Core；
- deep / fast / cached 模式和耗时。

未注入的候选默认不显示在普通 UI，诊断模式可查看。

### 9.4 Session 控制

会话菜单增加：

- “本对话使用已有记忆”；
- “允许本对话帮助未来记忆”。

必须显示继承来源：Global、Agent override 或 Session override。

## 10. 配置迁移

### 10.1 新配置草案

```json
{
  "memory": {
    "enabled": true,
    "core": {
      "enabled": true,
      "totalTokens": 1600,
      "hardMaxTokens": 2400,
      "globalTokens": 350,
      "agentTokens": 450,
      "projectTokens": 650,
      "topicReadMaxTokens": 800
    },
    "recall": {
      "enabled": true,
      "mode": "fast",
      "maxTokens": 800,
      "maxSelected": 5,
      "candidateLimit": 24,
      "timeoutMs": 100,
      "includeClaims": true,
      "includeProfile": true,
      "includeProcedures": true,
      "includeGraph": true
    },
    "deepRecall": {
      "enabled": false,
      "timeoutMs": 4500,
      "cacheTtlSecs": 60
    },
    "learning": {
      "mode": "smart",
      "promoteCoreAutomatically": false
    },
    "compatibility": {
      "legacyStaticMemory": false
    }
  }
}
```

Agent override 只覆盖以下内容：

- `enabled`；
- 是否共享 Global；
- recall enabled/mode/budget；
- deepRecall；
- learning mode；
- Core Agent layer budget。

### 10.2 旧字段迁移

| 旧字段 | 新字段 | 规则 |
|---|---|---|
| `memoryExtract.enabled` | `memory.enabled` | false 映射 master off |
| `autoExtract` + `flushBeforeCompact` | `learning.mode` | 都 false → manual；否则结合 reviewFirst |
| `reviewFirst` | `learning.mode=review_first` | 优先于 automatic |
| `activeMemory.enabled` | `deepRecall.enabled` | 保持用户显式选择 |
| `activeMemory.candidateLimit` | `recall.candidateLimit` | 钳值后迁移 |
| `activeMemory.maxChars` | `deepRecall.maxChars` | 兼容保留一个 minor |
| `memorySelection.enabled` | `deepRecall.enabled` | 若任一旧语义启用，迁移后开启并提示 |
| `memoryBudget.totalChars` | `core.totalTokens` | 通过校准估算，保存 legacy 备份 |
| `sqliteSections` | 无普通配置 | 只在 compatibility 模式继续消费 |
| `procedureMemory.enabled` | `recall.includeProcedures` | 保持值 |
| `graphMemory.enabled` | `recall.includeGraph` | 保持值 |

迁移要求：

- 幂等，重复启动不重复改写。
- 迁移前备份原 config 和 memory files。
- Global / Agent 小写 `memory.md` 按 §5.1.1 的 manifest 双写协议迁移到大写
  `MEMORY.md`，不得直接删除或 case-only rename。
- 所有读写入口先收敛到 `CoreMemoryPathResolver`，再开启物理迁移。
- 兼容窗口内支持“旧版本只修改小写文件”后重新升级，不丢失该次修改。
- 双文件都发生变化时进入显式冲突态，不按 mtime 自动选边。
- UI 首次展示迁移摘要，不强迫用户理解旧字段。
- 旧字段至少一个 minor 版本继续反序列化。
- 回滚 legacy 时使用原字段快照，不从新字段反推覆盖用户旧值。

## 11. API、Transport 与事件

### 11.1 Owner API

新增或统一：

```text
GET    /api/memory/core?scope=&agentId=&projectId=
GET    /api/memory/core/files/*
PUT    /api/memory/core/files/*
DELETE /api/memory/core/files/*
POST   /api/memory/core/promote
POST   /api/memory/core/reload

GET    /api/memory/settings/effective?agentId=&sessionId=
PATCH  /api/memory/settings
PATCH  /api/sessions/{id}/memory-policy

GET    /api/memory/unassigned
POST   /api/memory/unassigned/{id}/assign
DELETE /api/memory/unassigned/{id}
```

所有新 owner endpoint 必须同时提供 Tauri command、HTTP route 和前端 Transport 映射。

### 11.2 EventBus

```text
memory:core_changed
memory:core_snapshot_reloaded
memory:recall_completed
memory:learning_candidate_created
memory:unassigned_created
memory:promotion_proposed
memory:policy_changed
```

事件不携带完整敏感原文；UI 需要内容时走 owner read API。

## 12. 分阶段实施路线

每个阶段以可独立合并、可回滚、能力不丢失为原则。不得在新链路未验证前直接删除旧链路。

### Phase 0：基线、真相源与 Feature Flag

目标：先能证明当前每轮到底注入了什么，再改变行为。

任务：

- 增加 `MemoryContextManifest`，接入现有 `RoundTokenManifest`。
- 对 Core、SQLite、Profile、Pinned Claims、Active、Procedure、Graph 分项统计。
- 建立 feature flags：
  - `memoryUxV2.enabled=false`；
  - `memoryUxV2.dynamicRecall=false`；
  - `memoryUxV2.coreRepository=false`；
  - `compatibility.legacyStaticMemory=true`。
- 建立规范 fixture：
  - `hi`；
  - 明确个人偏好；
  - 项目架构问题；
  - “上次怎么处理”；
  - 无项目的项目类对话；
  - Incognito。
- 记录旧链路实际 input、static memory token、dynamic token、TTFT、cache read/write。

完成标准：

- `/context` 能展示真实 memory 分项。
- 同一输入能并行计算 v1/v2 plan diff，但 v2 不进入模型。
- 日志不包含原始记忆内容。

回滚：关闭所有 `memoryUxV2` flags，行为与基线提交一致。

### Phase 1：动态召回统一编排

目标：建立 `MemoryRecallPlanner`，但先以 shadow / opt-in 方式运行。

任务：

- 抽取 Active Memory 现有 shortlist、Claims shortlist、Retrieval Planner source fusion。
- 实现 intent gate、candidate pool、确定性 ranker、token-bound renderer。
- 将 Procedure / Graph 作为 planner source，而不是独立顶层注入决策。
- 将现有 LLM Active Memory 变为 `deepRecall` 可选步骤。
- 失败时退化为 deterministic Top-K，不退化为全量注入。
- Provider adapters 接收统一 `DynamicRecallPack`。
- 增加 fast/deep/cached/skip trace。

完成标准：

- fixture 的 scope、排序、去重结果稳定。
- `hi` 召回 0 条。
- 本地召回 P95 ≤100ms。
- 至少 90% 基准 turn 不调用额外 LLM。
- V1 静态注入仍可并存，便于 A/B 对比。

### Phase 2：停止长期数据库静态注入

目标：修正最重要的静态/动态边界。

任务：

- `build_memory_section` 只保留 Core Memory 和协议。
- SQLite、Profile、自动 Pinned Claims 从 stable prompt 移除。
- 用户明确提升的 Core 项继续静态注入。
- `static_memory_refs` 改为 core refs；动态 refs 来自 recall pack。
- `legacyStaticMemory` 支持按 Agent / session 回滚。
- 更新 `/context`、Message Memory Trace 和 Dashboard。

完成标准：

- 拥有数百条 SQLite 记忆也不会增加空会话静态 Prompt。
- `hi` 不显示误导性的“使用了 49 条记忆”。
- 显式相关问题能从动态库召回正确条目。
- memory off / incognito / Agent disabled 行为与旧契约一致。

### Phase 3：统一 CoreMemoryRepository

目标：把已完成的 Project 渐进式能力推广到 Global / Agent。

任务：

- 将 `project/memory.rs` 中通用逻辑迁移到 `memory/core_repository.rs`。
- 保留 Project 薄适配器和现有 API。
- 增加 Global / Agent `MEMORY.md + topics` 路径。
- 实现 `CoreMemoryPathResolver`，先将 loader、Tauri、HTTP、tools、import、backup、
  restore 全部收敛到单一入口，禁止继续散落拼接 `memory.md` / `MEMORY.md`。
- 实现 §5.1.1 的小写→大写 manifest 双写迁移、conflict state、revision、expected
  hash、atomic write 和 downgrade 恢复。
- 引入 token-aware aggregate budget。
- 实现 `CoreMemorySnapshot` 和 session lifecycle。
- 统一 `core_memory` 工具，保留旧工具别名。

完成标准：

- 三个 scope 使用同一校验、锁、原子写、分页读和 topic 限额。
- 当前 Project 数据无需物理迁移即可继续读取。
- Global / Agent 旧文件内容零丢失。
- 兼容窗口内降级到旧版本写小写文件，再升级后能自动同步到大写。
- 大小写敏感文件系统上的同名/双文件冲突可检测、可解释、不会静默覆盖。
- 后台写入不改变进行中会话 stable fingerprint。
- 显式 reload 后 snapshot 才更新。

### Phase 4：学习控制与 Scope 修复

目标：将 use / recall / generate 分开，并消除项目内容落入 Agent 的污染。

任务：

- 实现新 learning modes 和 per-session policy。
- review-first 覆盖所有自动提取产物，不只 Claims shadow。
- 新增 unassigned candidates。
- 修改 `resolve_extract_scope`：无项目的 project-like 内容不回退 Agent。
- 增加 Core promotion proposal。
- Dreaming 只整理动态 store、产生提升建议，不直接膨胀 Core index。
- Profile synthesis 继续生成 snapshot，但默认只作为 recall source。

完成标准：

- `review_first` 未批准内容对所有 Prompt 路径不可见。
- 无项目 fixture 不产生 Agent scope 的 project-type 静态污染。
- per-session use/contribute 独立生效。
- Incognito 强制覆盖任何 allow 配置。

### Phase 5：GUI 收敛

目标：普通用户只看到三项产品能力。

任务：

- 重构 Memory Overview 三卡布局。
- 增加三层 Core Memory 浏览/编辑器和 token meter。
- 增加相关召回 fast/deep 说明和 trace。
- 增加学习模式、Review、Unassigned、Promotion Inbox。
- 高级引擎配置折叠，旧配置仍可编辑。
- Session 菜单增加 use/contribute 开关。
- 更新 12 语言文案，并明确深度召回会增加延迟和 token。

完成标准：

- 新用户无需理解 FTS、RRF、Claims、Dreaming 即可正确配置。
- GUI 能回答：记忆是否启用、是否召回、是否学习、当前用了什么。
- 加载/保存失败继续遵守现有脱敏和 retry 契约。

### Phase 6：默认开启、迁移与清理

目标：安全切换默认行为，并保留完整回滚窗口。

任务：

- 运行 v1/v2 shadow 数据对比。
- 先对新会话、新用户开启，再迁移既有用户。
- 发布迁移摘要和可逆配置备份。
- 观察一个 minor 版本的 recall miss、false recall、TTFT、cache ratio。
- 达标后默认 `legacyStaticMemory=false`。
- 再观察一个 minor 后删除运行时双算，但保留旧配置反序列化。
- 把最终事实更新进 architecture docs，并归档本路线文档。

完成标准：

- 无能力集合或数据丢失回归。
- 关键 token / latency / recall 指标达标。
- legacy 回滚演练成功。

## 13. 建议 PR 拆分

| PR | 内容 | 依赖 |
|---|---|---|
| 1 | Manifest、fixtures、flags、v1/v2 shadow plan | 当前基线 |
| 2 | `MemoryRecallPlanner` + deterministic ranker | PR 1 |
| 3 | Provider dynamic pack + deep recall 兼容 | PR 2 |
| 4 | SQLite/Profile/Pinned 退出静态 Prompt | PR 2–3 |
| 5 | `CoreMemoryRepository` 抽象与 Project 迁移 | 当前 Project 实现 |
| 6 | Global/Agent progressive files + snapshot | PR 5 |
| 7 | 新配置 schema + migration + session policy | PR 1、6 |
| 8 | Scope router + unassigned + review-first 修复 | PR 7 |
| 9 | Core promotion workflow | PR 6、8 |
| 10 | Memory Overview 三卡 UX | PR 3、6–9 |
| 11 | `/context`、Dashboard、trace、文档收尾 | 全部 |
| 12 | 默认切换与 legacy cleanup 第一阶段 | 观察数据达标后 |

每个 PR 必须包含兼容测试和 feature flag，不允许一次性跨 10 个 PR 范围提交后再补安全边界。

## 14. 测试矩阵

### 14.1 Core Memory

- Global only、Agent only、Project only、三层同时存在。
- 空文件、超预算、损坏 UTF-8、超长单条、循环链接、路径穿越。
- topic pagination、expected hash stale write、并发写锁、原子替换。
- legacy single-file 迁移、重复迁移、迁移中断恢复。
- Project > Agent > Global 冲突覆盖。
- snapshot 创建、复用、reload、compact、Project detach、memory off。

### 14.2 Dynamic Recall

- 寒暄零召回。
- 精确关键词、中文片段、identifier 中段、向量语义命中。
- Project/Agent/Global scope 隔离。
- memory/claim/profile canonical dedup。
- expired/superseded/archived/needs_review 抑制。
- graph 扩展不会引入跨 scope 节点。
- timeout、busy、embedding unavailable、external provider failure。
- fast → deep → deterministic fallback。

### 14.3 Learning

- smart 高置信写入、冲突进 Review。
- review-first 的 legacy、claim、profile 全链不可见。
- manual 不自动提取但显式保存可用。
- use=false 与 contribute=true/false 的正交组合。
- 无项目 project-like 内容进入 Unassigned。
- Core promotion 的显式、建议、拒绝、scope 修改。

### 14.4 Cache

- 同一 session 连续回合 stable fingerprint 完全一致。
- dynamic recall 内容变化不改变 stable fingerprint。
- background extract / Dreaming / Profile update 不改变当前 session snapshot。
- reload 后只产生一次预期 cache miss。
- Anthropic / OpenAI Chat / Responses / Codex golden request。
- Failover 后 stable/dynamic 语义一致。

### 14.5 Privacy / Security

- Incognito 三条链全部归零。
- 删除/焚毁 session 后异步学习不落库。
- secret scanner 阻止 Core promotion。
- untrusted external memory 不提升为 instruction。
- HTTP/Tauri owner 与 agent plane 权限隔离。
- Project tool 不接受越权 project id。

## 15. 评测与验收指标

### 15.1 Token / Cache

| 指标 | 目标 |
|---|---:|
| Core Memory 默认静态 token | ≤1,600 |
| Core Memory 默认硬上限 | ≤2,400 |
| 动态召回 token | ≤800 |
| 动态召回条数 | ≤5 |
| `hi` 动态召回 | 0 |
| stable fingerprint 跨普通回合变化 | 0 |
| 热缓存 stable read ratio | ≥80% |

### 15.2 Latency / Cost

| 指标 | 目标 |
|---|---:|
| fast recall P50 | ≤30ms |
| fast recall P95 | ≤100ms |
| 普通回合无额外 LLM 比例 | ≥90% |
| deep recall timeout | ≤4.5s，fail-soft |
| 因记忆导致的主回答 hard failure | 0 |

### 15.3 Quality

建立至少 100 个确定性/标注 fixture，覆盖：

- 个人偏好；
- 项目命令与架构；
- 历史决策；
- 用户纠正；
- 时间变化与过期；
- 多 scope 冲突；
- 无关相似文本；
- 中英文和代码 identifier。

目标：

- Recall@5 ≥90%；
- 无关回合 false recall ≤5%；
- scope leakage = 0；
- 已批准 manual correction 被旧自动事实覆盖 = 0；
- Core promotion false positive = 0（默认需要显式或审核）。

## 16. 风险与缓解

### 风险 1：退出静态 SQLite 后短期感觉“变笨”

缓解：

- v1/v2 shadow 对比；
- deterministic recall 先达到质量门槛再切换；
- per-agent `legacyStaticMemory` 回滚；
- 对高价值现有静态记忆提供 Core promotion 建议。

### 风险 2：Global / Agent / Project 文件迁移造成内容丢失

缓解：

- 所有入口先接入 `CoreMemoryPathResolver`，再执行物理迁移；
- copy + fsync + raw BLAKE3 回读校验，不做不可靠的 case-only rename；
- 至少一个 minor 版本保留小写 mirror 双写和 migration manifest；
- 通过 manifest 区分旧版本只改小写、新版本只改大写和双边冲突；
- 双边冲突进入显式 merge，不按 mtime 静默选边；
- 幂等迁移、迁移前 autosave、失败时永不删除唯一副本。

### 风险 3：召回链路过度复杂化

缓解：

- 只有一个 `MemoryRecallPlanner` 编排入口；
- Graph / Procedure / Profile 只实现 source trait；
- 普通模式固定预算和默认值；
- LLM deep recall 是最后一步且默认关闭。

### 风险 4：Session Snapshot 延迟用户刚保存的记忆

缓解：

- 当前 turn 注入 `ExplicitMemoryNotice`；
- 提供显式“重新加载”；
- compact/new session 自动刷新；
- UI 显示“已保存，下次记忆刷新后固定加载”。

### 风险 5：缓存指标很好但上下文仍过大

缓解：

- 同时展示 context tokens 与 cache read tokens；
- `/context` 明确 cache 不减少上下文窗口占用；
- Core、Dynamic、History 分别设预算。

## 17. 已锁定的默认决策

- Core Memory 使用 Global / Agent / Project 三层 Markdown 渐进式结构。
- 三个 scope 的最终 canonical 索引文件名统一为大写 `MEMORY.md`；Global / Agent
  现有小写 `memory.md` 通过 manifest 双写兼容窗口迁移，禁止直接覆盖或删除。
- 只有 Core index 固定进入 Prompt；topic 按需读取。
- SQLite、Profile、自动 Claims 默认动态召回。
- fast recall 默认开启且不调用额外 LLM。
- deep recall 默认关闭，并在 UI 明确说明延迟/token 代价。
- 学习与使用分离。
- Memory Off 是 master switch，不再是 learning mode。
- Core 自动提升默认关闭；自动系统只产生建议。
- 当前会话冻结 Core snapshot；后台更新不破坏 stable prefix。
- 兼容层和 legacy 回滚至少保留一个 minor 版本。
- 不删除任何现有底层记忆能力。

## 18. 实施完成定义

只有满足以下全部条件，Memory UX v2 才算完成：

- 三层 Core Memory 统一仓库、预算、工具和 GUI。
- 长期数据库不再默认静态注入。
- 单一动态召回编排器覆盖 memory / claims / profile / procedure / graph。
- 使用、召回、学习、深度召回四个语义清晰且正交。
- review-first 和 Incognito 全链 fail-closed。
- Scope 路由不再产生无项目的伪项目 Agent 污染。
- 当前会话 stable prefix 不受后台记忆更新影响。
- 四 Provider golden tests、token/cache 指标和 recall fixtures 达标。
- 迁移、回滚和数据完整性演练通过。
- 普通 GUI 只暴露三张主卡，高级能力无丢失。
- 最终架构文档和用户说明完成更新。

## 19. 设计参考

- [Claude Code：CLAUDE.md 与 Auto Memory](https://code.claude.com/docs/en/memory)
- [Claude Code：Prompt Caching](https://code.claude.com/docs/en/prompt-caching)
- [Codex：Memories](https://learn.chatgpt.com/docs/customization/memories)
- [Codex：Customization](https://learn.chatgpt.com/docs/customization/overview)
- [Letta：Context Hierarchy](https://docs.letta.com/guides/core-concepts/memory/context-hierarchy)

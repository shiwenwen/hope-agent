# 模型 vs Agent 统一配置（Automation Model）

> 返回 [文档索引](../README.md)

> 后台一次性 LLM 调用（非主对话）的统一执行原语与配置形状：`crate::automation`、`function_models.automation` 全局默认链、15 个消费者（Phase 1 的 9 个 + Phase 2 的 6 个）的字段对照 + `automation::run_vision` 视觉能力原语。与 [视觉桥](provider-system.md#12-视觉桥vision-bridgeissue-434) 同属 `function_models.*` 下的具体功能，两篇文档互相交叉引用。

---

## 1. 决策框架：什么时候是"模型"，什么时候是"Agent"

Hope Agent 里大量功能需要"调一次 LLM"，但需求形态差异很大——主对话需要完整 Agent（人格、system prompt、工具循环、记忆），而 Recap 摘要、Dreaming 叙事重写、Session Title 生成这类后台一次性调用只需要"给一段 prompt，拿一段文字回来"。用错抽象的代价是双向的：给一次性调用套完整 Agent 配置是过度设计；反过来给真正需要工具/人格的功能只配一个模型会削弱能力。

判断标准是两道闸，都过不去才归"模型"类：

1. **要不要执行 Tool？** 需要 Tool Loop（读文件、调用 MCP、执行代码等）→ Agent。
2. **要不要独立 persona / system prompt？** 需要区别于主对话的身份、指令集 → Agent。

两道闸都过不去的，只需要跟随一条**可配置的全局模型链（带跨模型降级）**，不需要 Agent 的模型链、能力开关、记忆配置等全套设置。本文档覆盖的正是这一类消费者。

Agent Team / Subagent 等确实需要完整 Agent 配置的功能不在本文档范围，见 [Agent 配置与解析链](agent-config.md) / [子 Agent 系统](subagent.md)。

---

## 2. 核心类型与执行原语

### 2.1 `ModelChain`（`crates/ha-core/src/provider/types.rs`）

```rust
pub struct ModelChain {
    pub primary: ActiveModel,
    #[serde(default)]
    pub fallbacks: Vec<ActiveModel>,
}
impl ModelChain {
    pub fn into_vec(self) -> Vec<ActiveModel>  // [primary, ...fallbacks] 拍平
}
```

序列化沿用 `ActiveModel` 的 camelCase 风格（`providerId`/`modelId`），不引入新分隔符约定。`ActiveModel`/`ModelChain` 均派生 `PartialEq, Eq`（供各消费者 config struct 的 `PartialEq` derive 传递）。

### 2.2 `function_models.automation`（`AppConfig.function_models: FunctionModelsConfig`）

```rust
pub struct FunctionModelsConfig {
    pub vision: Option<ActiveModel>,      // 视觉桥（issue #434），与本字段平级但互不影响
    pub automation: Option<ModelChain>,   // 本文档：后台一次性任务的全局默认链
}
```

`None` = 未设置全局自动化链，各消费者继续往下兜底到聊天全局 `active_model`/`fallback_models`（新装机零配置可用）。`function_models` 是 PR #438（视觉桥）已注册的 settings category（MEDIUM），`automation` 只是这个 struct 里新增的字段，`read_category`/`update_app_config` 走整体 `serde_json::to_value`/`merge_field`，无需重新注册 category。

### 2.3 `crate::automation` 模块（`crates/ha-core/src/automation/mod.rs`）

**`effective_chain(config, override_chain) -> Vec<ActiveModel>`**：纯函数，优先级 = 调用方解析出的 `override_chain`（消费者自己的 `model_override`，或旧字段惰性解析出的等价值）→ `function_models.automation` → `active_model`/`fallback_models`（聊天全局链）。都拿不到时返回空 `Vec`，调用方据此报"未配置模型"的清晰错误。

**`run(spec: ModelTaskSpec) -> Result<ModelTaskOutput>`**：统一执行原语，纯文本。核心是**真正的跨模型降级循环**：

```rust
for candidate in &spec.chain {
    let provider = find_provider(...)?;                       // 找不到 → continue 下一个候选
    let mut agent = AssistantAgent::try_new_from_provider(...)?;  // 构造失败 → continue
    agent = agent.with_failover_context(provider);
    agent.set_session_id(spec.session_key);                    // 关键一行，见 §2.4
    match agent.side_query_with_purpose(spec.purpose, ...).await {
        Ok(result) => return Ok(...),                          // 成功即返回
        Err(e) => continue,                                    // 失败 → 下一个候选
    }
}
```

这镜像 `chat_engine::engine::run_chat_engine` 的 `for model_ref in model_chain { ... continue on failure ... }` 循环，取代旧版 `recap::report::build_analysis_agent_for_id` 那种"构造期选中第一个可构造的模型、选完就不再变"的假降级。

**`resolve_legacy_agent_chain(config, agent_id) -> Option<ModelChain>`**：把旧的 `agent_id` 字段（Recap `analysis_agent`、Knowledge Compile `agent_id`）解析成等价 `ModelChain`——读该 agent 的 `agent.json` 模型配置，走现有 `provider::resolve_model_chain`（不改动），只是物化一次而不是保持一个 Agent 间接层。

**`parse_legacy_model_string(value) -> Option<ModelChain>`**：把旧的单冒号 `"provider_id:model_id"` 字符串（Dreaming/Skills auto_review/Hooks `prompt` 用的形状）解析成单元素 `ModelChain`（无 fallbacks）。刻意不复用 `provider::parse_model_ref`（双冒号 `"::"` 分隔，`AgentModelConfig` 用）——两种分隔符是历史遗留的不一致，此模块不做静默"纠正"（会静默破坏已有单冒号配置）。

**`model_label(config, model) -> String`**：`"Provider Name / Model Name"` 展示标签，查不到时回退 `provider_id::model_id`。

**`ModelTaskOutput { text, model }`**（Phase 2 加了 `model` 字段）：`model` 是真正产出 `text` 的那个候选——不一定是 `chain[0]`，可能是降级后的备用模型。持久化"生成模型"标签的调用方（Compile 的 `model` 返回值、OCR 的 `OCR-Model:` 头）应该读这个字段，不要像 Phase 1 阶段一些消费者那样在调用前用 `chain[0]` 预算——那样一旦真的走了降级，标签就悄悄错了。

**`build_candidate_agent(config, candidate, session_key) -> Result<AssistantAgent>`**（Phase 2 新增的私有 helper）：把"解析 provider → 构造 Agent → 挂 failover context → 设置 session_id"这四步从 `run()` 里抽出来，`run()` 和 `run_vision()`（见 §2.6）共享同一份实现——这正是防止"新增一条 one-shot 路径却忘了 `set_session_id`"这类回归的地方，也是本模块存在的原因。

### 2.4 重试修复：`set_session_id` 是关键一行

在引入本模块前，`recap::report::build_analysis_agent_for_id` 构造 `AssistantAgent` 时只调用 `.with_failover_context(prov)`，从不调用 `.set_session_id(...)`。而 `side_query()` 要求 `provider_config` 和 `session_id` **同时** `Some` 才会走 `execute_with_failover`（`FailoverPolicy::side_query_default()` 的 profile 轮换重试），否则直接走无重试的 "direct" 分支。

也就是说，Recap/Dreaming/Knowledge/Skills/Hooks 等后台调用在本次改动前**不仅没有跨模型降级，甚至没有任何重试**——一次网络抖动就整体失败。`automation::run` 给每个候选都调用 `agent.set_session_id(spec.session_key)`：有真实会话就传真实 `session_id`，没有就传合成键（如 `"automation:recap.facets"`）；`execute_with_failover` 只把它当 `PROFILE_STICKY`/`PROFILE_COOLDOWNS` 的 key 用，不要求对应真实会话行。

### 2.5 Purpose 标签与用量记录

`ModelTaskSpec.purpose: &'static str` 穿透进 `record_side_query_usage`（`agent/side_query.rs` 内部 `side_query_tagged`，写入 `ModelUsageEvent.operation`），替换掉旧版硬编码的 `"agent.side_query"`。`side_query()` 公开签名不变（仍写 `"agent.side_query"`），新增内部 `pub(crate) async fn side_query_with_purpose(purpose, ...)` 供 `automation::run` 调用，两者共享私有 `side_query_tagged(operation, ...)` 实现。

当前使用的 purpose 标签（`sqlite3 ~/.hope-agent/sessions.db "select operation, session_id from model_usage_events ..."` 可直接查）：

| Purpose | 消费者 |
|---|---|
| `recap.facets` / `recap.facets_merge` | Recap 逐会话 facet 提取 / 合并 |
| `recap.sections` / `recap.at_a_glance` | Recap 报告分段生成 / 一览摘要 |
| `dreaming.narrative` | Dreaming 叙事重写 |
| `dreaming.profile_rewrite` | Dreaming Profile 重写（Phase 4） |
| `dreaming.resolver` | Dreaming Deep resolver 冲突判定 |
| `knowledge.compile` | Knowledge Compile 摘要生成 |
| `hooks.prompt` | Hooks `prompt` handler side-query |
| `skills.auto_review` | Skills 自动评审 pipeline |
| `session_title` | 会话标题生成 |
| `awareness.extraction` | Awareness 行为提取（仅当设置了 `model_override` 时才走此路径，见 §3） |
| `knowledge.ocr` | 图片 OCR 导入（Phase 2，走 `run_vision`，见 §2.6 / §3.5） |
| `knowledge.ai_rewrite` | 知识空间 AI 改写（Phase 2，见 §3.5） |
| `knowledge_maintenance.auto_tag` / `.moc_upkeep` / `.memory_to_note` / `.source_conflict` | 知识空间维护 4 生成器（Phase 2，共享一个 `model_override` 但各自独立打标，供未来按生成器拆分成本，见 §3.5）。用完整类目名 `knowledge_maintenance` 做前缀而非简写 `maintenance`——`AppConfig.LocalLlmConfig.auto_maintenance`（本地模型预拉取看门狗，语义无关）已经占了 `maintenance` 这个词 |
| `sprite.observe` | Sprite 精灵模式观察调用（Phase 2，见 §3.5） |
| `note_tools.distill` / `.moc` / `.session_to_note` | 笔记三件套（Phase 2，共享一个 `model_override` 但各自独立打标，见 §3.5） |
| `recall_summary` | Recall Summary 召回摘要（Phase 2，见 §3.5） |

这不是与 `KIND_VISION`/`KIND_SIDE_QUERY` 竞争的新 `KIND_*`——所有消费者调用形态完全相同（纯文本或纯图片 one-shot、无 tool），只是"谁在调、为什么调"不同，`purpose` 是 `KIND_SIDE_QUERY` 内部更细的一层维度，供 Dashboard 按消费者拆分成本（已实现，见 [`dashboard.md`](dashboard.md) 第 2 节的 `by_operation`/`by_domain`）。命名规则：`purpose = "<域前缀>.<动作>"`，域前缀优先复用已有的大类前缀（`knowledge.*` 覆盖 compile/ocr/ai_rewrite 等所有知识空间功能，`dreaming.*`/`sprite.*` 同理），只有简写会与无关配置项撞名时才用完整类目名（`knowledge_maintenance.*` 是唯一例外）。

### 2.6 `automation::run_vision`（Phase 2）——视觉能力执行原语

图片 OCR（唯一需要带图片的 Phase 2 消费者）需要"能带 attachments"的执行原语，但没有直接给 `run()`/`ModelTaskSpec` 加一个可选 `attachments` 字段——核对代码发现,那样需要给 Phase 1 全部 9 个消费者的 **13 处** `ModelTaskSpec { ... }` 字面量调用点都补一行 `attachments: None,`（`ModelTaskSpec` 不 derive `Default`,13 处调用也都不用 `..Default::default()` 展开语法,Rust 字面量语法要求每个字段都显式列出）——这是一次性但真实的 9 文件机械改动,还让 9 个纯文本消费者的代码里永远躺着一个对它们毫无意义的字段。

改为新增一个**独立**的执行原语 `run_vision(spec: VisionTaskSpec) -> Result<ModelTaskOutput>`，与 `run()` 只共享 §2.3 提到的 `build_candidate_agent` 这一个私有 helper（provider 解析 + Agent 构造 + failover/session 挂载），核心循环从零为"带图片 + 逐候选视觉能力过滤"设计：

```rust
pub struct VisionTaskSpec<'a> {
    pub purpose: &'static str,
    pub chain: Vec<ActiveModel>,       // 允许混杂视觉/非视觉模型
    pub session_key: &'a str,
    pub system: &'a str,               // 框定 attachments 为不可信数据的 system prompt
    pub instruction: &'a str,
    pub attachments: &'a [crate::agent::Attachment],
    pub max_tokens: u32,
}

pub async fn run_vision(spec: VisionTaskSpec<'_>) -> Result<ModelTaskOutput> {
    for candidate in &spec.chain {
        let provider = find_provider(...)?;
        if !provider.model_supports_vision(&candidate.model_id) {
            continue;  // 不是失败——这个候选本来就不适用视觉任务，跳过不计入尝试次数
        }
        let agent = build_candidate_agent(...).await?;  // 与 run() 共享
        match agent.independent_query_with_attachments(spec.purpose, spec.system, spec.instruction, spec.attachments, spec.max_tokens).await {
            Ok(result) => return Ok(ModelTaskOutput { text: result.text, model: candidate.clone() }),
            Err(e) => continue,
        }
    }
    // 链上一个视觉候选都没有 → "no vision-capable model configured" 清晰报错
    // 链上有视觉候选但全部失败 → 聚合最后一个错误
}
```

**硬约束（与视觉桥物理隔离）**：`AssistantAgent::independent_query_with_attachments`（`agent/side_query.rs`）加了 `purpose: &str` 参数（今天硬编码 `"agent.independent_query_with_attachments"`；全仓库唯一调用点就是 OCR，改动前即将被这次迁移替换，不需要像 `side_query`/`side_query_with_purpose` 那样保留旧签名兼容分支），但**完全不碰**共享的私有 `run_one_shot_with_attachments`，也不碰视觉桥自己的 `transcribe_images_for_vision_bridge`。视觉桥从不调用 `automation::run_vision`——两条路径在函数层面就已隔离，不是靠约定维持。已知限制（明确接受，不在本次修）：`run_one_shot_with_attachments` 本身仍不接 `execute_with_failover`，所以 OCR 只拿到"跨模型"重试（`run_vision` 外层循环），拿不到"同模型跨 profile"重试——碰这个共享函数有污染视觉桥延迟预算的风险，留作 Phase 3+ 独立课题。

---

## 3. 消费者清单：新旧字段对照

**迁移策略**：新字段 `model_override` 与旧字段并存，旧字段标记 deprecated 但保留；每个消费者的解析逻辑是"新字段优先，否则按旧逻辑原样解析旧字段（不变），否则落到 `function_models.automation` 全局默认链"。**不做 config.json 物理迁移**——GUI 只写新字段，旧字段自然被晾在一边，直到用户下次在对应面板保存。`AppConfig.embedding` 是这个模式的现成先例。

选择"新旧字段共存、消费点惰性解析"而非启动期文件手术，是因为后者被证明不可行：`hooks::init()`、以及 server/acp 模式下 `onboarding::state::get_state()` 都会在任何自定义迁移代码有机会跑之前就已经触发首次类型化解析；Hooks 的 `model` 字段还分散在 4 个独立文件（`config.json` + 托管/项目/本地三个 `hooks.json`），物理迁移天然覆盖不到后三个。

### 3.1 完整走 `automation::run`（真跨模型降级 + purpose 记账）的 7 个消费者

| 消费者 | 配置位置 | 旧字段（deprecated，仍兼容） | 新字段 | 旧值解析方式 |
|---|---|---|---|---|
| Recap | `RecapConfig` | `analysis_agent: Option<String>`（agent_id） | `model_override: Option<ModelChain>` | `automation::resolve_legacy_agent_chain` |
| Knowledge Compile | `KnowledgeCompileConfig` | `agent_id: Option<String>` | 同上 | 同上 |
| Dreaming | `DreamingConfig` | `narrative_model: Option<String>`（单冒号） | `model_override: Option<ModelChain>` | `automation::parse_legacy_model_string` |
| Skills auto_review | `SkillsAutoReviewConfig` | `review_model: Option<String>`（单冒号） | 同上 | 同上 |
| Hooks `prompt` handler | `PromptHookConfig` | `model: Option<String>`（单冒号，per-hook 实例字段） | `model_override: Option<ModelChain>` | 同上；新旧字段共存天然覆盖 `config.json` + 托管/项目/本地三个 `hooks.json` 全部 4 处 |
| Session Title | `SessionTitleConfig` | `provider_id`/`model_id: Option<String>`（裸字段对） | `model_override: Option<ModelChain>` | 直接包成单元素链；解析链**始终追加当前会话的 `chat_model`** 作保底 fallback（题目生成不该因为自动化默认链未配就彻底失败） |
| Awareness 提取 | `LlmExtractionConfig` | 无（`extraction_agent`/`extraction_model` 是死配置，见 §3.3，直接删除不保留） | `model_override: Option<ModelChain>` | 无——纯新字段，`None`（默认）保持现状：复用当前 chat agent 的 `self.side_query(...)`（cache-friendly，见 §3.3） |

Recap 和 Knowledge Compile 是仅有的两个需要读 `agent.json` 的字段（`resolve_legacy_agent_chain`）；其余都是单冒号字符串（`parse_legacy_model_string`）或裸字段对。

**Recap 的特殊结构**：一次报告要跑几十次独立 LLM 调用（逐会话 facet 提取 + 多段落生成），如果每次都重新解析配置代价不小。`recap/report.rs` 新增 `resolve_recap_chain()` 在报告开始时解析一次，产出 `Arc<Vec<ActiveModel>>` 贯穿 `facets.rs`/`sections.rs` 的每个独立调用（而非共享一个 `AssistantAgent`），这样每个调用仍各自独立走 `automation::run` 的降级循环，不会因为共享 Agent 而共享失败状态。

### 3.2 仅新增字段、维持原执行路径的 2 个消费者

Memory Extract 和 Compact 摘要模型的现有执行签名（分别是 `provider_config: &ProviderConfig, model_id: &str` 和摘要专用调用路径）不支持链式循环，为此重构判断不成比例——它们只是在原有解析优先级链里插入新字段，**不经过 `automation::run`，没有跨模型降级，也不打 purpose 标签**：

| 消费者 | 配置位置 | 旧字段（deprecated） | 新字段 | 说明 |
|---|---|---|---|---|
| Memory Extract | `MemoryExtractConfig` | `extract_provider_id`/`extract_model_id: Option<String>`（裸字段对） | `model_override: Option<ActiveModel>`（单模型，非 `ModelChain`） | 解析优先级：per-agent 覆盖 → `model_override` → 旧裸字段对 → 兜底逻辑（`memory_extract.rs` + `chat_engine/context.rs` 两处解析点） |
| Compact 摘要 | `CompactConfig` | `summarization_model: Option<String>`（单冒号，文档"providerId:modelId"） | `model_override: Option<ActiveModel>` | 新增 `effective_summarization_model_ref() -> Option<String>` 方法：`model_override` 优先，否则回退 `summarization_model`；**刻意不接入 `function_models.automation`**——Tier-3 摘要是 fail-fast 设计，不希望因为全局自动化链配置错误而拖慢/连锁失败上下文压缩这个关键路径 |

### 3.3 Awareness：死配置清理，不是重塑

`LlmExtractionConfig` 原有的 `extraction_agent: Option<String>` 和 `extraction_model: Option<ExtractionModelRef>` 两个字段已确认是**死配置**——`extraction_agent` 读了但从未真正切换 agent（`run_extraction_inline` 判断不同就打日志然后继续用当前 agent），`extraction_model` 全仓库零消费点。两者**直接删除**，不保留兼容读取（没有"保留旧值"的意义，因为旧值从未真正生效过）。

新的 `model_override: Option<ModelChain>` 从第一天就真正接线：`None`（默认，即所有现存配置的状态，因为这是新字段）保持现状——复用当前 chat agent 的 `self.side_query(...)`，享受与主对话共享的 prompt cache 前缀；设置了 `model_override` 则切换到 `automation::run`（purpose `"awareness.extraction"`），换取"可指定独立/更便宜的模型"，代价是放弃这个 cache 共享——这是一个用户主动选择的权衡，不是免费升级。

### 3.4 Smart 审批 Judge：不属于本次重塑范围

`SmartModeConfig.judge_model: Option<JudgeModelConfig>`（`provider_id` + `model` + `extra_prompt`，[`permission/mode.rs`](../../crates/ha-core/src/permission/mode.rs)）经过评估后**维持后端结构不变**——Judge 是一个有严格延迟预算的实时安全检查（approval 超时通常以秒计），刻意不引入模型链/跨模型重试（会员在预算内叠加多次网络往返），也不接入 `function_models.automation`。本次只把 GUI（[`SmartModeSection.tsx`](../../src/components/settings/approval-panel/SmartModeSection.tsx)）从两个裸文本输入框换成 `<ModelSelector>` 下拉选择器，写入的仍是同一个 `{providerId, model}` JSON 形状，纯 UX 改进。

### 3.5 Phase 2 消费者：全部纯新增字段，无遗留字段兼容

Phase 2 调研发现一个比预想更整齐的事实：图片 OCR、知识空间维护 4 生成器、Sprite、笔记三件套、Recall Summary，以及调研中额外发现的知识空间 "AI 改写"（`ai_rewrite`），**全部**曾经走同一个遗留兜底函数 `crate::recap::report::build_analysis_agent()`——读**已废弃**的 `RecapConfig.analysis_agent`（不是 Recap 自己 GUI 早已写入的新字段 `recap.model_override`，一个真实的孤立配置 bug）→ `AppConfig.default_agent_id` → 硬编码 `"ha-main"`，且**从不调用 `.set_session_id(...)`**——同一个零重试 bug，Phase 1 之外的 6 个消费者全都没漏。

**与 Phase 1 的关键区别**：这 6 个消费者过去从未有过专属配置字段（隐性继承 Recap 的），所以 Phase 2 的字段全部是**纯新增**，没有旧字段要惰性解析、没有 deprecated 兼容分支——迁移比 Phase 1 更简单。

| 消费者 | 配置位置 | 新字段 | 走 `automation::run`/`run_vision`？ |
|---|---|---|---|
| 图片 OCR | `KnowledgeVisionConfig`（新结构体，`knowledge/types.rs`） | `model_override: Option<ModelChain>` + `timeout_secs`/`max_tokens`（OCR 过去完全没有超时，一个候选卡住会让后面的候选永远没机会试） | `run_vision`，purpose `knowledge.ocr` |
| 知识空间维护 4 生成器 | `MaintenanceConfig` 加字段 | `model_override: Option<ModelChain>`——**一个共享字段，不是 4 个独立的**：`llm_timeout_secs`/`llm_max_tokens` 本来就是"一个数管全部 4 个任务"的形状，模型选择跟着同样粒度是内部一致的 | `run`，purpose 见 §2.5（各生成器独立打标） |
| Sprite | `SpriteConfig` 加字段 | `model_override: Option<ModelChain>`——**给完整链，不是 Judge 式单模型**：Sprite 的调用是真正 fire-and-forget（`kb_sprite_observe_cmd` 用 `tokio::spawn` 不 await），没有 Judge 那种阻塞审批链路的硬延迟预算，只有"施法中"光效的观感，值得用真降级换可靠性 | `run`，purpose `sprite.observe`；"施法中"光效改为包住整个降级循环而不是一次 `side_query`（真实行为变化：`timeout_secs` 语义从"一次尝试的超时"变成"全部尝试加起来的总预算"） |
| 笔记三件套 | `NoteToolsConfig`（新结构体，`knowledge/types.rs`；不放 `tools::note`——该模块 `pub(crate)`，`src-tauri`/`ha-server` 引用不到） | `model_override: Option<ModelChain>`——**一个共享字段，不是三个独立的**（用户已确认）：三者本来就共用一个代码入口 `run_kb_side_query`，今天连 GUI 都没有；真需要分开调，加字段是纯增量、不破坏兼容的后续动作 | `run`，purpose `note_tools.distill`/`.moc`/`.session_to_note`（各自独立打标）；三者都带 `ctx: &ToolExecContext`，`ctx.session_id` 天然可用，比 OCR/Recall Summary 更好命拿到真实会话亲和性 |
| Recall Summary | `RecallSummaryConfig` 加字段 | `model_override: Option<ModelChain>`。**不是 greenfield**——`enabled`/`min_hits`/`context_char_budget`/`timeout_secs`/`max_tokens`/`include_history` 早已是完整实现的 opt-in 功能，缺的只是模型字段 + **`enabled` 开关本身第一次拥有 GUI**（此前只有 Dashboard Learning 面板一个只读用量计数器） | `run`，purpose `recall_summary` |
| 知识空间 AI 改写（`ai_rewrite`） | 无持久配置——`QuickRewriteBar.tsx` 已有自己的 per-request、用户显式挑选的模型下拉，加持久 `model_override` 字段是纯冗余 | 无新字段；只重写内部解析 `resolve_rewrite_chain`：用户显式选了模型 → 单模型钉死绝不静默换掉；没选/解析不出来 → `automation::effective_chain(config, None)` 真正的降级链（替换旧的、同样零重试的 `build_rewrite_agent` 兜底） | `run`，purpose `knowledge.ai_rewrite` |

**GUI 归属**：图片 OCR / 笔记三件套的区块加在 `KnowledgePanel.tsx`（紧邻 `CompileAgentSection`）；知识空间维护 / Sprite 各在自己现有的 `KnowledgeMaintenanceSection.tsx` / `SpriteSection.tsx` 里加一行 `<ModelChainEditor>`；Recall Summary 是全新 `RecallSummarySection.tsx`，挂在 `MemorySettingsView.tsx` 里 `<ExtractConfig>` 之后、`<BudgetConfig>` 之前——这里是"记忆相关工具在调用时的行为调优"的家（`ExtractConfig` 配的就是同一类东西），不挂到 Dreaming 的 tab（Dreaming 是有自己状态/报告 UI 的后台调度生命周期，Recall Summary 是查询时行为，性质不同）。全部复用已有的 `get_available_models` + `<ModelChainEditor>`，没有新发明任何前端组件。

**新设置类目**：`knowledge_vision`、`note_tools`（均 MEDIUM，`tools/settings.rs` + SKILL.md）——`knowledge_maintenance`/`sprite`/`recall_summary` 都是已注册类目加字段，`merge_field` 全字段通用读写自动覆盖，零额外注册。

**遗留函数整体删除**：调研 + 最终 grep 双重确认，`recap::report::build_analysis_agent` 家族全仓库只有本节 6 个真实调用点（外加一个**已经零调用**的死函数 `build_analysis_agent_with_explicit_agent`，作为独立第一步先行删除，跟 Phase 2 无关）。6 项全部迁移完成后，整个遗留 section（`build_analysis_agent`/`build_vision_analysis_agent`/`build_analysis_agent_inner`/`build_analysis_agent_from_explicit`/`normalize_agent_id`/`build_analysis_agent_for_id`/`analysis_model_chain`/`push_model_dedup`）已从 `recap/report.rs` 删除，`recap/mod.rs` 的 re-export 同步清理。这是一次纯内部私有函数删除（`git revert` 即可回滚，无数据库 schema/公开 API 那种不可逆风险），选择跟 Phase 2 一起做而不是等一个发布周期，因为留着这段"能用"的死代码本身是隐患——注释已经写了"can be deleted"，留着就是邀请未来有人偷懒抄这条路而不是走 `automation`。

**已实现的相邻功能——扫描版 PDF 逐页 OCR 兜底**：`file_extract.rs` 早就把 PDF 页面光栅化成 PNG（供聊天附件用），但知识空间导入路径过去只取文本、丢弃图片，纯图片扫描版 PDF 导入直接失败。`KnowledgeVisionConfig`/`automation::run_vision` 当初特意按"任意数量 attachments + 视觉能力过滤"设计，就是为这个功能铺路——Phase 3 把它接上，按逐页粒度追踪成败 + 支持只重试失败页，而不是让整份文档因个别页失败被迫全部重来。完整设计（`knowledge_source_ocr_pages` 表、异步执行、并发/重试语义、Markdown 约定）见 [`knowledge-base.md`](knowledge-base.md#扫描版-pdf-ocr-兜底逐页追踪)；本文件只关心它复用了哪条模型执行原语——`crate::automation::run_vision`（purpose `knowledge.ocr`，与既有单图 OCR 共用，`KnowledgeVisionConfig` 新增 `ocr_concurrency`/`max_ocr_pages` 两个纯增量字段）。

---

## 4. GUI

### 4.1 共享组件 `ModelChainEditor`（`src/components/ui/model-chain-editor.tsx`）

组合已有的 `<ModelSelector>`（provider→model 两级下拉）+ dnd-kit 排序（`@dnd-kit/core`/`@dnd-kit/sortable`/`@dnd-kit/utilities`）的可拖拽 fallback 列表。

```tsx
interface ModelChainRef {
  primary: { providerId: string; modelId: string }
  fallbacks: { providerId: string; modelId: string }[]
}
interface ModelChainEditorProps {
  value: ModelChainRef | null   // null = 继承上一层
  onChange: (next: ModelChainRef | null) => void
  availableModels: AvailableModel[]
  inheritLabel: string          // value=null 时主选择器的占位文案
  allowFallbacks?: boolean      // 默认 true；Smart Judge 场景故意不用此组件（见 §3.4）
  className?: string
}
```

`allowFallbacks=false` 的使用场景是"需要单模型选择但不要暴露降级承诺"的 UI；本次 7 个消费者面板 + 全局自动化区块全部用默认 `true`。清除按钮（回到 `null`/继承态）用 `<IconTip>` 包装（非原生 `title`，遵守 AGENTS.md 前端规范）。

### 4.2 `GlobalModelPanel.tsx` 新增区块

紧邻已有的 Vision Bridge 区块（同一种排版：标题 + 说明文案 + 选择器 + 清除按钮），用 `<ModelChainEditor>` 绑定 `function_models.automation`。专用命令镜像 `get_vision_model`/`set_vision_model` 的写法：

- `get_automation_model_chain` / `set_automation_model_chain`（Tauri，[`commands/provider/models.rs`](../../src-tauri/src/commands/provider/models.rs)，经 `mutate_config_async(("function_models", "ui"), ...)`）
- HTTP `GET`/`PUT /api/models/automation`（[`routes/models.rs`](../../crates/ha-server/src/routes/models.rs)）

未新建独立 `AutomationPanel.tsx`、未新增 Settings nav 项——`function_models` 类别的两个功能（vision + automation）共用 `GlobalModelPanel.tsx` 一个页面。

### 4.3 Phase 1 消费者面板

| 面板 | 绑定字段 |
|---|---|
| `RecapSettingsPanel.tsx` | `modelOverride`（原 Agent 下拉整体替换） |
| `DreamingPanel.tsx`（`memory-panel/`） | `modelOverride`（原 `ModelSelector` 单模型 + 清除按钮整体替换） |
| `KnowledgePanel.tsx`（`CompileAgentSection`） | `modelOverride`（原 Agent 下拉整体替换） |
| `SkillEvolutionView.tsx`（`skills-panel/`） | `modelOverride`（原单冒号字符串 `StringField` 替换；`patchField`/`saveStatus` 三态保存机制不变） |
| `HooksPanel.tsx` | `prompt` handler 的 `modelOverride` 字段（`FieldDef.kind` 新增 `"modelChain"` 分支；`availableModels` 经 `HooksPanel` → `GroupCard` → `HandlerCard` → `FieldRow` → `FieldInput` 逐层透传） |
| `SmartModeSection.tsx`（`approval-panel/`） | 见 §3.4，用 `<ModelSelector>` 非 `<ModelChainEditor>` |
| Session Title | 无独立 GUI 面板（`SessionTitleConfig` 目前无对应设置页；`model_override` 字段已就绪，待有归属面板时接线） |

### 4.4 Phase 2 消费者面板

| 面板 | 绑定字段 |
|---|---|
| `KnowledgePanel.tsx`（新增 `KnowledgeVisionSection`） | `KnowledgeVisionConfig.modelOverride`，紧邻 `CompileAgentSection` |
| `KnowledgePanel.tsx`（新增 `NoteToolsSection`） | `NoteToolsConfig.modelOverride`（一个共享字段，一张卡片一个 `<ModelChainEditor>`，不是三行） |
| `KnowledgeMaintenanceSection.tsx` | 新增一行 `<ModelChainEditor>`，绑定 `MaintenanceConfig.modelOverride`，放在"任务"网格之后、保存按钮之前 |
| `SpriteSection.tsx` | 新增一行 `<ModelChainEditor>`，绑定 `SpriteConfig.modelOverride`，放在"senses"网格之后、保存按钮之前 |
| `memory-panel/RecallSummarySection.tsx`（新文件） | `enabled` 主开关（第一次拥有 GUI）+ `minHits`/`contextCharBudget`/`timeoutSecs`/`maxTokens`/`includeHistory` 调优项 + `modelOverride`；挂载在 `MemorySettingsView.tsx` |
| `ai_rewrite` | 无新增 GUI——`QuickRewriteBar.tsx` 已有的 per-request 模型下拉不变 |

---

## 5. 设置三件套

**Phase 1**：不需要新注册 settings category——`function_models`（MEDIUM）已在 PR #438（视觉桥）注册完整，`automation` 只是这个 struct 里新增字段，`read_category`/`update_app_config` 走整体 `serde_json::to_value`/`merge_field` 自动覆盖，`tools/settings.rs`/`core_tools.rs`/SKILL.md 均无需改动。9 个消费者各自现有的 `get_xxx_config`/`save_xxx_config` 命令（如 `save_recap_config`）在 PR #438 已 retrofit 成 `mutate_config_async`，本次只改内部 struct 字段，命令体本身不变。

新增的 `get_automation_model_chain`/`set_automation_model_chain` 从第一天就用 `mutate_config_async`（不是旧版 `save_recap_config` 曾经用过的 inline 同步写法）——async Tauri/HTTP 处理器里的同步文件 IO 一律经 `mutate_config_async`/`SessionDB::run`/`blocking::run_blocking` 下放到 blocking 池，这是 PR #438 确立的硬红线。

**Phase 2**：`knowledge_maintenance`（HIGH，加字段）/`sprite`（MEDIUM，加字段）/`recall_summary`（MEDIUM，加字段）三个已注册类目零额外改动，`merge_field` 全字段通用读写自动覆盖新的 `model_override` 字段。`knowledge_vision`/`note_tools` 是两个**新注册**类目（均 MEDIUM），在 `tools/settings.rs` 的 `risk_level()`/`read_category()`/`update_app_config()` 三处 match 各加一条，`core_tools.rs` 的 `get_settings`/`update_settings` 工具 schema 的 `enum` 列表两处（read + write）各加两个类目名，SKILL.md 补两行登记 + 顺手修正 3 处此前遗留的 Phase 1 文档债（`dreaming`/`recap`/`skills_auto_review`/`awareness` 四行仍写着已废弃的字段名，如 `narrativeModel`/`analysisAgent`/`reviewModel`/`extractionAgent`）。新增的 `KnowledgeVisionConfig`/`NoteToolsConfig` 走 `knowledge::service` 里的 `get_vision_config`/`set_vision_config`（`async fn`，`mutate_config_async`）/`get_note_tools_config`/`set_note_tools_config`，新的 `RecallSummaryConfig` 命令直接写在 `commands/config.rs`（`get_recall_summary_config`/`save_recall_summary_config`，同样 `mutate_config_async`），均无同步文件 IO。

---

## 6. 关键文件索引

| 模块 | 文件 | 职责 |
|---|---|---|
| 核心类型 | `crates/ha-core/src/provider/types.rs` | `ModelChain` |
| 全局配置 | `crates/ha-core/src/config/mod.rs` | `FunctionModelsConfig.automation` |
| 执行原语 | `crates/ha-core/src/automation/mod.rs` | `effective_chain` / `run` / `resolve_legacy_agent_chain` / `parse_legacy_model_string` / `model_label` |
| Purpose 记账 | `crates/ha-core/src/agent/side_query.rs` | `side_query_with_purpose` / `side_query_tagged` |
| Recap | `crates/ha-core/src/recap/{report,facets,sections}.rs` | `resolve_recap_chain` + facet/section 生成 |
| Knowledge Compile | `crates/ha-core/src/knowledge/compile.rs` | `generate_summary` |
| Dreaming | `crates/ha-core/src/memory/dreaming/{pipeline,narrative,profile,resolver}.rs` | `resolve_dreaming_chain` + 叙事/Profile/resolver 三处调用 |
| Skills auto_review | `crates/ha-core/src/skills/auto_review/pipeline.rs` | `query_review_agent` |
| Hooks | `crates/ha-core/src/hooks/runner/prompt.rs` | `resolve_prompt_hook_chain` |
| Session Title | `crates/ha-core/src/session_title.rs` | `generate_and_update_title` |
| Memory Extract | `crates/ha-core/src/memory_extract.rs` + `crates/ha-core/src/chat_engine/context.rs` | 两处独立解析点 |
| Compact | `crates/ha-core/src/context_compact/config.rs` | `effective_summarization_model_ref` |
| Awareness | `crates/ha-core/src/agent/mod.rs`（`run_extraction_inline`）+ `crates/ha-core/src/awareness/config.rs` | `LlmExtractionConfig.model_override` |
| Smart Judge | `crates/ha-core/src/permission/{mode,judge}.rs` | 未改动，见 §3.4 |
| 前端共享组件 | `src/components/ui/model-chain-editor.tsx` | `ModelChainEditor` |
| 全局面板 | `src/components/settings/GlobalModelPanel.tsx` | 自动化默认链区块 |
| 命令/路由 | `src-tauri/src/commands/provider/models.rs`、`crates/ha-server/src/routes/models.rs` | `get/set_automation_model_chain` |
| 视觉能力原语（Phase 2） | `crates/ha-core/src/automation/mod.rs` | `run_vision`/`VisionTaskSpec`/`build_candidate_agent`，`ModelTaskOutput.model` |
| 图片 OCR（Phase 2） | `crates/ha-core/src/knowledge/{types,source,service}.rs` | `KnowledgeVisionConfig`、`ocr_image_bytes`、`get/set_vision_config` |
| 知识空间维护（Phase 2） | `crates/ha-core/src/knowledge/maintenance/{config,generators}.rs` | `MaintenanceConfig.model_override`、`run_side_query` + 4 生成器调用点 |
| Sprite（Phase 2） | `crates/ha-core/src/sprite/{config,mod}.rs` | `SpriteConfig.model_override`、`observe_and_maybe_speak` |
| 笔记三件套（Phase 2） | `crates/ha-core/src/knowledge/types.rs` + `crates/ha-core/src/tools/note.rs` | `NoteToolsConfig`、`run_kb_side_query` + 3 工具调用点 |
| Recall Summary（Phase 2） | `crates/ha-core/src/memory/recall_summary.rs` | `RecallSummaryConfig.model_override`、`run_summary` |
| AI 改写（Phase 2） | `crates/ha-core/src/knowledge/service.rs` | `ai_rewrite`、`resolve_rewrite_chain`（`build_rewrite_agent` 已删除） |
| Phase 2 GUI | `src/components/settings/KnowledgePanel.tsx`（`KnowledgeVisionSection`/`NoteToolsSection`）、`KnowledgeMaintenanceSection.tsx`、`SpriteSection.tsx`、`src/components/settings/memory-panel/RecallSummarySection.tsx`（新文件） | 见 §4.4 |
| 遗留函数删除 | `crates/ha-core/src/recap/report.rs` + `recap/mod.rs` | `build_analysis_agent` 家族已整体删除（Phase 2 收尾），确认全仓库零残留调用 |
| Dashboard purpose 拆分（Phase 3） | `crates/ha-core/src/dashboard/{types,queries,filters}.rs` | `TokenByOperation`/`TokenByDomain`、`operation_domain`、`DashboardFilter.operation` |
| 扫描版 PDF OCR 兜底（Phase 3） | `crates/ha-core/src/file_extract.rs`、`crates/ha-core/src/knowledge/{types,registry,source,service}.rs`、`crates/ha-core/src/async_jobs/manager.rs` | `render_pdf_bytes_isolated`、`knowledge_source_ocr_pages` 表、`import_pdf_ocr_fallback`/`retry_source_ocr_pages`，详见 [`knowledge-base.md`](knowledge-base.md#扫描版-pdf-ocr-兜底逐页追踪) |

---

## 7. 后续（不在本次范围）

- **混合文本 + 扫描页的 PDF**：Phase 3 的扫描版 PDF OCR 兜底（见 [`knowledge-base.md`](knowledge-base.md#扫描版-pdf-ocr-兜底逐页追踪)）只在整份 PDF **完全没有文本层**时触发——只要 `extract_pdf_text` 返回非空文本（哪怕只是文档里一页扫描附录之外、其余页正常抽取到的文本），就走普通文本路径，不会对文本抽取"漏掉"的个别扫描页单独尝试 OCR。这是刻意的最小范围（避免给每一份"恰好某页没抽出文本"的正常 PDF 都加一次视觉调用），留作后续按实际需求单独评估的方向，不是本次遗漏。

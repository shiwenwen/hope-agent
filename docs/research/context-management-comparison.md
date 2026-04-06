# 上下文管理对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.6

---

## 一、架构总览

三个项目都面临同一核心问题：LLM 上下文窗口有限，长对话和大量工具调用会导致上下文溢出。三者采用了不同层次的解决方案：

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| **语言** | Rust（后端原生） | TypeScript（Node/Bun） | TypeScript（Node） |
| **压缩层级** | 5 层渐进式（Tier 0-4） | 3 层（Micro → Auto/Session Memory → Emergency PTL） | 2 层（Context Pruning → Compaction Safeguard） |
| **LLM 摘要** | Side Query 缓存复用 | Forked Agent（缓存共享）或流式独立调用 | `summarizeInStages` 分段摘要 |
| **消息分组** | `_oc_round` 元数据标记 | `groupMessagesByApiRound`（assistant.id 边界） | 无显式分组（依赖 tool_use/tool_result 配对修复） |
| **后压缩恢复** | 文件内容恢复（磁盘读取） | 文件附件恢复 + 技能/计划/工具 delta 注入 | 工具失败摘要 + 文件操作摘要 + 结构化 fallback |
| **Context Engine** | 无（内联在 agent 中） | 无（内联在 query loop 中） | 插件化 `ContextEngine` 接口（registry + factory） |
| **缓存感知** | Anthropic 显式 `cache_control` + OpenAI 自动前缀缓存 | Cached Microcompact（`cache_edits` API）+ 时间 TTL | Cache-TTL 模式（`lastCacheTouchAt` 过期判断） |

---

## 二、OpenComputer 实现

### 2.1 五层渐进式压缩

OpenComputer 在 `context_compact/` 目录实现了 5 层渐进式压缩，由 `compact_if_needed()` 统一调度：

**Tier 0 — Microcompaction（零成本）**
- 文件：`compact.rs` → `microcompact()`
- 机制：扫描 assistant 消息构建 `tool_use_id → tool_name` 映射，然后清除保护边界之前的临时工具结果
- 目标工具（可配置）：`ls`、`grep`、`find`、`process`、`sessions_list`、`agents_list`
- 保护策略：保留最近 N 个 assistant 消息（默认 4）之后的所有内容
- 替换文本：`"[Ephemeral tool result cleared]"`
- 多格式支持：同时处理 Anthropic（`tool_use` blocks）、OpenAI Chat（`tool_calls` array）、OpenAI Responses（`function_call`）三种格式

**Tier 1 — 工具结果截断**
- 文件：`truncation.rs` → `truncate_tool_results()`
- 机制：单个工具结果超过上下文窗口的 30%（`MAX_TOOL_RESULT_CONTEXT_SHARE`）或 400K 字符硬上限时截断
- 截断策略：
  - 检测尾部重要内容（错误信息、JSON 闭合、结果摘要）→ 使用 head+tail 模式（tail 占 30%，上限 4000 字符）
  - 否则只保留头部
  - 结构感知切点：优先在空行 > JSON 闭合 > 代码块结束 > 普通换行处切割
- 最小保留：2000 字符（`MIN_KEEP_CHARS`）

**Tier 2 — 上下文修剪**
- 文件：`pruning.rs` → `prune_old_context()`
- 两阶段执行：
  1. **Soft Trim**：对超过 6000 字符的工具结果做 head+tail 截断（各保留 2000 字符），按优先级排序（`age * 0.6 + size * 0.4`），直到比率降到 `hardClearRatio`（0.70）以下
  2. **Hard Clear**：替换为占位符 `"[Old tool result content cleared]"`，直到比率降到 `hardClearRatio` 以下
- 触发阈值：`softTrimRatio` = 0.50，`hardClearRatio` = 0.70
- 保护机制：
  - 最近 N 个 assistant 消息（默认 4）不修剪
  - 第一条 user 消息之前的内容不修剪（保护 bootstrap 上下文）
  - 拒绝修剪的工具（`tools_deny_prune`）：web_search、web_fetch、save_memory 等 8 个

**Tier 3 — LLM 摘要**
- 文件：`summarization.rs`
- `compact_if_needed()` 返回 `tier_applied: 3, description: "summarization_needed"` 信号，由 `agent.rs` 异步执行
- 分割策略：`split_for_summarization()` 从末尾数 N 个 user 消息作为保留边界（默认 4，最大 12），调整到 round-safe 边界
- 摘要 prompt 结构化输出：`## Decisions / ## Open TODOs / ## Constraints/Rules / ## Pending user asks / ## Exact identifiers / ## Conversation summary`
- 标识符保护策略：`strict`（默认）/ `off` / `custom`
- 支持增量摘要：前一次摘要作为 "Previous conversation summary" 注入
- 摘要上限：16,000 字符（`MAX_COMPACTION_SUMMARY_CHARS`）
- 自适应分块：`compute_adaptive_chunk_ratio()` 根据平均消息大小动态调整分块比例（0.15 ~ 0.40）

**Tier 4 — 紧急压缩**
- 文件：`compact.rs` → `emergency_compact()`
- 触发条件：API 返回 ContextOverflow 错误
- 策略：
  1. 清除所有工具结果内容
  2. 只保留最近 N 个 user turn（默认 4，上限 12）
  3. 调整到 round-safe 边界（使用 `find_round_safe_boundary_forward`）

### 2.2 API-Round 分组保护

文件：`round_grouping.rs`

核心问题：Tier 3 摘要和 Tier 4 紧急压缩在切割消息时，不能拆散 tool_use/tool_result 配对。

实现机制：
- **打标**：Tool loop 中通过 `push_and_stamp()` 给每条消息打上 `_oc_round: "r{N}"` 元数据
- **分割**：`find_round_safe_boundary()` 向前/向后查找 round 边界（相邻消息 round ID 不同的位置）
- **清理**：`prepare_messages_for_api()` 在发送 API 请求前克隆消息并剥离 `_oc_round` 元数据
- **向后兼容**：无 `_oc_round` 标记的旧会话退化为原行为（直接在目标索引切割）

### 2.3 后压缩文件恢复

文件：`recovery.rs`

Tier 3 摘要后，最近编辑的文件内容从对话历史中丢失。恢复机制：
- **扫描**：从被摘要的消息中提取 write/edit/apply_patch 工具调用的文件路径
- **去重**：排除已在保留消息中引用的文件
- **读取**：从磁盘读取最近编辑的文件当前内容（最多 5 文件，每文件 16KB）
- **预算**：释放 token 的 10%，兜底 100K 字符
- **注入**：构建 `<file path="...">` 格式的恢复消息
- **多格式支持**：同时处理三种 API 格式的 tool_call 消息
- **apply_patch 特殊处理**：解析 patch header（`*** Add File:` / `*** Update File:` / `*** Move to:`）提取路径

### 2.4 Side Query 缓存

文件：`agent/side_query.rs`

设计目标：侧查询（Tier 3 摘要、记忆提取等）复用主对话的 prompt cache，降低约 90% 成本。

实现机制：
- **快照**：每轮主请求 compaction 后调用 `save_cache_safe_params()`，使用 `Arc` 存储 `CacheSafeParams`（system_prompt + tool_schemas + conversation_history）
- **缓存友好请求**：
  - Anthropic：复制 system + tools（加 `cache_control: ephemeral`）+ history + 侧查询指令，保持字节一致前缀
  - OpenAI Chat：system + tools + history + 侧查询指令（依赖自动前缀缓存）
  - OpenAI Responses / Codex：instructions + input + tools
- **特性**：非流式、单轮、无 tool loop、无 compaction
- **退化**：无 cache params 时退化为普通的单条消息请求

### 2.5 Token 估算

文件：`estimation.rs`

- 通用文本：`chars / 4`（`CHARS_PER_TOKEN = 4`）
- 工具结果：`chars / 2`（更紧凑，参考 OpenClaw 的 `TOOL_RESULT_CHARS_PER_TOKEN_ESTIMATE = 2`）
- 图片内容：固定 8000 字符估算
- **自适应校准器**：`TokenEstimateCalibrator` 使用 EMA（alpha=0.3）根据 API 实际 token 使用量校准估算因子
- 总请求 token：`system_prompt_tokens + message_tokens + max_output_tokens`

---

## 三、Claude Code 实现

### 3.1 压缩层级体系

Claude Code 的压缩分为三个主要层次，但没有 OpenComputer 那样的编号分层：

1. **Microcompact**（请求前）→ 清除旧工具结果内容
2. **Auto Compact / Session Memory Compact**（请求间）→ LLM 摘要或会话记忆替换
3. **Reactive Compact**（API 错误后）→ prompt-too-long 重试

触发路径：
```
每轮请求前 → microcompactMessages()
  ├─ 时间触发（cache miss 时全量清理）
  └─ Cached MC（cache editing API 增量删除）

每轮请求后 → autoCompactIfNeeded()
  ├─ trySessionMemoryCompaction()（优先尝试）
  └─ compactConversation()（LLM 摘要）

API 413 错误 → reactiveCompact / truncateHeadForPTLRetry()
```

### 3.2 Micro Compact

文件：`microCompact.ts`

Claude Code 的 microcompact 有三种路径：

**1. 时间触发路径（Time-based MC）**
- 条件：距上次 assistant 消息的时间差超过阈值（通过 GrowthBook 远程配置）
- 策略：content-clear 所有可压缩工具结果，只保留最近 N 个
- 直接修改消息内容（因为 cache 已过期，无需保留前缀）
- 替换文本：`"[Old tool result content cleared]"`

**2. Cached Microcompact 路径（Cached MC）**
- 仅限 Anthropic 内部用户 + 支持 cache editing 的模型
- 不修改本地消息内容——通过 `cache_edits` API 在服务端删除工具结果
- 维护全局状态（`cachedMCState`）追踪已注册和已删除的 tool results
- `pinCacheEdits()` 将删除操作固定到特定 user 消息位置，后续请求重发

**3. 可压缩工具集**
- `FileRead`、`Shell`（Bash 等）、`Grep`、`Glob`、`WebSearch`、`WebFetch`、`FileEdit`、`FileWrite`

**Token 估算**
- 文本：`roughTokenCountEstimation(text)` → chars/4，乘 4/3 保守系数
- 图片/文档：固定 2000 tokens
- Thinking：只计算 thinking 文本，不计算 JSON wrapper 或签名
- Tool use：计算 name + input JSON

### 3.3 Auto Compact

文件：`autoCompact.ts`

**触发阈值**：
- `effectiveContextWindow = contextWindow - max(modelMaxOutput, 20000)`
- `autoCompactThreshold = effectiveContextWindow - 13000`（`AUTOCOMPACT_BUFFER_TOKENS`）
- 支持环境变量覆盖：`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`、`CLAUDE_CODE_AUTO_COMPACT_WINDOW`

**熔断器**：
- 连续失败 3 次后停止尝试（`MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES = 3`）
- 成功后重置计数

**执行流程**：
1. 先尝试 `trySessionMemoryCompaction()`
2. 失败则调用 `compactConversation()` 做 LLM 摘要
3. 支持 recompaction 诊断（追踪是否同链路重复压缩）

**递归保护**：
- `session_memory`、`compact` 源不触发
- Context Collapse 模式下不触发（由 collapse 系统自己管理）
- Reactive-only 模式下不触发

### 3.4 消息分组策略

文件：`grouping.ts`

`groupMessagesByApiRound()` 按 API 回合分组：
- 边界条件：新 assistant 响应开始（`message.id` 不同于上一个 assistant）
- 流式 chunk 共享同一 `id`，因此同一回合的多个 chunk 归入同一组
- 比 OpenComputer 的 `_oc_round` 更简洁——不需要打标，直接利用 assistant message ID
- 用于 PTL 重试时的消息裁剪（`truncateHeadForPTLRetry`）

### 3.5 Session Memory Compact

文件：`sessionMemoryCompact.ts`

实验性功能，优先于 LLM 摘要尝试：

**原理**：用已提取的 Session Memory 文件内容（结构化知识提取）代替 LLM 摘要，避免额外的 API 调用。

**消息保留策略**（`calculateMessagesToKeepIndex`）：
- 从 `lastSummarizedMessageId` 开始
- 向前扩展直到满足：`minTokens`（10K）AND `minTextBlockMessages`（5）
- 硬上限：`maxTokens`（40K）
- 调整 `adjustIndexToPreserveAPIInvariants`：
  - 确保 tool_use/tool_result 配对完整
  - 确保 thinking blocks 与同 message.id 的 assistant 消息一起保留

**配置来源**：GrowthBook 远程配置，Feature flag 门控（`tengu_session_memory` + `tengu_sm_compact`）

### 3.6 Tool Use Summary

文件：`toolUseSummaryGenerator.ts`

- 用途：SDK 模式下为完成的工具批次生成人类可读的单行摘要
- 模型：Haiku（轻量、低成本）
- Prompt：要求生成 git-commit-subject 风格的标签（约 30 字符），过去时态
- 输入：工具名称 + 输入/输出的 JSON 截断（各 300 字符）+ 上一条 assistant 文本的前 200 字符
- 非关键路径：失败不影响主流程

### 3.7 Post-Compact Cleanup

文件：`postCompactCleanup.ts`

压缩后清理缓存和追踪状态：
- `resetMicrocompactState()`：重置 cached MC 状态
- `resetContextCollapse()`：重置 Context Collapse 状态（仅主线程）
- `getUserContext.cache.clear()`：清除用户上下文缓存（仅主线程）
- `resetGetMemoryFilesCache('compact')`：重新加载 CLAUDE.md 等内存文件
- `clearSystemPromptSections()`：清除系统提示词段落缓存
- `clearClassifierApprovals()`：清除分类器审批缓存
- `clearSpeculativeChecks()`：清除推测性权限检查
- `clearBetaTracingState()`：清除 beta tracing 状态
- `sweepFileContentCache()`：清理文件内容缓存（commit attribution）
- 不清除 `sentSkillNames`（技能内容需跨压缩保持）

**主线程保护**：子 Agent 共享进程但不应重置主线程的模块级状态，通过 `querySource` 判断。

**compactConversation() 后的恢复注入**：
- 文件附件：`createPostCompactFileAttachments()`（最多 5 文件，每文件 5K tokens，总预算 50K tokens）
- 技能附件：`createSkillAttachmentIfNeeded()`（每技能 5K tokens 上限，总预算 25K tokens）
- 计划附件：`createPlanAttachmentIfNeeded()`
- 工具 delta：`getDeferredToolsDeltaAttachment()` / `getAgentListingDeltaAttachment()` / `getMcpInstructionsDeltaAttachment()`
- SessionStart hooks：重新加载 CLAUDE.md 等上下文
- 会话元数据：`reAppendSessionMetadata()` 确保自定义标题保持在 16KB 尾部窗口内

---

## 四、OpenClaw 实现

### 4.1 Context Pruning

文件：`context-pruning/pruner.ts` + `settings.ts` + `extension.ts` + `runtime.ts` + `tools.ts`

OpenClaw 的上下文修剪是 OpenComputer Tier 2 的直接上游参考：

**触发模式**：`cache-ttl`
- 通过 `lastCacheTouchAt` 追踪上次缓存触碰时间
- TTL 默认 5 分钟（300,000ms）
- TTL 过期才执行修剪（避免在 cache 有效期内修改前缀导致 cache miss）

**默认阈值**：
| 参数 | 默认值 |
|------|--------|
| `keepLastAssistants` | 3 |
| `softTrimRatio` | 0.3 |
| `hardClearRatio` | 0.5 |
| `minPrunableToolChars` | 50,000 |
| `softTrim.maxChars` | 4,000 |
| `softTrim.headChars` | 1,500 |
| `softTrim.tailChars` | 1,500 |
| `hardClear.placeholder` | `"[Old tool result content cleared]"` |

**执行流程**（`pruneContextMessages`）：
1. 计算 `charWindow = contextWindowTokens * CHARS_PER_TOKEN_ESTIMATE`
2. 找到保护边界（最后 N 个 assistant 消息）和起始边界（第一个 user 消息）
3. 在可修剪范围内收集 toolResult 消息（跳过 deny 列表中的工具）
4. **Soft Trim**：对超过 `maxChars` 的工具结果做 head+tail 截断
5. **Hard Clear**：比率仍超过 `hardClearRatio` 时替换为占位符

**工具匹配**（`tools.ts`）：
- 支持 `allow` + `deny` 双列表
- glob 模式匹配（小写归一化）
- deny 优先于 allow

**CJK 感知**：`estimateStringChars()` 对 CJK 字符加权估算（来自 `cjk-chars.ts`）

**扩展架构**：作为 Pi Agent Extension 注册到 `context` 事件，返回修改后的消息数组或 `undefined`（无变化）

### 4.2 Compaction 指令与安全检查

**指令解析**（`compaction-instructions.ts`）：
- 优先级链：`事件指令（SDK）→ 运行时配置 → DEFAULT_COMPACTION_INSTRUCTIONS`
- 默认指令要求：使用对话主语言、保持事实性、保持结构不变、不翻译代码/路径/标识符
- 长度上限：800 字符（~200 tokens）

**质量守卫**（`compaction-safeguard-quality.ts`）：
- 必需的摘要段落：`## Decisions` / `## Open TODOs` / `## Constraints/Rules` / `## Pending user asks` / `## Exact identifiers`
- `auditSummaryQuality()` 检查：
  1. 段落完整性（按顺序出现）
  2. 标识符保留（strict 模式下提取的 opaque identifiers 必须在摘要中出现）
  3. 用户最新请求覆盖（token overlap 检查）
- 标识符提取（`extractOpaqueIdentifiers`）：hex 串、URL、文件路径、端口号、长数字，最多 12 个
- 结构化 fallback（`buildStructuredFallbackSummary`）：质量检查失败时构建最小合规摘要

**运行时配置**（`compaction-safeguard-runtime.ts`）：
- `maxHistoryShare`：历史占上下文窗口的最大比例
- `recentTurnsPreserve`：保留最近 N 轮（默认 3，最大 12）
- `qualityGuardEnabled` / `qualityGuardMaxRetries`：质量守卫开关和最大重试次数（默认 1，最大 3）
- `identifierPolicy`：`strict` / `off` / `custom`
- `cancelReason`：取消原因（消费后清除）

**Compaction Safeguard 主流程**（`compaction-safeguard.ts`）：
- 收集工具失败信息（`collectToolFailures`）→ 格式化为 `## Tool Failures` 段落
- 文件操作摘要：最大 2000 字符（`MAX_FILE_OPS_SECTION_CHARS`）
- 摘要长度上限：16,000 字符（`MAX_COMPACTION_SUMMARY_CHARS`）
- 分段摘要：委托给 `summarizeInStages`（自适应分块、oversized 消息检测）
- 分裂 turn 处理：`composeSplitTurnInstructions` 用于 split-turn 场景的特殊指令
- 模型解析：从运行时配置获取 compaction 模型及 API 认证

### 4.3 Context Engine

文件：`context-engine/types.ts` + `registry.ts` + `delegate.ts` + `legacy.ts`

OpenClaw 独有的**插件化上下文引擎**架构：

**接口定义**（`ContextEngine`）：
- `info`：引擎元数据（id、name、version、ownsCompaction）
- `bootstrap()`：初始化引擎状态（可导入历史上下文）
- `maintain()`：转录维护（支持 `rewriteTranscriptEntries` 安全重写）
- `ingest()` / `ingestBatch()`：摄入消息
- `afterTurn()`：后轮生命周期（持久化、后台 compaction 决策）
- `assemble()`：在 token 预算内组装模型上下文
- `compact()`：压缩上下文
- `prepareSubagentSpawn()` / `onSubagentEnded()`：子 Agent 生命周期
- `dispose()`：资源释放

**注册表**：
- 进程全局单例（`Symbol.for` 确保跨 chunk 共享）
- 双 owner 模型：`core`（内部）和 `public-sdk`（第三方）
- 工厂模式：`ContextEngineFactory = () => ContextEngine | Promise<ContextEngine>`
- 解析顺序：`config.plugins.slots.contextEngine` → 默认 slot（`"legacy"`）

**Legacy 兼容层**：
- `LegacyContextEngine`：默认引擎，委托给内置运行时
- `delegateCompactionToRuntime()`：第三方引擎可调用内置 compaction 路径
- `wrapContextEngineWithSessionKeyCompat()`：Proxy 包装，自动处理旧版引擎不识别 `sessionKey`/`prompt` 参数的兼容问题

**Transcript Rewrite**：
- `TranscriptRewriteRequest`：批量替换转录条目
- 引擎决定重写内容，运行时负责 DAG 更新
- 结果包含 `bytesFreed`、`rewrittenEntries`

---

## 五、逐项功能对比

| 功能 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| **零成本清理** | Tier 0 Microcompact（可配置工具列表） | Time-based MC + Cached MC（cache_edits API） | Context Pruning soft trim |
| **工具结果截断** | Tier 1（结构感知 head+tail） | 无独立截断层（MC 直接替换） | Soft trim（head+tail，无结构感知） |
| **上下文修剪** | Tier 2（age×size 优先级排序） | 无独立修剪层 | Context Pruning（线性遍历） |
| **LLM 摘要** | Tier 3 + Side Query 缓存 | compactConversation（forked agent） | Compaction Safeguard（summarizeInStages） |
| **紧急压缩** | Tier 4（清除所有 + 保留最近 N 轮） | truncateHeadForPTLRetry（按 round 丢弃最旧 group） | 无独立紧急层 |
| **消息分组** | `_oc_round` 标记（显式） | assistant message.id 边界（隐式） | 无（修复配对靠后处理） |
| **缓存感知** | Side Query prefix 对齐 | Cached MC + cache_edits API + TTL | cache-ttl 模式（避免 cache miss 期间修剪） |
| **摘要质量守卫** | 无 | 无 | `auditSummaryQuality()`（段落检查 + 标识符保留 + 用户请求覆盖） |
| **标识符保护** | `identifier_policy: strict/off/custom` | 无显式策略 | `identifierPolicy: strict/off/custom` + 自动提取验证 |
| **后压缩文件恢复** | 磁盘读取（5 文件 × 16KB，10% budget） | `createPostCompactFileAttachments`（5 文件 × 5K tokens，50K budget） | 无独立文件恢复 |
| **后压缩技能恢复** | 无 | `createSkillAttachmentIfNeeded`（5K tokens/skill，25K total） | 无 |
| **后压缩工具 delta** | 无 | deferred tools + agent listing + MCP instructions 重新注入 | 无 |
| **Session Memory 压缩** | 自动记忆提取（side_query） | trySessionMemoryCompaction（完整替换方案） | 无 |
| **Token 估算校准** | EMA 自适应校准器 | 固定 4/3 保守系数 | 无校准（固定 `CHARS_PER_TOKEN_ESTIMATE`） |
| **CJK 字符处理** | 无（统一 chars/4） | 无 | `estimateStringChars()`（CJK 加权） |
| **插件化架构** | 无 | 无 | ContextEngine 接口 + Registry + Factory |
| **多 Provider 格式** | 3 种（Anthropic/OpenAI Chat/Responses） | 1 种（Anthropic） | 1 种（Pi AI 格式） |
| **配置粒度** | 20+ 可配置参数（config.json） | 环境变量 + GrowthBook 远程配置 | TOML config + runtime registry |
| **自定义摘要指令** | `custom_instructions` 字段 | `/compact` 命令参数 + PreCompact hooks | `resolveCompactionInstructions` 优先级链 |
| **Transcript 路径引用** | 无 | 摘要中包含完整 transcript 路径 | 无 |
| **转录重写** | 无 | 无 | `TranscriptRewriteRequest`（批量条目替换） |

---

## 六、差距分析与建议

### OpenComputer 的优势

1. **最完整的分层体系**：5 层渐进式压缩（Tier 0-4）比 Claude Code 和 OpenClaw 的分层更细致、更可控
2. **多 Provider 格式支持**：统一处理 Anthropic/OpenAI Chat/OpenAI Responses 三种消息格式，其他两者都只处理单一格式
3. **结构感知截断**：`find_structure_boundary()` 在 JSON 闭合、代码块、段落边界处切割，比 OpenClaw 的纯字符截断更智能
4. **Side Query 缓存复用**：通过 `CacheSafeParams` 共享前缀实现约 90% 的成本降低，Claude Code 类似但实现更复杂（forked agent）
5. **配置粒度最高**：20+ 可配置参数，用户可精细调节每一层的行为

### OpenComputer 可改进的方向

1. **缺少摘要质量守卫**：OpenClaw 的 `auditSummaryQuality()` 能自动检测摘要是否丢失关键段落、标识符和用户最新请求，建议参考实现
2. **缺少后压缩技能/工具/计划恢复**：Claude Code 在压缩后重新注入技能附件、工具 delta、计划状态，OpenComputer 只恢复文件内容
3. **无 cache editing API 支持**：Claude Code 的 Cached MC 通过 `cache_edits` API 在服务端删除内容而不破坏缓存前缀，OpenComputer 的 Microcompact 直接修改消息内容会导致 cache miss
4. **无 cache-TTL 意识**：OpenClaw 的 context pruning 只在 cache 过期后才执行修剪，OpenComputer 的各层没有这个概念
5. **CJK 字符估算**：OpenClaw 对 CJK 字符做加权处理（一个 CJK 字符约 1.5 个 ASCII 字符的 token 量），OpenComputer 统一用 chars/4 会低估 CJK 密集内容的 token 数
6. **无 Context Engine 插件架构**：OpenClaw 的 `ContextEngine` 接口允许第三方实现自定义上下文管理策略，OpenComputer 的压缩逻辑内联在 agent 中，不易扩展
7. **无 PTL 重试**：Claude Code 在 compact 请求本身触发 prompt-too-long 时会裁剪最旧的 round groups 重试（最多 3 次），OpenComputer 没有这个容错路径
8. **连续压缩失败熔断**：Claude Code 有 `MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES = 3` 的熔断机制，避免无效重试浪费 API 调用

### 优先建议

| 优先级 | 建议 | 参考 |
|--------|------|------|
| **P0** | 增加 cache-TTL 感知，避免在 cache 有效期内修剪前缀 | OpenClaw `cache-ttl` mode |
| **P1** | 增加摘要质量守卫（段落检查 + 标识符保留验证） | OpenClaw `auditSummaryQuality` |
| **P1** | 增加连续压缩失败的熔断机制 | Claude Code `MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES` |
| **P2** | 后压缩恢复扩展：注入 plan 状态、skill 内容、工具 delta | Claude Code `compactConversation` 后续逻辑 |
| **P2** | 增加 compact 请求 PTL 重试 | Claude Code `truncateHeadForPTLRetry` |
| **P3** | CJK 字符加权估算 | OpenClaw `estimateStringChars` |
| **P3** | 探索 cache_edits API 支持（Anthropic Provider 专用） | Claude Code Cached MC |

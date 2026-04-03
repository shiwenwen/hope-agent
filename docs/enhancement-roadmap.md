# OpenComputer 能力增强路线图

> 基于 claude-code 源码深度分析，提炼可借鉴的架构模式，制定分阶段改进计划。
> 每个改进项将有独立的设计文档（`docs/enhancements/XXX.md`），本文档为总纲。

## 背景

通过对 claude-code（Anthropic 官方 CLI 工具）源码的全面分析，发现其在以下领域有成熟的工程化设计：

| 领域 | Claude Code 核心模式 | OpenComputer 现状 |
|------|---------------------|-------------------|
| Tool 加载 | 延迟加载 + ToolSearch 元工具，节省 3-5K token/请求 | 35+ 工具 schema 全量发送 |
| Tool 执行 | `isConcurrencySafe` 标记，并发安全工具批量并行 | 纯串行 `for` 循环 |
| Tool 结果 | 大结果落盘（preview + 路径），`maxResultSizeChars` 阈值 | Tier 1 截断（30% context/400KB） |
| 侧查询成本 | Forked Agent 共享 prompt cache，侧查询成本降低 90% | 无 forked agent 机制，侧查询全额计费 |
| 上下文压缩 | 微压缩（零成本清旧结果）+ 后压缩恢复（重注入文件/Skill） | 4 层渐进压缩，无微压缩和恢复 |
| 消息分组 | API-Round 边界标记，压缩不拆散 tool_use/result 配对 | 无分组保护 |
| 记忆提取 | 对话结束后自动后台提取，Forked Agent + 权限隔离 | 仅 flush_before_compact 时提取 |
| 记忆选择 | Sonnet 侧查询从 manifest 选 ≤5 条相关记忆 | BM25 + vector RRF，无 LLM 过滤 |
| Skill 隔离 | `allowed-tools` 限制 + `context: fork` 独立子 Agent | 无工具限制，无 fork 模式 |
| Plan 权限 | 权限系统层面禁止写入（`mode: 'plan'`） | 仅 schema 过滤，执行层无兜底 |
| Agent 调度 | 前台/后台自动切换（`autoBackgroundMs`），Coordinator 模式 | 无前后台切换 |

---

## 改进项总览

### 优先级 & 复杂度矩阵

```
         影响大
           │
    ┌──────┼──────┐
    │ 1A   │ 7A   │
    │ 1B   │ 6A   │
    │ 2A   │ 5C   │
    │ 4A   │      │
    ├──────┼──────┤
    │ 3A   │      │
    │ 3B   │      │
    │ 4B   │      │
    │ 5A   │      │
    │ 5B   │      │
    │ 1C   │      │
    └──────┼──────┘
       简单    复杂
```

| ID | 改进项 | 复杂度 | 阶段 | 关键收益 |
|----|--------|--------|------|---------|
| **1A** | 工具并发执行 | M | Phase 1 | 多工具轮次 2-5x 加速 |
| **1B** | 微压缩 Tier 0 | S | Phase 1 | 零成本节省 10-20% token |
| **1C** | 工具结果磁盘持久化 | M | Phase 1 | 大结果不撑爆上下文 |
| **2A** | Forked Agent 缓存共享 | M | Phase 2 | 所有侧查询成本降低 ~90% |
| **3A** | 后压缩恢复 | M | Phase 3 | 压缩后无需重新 read 文件 |
| **3B** | API-Round 消息分组 | S | Phase 3 | 压缩不破坏消息配对 |
| **4A** | 自动记忆提取 | M | Phase 4 | 记忆自动积累，无需手动 |
| **4B** | LLM 记忆语义选择 | M | Phase 4 | 更精准的记忆注入 |
| **5A** | Skill allowed-tools | S | Phase 5 | Skill 工具隔离 |
| **5B** | Plan 执行层权限强制 | M | Phase 5 | 纵深防御 |
| **5C** | Skill Fork 模式 | L | Phase 5 | Skill 上下文隔离 |
| **6A** | Agent 前台/后台切换 | L | Phase 6 | 长任务不阻塞主对话 |
| **7A** | 延迟工具加载 | L | Phase 7 | 每请求省 ~40% 工具 token |

---

## Phase 1: Tool 系统基础增强 ✅

> 目标：提升工具执行效率，优化上下文空间利用
> **状态：已完成** — 详见 `docs/tool-execution-architecture.md`

### 1A. 工具并发执行 [M]

**问题**：4 个 provider 的 tool loop 均为串行 `for tc in &tool_calls`。当模型同时请求 `grep` + `ls` + `read` 时逐个等待。

**方案**：
- `ToolDefinition` 增加 `concurrent_safe: bool` 字段
- 并发安全工具（`read`, `ls`, `grep`, `find`, `recall_memory`, `memory_get`, `web_search`, `web_fetch`, `agents_list`, `sessions_list`, `session_status`, `sessions_history`, `image`, `pdf`, `get_weather`）使用 `futures::future::join_all` 并行执行
- 串行工具（`exec`, `write`, `edit`, `apply_patch`, `save_memory`, `browser`, `subagent` 等）保持顺序执行
- 分组策略：先并行执行全部并发安全工具，再串行执行写入工具
- 每个并发 task 独立包装 `tokio::select!` cancel 检测

**影响文件**：
- `src-tauri/src/tools/definitions.rs` — ToolDefinition 结构体
- `src-tauri/src/tools/mod.rs` — `is_concurrent_safe()` 查询
- `src-tauri/src/agent/providers/anthropic.rs:354-476` — tool loop 重构
- `src-tauri/src/agent/providers/openai_chat.rs` — 同步
- `src-tauri/src/agent/providers/openai_responses.rs` — 同步
- `src-tauri/src/agent/providers/codex.rs` — 同步

**验证**：3 个 read 工具并发，总耗时 ≈ max(单次) 而非 sum(三次)

---

### 1B. 微压缩 Tier 0 [S]

**问题**：压缩从 Tier 1（50% 使用率）才启动。`ls`/`grep`/`find` 等临时结果长期驻留。

**方案**：
- 在 `compact_if_needed()` 最前面插入 Tier 0
- 零成本清除 `keep_last_assistants` 边界之前的临时工具结果
- 目标工具：`ls`, `grep`, `find`, `process`, `sessions_list`, `agents_list`
- 替换为 `[Ephemeral tool result cleared]` 占位符

**影响文件**：
- `src-tauri/src/context_compact/config.rs` — 新增配置字段
- `src-tauri/src/context_compact/compact.rs` — 插入 Tier 0
- `src-tauri/src/context_compact/pruning.rs` — 复用辅助函数

**验证**：50+ 轮对话，token 使用率下降 10-20%

---

### 1C. 工具结果磁盘持久化 [M]

**问题**：200KB 的 read 输出全量占上下文。Tier 1 截断虽有效但丢失尾部信息。

**方案**：
- 工具结果超 50KB（可配置）时写入 `~/.opencomputer/tool_results/{session_id}/{call_id}.txt`
- 上下文中保留 head 2KB + tail 1KB + 路径引用
- 模型可通过 read 工具访问完整文件
- 会话删除或启动时清理 24h 前的结果文件

**影响文件**：
- `src-tauri/src/tools/execution.rs` — 结果大小判断 + 落盘
- `src-tauri/src/tools/mod.rs` — 阈值常量 + 路径构建
- `src-tauri/src/tools/read.rs` — 确保可访问 tool_results 目录

**验证**：read 100KB 文件，上下文 ~3KB，磁盘有完整文件

---

## Phase 2: Forked Agent 缓存共享基础设施 ✅

> 目标：建立低成本"侧查询"能力，为后续记忆提取、LLM 摘要、记忆选择提供统一基础
> 依赖：无（可与 Phase 1 并行）
> 受益者：Phase 3（压缩 Tier 3 摘要）、Phase 4（记忆提取/选择）
> **状态：已完成** — 详见 `docs/side-query-architecture.md`

### 2A. Forked Agent 缓存共享 [M]

**问题**：所有 LLM 侧查询（Tier 3 摘要、记忆提取、记忆选择）都是独立 API 调用，全额计费。一次 50K token 对话的侧查询成本 ~$0.15，频繁调用不可承受。

**原理**：Anthropic/OpenAI 的 prompt caching 基于**前缀匹配**。如果侧查询复用主对话的 `system_prompt + tools + conversation_history` 前缀，缓存命中后这部分 token 成本降低 90%（Anthropic）或免费（OpenAI）。

```
主对话请求:   [system_prompt][tools][msg1..msgN][user_msg]     → 缓存创建
                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
侧查询请求:   [system_prompt][tools][msg1..msgN][side_query]   → 缓存命中！
                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
              只有 [side_query] 部分是新增 input token
```

**方案**：

1. 新建 `src-tauri/src/agent/forked_query.rs` 模块，提供统一的侧查询接口：

```rust
pub struct ForkedQueryParams {
    /// 复用主对话的 system prompt（保证前缀一致）
    pub system_prompt: String,
    /// 复用主对话的 tool schemas（保证前缀一致）
    pub tool_schemas: Vec<Value>,
    /// 复用主对话的历史消息（保证前缀一致）
    pub conversation_history: Vec<Value>,
    /// 侧查询专用指令（追加到末尾，不影响缓存前缀）
    pub query_instruction: String,
    /// 可选：限制侧查询可用的工具（记忆提取只需 read/grep）
    pub allowed_tools: Option<Vec<String>>,
    /// 最大输出 token
    pub max_tokens: u32,
    /// 超时
    pub timeout: Duration,
}

pub struct ForkedQueryResult {
    pub response_text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,    // 命中缓存的 token 数
    pub cache_creation_tokens: u64,
}

/// 发起一次复用主对话缓存的侧查询
pub async fn forked_query(
    provider: &LlmProvider,
    params: ForkedQueryParams,
) -> anyhow::Result<ForkedQueryResult>
```

2. 在 `AssistantAgent` 上暴露 `side_query()` 便捷方法，自动传入当前 system_prompt + tools + history：

```rust
impl AssistantAgent {
    pub async fn side_query(&self, instruction: &str, max_tokens: u32) -> Result<String> {
        let history = self.conversation_history.lock().await.clone();
        forked_query(&self.provider, ForkedQueryParams {
            system_prompt: self.current_system_prompt.clone(),
            tool_schemas: self.current_tool_schemas.clone(),
            conversation_history: history,
            query_instruction: instruction.into(),
            max_tokens,
            ..Default::default()
        }).await.map(|r| r.response_text)
    }
}
```

3. 缓存命中条件与约束：
   - **时效性**：侧查询必须在主对话 API 调用后 5 分钟内发出（Anthropic cache TTL）
   - **前缀一致**：system_prompt + tool_schemas 必须与主对话完全一致（字节级），否则缓存失效
   - **Provider 感知**：Anthropic 用显式 `cache_control` 标记；OpenAI 自动缓存无需额外处理；其他 provider 退化为普通调用
   - **tool_schemas 排序稳定**：确保工具定义排序不变（当前已按名称排序）

4. 自适应策略：检测 provider 是否支持缓存，选择不同的侧查询频率

```rust
pub fn provider_supports_cache(provider: &LlmProvider) -> bool {
    matches!(provider, LlmProvider::Anthropic { .. } | LlmProvider::OpenAIChat { .. } | LlmProvider::OpenAIResponses { .. })
}
```

**成本对比**（50K token 对话上下文）：

| 场景 | 无缓存 | 有缓存 | 节省 |
|------|--------|--------|------|
| Tier 3 摘要 | ~$0.15 input | ~$0.015 input | 90% |
| 记忆提取 | ~$0.15 input | ~$0.015 input | 90% |
| 记忆选择 | ~$0.15 input | ~$0.015 input | 90% |
| 每会话 3 次侧查询 | ~$0.45 | ~$0.045 | 90% |

**影响文件**：
- `src-tauri/src/agent/forked_query.rs` — **新建**，统一侧查询接口
- `src-tauri/src/agent/mod.rs` — 注册模块，`AssistantAgent` 增加 `side_query()` 方法
- `src-tauri/src/agent/types.rs` — 增加 `current_system_prompt` 和 `current_tool_schemas` 缓存字段
- `src-tauri/src/agent/providers/anthropic.rs` — 保存每轮 system_prompt/tools 到 agent 字段

**改造现有侧查询**：
- `src-tauri/src/context_compact/summarization.rs` — Tier 3 摘要改用 `side_query()`
- `src-tauri/src/memory_extract.rs` — 记忆提取改用 `side_query()`（Phase 4 实施时）
- `src-tauri/src/memory/selection.rs` — 记忆选择改用 `side_query()`（Phase 4 实施时）

**验证**：
- 主对话后立即发起 side_query，检查 `cache_read_tokens > 0`
- 对比 side_query 前后的 input token 成本（应降低 ~90%）
- 超过 5 分钟后发起 side_query，确认退化为正常调用（无报错）

---

## Phase 3: 上下文压缩增强

> 目标：提升长对话的信息保真度
> 依赖：Phase 2（Tier 3 摘要可复用 forked query 降低成本）

### 3A. 后压缩恢复 [M]

**问题**：Tier 3 LLM 摘要后丢失最近编辑文件的精确内容，模型需重新 read。

**方案**：
- 摘要完成后扫描被压缩消息中的 `write`/`edit`/`apply_patch` 目标文件
- 取最近 3 个文件，读取当前磁盘内容（每文件 ≤ 4KB）
- 作为合成 tool_result 注入压缩后的对话
- 恢复 token 总量不超过释放量的 10%

**影响文件**：
- `src-tauri/src/context_compact/recovery.rs` — **新建**
- `src-tauri/src/context_compact/mod.rs` — 注册模块
- `src-tauri/src/agent/context.rs` — 摘要后调用 recovery

**验证**：编辑文件 → 触发 Tier 3 → 压缩后仍有文件最新内容

---

### 3B. API-Round 消息分组 [S]

**问题**：压缩分割可能拆散 `assistant(tool_use)` + `user(tool_result)` 配对。

**方案**：
- Tool loop 中给 assistant 和 user 消息打 `"_round": N` 标签
- 压缩切割点对齐到 round 边界
- `_round` 字段在 API 调用前自动忽略（不影响上游接口）

**影响文件**：
- `src-tauri/src/agent/providers/*.rs` — 消息标记
- `src-tauri/src/context_compact/summarization.rs` — 切割对齐

**验证**：压缩后 tool_use 和 tool_result 始终成对

---

## Phase 4: 记忆系统升级

> 目标：记忆自动积累 + 精准注入
> 依赖：Phase 2（forked query 降低提取/选择成本）

### 4A. 自动记忆提取 [M]

**问题**：记忆只在 `flush_before_compact` 时提取，大量信息流失。

**方案**：
- 每轮 non-tool_use 响应后检测是否应提取
- 后台异步调用 `side_query()`（复用 Phase 2 的 forked agent 机制）
- **自适应频率**：
  - 支持 prompt cache 的 provider（Anthropic/OpenAI）：**3 轮间隔 / 每会话 5 次**
  - 不支持 cache 的 provider：**10 轮间隔 / 每会话 3 次**
- 非 plan mode 才触发
- 互斥：主 Agent 已调用 save_memory 时跳过自动提取

**影响文件**：
- `src-tauri/src/memory_extract.rs` — `should_auto_extract()` 边界检测，改用 `side_query()`
- `src-tauri/src/agent/providers/anthropic.rs` — tool loop 结束后调用
- `src-tauri/src/agent/types.rs` — 增加 `last_extraction_turn`、`extraction_count`

**验证**：20 轮对话后 memory 表自动出现提取条目；Anthropic provider 下 cache_read_tokens > 0

---

### 4B. LLM 记忆语义选择 [M]

**问题**：hybrid search 可能返回不相关记忆，无最终过滤。

**方案**：
- 候选数 > 8 时，用 `side_query()` 选 ≤ 5 条最相关记忆
- 单次快速 API 调用（小输入/小输出）
- 默认使用主模型，可配置覆盖
- Opt-in 配置：`memory.llm_selection: bool`

**影响文件**：
- `src-tauri/src/memory/sqlite.rs` — `build_prompt_summary()` 增加 LLM 过滤
- `src-tauri/src/memory/selection.rs` — **新建**，调用 `side_query()`
- `src-tauri/src/agent/config.rs` — 配置字段

**验证**：20 条记忆中，注入的 5 条与当前问题高度相关

---

## Phase 5: Skill 和 Plan 模式加固

> 目标：工具隔离 + 纵深防御
> 依赖：5C 依赖子 Agent 系统（现有）

### 5A. Skill allowed-tools [S]

**问题**：Skill 激活时可使用所有工具。

**方案**：SKILL.md frontmatter 增加 `allowed-tools` 字段，激活时只保留指定工具。向后兼容（空 = 全部）。

**影响文件**：
- `src-tauri/src/skills.rs` — SkillEntry + frontmatter 解析
- `src-tauri/src/agent/mod.rs` — `apply_skill_tools()` 过滤

---

### 5B. Plan 执行层权限强制 [M]

**问题**：Plan 模式仅靠 schema 过滤，执行层无兜底。

**方案**：`execute_tool_with_context()` 增加 plan mode 工具白名单检查。

**影响文件**：
- `src-tauri/src/tools/execution.rs` — plan mode 拦截
- `src-tauri/src/plan.rs` — 暴露 allowed_tools
- `src-tauri/src/tools/execution.rs` — ToolExecContext 增加字段

---

### 5C. Skill Fork 模式 [L]

**问题**：Skill 内部 tool_call 污染主对话历史。

**方案**：`context: fork` frontmatter，激活时 spawn 子 Agent 执行，结果注入主对话。

**影响文件**：
- `src-tauri/src/skills.rs` — context_mode 字段
- `src-tauri/src/subagent/spawn.rs` — skill-aware spawn
- `src-tauri/src/tools/subagent.rs` — skill_fork action

---

## Phase 6: Agent 调度增强

> 目标：长任务不阻塞主对话
> 依赖：现有子 Agent 系统

### 6A. 前台/后台自动切换 [L]

**问题**：子 Agent 阻塞 tool_result 返回。

**方案**：新增 `spawn_and_wait` action，超时后自动转后台。后台完成通过 steer mailbox 注入结果。

**影响文件**：
- `src-tauri/src/tools/subagent.rs` — spawn_and_wait action
- `src-tauri/src/subagent/spawn.rs` — 超时等待 + 后台转换
- `src-tauri/src/subagent/injection.rs` — 后台结果注入
- `src-tauri/src/system_prompt.rs` — 工具描述更新

---

## Phase 7: 延迟工具加载

> 目标：显著减少每请求 token 消耗
> 依赖：所有工具定义稳定后实施

### 7A. Tool 延迟加载 [L]

**问题**：35+ 工具 schema 全量发送，浪费 3-5K token/请求。

**方案**：
- 核心工具（~10 个）始终加载：`exec`, `read`, `write`, `edit`, `ls`, `grep`, `find`, `apply_patch`, `subagent`, `tool_search`
- 其余 ~25 个工具延迟加载：系统提示只列名称 + 一行描述
- 新增 `tool_search` 元工具：按关键词返回完整 schema
- 容错：模型直接调用 deferred 工具时仍正常执行
- Opt-in 配置开关，默认关闭

**影响文件**：
- `src-tauri/src/tools/definitions.rs` — deferred 标志 + tool_search 定义
- `src-tauri/src/tools/execution.rs` — tool_search 执行
- `src-tauri/src/tools/mod.rs` — 核心/延迟分离函数
- `src-tauri/src/system_prompt.rs` — 延迟工具摘要段
- `src-tauri/src/agent/providers/*.rs` — 只发核心 schema

---

## 实施时间线

```
Phase 1 ─── Tool 基础 (1A 并发 + 1B 微压缩 + 1C 落盘)
  │
Phase 2 ─── Forked Agent 缓存共享 (2A 统一侧查询接口)  ← 可与 Phase 1 并行
  │
  ├── Phase 3 ─── 压缩增强 (3A 恢复 + 3B 分组)         ← 依赖 Phase 2
  │
  └── Phase 4 ─── 记忆升级 (4A 自动提取 + 4B LLM 选择)  ← 依赖 Phase 2，可与 Phase 3 并行
        │
Phase 5 ─── Skill/Plan 加固 (5A allowed-tools + 5B 权限强制 + 5C Fork)
  │
Phase 6 ─── Agent 调度 (6A 前后台切换)
  │
Phase 7 ─── 延迟加载 (7A tool_search)
```

Phase 1 + Phase 2 可并行启动。Phase 3 和 Phase 4 依赖 Phase 2 但彼此可并行。

---

## 架构决策记录（已确认）

| # | 决策 | 最终选择 | 理由 |
|---|------|---------|------|
| 1 | 并发执行粒度 | **分组执行**（并发组先并行，串行组后执行） | 简单高效，tool_calls 本身无序 |
| 2 | 磁盘持久化阈值 | **可配置**，默认 50KB | 不同模型上下文差异大，需要灵活调整 |
| 3 | 延迟加载默认 | **默认关闭**，opt-in | 不同 provider/模型理解力不同，需充分测试 |
| 4 | 记忆提取频率 | **自适应**：有 cache 的 provider 3 轮/5 次，无 cache 的 10 轮/3 次 | Forked Agent 缓存共享使激进策略成本可控 |
| 5 | Skill fork 生命周期 | **单次（one-shot）** | 简单可控，持续交互应用 subagent |
| 6 | LLM 记忆选择模型 | **默认主模型，可配置覆盖** | 与现有模型选择策略一致，不引入新概念 |

# Side Query 架构（Prompt Cache Sharing）

> 低成本 LLM 侧查询基础设施，复用主对话 prompt cache，降低侧查询成本约 90%。

## 概述

Side Query 是一种缓存友好的非流式 LLM 调用机制。它复用主对话的 `system_prompt + tool_schemas + conversation_history` 作为请求前缀，利用 Anthropic 的显式 prompt caching 和 OpenAI 的自动前缀缓存，使侧查询只需为新增的指令部分付费。

```
主对话请求:   [system_prompt][tools][msg1..msgN][user_msg]     → 缓存创建
                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
侧查询请求:   [system_prompt][tools][msg1..msgN][side_query]   → 缓存命中！
                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
              只有 [side_query] 部分按全价计费
```

## 核心组件

### CacheSafeParams（缓存安全参数）

每次主对话 API 调用构建完请求后，保存一份快照：

```rust
// agent/types.rs
pub(super) struct CacheSafeParams {
    pub system_prompt: String,          // 系统提示词原文
    pub tool_schemas: Vec<Value>,       // 工具 schema（provider 格式）
    pub conversation_history: Vec<Value>, // 对话历史（provider 格式，已归一化）
    pub provider_format: ProviderFormat,  // 标记来源 provider
}
```

- 存储在 `AssistantAgent.cache_safe_params: Mutex<Option<CacheSafeParams>>`
- 在 compaction 之后、tool loop 之前保存（4 个 provider 统一注入）
- 字节级一致性：使用与主请求完全相同的 system_prompt 和 tool_schemas

### side_query() 方法

```rust
// agent/side_query.rs
impl AssistantAgent {
    pub async fn side_query(
        &self,
        instruction: &str,  // 侧查询指令
        max_tokens: u32,    // 最大输出 token
    ) -> Result<SideQueryResult>
}
```

**特性**：
- 非流式（`stream: false`），单轮，无 tool loop，无 compaction
- 有缓存参数时：构建完整请求（system + tools + history + instruction），复用缓存前缀
- 无缓存参数时：退化为最小请求（仅 instruction）
- 返回 `SideQueryResult { text, usage }` 含缓存命中指标

### Provider 适配

| Provider | 缓存机制 | Side Query 处理 |
|----------|---------|----------------|
| Anthropic | 显式 `cache_control: { type: "ephemeral" }` | system + 最后一个 tool 标记 `cache_control`，字节一致 |
| OpenAI Chat | 自动前缀缓存 | 保持 system + history 前缀一致即可 |
| OpenAI Responses | 自动前缀缓存 | 保持 instructions + input 前缀一致 |
| Codex | 同 Responses | 同 Responses |

## 使用场景

### 1. Tier 3 上下文摘要（`context.rs`）

```rust
// summarize_with_model() 优先使用 side_query
if has_cache {
    let instruction = format!(
        "<summarization_instructions>{}</summarization_instructions>\n\n{}",
        SUMMARIZATION_SYSTEM_PROMPT, prompt
    );
    let result = self.side_query(&instruction, max_tokens).await?;
    return Ok(result.text);
}
// fallback: 直接 HTTP 调用
```

### 2. 记忆提取（`memory_extract.rs`）

```rust
// run_extraction() 接受可选的 main_agent 引用
pub async fn run_extraction(
    messages: &[Value],
    agent_id: &str,
    session_id: &str,
    provider_config: &ProviderConfig,
    model_id: &str,
    main_agent: Option<&AssistantAgent>,  // 有则用 side_query
)
```

### 3. 未来场景

- **记忆语义选择**：候选记忆 > 阈值时用 side_query 筛选（Phase 4）
- **自动记忆提取**：每 N 轮自动后台提取（Phase 4）

## 成本效益

以 50K token 对话上下文为例：

| 操作 | 无缓存 (input) | 有缓存 (input) | 节省 |
|------|---------------|---------------|------|
| Tier 3 摘要 | ~$0.15 | ~$0.015 | 90% |
| 记忆提取 | ~$0.15 | ~$0.015 | 90% |
| 每会话 3 次侧查询 | ~$0.45 | ~$0.045 | 90% |

## 缓存约束

1. **时效性**：Anthropic cache TTL 为 5 分钟，侧查询须在主请求后 5 分钟内发出
2. **前缀一致**：system_prompt + tool_schemas 必须字节级一致
3. **Provider 匹配**：缓存参数的 `provider_format` 必须与当前 provider 匹配
4. **Fallback 安全**：无缓存参数时自动退化为普通请求，不影响功能

## 文件清单

| 文件 | 职责 |
|------|------|
| `agent/types.rs` | CacheSafeParams / ProviderFormat / SideQueryResult 类型定义 |
| `agent/side_query.rs` | 核心实现：save_cache_safe_params() + side_query() + 4 个 provider 适配 |
| `agent/providers/*.rs` | 4 个 provider 在 compaction 后调用 save_cache_safe_params() |
| `agent/context.rs` | summarize_with_model() 优先走 side_query 路径 |
| `memory_extract.rs` | run_extraction() 支持 side_query 路径 |

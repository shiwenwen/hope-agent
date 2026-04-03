# 工具执行架构

## 工具定义

每个工具由 `ToolDefinition` 结构体定义（`tools/definitions.rs`）：

```rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,       // JSON Schema
    pub internal: bool,          // 内部工具免审批
    pub concurrent_safe: bool,   // 并发安全标记
}
```

### 并发安全标记

`concurrent_safe: bool` 决定工具是否可在同一轮次内与其他工具并行执行：

| 并发安全（parallel） | 串行执行（sequential） |
|---------------------|----------------------|
| read, ls, grep, find | exec, write, edit, apply_patch |
| recall_memory, memory_get | save_memory, update_memory, delete_memory |
| web_search, web_fetch | browser, subagent, canvas |
| agents_list, sessions_list | image_generate, sessions_send |
| session_status, sessions_history | update_core_memory, manage_cron |
| image, pdf, get_weather | send_notification, acp_spawn |
| plan_question | submit_plan, amend_plan, update_plan_step |

查询接口：`tools::is_concurrent_safe(name: &str) -> bool`

## Tool Loop 执行流程

```
模型响应包含 tool_calls[]
    ↓
分组: partition by is_concurrent_safe()
    ↓
Phase 1: 并发安全组 → join_all() 并行执行
    ↓
Phase 2: 串行组 → for loop 逐个执行
    ↓
所有结果合并为 tool_results[] 推入对话历史
    ↓
Tier 1 截断检查
    ↓
下一轮 API 调用（或退出 loop）
```

每个工具执行都通过 `tokio::select!` 与 cancel flag 竞争，支持用户随时取消。

## 工具结果磁盘持久化

当工具返回结果超过阈值时，自动写入磁盘：

- **阈值**：默认 50KB，通过 `config.json` → `toolResultDiskThreshold` 配置（0 = 禁用）
- **存储路径**：`~/.opencomputer/tool_results/{session_id}/{tool_name}_{timestamp}.txt`
- **上下文内容**：head 2KB + `[...N bytes omitted...]` + tail 1KB + 路径引用
- **访问方式**：模型可通过 read 工具读取完整文件

```
工具返回 200KB 结果
    ↓
result.len() > threshold (50KB)
    ↓
写入磁盘: ~/.opencomputer/tool_results/sess_abc/read_1712345678.txt
    ↓
返回给模型:
  [前 2000 字符]
  [...197000 bytes omitted...]
  [后 1000 字符]
  [Full result (200000B) saved to: ~/.opencomputer/tool_results/...]
  [Use read tool with this path to access full content]
```

## 上下文压缩（5 层渐进式）

```
Tier 0: 微压缩 — 零成本清除旧的临时工具结果 (ls/grep/find 等)
  ↓ (无条件运行)
Tier 1: 截断 — 单个过大工具结果 head+tail 截断
  ↓ (usage > 30%)
Tier 2: 裁剪 — 旧工具结果 soft-trim (head+tail) 或 hard-clear (占位符)
  ↓ (usage > 50%/70%)
Tier 3: LLM 摘要 — 调用模型压缩旧消息
  ↓ (usage > 85%)
Tier 4: 紧急 — 清除所有工具结果 + 只保留最近 N 轮
  ↓ (ContextOverflow 错误)
```

### Tier 0 微压缩

- **触发**：每次 `compact_if_needed()` 调用时最先运行
- **目标工具**：`ls`, `grep`, `find`, `process`, `sessions_list`, `agents_list`（可配置）
- **保护边界**：`keep_last_assistants` 个最近 assistant 消息之前的结果才清除
- **原理**：构建 `tool_use_id → tool_name` 映射（支持 Anthropic/OpenAI/Responses 三种消息格式），匹配目标工具后替换为 `[Ephemeral tool result cleared]`
- **配置**：`config.json` → `compact.microcompactEnabled` + `compact.microcompactTools`

## 工具审批

- `internal: true` 的工具永不需要审批（memory、cron、notification 等）
- `ToolPermissionMode` 三级模式：`FullApprove`（全通过）、`Auto`（按配置）、`AskEveryTime`
- Plan Mode 下通过 `plan_mode_allow_paths` 限制写入路径

# Phase 3: 上下文压缩增强

> 后压缩文件恢复 (3A) + API-Round 消息分组 (3B)

## 概述

Phase 3 增强了 5 层渐进式上下文压缩系统的信息保真度：

- **3A 后压缩恢复**：Tier 3 LLM 摘要后自动从磁盘读取最近编辑的文件内容并注入对话，省去额外的 read tool call
- **3B API-Round 分组**：通过 `_oc_round` 元数据标记 tool loop 中的消息，确保压缩切割不拆散 tool_use/tool_result 配对

---

## 3A: 后压缩文件恢复

### 问题

Tier 3 摘要将旧消息替换为 LLM 生成的摘要。工具结果（如文件内容）仅以 500 字符预览出现在摘要 prompt 中，精确内容不可恢复。模型继续编辑文件时必须先执行一轮 `read_file` tool call。

### 方案

`context_compact/recovery.rs` 在 `apply_summary()` 后执行：

1. 扫描被摘要的消息，提取 `write`/`edit`/`apply_patch` 工具调用的目标文件路径
2. 去重：跳过 preserved 消息中已有的文件
3. 按出现顺序取最近 5 个文件（可配置），从磁盘读取当前内容
4. 每文件最多 16KB，总量不超过释放 token 的 10%（兜底 100K chars ≈ 25K tokens）
5. 构建 `[Post-compaction file recovery]` 用户消息，插入 summary 之后、preserved 之前

### 恢复消息格式

```
[Post-compaction file recovery: current contents of recently-edited files]

<file path="/path/to/file.rs">
...file content...
</file>

<file path="/path/to/other.ts">
...file content...
</file>
```

### 格式无关的工具调用提取

支持所有 4 种 provider 格式：

| Provider | 工具调用格式 |
|----------|-------------|
| Anthropic | `content[].type == "tool_use"` → `input.path` |
| OpenAI Chat | `tool_calls[].function` → parse `arguments` JSON |
| Responses | `type == "function_call"` → parse `arguments` JSON |
| Codex | 同 Responses |

### 配置

```json
{
  "compact": {
    "recoveryEnabled": true,
    "recoveryMaxFiles": 5,
    "recoveryMaxFileBytes": 16384
  }
}
```

### 预算守卫

- 每文件 ≤ `recoveryMaxFileBytes`（默认 16KB）
- 总量 ≤ `min(freed_tokens * 10% * 4, 100_000)` 字符
- 文件不存在时静默跳过（可能已被删除或重命名）

---

## 3B: API-Round 消息分组

### 问题

OpenAI Chat 格式中，每个 tool result 是独立的 `{ role: "tool" }` 消息。`split_for_summarization()` 按 user 消息边界切割，理论上可能在 assistant(tool_calls) 和后续 tool 消息之间断开。`emergency_compact()` 同理。

虽然现有 `is_user_message()` 过滤已对 Anthropic 和 OpenAI Chat 格式提供隐式保护，但对 Responses/Codex 格式（`function_call`/`function_call_output` 没有 `role` 字段）存在实际风险。

### 方案

#### Round 标记

在 4 个 provider 的 tool loop 中，为 assistant 消息和对应的 tool result 消息打上 `_oc_round: "r{N}"` 标记：

```
msg[0]: { role: "user", content: "..." }
msg[1]: { role: "assistant", tool_calls: [...], _oc_round: "r0" }
msg[2]: { role: "tool", tool_call_id: "tc1", _oc_round: "r0" }
msg[3]: { role: "tool", tool_call_id: "tc2", _oc_round: "r0" }
msg[4]: { role: "user", content: "..." }
```

同一 round 的所有消息共享相同的 `_oc_round` 值。

#### API 调用前剥离

`prepare_messages_for_api()` 克隆消息并移除 `_oc_round` 字段，确保 API 请求体干净。

#### Round-Aware 压缩

- `split_for_summarization()`：找到原始边界后，调用 `find_round_safe_boundary()` 向前调整到 round 边界
- `emergency_compact()`：找到 `keep_from` 后，调用 `find_round_safe_boundary_forward()` 向后调整

#### 向后兼容

无 `_oc_round` 标记的旧会话（从 DB 恢复）退化为原始行为——`find_round_safe_boundary()` 在无标记时直接返回原始 index。

### 关键函数

| 函数 | 文件 | 用途 |
|------|------|------|
| `stamp_round()` | `round_grouping.rs` | 在消息上打 round 标记 |
| `prepare_messages_for_api()` | `round_grouping.rs` | 克隆并剥离标记 |
| `find_round_safe_boundary()` | `round_grouping.rs` | 向前找安全边界（Tier 3） |
| `find_round_safe_boundary_forward()` | `round_grouping.rs` | 向后找安全边界（Tier 4） |

---

## 文件清单

| 文件 | 操作 |
|------|------|
| `context_compact/round_grouping.rs` | **新建** — Round 分组 + 安全边界 |
| `context_compact/recovery.rs` | **新建** — 后压缩文件恢复 |
| `context_compact/mod.rs` | 修改 — 注册模块 + re-exports |
| `context_compact/config.rs` | 修改 — recovery_* 配置字段 |
| `context_compact/summarization.rs` | 修改 — round-aware split |
| `context_compact/compact.rs` | 修改 — round-aware emergency |
| `agent/context.rs` | 修改 — 注入恢复消息 |
| `agent/providers/*.rs` | 修改 — stamp round + strip for API |

---

## 与 claude-code 的对比

| 特性 | claude-code | OpenComputer |
|------|------------|-------------|
| 恢复文件数 | 5 | 5（可配置） |
| 每文件预算 | 5K tokens ≈ 20KB | 16KB（可配置） |
| 总预算 | 50K tokens | ≤ freed 10% 且 ≤ 25K tokens |
| Round 分组 | `groupMessagesByApiRound()` 按 message.id | `_oc_round` 元数据标记 |
| 配对修复 | `ensureToolResultPairing()` 事后修复 | 预防性：round-aware 切割 |
| Skill 恢复 | 支持（truncated skill content） | 暂不支持 |

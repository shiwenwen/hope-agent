# Phase 4: 记忆系统升级

> 自动记忆提取 (4A) + LLM 语义选择 (4B)

## 概述

Phase 4 升级记忆系统的两个核心能力：
- **4A 自动提取**：默认开启记忆提取，inline 执行（支持 side_query 缓存共享），互斥保护 + 频率控制
- **4B LLM 选择**：当候选记忆过多时，用 side_query 选择最相关的 ≤5 条注入系统提示

---

## 4A: 自动记忆提取增强

### 改进项

| 特性 | 改进前 | 改进后 |
|------|--------|--------|
| 默认启用 | `auto_extract: false` | `auto_extract: true` |
| flush_before_compact | 默认关闭 | 默认开启 |
| side_query 支持 | `None`（tokio::spawn 无法传引用）| `Some(&agent)`（inline 执行）|
| 频率控制 | 仅 `min_turns` 阈值 | + `max_extractions_per_session`（默认 5）|
| 互斥保护 | 无 | 检测 save_memory / update_core_memory 调用 |

### 架构变化

**提取触发从 `tokio::spawn` 改为 inline async**：

```
改进前:
  chat() → save_context → tokio::spawn(run_extraction(None)) → return
                                           ↑ 无法传 &agent

改进后:
  chat() → save_context → run_extraction(Some(&agent)).await → return
                                           ↑ 支持 side_query 缓存共享
```

Inline 执行增加 1-3 秒延迟，但：
1. 用户响应已流式完成，不影响感知延迟
2. side_query 命中缓存后仅需处理增量 token，成本降低 ~90%
3. 频率上限防止过度提取

### 互斥保护

4 个 Provider 的 tool 执行循环中检测 `save_memory` / `update_core_memory` 调用，设置 `manual_memory_saved` 标志。提取触发前检查此标志，避免重复提取。

### 配置

```json
{
  "memoryExtract": {
    "autoExtract": true,
    "extractMinTurns": 3,
    "flushBeforeCompact": true,
    "maxExtractionsPerSession": 5
  }
}
```

---

## 4B: LLM 记忆语义选择

### 问题

`build_prompt_summary()` 加载全部记忆（agent + global，最多 400 条），按类型分组后注入系统提示。当记忆积累到数十条时，大量无关记忆占据系统提示空间，降低模型注意力精度。

### 方案

`select_memories_if_needed()` 在每个 Provider 的系统提示构建后、cache 快照前运行：

```
1. 加载全部候选记忆（load_prompt_candidates）
2. 候选数 > threshold（默认 8）→ 触发 LLM 选择
3. 构建 compact manifest（id + 首行预览）
4. side_query() 调用选择 prompt
5. 解析 JSON 数组获取选中 ID
6. 用选中记忆重建 # Memory 段，替换系统提示中的原始段
7. 精简后的系统提示保存到 cache_safe_params
```

### 选择 Prompt

```
Given the user's current message and candidate memories,
select the most relevant memories (up to {MAX}).
Return ONLY a JSON array of memory IDs.

User's message: {MESSAGE}
Candidate memories (id: preview):
1: User prefers Rust
2: Project deadline Friday
...
```

### `build_prompt_summary` 重构

拆分为两个可复用函数：
- `load_prompt_candidates(agent_id, shared)` → `Vec<MemoryEntry>`
- `format_prompt_summary(entries, budget)` → `String`

`build_prompt_summary()` 保持原签名（内部调用 load + format），向后兼容。

### 配置

```json
{
  "memorySelection": {
    "enabled": false,
    "threshold": 8,
    "maxSelected": 5
  }
}
```

默认关闭（opt-in），通过设置面板开启。

### Fallback

- side_query 失败 → 退化为全量注入（app_warn 日志）
- 无候选或候选 ≤ threshold → 跳过选择，使用全量
- 解析失败（空数组）→ 保持原始系统提示不变

---

## 文件清单

| 文件 | 操作 |
|------|------|
| `memory/types.rs` | 修改：默认值 + max_extractions + MemorySelectionConfig |
| `memory/selection.rs` | **新建**：选择 prompt + 解析 + replace_memory_section |
| `memory/sqlite.rs` | 修改：load_prompt_candidates + format_prompt_summary |
| `memory/helpers.rs` | 修改：load_memory_selection_config |
| `memory/traits.rs` | 修改：load_prompt_candidates trait 方法 |
| `memory/mod.rs` | 修改：注册 selection 模块 |
| `agent/types.rs` | 修改：extraction_count + manual_memory_saved |
| `agent/mod.rs` | 修改：reset_chat_flags + select_memories_if_needed |
| `agent/providers/*.rs` | 修改：reset_chat_flags + 互斥检测 + select_memories |
| `chat_engine.rs` | 修改：inline 提取替代 spawn |
| `provider.rs` | 修改：MemorySelectionConfig 字段 |
| `commands/memory.rs` | 修改：选择配置命令 |
| `lib.rs` | 修改：注册新命令 |

---

## 与 claude-code 对比

| 特性 | claude-code | OpenComputer |
|------|------------|-------------|
| 提取触发 | 每轮 fire-and-forget（forked agent） | 每轮 inline async（side_query） |
| 提取能力 | 可读写文件（forked agent 有工具） | 仅 JSON 提取（无工具调用） |
| 互斥 | 检测文件写入 | 检测 save_memory 工具调用 |
| 频率控制 | turnsSinceLastExtraction | max_extractions_per_session |
| 选择输入 | 文件名 + 描述 + 类型（manifest） | id + 首行预览 |
| 选择模型 | Sonnet (hardcoded) | 主模型 side_query |
| 选择预取 | 非阻塞 prefetch | 同步（compaction 后） |
| 存储 | Markdown 文件 + frontmatter | SQLite + FTS5 + 向量 |

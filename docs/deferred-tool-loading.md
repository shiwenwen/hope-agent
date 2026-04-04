# Phase 7: 延迟工具加载

## 概述

Phase 7 实现延迟工具加载（Deferred Tool Loading）+ `tool_search` 元工具。30+ 工具 schema 从全量发送（3-5K token/请求）降低到核心 10 个（约 1.5K token），其余通过 `tool_search` 按需发现。

## 升级前后对比

| 特性 | 升级前 | 升级后 | claude-code 参考 |
|------|--------|--------|-----------------|
| 工具 schema 发送 | 全部 30+ 工具（3-5K token/请求） | 核心 10 个 + tool_search（约 1.5K token） | 相同策略：`shouldDefer` + `ToolSearchTool` |
| 工具发现 | 无需发现（全量加载） | `tool_search` 关键词 / 精确匹配搜索 | 相同：`select:name` + keyword search |
| 容错 | 不适用 | 直接调用 deferred 工具仍正常执行（execution dispatch 不变） | 相同：execution layer 不受影响 |
| 默认状态 | 不适用 | opt-in（`deferredTools.enabled`，默认关闭） | 自动开启（MCP 工具超 10% context window 时） |
| token 节省 | 0 | 约 50-70%（从 ~4K 降至 ~1.5K） | 类似节省幅度 |
| 自动启用 | 无 | 暂不实现（后续可加 auto:N% 阈值） | `ENABLE_TOOL_SEARCH=auto:N` 自适应阈值 |

## 核心工具（始终加载）

```rust
const CORE_TOOL_NAMES: &[&str] = &[
    "exec", "process", "read", "write", "edit",
    "ls", "grep", "find", "apply_patch",
];
```

加上 `subagent`（条件注入但始终加载）和 `tool_search` 自身。

## 延迟工具（通过 tool_search 发现）

- 内存工具：`save_memory`, `recall_memory`, `update_memory`, `delete_memory`, `update_core_memory`, `memory_get`
- Web 工具：`web_search`, `web_fetch`, `browser`
- 媒体工具：`image`, `image_generate`, `pdf`, `canvas`
- 系统工具：`manage_cron`, `send_notification`, `get_weather`
- 会话工具：`agents_list`, `sessions_list`, `session_status`, `sessions_history`, `sessions_send`
- 其他：`acp_spawn`

## tool_search 元工具

### 查询语法

| 语法 | 说明 | 示例 |
|------|------|------|
| `select:name1,name2` | 精确匹配（按名称） | `select:save_memory,recall_memory` |
| `keyword1 keyword2` | 模糊搜索（名称 + 描述评分） | `memory save` |

### 评分机制

| 匹配类型 | 分值 |
|----------|------|
| 精确名称匹配 | 10 |
| 名称包含完整查询 | 5 |
| 名称包含关键词 | 2 |
| 描述包含关键词 | 1 |

### 返回格式

```json
{
  "matched_tools": 3,
  "total_deferred_tools": 20,
  "tools": [
    {
      "name": "save_memory",
      "description": "Save a memory entry...",
      "parameters": { "type": "object", ... }
    }
  ]
}
```

## 系统提示集成

当 deferred 开启时，系统提示追加一个 "Additional Tools" 段落：

```
# Additional Tools (use tool_search to discover)
The following tools are available but their schemas are not loaded by default.
Use `tool_search(query="keyword")` to get the full schema before calling them.

- **web_search**: Search the web for information
- **save_memory**: Save a memory entry
- **browser**: Navigate web pages interactively
...
```

## 容错设计

1. **execution dispatch 不变**：所有工具仍在 `execute_tool_with_context()` 的 match 分支中，模型直接调用 deferred 工具仍正常执行
2. **默认关闭**：`deferredTools.enabled` 默认 `false`，不影响现有用户
3. **Plan 模式兼容**：Plan Agent 工具白名单不受 deferred 影响（Plan 过滤在 deferred 分支之后应用）

## 配置

```json
// config.json
{
  "deferredTools": {
    "enabled": true
  }
}
```

前端通过 `get_deferred_tools_config` / `save_deferred_tools_config` Tauri 命令管理。

## 关键文件

| 文件 | 变更 |
|------|------|
| `src-tauri/src/tools/definitions.rs` | `ToolDefinition` 新增 `deferred`/`always_load` 字段；`CORE_TOOL_NAMES` 常量；`get_core_tools()`/`get_deferred_tools()`/`get_core_tools_for_provider()`/`get_tool_search_tool()` 函数 |
| `src-tauri/src/tools/tool_search.rs` | **新建** — `tool_search()` handler，支持 select: 和关键词搜索 |
| `src-tauri/src/tools/mod.rs` | 新增 `tool_search` 模块、`TOOL_TOOL_SEARCH` 常量、re-exports |
| `src-tauri/src/tools/execution.rs` | dispatch match 新增 `TOOL_TOOL_SEARCH` |
| `src-tauri/src/agent/providers/*.rs` | 4 个 Provider 适配 deferred 分支（核心 schema + tool_search） |
| `src-tauri/src/system_prompt.rs` | `build_deferred_tools_section()` 生成延迟工具列表段落 |
| `src-tauri/src/provider.rs` | `DeferredToolsConfig` 配置；`ProviderStore` 新增字段 |
| `src-tauri/src/commands/config.rs` | `get_deferred_tools_config`/`save_deferred_tools_config` 命令 |
| `src-tauri/src/lib.rs` | 注册新 Tauri 命令 |

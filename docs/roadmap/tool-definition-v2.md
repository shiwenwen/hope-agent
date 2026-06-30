# ToolDefinition v2 RFC

> 返回 [路线图索引](README.md)
>
> 状态：Phase 1 implemented, pending broader coding workflow usage
>
> 更新时间：2026-06-30

## 背景

Phase 1 的目标是让工具成为可推理、可搜索、可审计的对象。旧版 `ToolDefinition` 只稳定表达 `tier`、`internal`、`concurrent_safe`、`async_capable` 和 provider schema；这足够执行工具，但不足以支撑后续 coding workflow / loop / review / LSP 对工具风险、输入形态、结果形态和按需发现的判断。

本 RFC 将 v2 元数据定义为 sidecar，而不是把所有字段塞进每个工具定义点：

```rust
impl ToolDefinition {
    pub fn v2_metadata(&self) -> ToolMetadata;
}
```

## 目标

1. 所有内置工具和动态 MCP 工具都能得到 v2 metadata。
2. `tool_search` 可基于 alias、search hint、参数、effect、risk 做加权检索。
3. API / Tauri 工具列表和 `tool_search` 返回同一套 metadata。
4. 第一版不改变执行期安全边界；权限仍由 `permission::engine`、Plan / Skill / KB gate、`ToolExecContext::is_tool_visible` 和执行层兜底负责。
5. 后续 workflow / loop 可以读取 metadata 来做 planning、review、read-only 执行和 trace 分类。

## 非目标

- 不把 metadata 当作新的审批引擎。
- 不为每个工具手填完整结构体。
- 不让模型根据 metadata 绕过 live gate。
- 不一次性实现 Coding Mode workflow。

## 字段

| 字段 | 说明 |
| --- | --- |
| `aliases` / `search_hints` | 工具发现和自然语言检索提示 |
| `effects` | 工具效果分类，如 `read_file_system`、`write_file_system`、`execute_process`、`external_service_write`、`task_write` |
| `risk` | `low` / `medium` / `high` / `strict` 粗风险摘要 |
| `read_only` / `destructive` / `open_world` / `strict` | workflow / review / UI 可读的行为特征 |
| `interrupt_behavior` | `immediate` / `graceful` / `long_running` / `human_blocked` |
| `permission` / `permission_matcher` | 粗粒度 subject + approval hint，供解释和未来策略层使用 |
| `input` | 从 JSON Schema 派生 required、path、command、url、query、content、timeout、action 等参数形态 |
| `path_extractor` | 路径归因的候选参数和 primary path 参数 |
| `validation` | strict schema、required、alias 参数提示 |
| `render` | 结果形态和主资源提示 |
| `search_text` | 已拼好的搜索语料 |
| `auto_classifier_input` / `classifier_tags` | workflow / review / loop 的分类输入 |

## 语义约定

- `runtime_control` 是运行时状态 / 控制域，不单独表示写入。`tool_search`、`job_status` 可以是 `read_only=true`；`runtime_cancel` 这类动作由 `destructive` 表达。
- 外部服务写入使用 `external_service_write`，不复用本机 `settings_write`。
- `strict=true` 表示工具可能触发 strict 审批路径，不代表每次调用都 strict。
- `read_only` 是工具级保守摘要；带 `action` 的多态工具未来可以再细化 action-level metadata。
- `permission.subject=internal` 只说明该工具内部执行不弹审批，不说明没有副作用；副作用应看 `effects`。

## Deferred 默认策略

Phase 1 默认启用 deferred tool loading，并预置低频、大 schema、场景化工具：

- `browser`
- `mac_control`
- `image`
- `pdf`
- `get_weather`
- `issue_report`
- `list_settings_backups`
- `restore_settings_backup`
- `knowledge_recall`
- `team`
- `acp_spawn`

这些工具仍必须同时满足自身 `default_deferred=true` 才会进入 `InjectDeferred`。Core 文件操作、shell、task 和基础会话工具不会被延迟。

## Prompt Render Debug

`system_prompt::build` 的 debug log 增加：

- `prompt_fingerprint`
- `section_debug[]`：每段的 `index`、`label`、`chars`、`fingerprint`

这不改变 prompt 内容，只帮助定位 prompt cache 失效来自哪个 section。

## 实现映射

| 需求 | 实现 |
| --- | --- |
| v2 sidecar | `crates/ha-core/src/tools/definitions/metadata.rs` |
| API/Tauri metadata | `ToolDefinition::to_api_metadata()` |
| tool_search v2 | `crates/ha-core/src/tools/tool_search.rs` |
| deferred 默认配置 | `DeferredToolsConfig::default()` |
| prompt render debug | `system_prompt::build` debug metadata |
| 最终架构说明 | `docs/architecture/tool-system.md` |

## 验收

- `cargo check -p ha-core`
- `cargo test -p ha-core metadata`
- `cargo test -p ha-core tool_search`
- `cargo test -p ha-core deferred`
- `cargo test -p ha-core prompt_render_debug`

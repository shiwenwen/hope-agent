# ToolDefinition v2 迁移 Checklist

> 返回 [ToolDefinition v2 RFC](tool-definition-v2.md)
>
> 更新时间：2026-06-30

## Phase 1 必做项

| 项目 | 证据 | 状态 |
| --- | --- | --- |
| `ToolDefinition::v2_metadata()` sidecar | `crates/ha-core/src/tools/definitions/types.rs` | 完成 |
| `ToolMetadata` 包含 Phase 1 字段 | `crates/ha-core/src/tools/definitions/metadata.rs` | 完成 |
| `to_api_metadata()` 返回 `metadata` | `crates/ha-core/src/tools/definitions/types.rs` | 完成 |
| `tool_search` 支持 alias / search hint / select 容错 / 加权 BM25 检索 | `crates/ha-core/src/tools/tool_search.rs` | 完成 |
| `tool_search` 返回 `effects` / `risk` 等摘要 | `metadata` 随完整 schema 返回 | 完成 |
| 多来源 schema | dispatcher built-ins + dynamic MCP tool definitions | 完成 |
| 默认 deferred 策略 | `DeferredToolsConfig::default()` | 完成 |
| prompt render debug | `system_prompt::build` debug log | 完成 |
| 架构文档更新 | `docs/architecture/tool-system.md` | 完成 |

## 核心工具覆盖

| 工具 | 关键 metadata | 状态 |
| --- | --- | --- |
| `read` | read-only filesystem, path extractor, file content render | 完成 |
| `write` | filesystem write, destructive, strict, file diff render | 完成 |
| `edit` | filesystem write, destructive, strict, alias params, file diff render | 完成 |
| `apply_patch` | filesystem write, destructive, strict, patch/diff render | 完成 |
| `exec` | process execution, open-world, strict, command param | 完成 |
| `grep` | read-only filesystem search, pattern query | 完成 |
| `find` | read-only filesystem search, pattern query | 完成 |
| `tool_search` | runtime status/discovery, read-only, JSON render | 完成 |
| `task_create` | task write / task list render | 完成 |
| `task_update` | task write / task list render | 完成 |
| `task_list` | task read / read-only / task list render | 完成 |

## Deferred 默认清单

| 工具 | 原因 | 状态 |
| --- | --- | --- |
| `browser` | 大 schema，场景化浏览器控制 | 默认 deferred |
| `mac_control` | 极大 schema，桌面自动化风险高 | 默认 deferred |
| `image` | 多模态输入，低频 | 默认 deferred |
| `pdf` | PDF / vision，低频 | 默认 deferred |
| `get_weather` | 场景化外部查询 | 默认 deferred |
| `issue_report` | Hope 项目反馈上报，低频 | 默认 deferred |
| `list_settings_backups` | 设置恢复辅助，低频 | 默认 deferred |
| `restore_settings_backup` | 设置恢复，高风险低频 | 默认 deferred |
| `knowledge_recall` | 双 store 检索，单 store 工具已覆盖常规场景 | 默认 deferred |
| `team` | 多 Agent 团队编排，低频复杂 schema | 默认 deferred |
| `acp_spawn` | 外部 coding agent 编排，低频复杂 schema | 默认 deferred |

## 后续增强

- 给带 `action` 的多态工具补 action-level metadata。
- 把 metadata 用进 Coding Mode workflow policy。
- 将 prompt render debug 暴露到 `/workflow trace` 或诊断面板。
- 为 deferred 默认策略增加 UI “恢复推荐默认值”按钮。

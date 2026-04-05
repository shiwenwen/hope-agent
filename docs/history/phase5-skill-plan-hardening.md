# Phase 5: Skill 和 Plan 模式加固

## 概述

Phase 5 实现了三项安全加固措施：Skill 工具隔离（5A）、Plan 执行层权限强制（5B）、Skill Fork 模式（5C），从 schema 级到执行层形成纵深防御。

## 升级前后对比

| 特性 | 升级前 | 升级后 | claude-code 参考 |
|------|--------|--------|-----------------|
| Skill 工具权限 | 激活后可使用全部工具，无隔离 | `allowed-tools` 白名单限制，仅允许指定工具 | 相同策略：`allowedTools` 字段 |
| Plan 执行层防护 | 仅 schema 级过滤（Provider 不发送被禁工具 schema） | schema + 执行层双重白名单，defense-in-depth | 更严格：双层防护 |
| Skill 上下文隔离 | 所有 tool_call 在主对话历史中执行，污染上下文 | `context: fork` 在子 Agent 中执行，结果注入回主对话 | 类似 fork subagent 模式 |

## 5A: Skill `allowed-tools` 工具隔离

### SKILL.md 新增 frontmatter 字段

```yaml
---
name: my-skill
description: A restricted skill
allowed-tools: [read, exec, grep]
---
```

当 `allowed-tools` 非空时，skill 激活期间只有列出的工具 schema 会发送给 LLM。空列表或未设置 = 全部工具可用（向后兼容）。

### 数据流

```
SKILL.md frontmatter
  → parse_frontmatter() 解析 allowed-tools
  → SkillEntry.allowed_tools
  → ChatEngineParams.skill_allowed_tools
  → agent.set_skill_allowed_tools()
  → Provider 中 tool_schemas.retain() 过滤
```

### 关键文件

- `src-tauri/src/skills.rs` — `SkillEntry`、`ParsedFrontmatter` 新增 `allowed_tools` 字段 + 解析
- `src-tauri/src/agent/types.rs` — `AssistantAgent.skill_allowed_tools`
- `src-tauri/src/agent/mod.rs` — setter + 构造函数初始化
- `src-tauri/src/agent/providers/*.rs` — 4 个 Provider 中 skill 工具过滤
- `src-tauri/src/chat_engine.rs` — `ChatEngineParams.skill_allowed_tools` 传递

## 5B: Plan 执行层权限强制

### 问题

Plan 模式之前仅在 Provider 层过滤 tool schema（不发送被禁工具的 schema），但如果模型通过其他方式调用了被禁工具（如训练数据中记住的工具名），执行层不会拦截。

### 方案

`ToolExecContext` 新增 `plan_mode_allowed_tools: Vec<String>`，在 `execute_tool_with_context()` 中增加白名单检查，形成 schema 级 + 执行级双重防护。

```rust
// tools/execution.rs
if !ctx.plan_mode_allowed_tools.is_empty()
    && !ctx.plan_mode_allowed_tools.iter().any(|t| t == name)
{
    return Err("Plan Mode restriction: tool not allowed");
}
```

白名单自动从 `PlanAgentConfig::default_config().allowed_tools` 填充，无需额外配置。

### 关键文件

- `src-tauri/src/tools/execution.rs` — `ToolExecContext.plan_mode_allowed_tools` + 执行层检查
- `src-tauri/src/agent/mod.rs` — `tool_context_with_usage()` 中从 `PlanAgentMode` 提取白名单

## 5C: Skill Fork 模式

### SKILL.md frontmatter

```yaml
---
name: my-forked-skill
description: Runs in isolated sub-agent
context: fork
allowed-tools: [read, grep]
---
```

`context: fork` 指定 skill 在子 Agent 中执行。子 Agent 继承 `allowed_tools`，SKILL.md 内容作为 `extra_system_context` 注入。执行完成后结果通过现有注入系统自动推送回主对话。

### 数据流

```
/skill-name args
  → dispatch_skill_command() 检测 context_mode == "fork"
  → dispatch_skill_fork() 构建 SpawnParams
  → spawn_subagent() 创建子会话
  → 子 Agent 执行 skill（受 allowed_tools 限制）
  → inject_and_run_parent() 注入结果
```

### 关键文件

- `src-tauri/src/slash_commands/handlers/mod.rs` — `dispatch_skill_fork()` 新增
- `src-tauri/src/slash_commands/types.rs` — `CommandAction::SkillFork` 变体
- `src-tauri/src/subagent/types.rs` — `SpawnParams.skill_allowed_tools`
- `src-tauri/src/subagent/spawn.rs` — `execute_subagent()` 中应用 `skill_allowed_tools`

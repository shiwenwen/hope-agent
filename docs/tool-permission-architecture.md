# 工具权限控制架构

本文档完整梳理 OpenComputer 中工具调用的权限控制流程，涵盖所有决策层级、优先级关系及特殊规则。

---

## 概览

系统中存在 **四个独立的权限控制维度**，按作用阶段分为两大类：

| 类别 | 维度 | 作用 | 配置位置 |
|------|------|------|----------|
| **可见性控制** | Agent 工具过滤（FilterConfig） | 决定 LLM **能看到**哪些工具 | Agent 设置 → 工具 |
| **可见性控制** | 子 Agent 工具拒绝（denied_tools） | 从 LLM 可见的工具列表中移除 | Agent 设置 → 子 Agent |
| **执行审批** | 会话权限模式（ToolPermissionMode） | 决定工具执行前**是否弹审批** | 输入框盾牌按钮 |
| **执行审批** | Agent 审批列表（require_approval） | 指定哪些工具需要审批 | Agent 设置 → 行为 |

此外还有 **Plan Mode 路径限制** 和 **exec 命令级 Allowlist** 两个特殊机制。

---

## 第一类：工具可见性控制（LLM 能不能用）

### 1. Agent 工具过滤（FilterConfig）

**源码**：`agent_config.rs` → `AgentConfig.tools: FilterConfig`
**UI**：Agent 设置面板 → 工具标签页
**生效位置**：`system_prompt.rs:build_tools_section()` — 构建系统提示词时过滤工具描述

```rust
pub struct FilterConfig {
    pub allow: Vec<String>,  // 白名单（非空时仅允许列表中的工具）
    pub deny: Vec<String>,   // 黑名单（始终排除）
}
```

**判断逻辑**（`FilterConfig::is_allowed()`）：

```
allow 非空 且 工具不在 allow 中 → 拒绝
工具在 deny 中 → 拒绝
其他 → 允许
```

- 默认值：`allow=[]`, `deny=[]`（即不过滤，所有工具可见）
- **作用范围**：影响系统提示词中的工具描述（Section ⑥），但目前**不影响**实际发送给 LLM 的 tool schema 列表（仅在提示词中标注"Only the following tools are enabled"）

### 2. 子 Agent 工具拒绝（denied_tools）

**源码**：`agent_config.rs` → `SubagentConfig.denied_tools: Vec<String>`
**生效位置**：四种 Provider 实现（`anthropic.rs` / `openai_chat.rs` / `openai_responses.rs` / `codex.rs`）中 `tool_schemas.retain()` 过滤

```rust
// 所有 Provider 中的统一逻辑
if !self.denied_tools.is_empty() {
    tool_schemas.retain(|t| {
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
        !self.denied_tools.contains(&name.to_string())
    });
}
```

- **作用范围**：从实际发送给 LLM API 的 tool schema 中移除，LLM 完全不知道这些工具的存在
- **使用场景**：子 Agent 深度分层工具策略，防止子 Agent 调用特定危险工具

---

## 第二类：执行审批控制（用不用弹对话框）

### 3. 会话权限模式（ToolPermissionMode）— 最高优先级

**源码**：`tools/approval.rs` → `ToolPermissionMode` 枚举
**UI**：输入框左侧盾牌按钮（三态切换）
**生效位置**：`tools/execution.rs:execute_tool_with_context()` — 工具执行入口

```rust
pub enum ToolPermissionMode {
    Auto,           // 默认：由 Agent 配置决定
    AskEveryTime,   // 所有工具都弹审批
    FullApprove,    // 全部自动放行
}
```

**存储**：进程级全局单例（`OnceLock<TokioMutex>`），每次发消息时由前端通过 `chat` 命令参数设置。

> ⚠️ **注意**：这是进程级全局状态，多窗口/多会话共享同一个值。

### 4. Agent 审批列表（require_approval）

**源码**：`agent_config.rs` → `BehaviorConfig.require_approval: Vec<String>`
**UI**：Agent 设置面板 → 行为标签页（三种模式：全部/无/自定义）
**生效位置**：`tools/execution.rs:tool_needs_approval()`

| 配置值 | 效果 |
|--------|------|
| `["*"]`（默认） | 所有非内部工具需审批 |
| `[]` | 所有工具自动放行 |
| `["exec", "web_fetch"]` | 仅指定工具需审批 |

**仅在 `ToolPermissionMode::Auto` 时生效**。

---

## 完整决策流程

```
┌─────────────────────────────────────────────────────────────┐
│                    工具调用触发                               │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
              ┌─────────────────────────┐
              │  工具是否在 Provider 的   │ ← denied_tools（子 Agent）
              │  tool_schemas 中？       │
              └────────┬────────────────┘
                       │ 不在 → LLM 根本不会调用
                       │ 在 ↓
                       ▼
              ┌─────────────────────────┐
              │  是 internal tool？      │ ← is_internal_tool()
              │  (plan_question/         │    ToolDefinition.internal=true
              │   submit_plan/...)       │
              └────────┬────────────────┘
                       │ 是 → 直接执行，永不审批
                       │ 否 ↓
                       ▼
              ┌─────────────────────────┐
              │  是 SKILL.md 读取？      │ ← read 工具 + 路径以 SKILL.md 结尾
              └────────┬────────────────┘
                       │ 是 → 直接执行（技能预授权）
                       │ 否 ↓
                       ▼
              ┌─────────────────────────┐
              │  是 exec 工具？          │
              └────────┬────────────────┘
                       │ 是 → 走 exec 独立审批流程（见下方）
                       │ 否 ↓
                       ▼
        ┌──────────────────────────────────┐
        │  读取 ToolPermissionMode（盾牌） │
        └──────────┬───────────────────────┘
                   │
         ┌─────────┼──────────┐
         │         │          │
         ▼         ▼          ▼
    FullApprove  AskEvery   Auto
         │       Time        │
         │         │         ▼
         │         │   ┌──────────────────────┐
         │         │   │ 读取 Agent 的          │
         │         │   │ require_approval      │
         │         │   └─────────┬────────────┘
         │         │             │
         │         │     ┌───────┼────────┐
         │         │     │       │        │
         │         │   ["*"]  ["具体"]    []
         │         │     │    工具名      │
         │         │     │       │        │
         │         │     ▼       ▼        ▼
         │         │   需审批  匹配→审批  不需审批
         │         │          不匹配→放行
         │         ▼
         │       弹审批
         ▼
       直接执行
```

### 审批对话框交互

当判定需要审批时，后端发射 `approval_required` 事件，前端 `ApprovalDialog` 显示三个选项：

| 选项 | 行为 |
|------|------|
| **允许一次**（AllowOnce） | 本次放行，下次同样弹出 |
| **始终允许**（AllowAlways） | Auto 模式：写入 `exec-approvals.json` allowlist；AskEveryTime 模式：等同于 AllowOnce（不写 allowlist） |
| **拒绝**（Deny） | 工具返回错误 `"Tool '{}' execution denied by user"` |

审批超时 5 分钟自动拒绝。

---

## exec 工具的独立审批流程

exec 被排除在通用审批门（`name != TOOL_EXEC`）之外，在 `tools/exec.rs` 内部实现自己的命令级审批逻辑：

```
┌──────────────────────────────────┐
│     exec 工具被调用               │
└───────────────┬──────────────────┘
                │
                ▼
    ┌───────────────────────┐
    │ 读取 ToolPermissionMode│
    └─────────┬─────────────┘
              │
    ┌─────────┼──────────┐
    │         │          │
    ▼         ▼          ▼
FullApprove AskEvery   Auto
    │       Time        │
    │         │         ▼
    │         │   ┌──────────────────┐
    │         │   │ 查 exec-approvals│
    │         │   │ .json allowlist  │
    │         │   └────────┬─────────┘
    │         │            │
    │         │     命中 → 放行
    │         │     未命中 ↓
    │         │            │
    │         ▼            ▼
    │       弹审批       弹审批
    │    (AllowAlways   (AllowAlways
    │     不写allowlist)  写入allowlist)
    ▼
  直接执行
```

**Allowlist 持久化文件**：`~/.opencomputer/exec-approvals.json`
**匹配规则**：`extract_command_prefix()` 提取命令首个空格前的单词作为 pattern，前缀匹配。

---

## Plan Mode 路径限制

**源码**：`tools/execution.rs:201-219`
**触发条件**：`ToolExecContext.plan_mode_allow_paths` 非空时（Planning 阶段自动设置）

在审批门**之后**、实际执行**之前**做路径检查：
- 仅影响 `write` / `edit` / `apply_patch` 三个工具
- 只允许修改 plan 文件（`~/.opencomputer/plans/` 下的文件）
- 其他路径返回错误：`"Plan Mode restriction: cannot modify '...'""`

这是一个**独立于审批的硬限制**，即使审批通过也会被拦截。

---

## 特殊豁免规则

### Internal Tools（永不审批）

通过 `ToolDefinition.internal = true` 标记，`is_internal_tool()` 检查。包括：

- Plan Mode 工具：`plan_question` / `submit_plan` / `update_plan_step` / `amend_plan`
- 条件注入工具：`send_notification` / `subagent` / `image_generate` / `canvas` / `acp_spawn`
- 其他内部工具：由 `INTERNAL_TOOL_NAMES` 静态集合管理

### SKILL.md 读取（技能预授权）

`is_skill_read()` 检查 — 当 `read` 工具的路径以 `/SKILL.md` 结尾时，在 `AskEveryTime` 和 `Auto` 模式下均跳过审批。

---

## 关键源文件索引

| 文件 | 职责 |
|------|------|
| `src-tauri/src/tools/approval.rs` | ToolPermissionMode 定义、审批请求/响应、Allowlist 管理 |
| `src-tauri/src/tools/execution.rs` | 统一审批门（`execute_tool_with_context`）、Plan Mode 路径检查 |
| `src-tauri/src/tools/exec.rs` | exec 独立命令级审批逻辑 |
| `src-tauri/src/tools/definitions.rs` | Internal Tool 集合（`INTERNAL_TOOL_NAMES`）、`is_internal_tool()` |
| `src-tauri/src/agent_config.rs` | `FilterConfig`（allow/deny）、`BehaviorConfig.require_approval`、`SubagentConfig.denied_tools` |
| `src-tauri/src/agent/mod.rs` | `tool_context()` 构建 ToolExecContext，传递 require_approval |
| `src-tauri/src/agent/providers/*.rs` | denied_tools 过滤 tool_schemas |
| `src-tauri/src/system_prompt.rs` | `build_tools_section()` 按 FilterConfig 过滤提示词 |
| `src-tauri/src/commands/chat.rs` | 解析前端 tool_permission_mode 参数并设置全局模式 |
| `src/components/chat/ChatInput.tsx` | 盾牌按钮 UI（三态切换） |
| `src/components/chat/ApprovalDialog.tsx` | 审批弹窗 UI |
| `src/components/settings/agent-panel/tabs/BehaviorTab.tsx` | Agent 审批配置 UI |

---

## 优先级总结

```
最高 ─── ToolPermissionMode（输入框盾牌）
  │         FullApprove → 跳过一切审批（含 exec）
  │         AskEveryTime → 强制审批一切（含 exec，无视 Agent 配置）
  │         Auto → 交给下一层
  │
  ├─── Agent require_approval（Agent 设置 → 行为）
  │         仅在 Auto 模式下生效
  │         ["*"] / [] / ["具体工具"]
  │
  ├─── exec Allowlist（命令级持久化白名单）
  │         仅在 Auto 模式下生效
  │         exec-approvals.json 前缀匹配
  │
  └─── 特殊豁免
最低         Internal Tools → 永不审批
             SKILL.md 读取 → 永不审批
```

> **关键理解**：输入框的盾牌（ToolPermissionMode）是全局最高优先级开关，它能完全覆盖 Agent 设置中的 `require_approval` 配置。Agent 设置中的审批配置只在盾牌为 Auto（默认）时才参与决策。

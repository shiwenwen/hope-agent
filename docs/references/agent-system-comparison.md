# Agent / 子 Agent 系统对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.2

---

## 一、架构总览

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| **语言** | Rust (tokio) | TypeScript (Bun) | TypeScript (Node.js) |
| **子 Agent 触发方式** | `subagent` 工具（action 参数） | `Agent` 工具（subagent_type 参数） | `sessions_spawn` 工具调用 |
| **内置 Agent 类型** | 无预设类型，引用 `agent_id` | 6 种内置（worker/explore/plan/verification/guide/statusline） | 无预设类型，按 `agentId` 路由 |
| **并发模型** | tokio::spawn 异步任务 | AsyncGenerator + AbortController | Gateway HTTP 调用 + 事件驱动 |
| **隔离机制** | 独立 SQLite 会话 | 独立 conversation context（进程内） | 独立 session（store 文件隔离） |
| **最大嵌套深度** | 默认 3，可配置 1-5 | 无显式限制（递归防护） | 默认 `DEFAULT_SUBAGENT_MAX_SPAWN_DEPTH`，可配置 |
| **并发上限** | 5 per session | 无硬限制（受系统资源约束） | 无硬编码上限 |
| **结果回传方式** | 自动注入 + 流式事件 | task-notification（user message 注入） | push-based announce（自动送达） |
| **持久化** | SQLite（subagent_runs 表） | 文件系统（session storage） | 文件系统（session store JSON） + 内存注册表 |
| **沙箱支持** | Docker 沙箱（全局） | Git Worktree 隔离 | Docker/SSH 沙箱（per-agent 配置） |
| **多 Agent 协作** | 无 Team 概念 | Team/Swarm + Coordinator 模式 | 无 Team 概念（但支持嵌套编排） |
| **双向消息** | Mailbox steer 机制 | SendMessage 工具 | sessions_send / steer 工具 |

## 二、OpenComputer 实现

### 2.1 子 Agent 产卵机制

**关键代码路径**：
- 工具入口：`src-tauri/src/tools/subagent.rs` → `tool_subagent()`
- 产卵逻辑：`src-tauri/src/subagent/spawn.rs` → `spawn_subagent()`
- 类型定义：`src-tauri/src/subagent/types.rs`

`subagent` 工具通过 `action` 参数分发到 10 种操作：

```
spawn | check | list | result | kill | kill_all | steer | batch_spawn | wait_all | spawn_and_wait
```

**SpawnParams 核心字段**：

```rust
pub struct SpawnParams {
    pub task: String,                    // 任务描述
    pub agent_id: String,                // 目标 agent 定义 ID
    pub parent_session_id: String,       // 父会话 ID
    pub depth: u32,                      // 当前嵌套深度
    pub timeout_secs: Option<u64>,       // 超时（默认 300s，上限 1800s）
    pub model_override: Option<String>,  // 模型覆盖
    pub attachments: Vec<Attachment>,    // 文件附件
    pub plan_agent_mode: Option<PlanAgentMode>, // Plan 模式
    pub skill_allowed_tools: Vec<String>, // Skill 工具白名单
    pub skip_parent_injection: bool,     // 跳过自动注入
}
```

产卵流程：
1. 验证嵌套深度（`max_depth_for_agent` 查询 agent 配置的 `subagents.max_spawn_depth`，clamp 到 1-5）
2. 检查并发上限（`MAX_CONCURRENT_PER_SESSION = 5`）
3. 验证目标 agent 定义存在
4. 创建子会话（`create_session_with_parent`，SQLite 隔离）
5. 插入 `SubagentRun` 记录
6. 注册 cancel flag + mailbox slot
7. Emit `spawned` 事件到前端
8. `tokio::spawn` 异步任务执行

执行阶段 (`execute_subagent`) 实现了完整的 failover 链：
- 从 agent 配置解析 model chain（primary + fallbacks）
- 每个模型最多重试 2 次，指数退避（1s-10s）
- 支持 `subagents.model` 配置子 agent 专用模型
- 注入深度感知的 system context（是否可继续产卵）
- 继承 `denied_tools`（包括 plan mode 限制）
- `catch_unwind` 包裹确保 panic 也能发出完成事件

### 2.2 前后台自动切换（spawn_and_wait）

**代码路径**：`src-tauri/src/tools/subagent.rs` → `action_spawn_and_wait()`

这是 OpenComputer 的独特设计：

```
spawn_and_wait(foreground_timeout=30s)
  └─ 短任务 → 同步返回结果（mode: "foreground"）
  └─ 长任务 → 自动转后台（mode: "background"），注入系统接管
```

实现方式：先调用 `do_spawn()` 产卵，然后在前台 poll DB（每 2s），若在 `foreground_timeout`（默认 30s，上限 120s）内完成则直接返回结果；超时则返回 `backgrounded` 状态，后续由注入系统自动推送。

### 2.3 结果注入（inject_and_run_parent）

**代码路径**：`src-tauri/src/subagent/injection.rs`

这是 OpenComputer 子 Agent 系统最复杂的部分——后端驱动的自动结果注入：

1. **空闲等待**：通过 `ACTIVE_CHAT_SESSIONS` 检测父会话是否忙碌，使用 `tokio::sync::Notify`（非轮询）等待空闲
2. **用户优先**：注入期间若用户发起新聊天，`ChatSessionGuard` 自动 cancel 注入
3. **重试队列**：被 cancel 的注入进入 `PENDING_INJECTIONS` 队列，`ChatSessionGuard::drop` 时自动重试
4. **结果已读跳过**：若 agent 已通过 `check`/`result` action 获取结果（`FETCHED_RUN_IDS`），跳过注入
5. **串行保护**：`INJECTING_SESSIONS` 防止同一会话并发注入
6. **流式推送**：注入过程通过 `parent_agent_stream` 事件流式推送到前端

注入实质上是"自动帮父 agent 发一条包含子 agent 结果的消息，并让父 agent 继续对话"。

**注入消息格式**：
```
[Sub-Agent Completion — auto-delivered]
Run ID: {run_id}
Agent: {agent_id}
Task: {task_preview}
Status: {status}
Duration: {duration}
<<<BEGIN_SUBAGENT_RESULT>>>
{content}
<<<END_SUBAGENT_RESULT>>>
```

### 2.4 Mailbox 实时引导

**代码路径**：`src-tauri/src/subagent/mailbox.rs`

`SubagentMailbox` 是一个全局的 per-run 消息队列（`HashMap<String, Vec<String>>`），实现运行时引导（steer）：

- **push**：父 agent 通过 `steer` action 向子 agent 投递消息
- **drain**：子 agent 的 tool loop 每轮轮询一次（由 Provider 层调用）
- **register/remove**：spawn 时注册，完成时清理

`ChatSessionGuard` 是 RAII 模式的会话互斥保护：
- 构造时：标记会话为 active，cancel 该会话的任何运行中注入
- 析构时：从 active set 移除，`notify_waiters` 唤醒注入等待者，`flush_pending_injections` 重试被中断的注入

### 2.5 深度/并发控制

| 参数 | 值 | 说明 |
|------|-----|------|
| `DEFAULT_MAX_DEPTH` | 3 | 默认最大嵌套深度 |
| 可配置范围 | 1-5 | agent 配置的 `subagents.max_spawn_depth` |
| `MAX_CONCURRENT_PER_SESSION` | 5 | 单个父会话最大并发子 agent |
| `DEFAULT_TIMEOUT_SECS` | 300 | 默认超时 5 分钟 |
| `MAX_RESULT_CHARS` | 10,000 | 结果存储截断上限 |

**权限控制**：
- `subagents.enabled`：agent 级别开关
- `subagents.is_agent_allowed()`：agent 委派白名单
- `subagents.denied_tools`：子 agent 工具黑名单
- Plan mode 限制自动继承（防止子 agent 绕过 plan mode 安全）

## 三、Claude Code 实现

### 3.1 AgentTool 核心

**关键代码路径**：
- 工具定义：`src/tools/AgentTool/AgentTool.tsx`（157K tokens，包含完整的工具逻辑）
- 运行引擎：`src/tools/AgentTool/runAgent.ts`
- 工具过滤：`src/tools/AgentTool/agentToolUtils.ts`
- 常量：`src/tools/AgentTool/constants.ts`

Claude Code 的 `Agent` 工具（原名 `Task`）通过 `subagent_type` 参数选择 agent 类型。核心能力：

**工具过滤机制** (`filterToolsForAgent`)：
- `ALL_AGENT_DISALLOWED_TOOLS`：所有子 agent 不可用的工具（如 `Agent` 自身，防递归）
- `CUSTOM_AGENT_DISALLOWED_TOOLS`：自定义 agent 额外限制
- `ASYNC_AGENT_ALLOWED_TOOLS`：异步 agent 的工具白名单
- MCP 工具始终允许
- `disallowedTools` 黑名单 per-agent 配置

**工具解析** (`resolveAgentTools`)：
- 支持通配符 `*`（所有工具）
- 支持 `Agent(worker, researcher)` 语法限制子 agent 可产卵的类型
- 工具规格中嵌入权限模式（`permissionRuleValueFromString`）

**结果格式** (`AgentToolResult`)：
```typescript
{
  agentId: string,
  agentType: string,
  content: [{type: 'text', text: string}],
  totalToolUseCount: number,
  totalDurationMs: number,
  totalTokens: number,
  usage: { input_tokens, output_tokens, cache_*, service_tier }
}
```

### 3.2 内置 Agent 类型

**代码路径**：`src/tools/AgentTool/built-in/`

| 类型 | 文件 | 用途 | 工具限制 | 特殊约束 |
|------|------|------|---------|---------|
| `worker` | `generalPurposeAgent.ts` | 通用 worker | 全部工具 | 无 |
| `Explore` | `exploreAgent.ts` | 代码搜索 | Glob/Grep/Read/Bash（只读） | 严格只读，不可修改文件 |
| `Plan` | `planAgent.ts` | 架构规划 | 搜索+读取工具 | 只读，输出实现计划 |
| `verification` | `verificationAgent.ts` | 验证专家 | 读取+Bash | 不可修改项目文件，可写 /tmp |
| `claude-code-guide` | `claudeCodeGuideAgent.ts` | 使用指南 | - | 非 SDK 环境 |
| `statusline-setup` | `statuslineSetup.ts` | 状态栏配置 | - | - |

**Explore/Plan 为一次性 agent**（`ONE_SHOT_BUILTIN_AGENT_TYPES`），完成后不会通过 SendMessage 继续。

### 3.3 Team/Swarm 多 Agent 协作

**关键代码路径**：
- 创建：`src/tools/TeamCreateTool/TeamCreateTool.ts`
- 删除：`src/tools/TeamDeleteTool/TeamDeleteTool.ts`

Team 系统（Agent Swarms 特性）：

```typescript
// TeamCreate 输入
{ team_name: string, description?: string, agent_type?: string }
```

- 创建 Team 时指定名称和 team lead 的 agent 类型
- Team 文件持久化到磁盘（`teamHelpers.ts` 管理）
- 每个 leader 只能同时管理一个 team
- 生成 team lead 的 agent ID（`formatAgentId(TEAM_LEAD_NAME, teamName)`）
- 团队成员通过 `SendMessage` 通信
- `isAgentSwarmsEnabled()` 特性门控

### 3.4 Coordinator 编排模式

**代码路径**：`src/coordinator/coordinatorMode.ts`

通过 `CLAUDE_CODE_COORDINATOR_MODE=1` 环境变量启用。Coordinator 是一种特殊的主 agent 模式：

**核心设计**：
- Coordinator 自身不执行代码，只负责分解任务和协调 worker
- Worker 通过 `Agent` 工具产卵，全部异步执行
- Worker 结果通过 `<task-notification>` XML 格式注入（伪装为 user message）
- 支持 `SendMessage` 继续已完成的 worker（复用 context）
- 支持 `TaskStop` 终止偏离方向的 worker

**Worker 工具集**：
- 简化模式（`CLAUDE_CODE_SIMPLE`）：Bash + Read + Edit
- 标准模式：`ASYNC_AGENT_ALLOWED_TOOLS` 全集

**Scratchpad 目录**：跨 worker 共享的持久化工作区（无需权限提示）。

**任务工作流**：Research（并行）→ Synthesis（coordinator）→ Implementation（worker）→ Verification（worker）

### 3.5 Git Worktree 隔离

**代码路径**：
- `src/tools/EnterWorktreeTool/EnterWorktreeTool.ts`
- `src/tools/ExitWorktreeTool/ExitWorktreeTool.ts`

```typescript
// EnterWorktree 输入
{ name?: string }  // 可选 slug，自动生成分支
```

- 为当前会话创建独立的 git worktree（同 repo、不同 working copy）
- 子 agent 可在 worktree 中安全编写代码，不影响主分支
- Fork 子 agent 自动注入 worktree 路径映射提示
- 会话结束时提示清理

### 3.6 Agent 间双向消息（SendMessage）

**代码路径**：`src/tools/SendMessageTool/SendMessageTool.ts`

```typescript
// 输入
{
  to: string,     // 目标：agent name / "*" 广播 / "uds:<socket>" / "bridge:<session>"
  summary?: string,
  message: string | StructuredMessage
}
```

**消息路由**：
- 向特定 agent 发送（通过 agent ID 查找 task）
- 广播到所有 teammate（`*`）
- 支持结构化消息：`shutdown_request`、`shutdown_response`、`plan_approval_response`
- Teammate 间通信通过 `mailbox` 文件系统机制
- 支持 REPL bridge 模式（远程控制）

### 3.7 远程 Agent（RemoteAgentTask）

**代码路径**：`src/tasks/RemoteAgentTask/`

Task 类型之一，区别于 `LocalAgentTask`：
- 远程 agent 在独立进程/机器上运行
- 通过 task registry 管理生命周期

### 3.8 Task 系统

**代码路径**：`src/tasks/types.ts`

统一的任务状态类型：

```typescript
type TaskState =
  | LocalShellTaskState       // 本地 shell 命令
  | LocalAgentTaskState       // 本地 agent 任务
  | RemoteAgentTaskState      // 远程 agent 任务
  | InProcessTeammateTaskState // 进程内 teammate
  | LocalWorkflowTaskState    // 本地工作流
  | MonitorMcpTaskState       // MCP 监控
  | DreamTaskState            // Dream 任务
```

每种任务支持前台/后台切换（`isBackgrounded` 标记）。

**Fork 子 Agent** (`src/tools/AgentTool/forkSubagent.ts`)：
- 省略 `subagent_type` 时触发 fork
- 继承父 agent 完整对话 context 和 system prompt
- 通过 byte-identical API prefix 共享 prompt cache
- 防递归：检测 `FORK_BOILERPLATE_TAG` 阻止嵌套 fork
- Fork 子 agent 的严格输出格式：Scope → Result → Key files → Files changed → Issues

### 3.9 Agent Memory

**代码路径**：`src/tools/AgentTool/agentMemory.ts`

Agent 持久化记忆三级作用域：
- `user`：`~/.claude/agent-memory/` —— 用户全局
- `project`：`.claude/agent-memory/` —— 项目级
- `local`：`.claude/agent-memory-local/` —— 项目本地（不入 VCS）

每种 agent type 有独立的记忆目录。

## 四、OpenClaw 实现

### 4.1 Agent 路由与调度

**关键代码路径**：
- 路由：`src/routing/resolve-route.ts`
- Agent 作用域：`src/agents/agent-scope.ts`

OpenClaw 的 agent 路由系统基于 channel binding：

```typescript
type ResolveAgentRouteInput = {
  cfg: OpenClawConfig,
  channel: string,        // 渠道（telegram/wechat/discord 等）
  accountId?: string,     // 账号 ID
  peer?: RoutePeer,       // 对话对象
  parentPeer?: RoutePeer, // 线程父级
  guildId?: string,       // Discord guild
  memberRoleIds?: string[], // Discord 角色
}
```

匹配优先级：`binding.peer` > `binding.guild+roles` > `binding.guild` > `binding.team` > `binding.account` > `binding.channel` > `default`

每个路由解析产生 `sessionKey`（`agent:{agentId}:{channel}:{accountId}:{peerId}`），用于会话持久化和子 agent 追踪。

### 4.2 子 Agent 产卵与注册

**关键代码路径**：
- 产卵：`src/agents/subagent-spawn.ts`
- 注册表：`src/agents/subagent-registry.ts`
- 类型：`src/agents/subagent-registry.types.ts`
- 深度：`src/agents/subagent-depth.ts`
- 能力：`src/agents/subagent-capabilities.ts`
- 控制：`src/agents/subagent-control.ts`
- 通告：`src/agents/subagent-announce.ts`

**SpawnSubagentParams**：
```typescript
{
  task: string,
  label?: string,
  agentId?: string,
  model?: string,
  thinking?: string,        // 推理模式
  runTimeoutSeconds?: number,
  thread?: boolean,         // 线程绑定
  mode?: "run" | "session", // 一次性 vs 持久会话
  cleanup?: "delete" | "keep",
  sandbox?: "inherit" | "require",
  attachments?: Array<{name, content, encoding, mimeType}>,
}
```

**产卵流程**：
1. 通过 `resolveSubagentCapabilities()` 检查深度权限
2. 通过 Gateway HTTP 调用创建子会话
3. 注册到 `subagentRuns` 内存注册表
4. 持久化到磁盘（`persistSubagentRunsToDisk`）
5. 启动超时监控

**SubagentRunRecord** 包含丰富的生命周期元数据：
```typescript
{
  runId, childSessionKey, controllerSessionKey,
  requesterSessionKey, requesterOrigin,
  task, cleanup, label, model, workspaceDir,
  spawnMode, createdAt, startedAt, endedAt,
  outcome, archiveAtMs, cleanupCompletedAt,
  suppressAnnounceReason, endedReason,
  wakeOnDescendantSettle,          // 后代完成时唤醒
  frozenResultText, fallbackFrozenResultText, // 冻结结果
  endedHookEmittedAt, attachmentsDir,
}
```

### 4.3 角色与能力系统

**代码路径**：`src/agents/subagent-capabilities.ts`

OpenClaw 引入了三级角色模型：

| 角色 | depth | 可产卵 | 可控制子级 |
|------|-------|--------|-----------|
| `main` | 0 | Yes | Yes |
| `orchestrator` | 1 ~ maxDepth-1 | Yes | Yes |
| `leaf` | maxDepth | No | No |

```typescript
function resolveSubagentRoleForDepth({ depth, maxSpawnDepth }) {
  if (depth <= 0) return "main"
  return depth < maxSpawnDepth ? "orchestrator" : "leaf"
}
```

Orchestrator 可以继续产卵子 agent（形成多层编排），leaf 只能执行任务。

### 4.4 结果通告（Announce）

**代码路径**：`src/agents/subagent-announce.ts`、`src/agents/subagent-announce-delivery.ts`

OpenClaw 的 announce 系统是 push-based：

1. 子 agent 完成时产生 `SubagentRunOutcome`
2. `runSubagentAnnounceFlow` 执行通告流程
3. `deliverSubagentAnnouncement` 通过 Gateway 向 requester 发送完成消息
4. 支持重试（`MAX_ANNOUNCE_RETRY_COUNT`，退避策略 `resolveAnnounceRetryDelayMs`）
5. 通告超时 120s（`SUBAGENT_ANNOUNCE_TIMEOUT_MS`）
6. 生命周期错误有 15s 宽限期（`LIFECYCLE_ERROR_RETRY_GRACE_MS`），等待重试/恢复

**System Prompt 注入**（`buildSubagentSystemPrompt`）：
```
# Subagent Context
You are a **subagent** spawned by the {parent} for a specific task.
## Your Role
- You were created to handle: {task}
- Complete this task. That's your entire purpose.
## Rules
1. Stay focused
2. Complete the task
3. Don't initiate
4. Be ephemeral
5. Trust push-based completion
6. Recover from compacted/truncated tool output
```

### 4.5 控制操作

**代码路径**：`src/agents/subagent-control.ts`

- **list**：列出活跃/最近子 agent（默认 30 分钟内，最多 24 小时）
- **steer**：向运行中子 agent 发送引导消息（限速 2s，最大 4000 字符）
- **kill**：终止子 agent（abort embedded PI run + 5s settle 超时）
- **steer-restart**：steer 后重启子 agent（`markSubagentRunForSteerRestart` + `replaceSubagentRunAfterSteer`）

### 4.6 Sandbox 隔离

**代码路径**：`src/agents/sandbox/`（re-export from `src/agents/sandbox.ts`）

OpenClaw 提供完整的沙箱抽象：

- **Docker 沙箱**：`buildSandboxCreateArgs`、`resolveSandboxDockerConfig`
- **SSH 沙箱**：`createSshSandboxSessionFromSettings`、`runSshSandboxCommand`
- **Browser 沙箱**：独立的浏览器容器（`DEFAULT_SANDBOX_BROWSER_IMAGE`）
- **Scope 配置**：per-agent 沙箱策略（`resolveSandboxScope`）
- **FS Bridge**：沙箱文件系统桥接（`SandboxFsBridge`）
- **可插拔后端**：`registerSandboxBackend` + `SandboxBackendFactory` 模式

子 agent 的 sandbox 模式：
- `inherit`：继承父 agent 的沙箱配置
- `require`：强制要求沙箱环境

## 五、逐项功能对比

| 功能 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **基础产卵** | `spawn` action | `Agent` tool | `sessions_spawn` |
| **同步等待** | `spawn_and_wait` | 同步 agent 调用 | - |
| **异步后台** | 自动注入 | `runAsyncAgentLifecycle` | push-based announce |
| **前后台自动切换** | `spawn_and_wait` 超时转后台 | fork 默认后台 | - |
| **批量产卵** | `batch_spawn`（最多 10 个） | 多个并行 tool_use | 多次调用 |
| **等待全部** | `wait_all` | - | - |
| **运行时引导（steer）** | Mailbox push/drain | `SendMessage` to agent | `steer` action + rate limit |
| **结果自动注入** | `inject_and_run_parent` | task-notification 注入 | announce flow |
| **用户优先打断** | `ChatSessionGuard` cancel + 重试队列 | `AbortController` | abort embedded run |
| **子 agent 已读跳过** | `FETCHED_RUN_IDS` 集合 | - | `suppressAnnounceReason` |
| **嵌套深度控制** | per-agent 配置（1-5） | fork 递归防护 | 角色系统（main/orchestrator/leaf） |
| **并发控制** | 5 per session 硬限制 | 无硬限制 | 无硬限制 |
| **工具白名单** | `skill_allowed_tools` | `tools`/`disallowedTools` per-agent | per-agent 工具策略 |
| **工具黑名单** | `denied_tools` + plan mode 继承 | `ALL_AGENT_DISALLOWED_TOOLS` | sandbox tool policy |
| **模型 failover** | model chain + 指数退避 2 次 | agent model resolve | auth-profile rotation |
| **内置 agent 类型** | - | 6 种（explore/plan/verify 等） | - |
| **Fork（上下文继承）** | - | `forkSubagent`（byte-identical prefix） | - |
| **Team/Swarm** | - | TeamCreate/TeamDelete + Coordinator | - |
| **Agent 间消息** | steer（单向引导） | SendMessage（双向，含广播） | sessions_send + steer |
| **Git Worktree 隔离** | - | EnterWorktree/ExitWorktree | - |
| **Docker/SSH 沙箱** | 全局 Docker 沙箱 | - | per-agent Docker/SSH/Browser 沙箱 |
| **Agent 持久记忆** | 共享全局记忆系统 | per-agent 三级记忆（user/project/local） | session store |
| **孤儿清理** | `cleanup_orphan_runs`（启动时） | - | `reconcileOrphanedRun` |
| **附件传递** | `Attachment` struct（file/base64） | 进程内传递 | base64 编码 + 磁盘物化 |
| **Prompt Cache 优化** | - | byte-identical fork prefix | - |
| **安全分类器** | - | `classifyHandoffIfNeeded` | - |
| **Token 追踪** | `input_tokens/output_tokens` 字段 | 完整 usage 追踪 | `totalTokens` |
| **会话持久化** | SQLite `subagent_runs` 表 | session storage 文件 | JSON session store + 内存注册表 |
| **流式进度** | `subagent_event` Tauri 事件 | `ProgressTracker` + 实时 UI | lifecycle events |
| **Coordinator 模式** | - | 完整编排协议 | 通过嵌套实现 |
| **远程 Agent** | - | `RemoteAgentTask` | Gateway HTTP 调用 |
| **Steer 后重启** | - | - | `steer-restart` 机制 |
| **后代等待唤醒** | - | - | `wakeOnDescendantSettle` |
| **冻结结果快照** | - | - | `frozenResultText` |
| **Plan Mode 安全** | plan mode tool 继承 | plan mode 权限隔离 | - |

## 六、差距分析与建议

### 6.1 OpenComputer 的独特优势

1. **spawn_and_wait 前后台自动切换**：三个项目中唯一实现此模式的，对短任务体验极好
2. **batch_spawn + wait_all**：原生批量操作，减少 tool loop 轮次
3. **注入队列与用户优先机制**：`PENDING_INJECTIONS` + `ChatSessionGuard` 的设计精巧，确保用户消息永远优先
4. **Rust 级别的 panic 安全**：`catch_unwind` 确保即使子 agent panic 也能正确完成状态转换

### 6.2 对比 Claude Code 的差距

| 差距项 | 影响 | 建议优先级 |
|--------|------|-----------|
| **无内置 agent 类型** | 用户需要自行配置 agent 定义才能利用子 agent | P1 — 至少提供 explore/plan/verify 三种预设 |
| **无 Fork 机制** | 无法继承父 agent 完整上下文产卵，每个子 agent 从零开始 | P2 — 对长对话场景价值大 |
| **无 Team/Coordinator 模式** | 缺少多 agent 编排能力 | P3 — 目前单层子 agent 已够用 |
| **无 Git Worktree 隔离** | 子 agent 文件修改可能冲突 | P2 — 安全写入场景需要 |
| **无 SendMessage 双向通信** | 只有 steer（父→子），无子→父主动消息 | P3 — 注入机制已覆盖大部分场景 |
| **无 Agent 专属记忆** | 所有 agent 共享全局记忆 | P3 |
| **无安全分类器** | 子 agent 输出不经过安全审查 | P2 — 自动模式下有安全隐患 |
| **无 prompt cache 共享** | fork 子 agent 无法复用父 agent 的缓存前缀 | P3 — 需 provider 支持 |

### 6.3 对比 OpenClaw 的差距

| 差距项 | 影响 | 建议优先级 |
|--------|------|-----------|
| **无角色系统** | 嵌套 agent 缺乏 orchestrator/leaf 能力区分 | P3 — 当前深度限制已够用 |
| **无 per-agent 沙箱** | 子 agent 无法按需隔离 | P2 — 安全执行场景需要 |
| **无 steer-restart** | steer 后无法重启子 agent | P3 — 低频需求 |
| **无后代等待唤醒** | 父 agent 完成时子孙仍在运行需要手动处理 | P3 |
| **无冻结结果快照** | announce 时没有冻结结果的概念 | P3 — 当前 result 存储已满足 |

### 6.4 OpenComputer 优于两者的能力

| 独有优势 | 说明 |
|---------|------|
| **spawn_and_wait** | Claude Code 和 OpenClaw 均无此前后台自动切换机制 |
| **batch_spawn** | 原生批量操作，无需多次 tool call |
| **wait_all** | 等待多个子 agent 全部完成 |
| **SQLite 持久化** | 结构化查询优于文件系统方案 |
| **model chain failover** | 子 agent 执行层的完整模型降级链 |
| **Plan mode 安全继承** | 子 agent 自动继承父 agent 的 plan mode 限制 |

# Hooks 系统对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.11

---

## 一、架构总览

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| **Hooks 系统** | 无 | 完整的生命周期 Hook 系统 | 完整的事件驱动 Hook 系统 |
| **Hook 类型** | N/A | command / prompt / agent / http / callback | 内部事件 Hook + Webhook HTTP 管道 |
| **事件数量** | 0 | 27 种事件类型 | 5 大类（command / session / agent / gateway / message）+ 子动作 |
| **Hook 来源** | N/A | settings.json 配置 + 插件注册 + SDK callback | bundled / managed / workspace / plugin / legacy config |
| **执行模式** | N/A | 同步阻塞 + 异步后台 + async rewake | 顺序串行（handler 逐一 await） |
| **权限控制** | N/A | Hook 可决定 allow/deny/ask 工具权限 | 仅 eligibility 过滤（OS / bins / env / config） |
| **安全机制** | N/A | 工作区信任检查 + 超时 + 沙箱边界 | 路径边界检查 + realpath 校验 + 信任源分级警告 |

---

## 二、OpenComputer 实现

### 2.1 现状：无 Hooks 系统

OpenComputer 当前 **不具备 Hooks 系统**。经过对 `crates/oc-core/src/` 全目录搜索，仅在 `channel/` 子目录中发现 webhook 相关代码（LINE 和 Google Chat 渠道插件的入站 webhook 处理），但这属于 IM 渠道协议层的 webhook 接入，**不是通用的生命周期 Hook 机制**。

### 2.2 现有替代机制

| 机制 | 说明 |
|------|------|
| **Tool Loop** | 工具执行循环（最多 10 轮），但无 pre/post hook 注入点 |
| **Skill 工具隔离** | `allowed-tools` frontmatter 实现工具白名单，但不支持自定义逻辑注入 |
| **自动记忆提取** | 每轮对话结束后 inline 执行，类似 PostToolUse hook 但硬编码 |
| **上下文压缩** | 5 层渐进式压缩，compaction 回调类似 PreCompact/PostCompact 但无扩展点 |
| **IM Channel webhook** | LINE / Google Chat 渠道的入站 webhook，仅用于消息接收 |

### 2.3 缺失影响

- 无法在工具执行前后注入自定义验证/审计逻辑
- 无法在会话生命周期（开始/结束/压缩）注入自定义行为
- 无法通过外部脚本/HTTP 回调扩展系统行为
- 自动记忆提取、工具权限判定等逻辑硬编码，不可定制

---

## 三、Claude Code 实现

### 3.1 Hook 事件类型

Claude Code 定义了 **27 种** Hook 事件，覆盖完整的 Agent 生命周期：

#### 工具相关事件

| 事件 | 触发时机 | 可阻断 |
|------|---------|--------|
| `PreToolUse` | 工具执行前 | 是（可 allow/deny/ask） |
| `PostToolUse` | 工具执行成功后 | 是（可阻止后续 LLM 请求） |
| `PostToolUseFailure` | 工具执行失败后 | 否 |
| `PermissionRequest` | 工具请求权限时 | 是（可自动批准/拒绝） |
| `PermissionDenied` | 权限被拒绝后 | 否（可触发 retry） |

#### 会话生命周期事件

| 事件 | 触发时机 |
|------|---------|
| `SessionStart` | 会话启动 |
| `SessionEnd` | 会话结束（1.5s 超时） |
| `Setup` | 初始化设置完成 |
| `UserPromptSubmit` | 用户提交提示词 |
| `Stop` | Agent 停止 |
| `StopFailure` | Agent 停止失败 |

#### Agent 生态事件

| 事件 | 触发时机 |
|------|---------|
| `SubagentStart` | 子 Agent 启动 |
| `SubagentStop` | 子 Agent 停止 |
| `TeammateIdle` | 队友 Agent 空闲 |
| `TaskCreated` | 任务创建 |
| `TaskCompleted` | 任务完成 |

#### 压缩与记忆事件

| 事件 | 触发时机 |
|------|---------|
| `PreCompact` | 上下文压缩前 |
| `PostCompact` | 上下文压缩后 |

#### 通知与配置事件

| 事件 | 触发时机 |
|------|---------|
| `Notification` | 系统通知 |
| `ConfigChange` | 配置变更 |
| `InstructionsLoaded` | 指令文件加载完成 |
| `Elicitation` | MCP 信息征询请求 |
| `ElicitationResult` | 征询结果返回 |

#### 工作区事件

| 事件 | 触发时机 |
|------|---------|
| `CwdChanged` | 工作目录变更 |
| `FileChanged` | 文件变更 |
| `WorktreeCreate` | Git worktree 创建 |
| `WorktreeRemove` | Git worktree 移除 |

### 3.2 同步 Hook（拦截/批准）

Claude Code 的同步 Hook 可以：

1. **阻断执行**：返回 `{ decision: "block", reason: "..." }` 阻止工具运行
2. **批准执行**：返回 `{ decision: "approve" }` 跳过权限提示
3. **修改输入**：通过 `updatedInput` 修改工具参数
4. **权限决策**：`PreToolUse` hook 可返回 `permissionDecision: "allow" | "deny" | "ask"`
5. **注入上下文**：通过 `additionalContext` 向对话注入额外信息
6. **阻止后续**：`preventContinuation: true` + `stopReason` 终止整个查询循环

同步 Hook JSON 响应 schema：

```typescript
{
  continue?: boolean;        // 是否继续（默认 true）
  suppressOutput?: boolean;  // 隐藏 stdout
  stopReason?: string;       // continue=false 时的停止原因
  decision?: "approve" | "block";
  reason?: string;           // 决策说明
  systemMessage?: string;    // 向用户显示的警告
  hookSpecificOutput?: {     // 按事件类型的特定输出
    hookEventName: "PreToolUse" | "PostToolUse" | ...;
    permissionDecision?: "allow" | "deny" | "ask";
    updatedInput?: Record<string, unknown>;
    additionalContext?: string;
    // ... 更多事件特定字段
  };
}
```

### 3.3 异步 Hook

Claude Code 支持两种异步模式：

#### 标准异步（`async: true`）

```json
{ "async": true, "asyncTimeout": 30 }
```

- Hook 进程在后台运行，不阻塞主流程
- 通过 `AsyncHookRegistry` 注册和管理
- 完成后通过 SDK 事件系统通知

#### Async Rewake（`asyncRewake: true`）

```typescript
{
  type: "command",
  command: "long-running-check",
  asyncRewake: true  // 后台运行，exit code 2 时唤醒模型
}
```

- 后台运行不阻塞
- 正常完成（exit 0）静默结束
- 返回 exit code 2 时，将错误消息作为 task-notification 注入，唤醒模型处理

### 3.4 Hook 配置与注册

Claude Code 的 Hook 通过 `settings.json` 配置，支持四种类型：

#### Command Hook（Shell 命令）

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "echo 'checking...' && validate-bash-command $ARGUMENTS",
            "if": "Bash(git *)",
            "shell": "bash",
            "timeout": 30,
            "statusMessage": "Validating git command...",
            "once": false,
            "async": false,
            "asyncRewake": false
          }
        ]
      }
    ]
  }
}
```

- `if` 字段使用权限规则语法过滤（如 `"Bash(git *)"` 仅匹配 git 命令）
- `shell` 支持 bash/zsh/sh/powershell
- `once` 为 true 时执行一次后自动移除

#### Prompt Hook（LLM 评估）

```json
{
  "type": "prompt",
  "prompt": "Evaluate if this tool call is safe: $ARGUMENTS",
  "model": "claude-sonnet-4-6",
  "timeout": 30
}
```

- 使用 LLM 评估 hook 输入
- `$ARGUMENTS` 占位符替换为 hook 输入 JSON
- 可指定独立的模型

#### Agent Hook（代理验证器）

```json
{
  "type": "agent",
  "prompt": "Verify that unit tests ran and passed.",
  "model": "claude-sonnet-4-6",
  "timeout": 60
}
```

- 启动一个具有工具能力的 sub-agent 执行验证
- 比 prompt hook 更强大，可以运行命令、读取文件

#### HTTP Hook（外部服务调用）

```json
{
  "type": "http",
  "url": "https://my-server.com/webhook",
  "headers": {
    "Authorization": "Bearer $MY_TOKEN"
  },
  "allowedEnvVars": ["MY_TOKEN"],
  "timeout": 10
}
```

- POST hook 输入 JSON 到指定 URL
- 支持环境变量插值（需在 `allowedEnvVars` 中显式声明）
- 返回标准 hook JSON 响应

#### Callback Hook（SDK 编程注册）

```typescript
type HookCallback = {
  type: 'callback';
  callback: (input: HookInput, toolUseID, abort, hookIndex?, context?) => Promise<HookJSONOutput>;
  timeout?: number;
  internal?: boolean;  // 排除出 telemetry
};
```

- 仅 SDK 编程使用，不可通过 settings.json 配置
- 直接访问 AppState
- 用于内部功能（如 session file access analytics）

### 3.5 工具执行 Hook 流程

```
用户发起工具调用
  │
  ▼
runPreToolUseHooks()
  ├── 遍历匹配 PreToolUse hooks
  ├── 每个 hook 可返回:
  │   ├── permissionBehavior: allow → 跳过权限提示（但 deny rule 仍生效）
  │   ├── permissionBehavior: deny → 拒绝工具调用
  │   ├── permissionBehavior: ask → 强制弹出权限确认
  │   ├── blockingError → 拒绝并附带错误消息
  │   ├── preventContinuation → 终止整个查询循环
  │   ├── updatedInput → 修改工具输入参数
  │   └── additionalContext → 注入额外上下文
  │
  ▼
resolveHookPermissionDecision()
  ├── hook allow + 无 deny rule → 执行工具
  ├── hook allow + deny rule → deny rule 优先
  ├── hook allow + ask rule → 仍弹出确认
  ├── hook deny → 拒绝
  └── 无 hook 决策 → 正常权限流程
  │
  ▼
工具执行
  │
  ▼
runPostToolUseHooks()  或  runPostToolUseFailureHooks()
  ├── blockingError → 附加错误消息
  ├── preventContinuation → 终止后续 LLM 请求
  ├── additionalContext → 注入反馈信息
  └── updatedMCPToolOutput → 修改 MCP 工具输出
```

关键设计要点：
- **Hook allow 不覆盖 deny rule**：`resolveHookPermissionDecision()` 确保 settings.json 中的 deny/ask 规则始终优先于 hook 的 allow 决策（防御纵深）
- **工作区信任检查**：`shouldSkipHookDueToTrust()` 在交互模式下要求工作区信任已确认，防止不受信任的工作区中的 `.claude/settings.json` 中的恶意 hook 执行
- **超时保护**：工具 hook 默认 10 分钟超时，SessionEnd hook 仅 1.5 秒
- **abort 信号传播**：hook 执行尊重 `abortController.signal`，用户取消时 hook 也被终止

### 3.6 Hook 事件广播系统

独立于主消息流的事件广播系统（`hookEvents.ts`）：

```typescript
type HookExecutionEvent =
  | HookStartedEvent    // { hookId, hookName, hookEvent }
  | HookProgressEvent   // + { stdout, stderr, output }
  | HookResponseEvent;  // + { exitCode, outcome }
```

- `SessionStart` 和 `Setup` 事件始终广播
- 其他事件需 SDK 显式开启 `includeHookEvents`
- 支持 progress interval（默认 1s）实时推送 hook 输出
- 缓冲队列（最多 100 条）在 handler 注册前暂存事件

---

## 四、OpenClaw 实现

### 4.1 Hook 加载与安装

OpenClaw 的 Hook 系统基于 **目录发现 + 事件注册** 架构：

#### Hook 来源层级

| 来源 | 路径 | 说明 |
|------|------|------|
| `openclaw-bundled` | 内置 `bundled/` 目录 | 随代码发布的核心 hooks |
| `openclaw-managed` | `~/.openclaw/hooks/` | 通过 npm/git/archive 安装的 hooks |
| `openclaw-workspace` | 工作区内 `hooks/` 目录 | 项目级自定义 hooks |
| `openclaw-plugin` | 插件注册的 hooks 目录 | 通过插件系统扩展 |

#### Hook 目录结构

每个 Hook 是一个独立目录，包含：

```
my-hook/
  HOOK.md           # frontmatter 元数据（name, events, requires 等）
  handler.ts        # 或 handler.js / index.ts / index.js
  package.json      # 可选，用于依赖管理
```

#### HOOK.md Frontmatter 元数据

```yaml
---
name: session-memory
openclaw:
  events:
    - command:new
    - command:reset
  hookKey: session-memory
  always: true
  os:
    - darwin
    - linux
  requires:
    bins:
      - node
    env:
      - OPENAI_API_KEY
    config:
      - browser.enabled
  install:
    - kind: npm
      package: "@openclaw/hook-session-memory"
---
```

#### 加载流程

```
loadInternalHooks(cfg, workspaceDir)
  │
  ├── 1. loadWorkspaceHookEntries() — 目录发现
  │   ├── bundled hooks（内置）
  │   ├── managed hooks（~/.openclaw/hooks/）
  │   ├── workspace hooks（项目目录）
  │   └── plugin hooks（插件注册）
  │
  ├── 2. shouldIncludeHook() — 资格过滤
  │   ├── enabled 状态检查
  │   ├── OS 平台检查
  │   ├── 二进制依赖检查（bins / anyBins）
  │   ├── 环境变量检查
  │   └── 配置路径检查
  │
  ├── 3. 路径安全校验
  │   ├── openBoundaryFile() — 路径边界检查
  │   ├── realpath 解析防止符号链接逃逸
  │   └── 信任源分级警告（workspace/managed 源提示）
  │
  ├── 4. 动态导入 handler 模块
  │   ├── buildImportUrl() — mutable 源加 cache-bust
  │   └── resolveFunctionModuleExport() — 解析导出函数
  │
  └── 5. registerInternalHook(event, handler) — 注册到全局 Map
```

#### Hook 安装机制

支持三种安装方式：

```typescript
// 从 npm 包安装
installHooksFromNpmSpec({ spec: "@openclaw/hook-pack", ... })

// 从本地路径安装
installHooksFromPath({ path: "./my-hook", ... })

// 从归档文件安装
installHooksFromArchive({ archivePath: "hook.tar.gz", ... })
```

- 安装记录持久化到配置中（`hooks.internal.installs`）
- 支持 npm integrity 校验和漂移检测
- 支持 dry-run 模式
- 路径安全校验防止目录穿越

### 4.2 内置 Hooks

OpenClaw 提供三个 bundled hooks：

#### boot-md Hook

- **事件**: `gateway:startup`
- **功能**: 网关启动时对所有 agent 执行 boot checklist
- **触发**: 遍历所有 agentId，调用 `runBootOnce()` 执行启动检查

#### session-memory Hook

- **事件**: `command:new`, `command:reset`
- **功能**: 会话重置时自动保存会话上下文到记忆文件
- **流程**:
  1. 读取最近 N 条消息（默认 15 条）
  2. 调用 LLM 生成描述性 slug
  3. 写入 `{workspaceDir}/memory/{date}-{slug}.md`
- **配置**: `hookConfig.messages`（消息数）、`hookConfig.llmSlug`（是否用 LLM 生成 slug）

#### bootstrap-extra-files Hook

- **事件**: `agent:bootstrap`
- **功能**: Agent 启动时加载额外的 bootstrap 文件到上下文
- **配置**: `hookConfig.paths` / `hookConfig.patterns` / `hookConfig.files` 指定文件 glob 模式

#### command-logger Hook（示例）

- **事件**: `command`（所有命令事件）
- **功能**: 将命令事件记录到 `{stateDir}/logs/commands.log`
- **用途**: 审计/调试参考实现

### 4.3 内部事件系统

#### 事件类型体系

```typescript
type InternalHookEventType = "command" | "session" | "agent" | "gateway" | "message";

interface InternalHookEvent {
  type: InternalHookEventType;
  action: string;              // 如 "new", "reset", "bootstrap", "startup"
  sessionKey: string;
  context: Record<string, unknown>;
  timestamp: Date;
  messages: string[];          // hooks 可推送消息给用户
}
```

#### 具体事件

| 事件键 | 类型 | 说明 |
|--------|------|------|
| `command:new` | command | 新会话命令 |
| `command:reset` | command | 重置命令 |
| `session:patch` | session | 会话参数修改 |
| `agent:bootstrap` | agent | Agent 启动引导 |
| `gateway:startup` | gateway | 网关启动 |
| `message:received` | message | 消息接收 |
| `message:sent` | message | 消息发送 |
| `message:transcribed` | message | 语音转文本完成 |
| `message:preprocessed` | message | 消息预处理完成 |

#### 事件分发机制

```typescript
// 注册 — 支持类型级和类型:动作级
registerInternalHook("command", handler);       // 所有 command 事件
registerInternalHook("command:new", handler);   // 仅 command:new

// 触发 — 同时调用类型级和精确匹配的 handler
triggerInternalHook(event);
// → handlers.get("command") + handlers.get("command:new")
```

关键设计：
- **全局单例注册表**：使用 `Symbol.for("openclaw.internalHookHandlers")` 确保 bundle splitting 不会导致注册/触发不一致
- **错误隔离**：单个 handler 抛错不影响其他 handler 执行
- **顺序执行**：所有 handler 按注册顺序串行 `await`

### 4.4 Webhook 系统

OpenClaw 的 Webhook 系统独立于内部 Hook，面向外部 HTTP 集成：

#### Webhook 路径管理

```typescript
// 路径规范化
normalizeWebhookPath("/api/webhook/") → "/api/webhook"
normalizeWebhookPath("webhook")       → "/webhook"

// 路径解析优先级：显式 path > URL 提取 > 默认值
resolveWebhookPath({ webhookPath, webhookUrl, defaultPath })
```

#### Webhook Target 注册

```typescript
// 注册目标 + 自动安装插件 HTTP 路由
registerWebhookTargetWithPluginRoute({
  targetsByPath: Map<string, T[]>,
  target: { path: "/webhook" },
  route: { /* plugin HTTP route config */ },
})
```

- 首个目标注册时自动创建路由
- 最后一个目标取消注册时自动清理路由和 teardown
- 支持单目标精确匹配和授权校验

#### Webhook 安全防护

| 机制 | 说明 |
|------|------|
| **固定窗口限流** | 默认 60s/120 次，最多追踪 4096 个 key |
| **并发控制** | `WebhookInFlightLimiter` 防止重入 |
| **异常追踪** | 采样日志记录（每 25 次异常状态码记一次） |
| **方法限制** | `rejectNonPostWebhookRequest()` 仅允许 POST |
| **授权校验** | `resolveWebhookTargetWithAuthOrReject()` 精确匹配授权目标 |
| **Content-Type 校验** | 可选 `requireJsonContentType` |

#### 消息 Hook Mapper

`message-hook-mappers.ts` 提供标准化的消息上下文转换：

```
原始 FinalizedMsgContext
  ↓ deriveInboundMessageHookContext()
CanonicalInboundMessageHookContext
  ├── toPluginMessageReceivedEvent()       → 插件 Hook
  ├── toPluginInboundClaimEvent()          → 插件 claim Hook
  ├── toInternalMessageReceivedContext()   → 内部 Hook
  ├── toInternalMessageTranscribedContext() → 内部 Hook
  └── toInternalMessagePreprocessedContext() → 内部 Hook
```

实现了消息上下文在插件 Hook 和内部 Hook 之间的统一映射。

### 4.5 Fire-and-Forget 模式

```typescript
fireAndForgetHook(task: Promise<unknown>, label: string, logger?)
```

- 用于不需要等待结果的 hook 触发
- 静默捕获错误并记录
- 常用于非关键路径的通知类 hook

---

## 五、逐项功能对比

| 功能 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| **工具执行前 Hook** | 无 | PreToolUse（可 allow/deny/ask/修改输入） | 无（无工具概念） |
| **工具执行后 Hook** | 无 | PostToolUse / PostToolUseFailure | 无 |
| **会话开始 Hook** | 无 | SessionStart（可注入初始消息/watchPaths） | gateway:startup / agent:bootstrap |
| **会话结束 Hook** | 无 | SessionEnd（1.5s 超时限制） | command:new / command:reset |
| **消息接收 Hook** | 无 | UserPromptSubmit（可注入上下文） | message:received / message:preprocessed |
| **消息发送 Hook** | 无 | 无 | message:sent |
| **语音转录 Hook** | 无 | 无 | message:transcribed |
| **权限控制 Hook** | 无 | PermissionRequest / PermissionDenied | 无 |
| **上下文压缩 Hook** | 无 | PreCompact / PostCompact | 无 |
| **配置变更 Hook** | 无 | ConfigChange | session:patch |
| **文件变更 Hook** | 无 | FileChanged / CwdChanged | 无 |
| **子 Agent Hook** | 无 | SubagentStart / SubagentStop | 无 |
| **Shell 命令 Hook** | 无 | command 类型 | 无（handler 是 TS/JS 模块） |
| **LLM 评估 Hook** | 无 | prompt 类型 | 无 |
| **Agent 验证 Hook** | 无 | agent 类型 | 无 |
| **HTTP 回调 Hook** | 无 | http 类型（POST + env var 插值） | Webhook 管道（独立子系统） |
| **编程式 Hook** | 无 | callback 类型（SDK only） | handler 模块（TS/JS 函数） |
| **异步执行** | 无 | async / asyncRewake | fireAndForgetHook |
| **Hook 条件过滤** | 无 | `if` 字段（权限规则语法） | OS / bins / env / config 资格检查 |
| **一次性 Hook** | 无 | `once: true` | 无 |
| **Hook 安装管理** | 无 | 插件 hook 自动加载 | npm/git/archive 安装 + 版本管理 |
| **Webhook 限流** | 无 | 无 | 固定窗口限流 + 并发控制 |
| **Webhook 异常追踪** | 无 | 无 | 采样日志异常追踪器 |
| **消息上下文标准化** | 无 | 无 | CanonicalInboundMessageHookContext |
| **Hook 进度广播** | 无 | HookExecutionEvent 事件系统 | 无 |
| **Hook 超时** | 无 | 按 hook 配置（默认 10min，SessionEnd 1.5s） | 无显式超时 |

---

## 六、差距分析与建议

### 6.1 关键差距

#### P0 — 工具执行 Hook

OpenComputer 当前的 Tool Loop 缺乏 pre/post 扩展点。Claude Code 的 `PreToolUse` + `PostToolUse` 组合是最有价值的 Hook 能力：

- **PreToolUse**：可用于自定义工具审批策略、输入修改（如 SQL 注入防护）、权限增强
- **PostToolUse**：可用于执行结果审计、自动化测试验证、输出修改

建议在 `agent/` 模块的 Tool Loop 中增加 `before_tool_execute` 和 `after_tool_execute` 钩子，Rust 侧可定义 trait：

```rust
#[async_trait]
trait ToolHook: Send + Sync {
    async fn before_execute(&self, tool_name: &str, input: &Value) -> HookDecision;
    async fn after_execute(&self, tool_name: &str, input: &Value, output: &Value) -> HookAction;
}
```

#### P1 — 会话生命周期 Hook

OpenComputer 的自动记忆提取硬编码在对话结束流程中。建议抽象为可配置的 Hook 点：

- `SessionStart` — 注入初始上下文、加载项目特定记忆
- `SessionEnd` — 保存会话摘要、清理资源
- `PreCompact` / `PostCompact` — 压缩前后的自定义处理

#### P2 — 外部集成 Hook

Claude Code 的 HTTP hook 和 OpenClaw 的 Webhook 系统都支持外部服务集成。OpenComputer 可以：

- 在 `config.json` 中增加 `hooks` 配置段
- 支持 shell command 和 HTTP POST 两种类型
- 利用现有的 `tokio` 异步运行时执行

#### P3 — Hook 安全机制

借鉴 Claude Code 的安全设计：

- 工作区信任检查（防止不受信任目录中的恶意 hook）
- Hook 超时保护（防止 hook 无限阻塞）
- Hook allow 不覆盖 deny rule（防御纵深）
- 路径边界检查（防止 hook 访问沙箱外文件）

### 6.2 实现优先级建议

```
Phase 1: 基础框架
  ├── 定义 HookEvent enum 和 HookResult 类型
  ├── 实现 HookRegistry（Rust HashMap<HookEvent, Vec<HookHandler>>）
  └── 在 Tool Loop 中注入 PreToolUse / PostToolUse 调用点

Phase 2: 配置驱动
  ├── config.json hooks 段解析
  ├── Shell command hook 执行（tokio::process::Command）
  └── 超时和 abort 机制

Phase 3: 高级功能
  ├── HTTP hook（reqwest POST）
  ├── Hook 条件过滤（工具名匹配）
  ├── 异步 hook 支持
  └── 会话生命周期 hooks

Phase 4: 安全增强
  ├── 工作区信任检查
  ├── Hook 权限决策不覆盖 deny rule
  └── 路径边界和沙箱检查
```

### 6.3 各项目优势总结

| 项目 | 核心优势 |
|------|---------|
| **Claude Code** | Hook 类型最丰富（command/prompt/agent/http/callback），权限决策集成最深（allow/deny/ask + 防御纵深），事件覆盖最广（27 种） |
| **OpenClaw** | Hook 安装管理最成熟（npm/git/archive + integrity 校验），Webhook 防护最完善（限流/并发/异常追踪），消息管道标准化最好 |
| **OpenComputer** | 暂无优势，但 Rust 后端架构和 tokio 异步运行时为实现高性能 Hook 系统提供了良好基础 |

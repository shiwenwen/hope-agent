# 工具系统对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.1
> OpenComputer 当前工具数：40+ | Claude Code 当前工具数：43+ | OpenClaw 当前工具数：~31（+ 动态 Channel 插件工具）

## 架构差异

| 维度         | OpenComputer                                                                                                                                  | OpenClaw                                                                       |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| 定位         | 本地桌面 AI 助手（Tauri + Rust）                                                                                                              | 云端 Agent 平台（Node.js）                                                     |
| 编码工具来源 | Rust 自研（`crates/oc-core/src/tools/`）                                                                                                           | `@mariozechner/pi-coding-agent` 库 + 自研覆盖                                  |
| 工具注册     | Rust `get_available_tools()` + 条件注入                                                                                                       | `pi-tools.ts` 组装编码工具 + `openclaw-tools.ts` 组装平台工具                  |
| 扩展机制     | SKILL.md 技能系统（3 层加载：extra dirs → `~/.opencomputer/skills/` → `.opencomputer/skills/`，frontmatter 声明 + 环境检查 + 系统提示词注入） | `/skills/` 目录动态加载插件工具（`resolvePluginTools`，运行时注入为独立 tool） |
| 记忆工具     | Rust 自研（SQLite + FTS5 + 向量检索），6 个专用工具                                                                                           | `memory-core` 扩展插件（`extensions/memory-core/`），2 个工具 + 文件系统写入   |
| 浏览器       | Rust CDP 直连，核心工具                                                                                                                       | Plugin 注册（`tool-catalog.ts`），sandbox bridge 代理                          |

## 共有工具对比

### 文件系统 & 执行

| 工具        | OpenComputer  | OpenClaw                     | 功能差异                                                                                                                     |
| ----------- | ------------- | ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| read        | `read`        | `read`（pi-coding-agent）    | OC 支持图片 base64 读取；🔻待追：OpenClaw 支持 context window 自适应输出（根据剩余 token 动态截断）    |
| write       | `write`       | `write`（pi-coding-agent）   | 基本一致；OpenClaw 多 sandbox 双模式（与 OC 定位不同，不需追）                                                |
| edit        | `edit`        | `edit`（pi-coding-agent）    | OC 支持更多参数别名（old_string/oldText 等）；基本对齐                                                |
| apply_patch | `apply_patch` | `apply_patch`（自研）        | OC 始终可用；OpenClaw 仅 OpenAI provider + 白名单模型启用                                                                    |
| ls          | `ls`          | `ls`（pi-coding-agent）      | 基本一致                                                                                                                     |
| grep        | `grep`        | `grep`（pi-coding-agent）    | 基本一致，都遵守 .gitignore                                                                                                  |
| find        | `find`        | `find`（pi-coding-agent）    | 基本一致                                                                                                                     |
| exec        | `exec`        | `exec`（自研 bash-tools）    | OC 多 `pty`、Docker `sandbox` 参数；🔻待追：OpenClaw 的 approval 机制（敏感命令审批）、scopeKey 隔离 |
| process     | `process`     | `process`（自研 bash-tools） | OC 更多 action（log/write/clear/remove）；🔻待追：OpenClaw 的 scopeKey 隔离防跨 session 可见                                         |

### Web & 信息

| 工具       | OpenComputer | OpenClaw     | 功能差异                                                         |
| ---------- | ------------ | ------------ | ---------------------------------------------------------------- |
| web_search | `web_search` | `web_search` | 都支持多搜索引擎，基本一致；🔻待追：OpenClaw 支持 runtime 动态切换（运行时切换搜索引擎） |
| web_fetch  | `web_fetch`  | `web_fetch`  | 都用 Readability + Markdown；🔻待追：OpenClaw 额外支持 Firecrawl runtime（更好的 JS 渲染页面抓取） |

### 记忆

| 工具     | OpenComputer    | OpenClaw                            | 功能差异                                                                            |
| -------- | --------------- | ----------------------------------- | ----------------------------------------------------------------------------------- |
| 记忆搜索 | `recall_memory` | `memory_search`（memory-core 插件） | 功能类似（语义/关键词检索）；OC 用 SQLite FTS5 + 向量，OpenClaw 用 manager.search() |
| 记忆读取 | `memory_get`    | `memory_get`（memory-core 插件）    | OC 按 ID 读取完整元数据；OpenClaw 按文件路径 + 行号范围读取                         |

### 定时任务

| 工具 | OpenComputer  | OpenClaw | 功能差异                                |
| ---- | ------------- | -------- | --------------------------------------- |
| cron | `manage_cron` | `cron`   | 基本一致，都支持一次性/周期/cron 表达式 |

### 浏览器

| 工具    | OpenComputer          | OpenClaw                 | 功能差异                                                                             |
| ------- | --------------------- | ------------------------ | ------------------------------------------------------------------------------------ |
| browser | `browser`（核心工具） | `browser`（plugin 注册） | OC 用 CDP 直连，核心工具；OpenClaw 支持 sandbox bridge URL + node 远程浏览器代理路由 |

### 多模态 / 媒体

| 工具           | OpenComputer     | OpenClaw         | 功能差异                                                                                 |
| -------------- | ---------------- | ---------------- | ---------------------------------------------------------------------------------------- |
| image          | `image`          | `image`          | **OC 领先**：多图（10 张原始视觉数据直达模型）+ URL + 剪贴板 + 截屏；OpenClaw 多图（20 张）但仅生成文字描述，丢失视觉细节 |
| image_generate | `image_generate` | `image_generate` | OC 支持 OpenAI/Google/Fal 三 Provider；OpenClaw 按配置推断 Provider                      |
| pdf            | `pdf`            | `pdf`            | **OC 领先**：三模式（auto/text/vision），vision 渲染页面为图片直达模型（全 Provider 支持），auto 模式智能检测扫描件自动切换；支持 URL + 多 PDF（10 份）。OpenClaw 仅 Anthropic/Google 原生 + 其余 Provider 文本/图像回退 |

### Canvas

| 工具   | OpenComputer                        | OpenClaw                                             | 功能差异                                                 |
| ------ | ----------------------------------- | ---------------------------------------------------- | -------------------------------------------------------- |
| canvas | `canvas`（11 action，7 种内容类型） | `canvas`（present/hide/navigate/eval/snapshot/A2UI） | OC 功能更丰富（版本历史、导出等）；OpenClaw 多 A2UI 模式 |

### 子 Agent & 会话管理

| 工具              | OpenComputer                  | OpenClaw                                          | 功能差异                                                                                                             |
| ----------------- | ----------------------------- | ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| 子 Agent 生命周期 | `subagent`（单工具 9 action） | `sessions_spawn` + `subagents` + `sessions_yield` | OpenClaw 拆分为 3 个独立工具；OC 合并为 1 个工具（spawn/check/list/result/kill/kill_all/steer/batch_spawn/wait_all） |
| ACP Agent         | `acp_spawn`（独立工具）       | `sessions_spawn` 的 `runtime="acp"` 模式          | OC 单独拆出 ACP 启动；OpenClaw 统一在 sessions_spawn 中                                                              |
| 会话列表          | `sessions_list`               | `sessions_list`                                   | 基本一致                                                                                                             |
| 会话历史          | `sessions_history`            | `sessions_history`                                | 基本一致                                                                                                             |
| 跨会话消息        | `sessions_send`               | `sessions_send`                                   | OC 支持同步等待 + 异步投递；OpenClaw 通过 sessionKey/label 定位                                                      |
| 会话状态          | `session_status`              | `session_status`                                  | 基本一致                                                                                                             |
| Agent 列表        | `agents_list`                 | `agents_list`                                     | 基本一致                                                                                                             |

## OpenComputer 独有工具

| 工具                 | 说明                                                  | 备注                                                   |
| -------------------- | ----------------------------------------------------- | ------------------------------------------------------ |
| `save_memory`        | 显式保存记忆（4 种类型 + 2 种作用域）                 | OpenClaw 记忆写入通过文件系统（MEMORY.md）而非专用工具 |
| `update_memory`      | 按 ID 更新记忆内容和标签                              | OpenClaw 无此细粒度操作                                |
| `delete_memory`      | 按 ID 删除记忆                                        | OpenClaw 无此细粒度操作                                |
| `update_core_memory` | 更新核心记忆文件（memory.md），直接反映在系统提示词中 | OpenClaw 通过 write 工具写 MEMORY.md 实现类似效果      |
| `send_notification`  | macOS 原生桌面通知（条件注入）                        | OpenClaw 用 `message` 工具覆盖通知场景（多渠道）       |
| `get_weather`        | 天气查询（Open-Meteo API，免费无 key）                | OpenClaw 无对应工具                                    |
| `plan_question`      | Plan Mode：向用户发送结构化问题（选项 + 自定义输入）  | OpenClaw 无对应的计划系统                              |
| `submit_plan`        | Plan Mode：提交最终实施计划，进入 Review 状态         | 同上                                                   |
| `update_plan_step`   | Plan Mode：更新计划步骤状态（进行中/完成/跳过/失败）  | 同上                                                   |
| `amend_plan`         | Plan Mode：执行中修改计划（插入/删除/更新步骤）       | 同上                                                   |

## OpenClaw 独有工具

### 优先级 P2 — 扩展能力（尚未补齐）

| 工具      | 说明                                                 | 补齐建议                                                                                                                |
| --------- | ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `message` | 多渠道消息发送（Slack/Discord/Telegram/WhatsApp 等） | 需要先设计通道抽象层，OC 的 `send_notification` 仅覆盖桌面通知；OpenClaw 支持 auto-threading、reply-to 模式、group 路由 |
| `tts`     | 文本转语音                                           | 语音输出能力，OpenClaw 按 channel provider 条件启用                                                                     |
| `nodes`   | 设备控制（摄像头/截屏/定位/通知/invoke）             | IoT/设备集成，OpenClaw 支持 node 远程路由 + media invoke                                                                |
| `gateway` | 网关配置管理（restart/config/update）                | 平台运维能力，owner-only 权限控制                                                                                       |

## 数量统计

| 分类                                     | OpenComputer                                               | OpenClaw                                              |
| ---------------------------------------- | ---------------------------------------------------------- | ----------------------------------------------------- |
| **总工具数**                             | **36**                                                     | **~31** + Channel 插件                                |
| 文件系统（read/write/edit/ls/grep/find） | 6                                                          | 6（pi-coding-agent）                                  |
| 执行（exec/process）                     | 2                                                          | 2（bash-tools）                                       |
| 补丁（apply_patch）                      | 1                                                          | 1（条件启用）                                         |
| Web（search/fetch）                      | 2                                                          | 2                                                     |
| 记忆                                     | 6（recall/save/update/delete/get/update_core）             | 2（search/get，memory-core 插件）                     |
| 定时任务                                 | 1                                                          | 1                                                     |
| 浏览器                                   | 1                                                          | 1（plugin）                                           |
| 子 Agent / 会话                          | 6（subagent + acp*spawn + sessions*\*4）                   | 7（spawn/yield/send/list/history/status + subagents） |
| 通知 / 消息                              | 1（桌面通知）                                              | 1（多渠道消息）                                       |
| Agent 管理                               | 1（agents_list）                                           | 1（agents_list）                                      |
| 多模态 / 媒体                            | 3（image/image_generate/pdf）                              | 4（image/image_generate/tts/pdf）                     |
| 画布 / Canvas                            | 1（canvas）                                                | 1（canvas）                                           |
| 计划 / Plan                              | 4（plan_question/submit_plan/update_plan_step/amend_plan） | 0                                                     |
| 天气                                     | 1（get_weather）                                           | 0                                                     |
| 平台特有                                 | 0                                                          | 2（nodes/gateway）                                    |

## 差异总结

### OpenComputer 领先的领域

- **记忆系统**：6 个专用工具（save/recall/update/delete/get/update_core），SQLite + FTS5 + 向量检索，细粒度 CRUD；OpenClaw 仅 2 个工具（search/get）+ 文件系统写入
- **Plan Mode**：完整的 4 工具计划系统（六态状态机），OpenClaw 无对应能力
- **天气查询**：内置免费天气 API，OpenClaw 无对应
- **Canvas**：11 个 action + 7 种内容类型 + 版本历史，比 OpenClaw 更丰富
- **Image 视觉分析**：多图（10 张）+ URL + 剪贴板 + 截屏，且图片作为原始视觉数据直达模型（OpenClaw 经二次模型转述丢失细节）
- **PDF 视觉分析**：三模式（auto/text/vision），vision 渲染页面为图片全 Provider 直达模型；auto 智能检测扫描件；支持 URL + 多 PDF。OpenClaw 仅 Anthropic/Google 原生支持

### OpenClaw 领先的领域（🔻待追清单）

- 🔻 **多渠道消息**：`message` 工具支持 Slack/Discord/Telegram/WhatsApp 等多渠道，auto-threading、group 路由
- 🔻 **语音输出**：`tts` 文字转语音
- ~~**PDF 原生视觉分析**~~ → 已被 OC 超越（三模式 auto/text/vision，全 Provider 支持视觉渲染）
- 🔻 **read 自适应输出**：根据剩余 context window 动态截断输出
- 🔻 **exec approval 机制**：敏感命令审批 + scopeKey 隔离
- 🔻 **web_search runtime 切换**：运行时动态切换搜索引擎
- 🔻 **web_fetch Firecrawl**：JS 渲染页面更好的抓取能力
- **设备控制**：`nodes` 工具支持 IoT 远程设备（与桌面端定位不同，低优先级）
- **网关运维**：`gateway` 平台级配置管理（桌面端不适用）
- ~~**Image 多图 + URL**~~ → 已被 OC 超越

### 🔻 待追优先级汇总

| 优先级 | 类型     | 项目                           | 说明                                                       |
| ------ | -------- | ------------------------------ | ---------------------------------------------------------- |
| ~~P1~~ | ~~工具增强~~ | ~~`pdf` 原生视觉分析~~     | ✅ 已完成：三模式（auto/text/vision）+ URL + 多 PDF，全 Provider 视觉渲染 |
| P1     | 工具增强 | `read` context window 自适应   | 根据剩余 token 动态截断，避免大文件撑爆上下文              |
| P2     | 新工具   | `message` 多渠道消息           | 需设计通道抽象层，工程量较大                               |
| P2     | 工具增强 | `exec` approval + scopeKey     | 敏感命令审批机制 + 跨 session 隔离                         |
| P2     | 工具增强 | `web_fetch` Firecrawl          | JS 渲染页面抓取能力增强                                    |
| P3     | 工具增强 | `web_search` runtime 动态切换  | 运行时切换搜索引擎                                         |
| P3     | 新工具   | `tts` 语音输出                 | 语音场景在桌面端需求有限                                   |
| P4     | 新工具   | `nodes` 设备控制               | IoT 场景与桌面端定位不同                                   |
| P4     | 新工具   | `gateway` 网关运维             | 平台运维能力，桌面端不适用                                 |

---

## Claude Code 工具系统

### 1. 架构总览

Claude Code 是 Anthropic 官方推出的 CLI 智能编码助手，其工具系统基于 **TypeScript + React（Ink）** 构建，运行在 Node.js/Bun 环境中。整体设计遵循以下原则：

- **强类型安全**：所有工具输入使用 Zod v4 定义 schema，运行时严格校验，schema 同时用于 API 传输和本地验证
- **声明式工具定义**：通过 `buildTool()` 工厂函数统一构建，提供 fail-closed 默认值（如 `isConcurrencySafe` 默认 `false`，`isReadOnly` 默认 `false`）
- **并发分区执行**：工具调用按 `isConcurrencySafe` 标记自动分区为并行批次和串行批次
- **多层权限防护**：规则匹配 → 分类器 → Hook → 用户交互，形成纵深防御
- **延迟加载（Deferred Loading）**：通过 `ToolSearchTool` 元工具按需加载工具 schema，减少初始 prompt token 消耗
- **MCP 协议集成**：原生支持 Model Context Protocol，外部 MCP 工具与内置工具统一管理

技术栈：TypeScript、Zod v4（schema 校验）、React/Ink（终端 UI 渲染）、Bun bundler（feature flag 编译期消除）。

### 2. 工具定义与注册

**关键代码路径**：
- `~/Codes/claude-code/src/Tool.ts` — `Tool` 接口定义、`buildTool()` 工厂函数、`ToolUseContext` 执行上下文
- `~/Codes/claude-code/src/tools.ts` — 工具注册表 `getAllBaseTools()`、`getTools()`、`assembleToolPool()`
- `~/Codes/claude-code/src/tools/<ToolName>/` — 每个工具一个独立目录（prompt.ts、UI.tsx、主逻辑文件）

**Tool 接口核心字段**：

```typescript
type Tool<Input, Output, P> = {
  name: string
  aliases?: string[]           // 向后兼容别名
  searchHint?: string          // ToolSearch 关键词匹配短语
  shouldDefer?: boolean        // 是否延迟加载
  alwaysLoad?: boolean         // 强制不延迟（MCP 工具 opt-out）
  maxResultSizeChars: number   // 结果超限时持久化到磁盘
  strict?: boolean             // 严格模式（API 更严格遵守 schema）
  inputSchema: Input           // Zod v4 schema
  
  // 核心方法
  call(args, context, canUseTool, parentMessage, onProgress): Promise<ToolResult<Output>>
  checkPermissions(input, context): Promise<PermissionResult>
  validateInput?(input, context): Promise<ValidationResult>
  
  // 并发与安全标记
  isConcurrencySafe(input): boolean    // 是否可并行执行
  isReadOnly(input): boolean           // 是否只读
  isDestructive?(input): boolean       // 是否不可逆操作
  interruptBehavior?(): 'cancel' | 'block'  // 用户中断时行为
  
  // UI 渲染方法
  renderToolUseMessage(input, options): React.ReactNode
  renderToolResultMessage?(content, progress, options): React.ReactNode
  renderGroupedToolUse?(toolUses, options): React.ReactNode  // 并行工具分组渲染
  prompt(options): Promise<string>     // 生成发给 LLM 的工具描述
}
```

**`buildTool()` 工厂函数**提供 fail-closed 默认值：
- `isEnabled` → `true`
- `isConcurrencySafe` → `false`（保守假设不安全）
- `isReadOnly` → `false`（保守假设有写入）
- `isDestructive` → `false`
- `checkPermissions` → `{ behavior: 'allow', updatedInput }`（交由通用权限系统处理）

**注册机制**：`getAllBaseTools()` 函数是所有工具的唯一注册入口，通过条件表达式控制工具的加载：

```typescript
export function getAllBaseTools(): Tools {
  return [
    AgentTool, BashTool, FileReadTool, FileEditTool, FileWriteTool,
    // feature flag 条件加载
    ...(hasEmbeddedSearchTools() ? [] : [GlobTool, GrepTool]),
    // 环境变量条件加载
    ...(process.env.USER_TYPE === 'ant' ? [ConfigTool, TungstenTool] : []),
    // Bun feature flag（编译期消除死代码）
    ...(feature('KAIROS') ? [SleepTool] : []),
    // MCP 资源工具
    ListMcpResourcesTool, ReadMcpResourceTool,
    // ToolSearch 元工具
    ...(isToolSearchEnabledOptimistic() ? [ToolSearchTool] : []),
  ]
}
```

`getTools()` 在此基础上执行两层过滤：
1. `filterToolsByDenyRules()` — 移除被权限规则全局禁用的工具
2. `isEnabled()` 检查 — 移除运行时禁用的工具

`assembleToolPool()` 最终合并内置工具和 MCP 工具，按名称排序（内置工具作为前缀以保持 prompt cache 稳定性），`uniqBy` 确保同名时内置工具优先。

### 3. 工具执行引擎

**关键代码路径**：
- `~/Codes/claude-code/src/services/tools/toolExecution.ts` — `runToolUse()` 单工具执行流程、`checkPermissionsAndCallTool()` 权限检查+调用
- `~/Codes/claude-code/src/services/tools/toolOrchestration.ts` — `runTools()` 编排层、`partitionToolCalls()` 并发分区
- `~/Codes/claude-code/src/services/tools/StreamingToolExecutor.ts` — `StreamingToolExecutor` 类，流式并发执行器
- `~/Codes/claude-code/src/services/tools/toolHooks.ts` — `runPreToolUseHooks()`、`runPostToolUseHooks()` Hook 执行

**单工具执行流程**（`runToolUse()` → `checkPermissionsAndCallTool()`）：

```
1. Zod schema 校验输入 → 失败返回 InputValidationError
2. validateInput() 工具自定义校验 → 失败返回错误
3. 启动投机性 Bash 分类器检查（与后续步骤并行）
4. backfillObservableInput() 补充遗留/派生字段（浅拷贝，不影响 call()）
5. 执行 PreToolUse Hooks → 可产出 hookPermissionResult / updatedInput / stop
6. resolveHookPermissionDecision() 权限决策
   ├── Hook 已决策 → 直接使用
   └── Hook 未决策 → 走规则匹配 + 分类器 + 用户交互
7. 权限通过 → tool.call() 执行工具
8. 执行 PostToolUse Hooks → 可修改输出 / 阻止
9. 工具结果超限 → processToolResultBlock() 持久化到磁盘
10. 返回 ToolResult（含 newMessages、contextModifier）
```

**并发分区执行**（`partitionToolCalls()`）：

将一批 tool_call 按连续的 `isConcurrencySafe` 属性分区为多个批次（Batch）：
- **并发安全批次**：连续的 `isConcurrencySafe=true` 工具打包为一个批次，通过 `runToolsConcurrently()` 并行执行，最大并发度由 `CLAUDE_CODE_MAX_TOOL_USE_CONCURRENCY`（默认 10）控制
- **非并发安全批次**：单个工具独占一个批次，通过 `runToolsSerially()` 串行执行
- 批次间严格串行：并发批次内的 contextModifier 在批次结束后统一应用

**流式并发执行器**（`StreamingToolExecutor`）：

用于工具调用在流式响应中逐个到达的场景：
- 维护 `TrackedTool` 队列，每个工具有 `queued | executing | completed | yielded` 四种状态
- `addTool()` 添加新工具后立即尝试 `processQueue()`
- 并发控制：并发安全工具可与其他并发安全工具并行；非并发安全工具需要独占执行
- 错误传播：Bash 工具出错时通过 `siblingAbortController`（子 AbortController）取消兄弟工具
- 结果按工具接收顺序 buffer 并输出，保证消息顺序

### 4. 延迟加载（Deferred Loading）

**关键代码路径**：
- `~/Codes/claude-code/src/tools/ToolSearchTool/prompt.ts` — `isDeferredTool()` 判定逻辑
- `~/Codes/claude-code/src/tools/ToolSearchTool/ToolSearchTool.ts` — ToolSearch 元工具实现
- `~/Codes/claude-code/src/utils/toolSearch.ts` — `isToolSearchEnabledOptimistic()` 启用判定

**机制**：当工具数量超过阈值时，将部分工具的完整 schema 从初始 prompt 中移除，仅在 `<system-reminder>` 消息中列出工具名称。模型需要先调用 `ToolSearchTool` 获取完整 schema 后才能正确调用。

**`isDeferredTool()` 判定规则**（按优先级）：
1. `alwaysLoad === true` → 不延迟（MCP 工具可通过 `_meta['anthropic/alwaysLoad']` opt-out）
2. `isMcp === true` → 始终延迟（MCP 工具视为 workflow-specific）
3. `name === TOOL_SEARCH_TOOL_NAME` → 不延迟（元工具本身必须始终可用）
4. 特定核心工具豁免（如 AgentTool 在 FORK_SUBAGENT 模式下、BriefTool 等）
5. `shouldDefer === true` → 延迟

**ToolSearchTool 查询形式**：
- `"select:Read,Edit,Grep"` — 按名称精确选择
- `"notebook jupyter"` — 关键词搜索，返回最多 max_results 个匹配
- `"+slack send"` — 要求名称包含 "slack"，按其余词排名

**Schema 未发送检测**：当工具因延迟加载未发送 schema 而导致 Zod 校验失败时，`buildSchemaNotSentHint()` 自动在错误信息中追加提示，引导模型先调用 `ToolSearchTool` 加载 schema。

### 5. 权限过滤

**关键代码路径**：
- `~/Codes/claude-code/src/types/permissions.ts` — `PermissionMode`、`PermissionBehavior`、`PermissionRule` 类型定义
- `~/Codes/claude-code/src/utils/permissions/permissions.ts` — `checkRuleBasedPermissions()`、deny/allow/ask 规则匹配
- `~/Codes/claude-code/src/utils/permissions/bashClassifier.ts` — Bash 命令分类器
- `~/Codes/claude-code/src/utils/permissions/yoloClassifier.ts` — bypassPermissions 模式下的安全分类
- `~/Codes/claude-code/src/utils/permissions/denialTracking.ts` — 拒绝追踪与降级逻辑

**权限模式（PermissionMode）**：

| 模式 | 说明 |
|------|------|
| `default` | 默认模式，需要用户逐个审批 |
| `acceptEdits` | 自动接受文件编辑操作 |
| `plan` | 计划模式，只读操作 |
| `bypassPermissions` | 跳过权限检查（受 yoloClassifier 安全兜底） |
| `dontAsk` | 不询问，无权限时自动拒绝 |
| `auto` | 基于 transcript classifier 自动决策 |

**权限决策流程**（多层纵深防御）：

```
1. 规则匹配（Rule-based）
   ├── alwaysDenyRules → deny（全局禁止）
   ├── alwaysAskRules → ask（强制询问）
   └── alwaysAllowRules → allow（已授权）
   规则来源优先级：policySettings > flagSettings > projectSettings > localSettings > userSettings > session > cliArg

2. 工具自定义权限（tool.checkPermissions()）

3. PreToolUse Hooks → 外部 Hook 可决策 allow/deny/ask/stop

4. 分类器（Classifier）
   ├── bashClassifier — Bash 命令语义分类
   └── yoloClassifier — bypassPermissions 模式安全兜底
   
5. 用户交互 → canUseTool() → 弹出权限对话框
```

### 6. 内置工具清单

#### 文件操作类
| 工具 | 功能 |
|------|------|
| `FileReadTool` | 读取文件内容，支持 PDF（分页）、图片（多模态）、Jupyter Notebook |
| `FileEditTool` | 精确字符串替换编辑文件 |
| `FileWriteTool` | 创建/完整覆写文件 |
| `NotebookEditTool` | Jupyter Notebook 单元格编辑 |

#### 搜索与导航类
| 工具 | 功能 |
|------|------|
| `GrepTool` | 基于 ripgrep 的正则搜索，支持 glob 过滤、上下文行 |
| `GlobTool` | 文件名模式匹配搜索 |
| `BashTool` | 通用 Shell 命令执行，支持超时、沙箱、进度报告 |

#### Web 与搜索类
| 工具 | 功能 |
|------|------|
| `WebSearchTool` | Web 搜索 |
| `WebFetchTool` | 抓取网页内容 |

#### Agent 与任务管理类
| 工具 | 功能 |
|------|------|
| `AgentTool` | 启动子 Agent（支持 fork 模式） |
| `TaskCreateTool` | 创建任务 |
| `TaskGetTool` / `TaskUpdateTool` / `TaskListTool` / `TaskStopTool` | 任务 CRUD |
| `TodoWriteTool` | 写入 TODO 面板 |
| `SendMessageTool` | 向对等 Agent 发送消息 |
| `TeamCreateTool` / `TeamDeleteTool` | 创建/删除 Agent 团队（swarm 模式） |

#### 计划与模式控制类
| 工具 | 功能 |
|------|------|
| `EnterPlanModeTool` | 进入计划模式 |
| `ExitPlanModeV2Tool` | 退出计划模式 |
| `EnterWorktreeTool` / `ExitWorktreeTool` | Git worktree 隔离工作区 |

#### MCP 与扩展类
| 工具 | 功能 |
|------|------|
| `ToolSearchTool` | 延迟加载元工具，按名称/关键词搜索并返回完整 schema |
| `ListMcpResourcesTool` / `ReadMcpResourceTool` | MCP 资源访问 |
| `LSPTool` | Language Server Protocol 交互 |
| `SkillTool` | 调用 Skill 技能 |

#### 调度与自动化类
| 工具 | 功能 |
|------|------|
| `CronCreateTool` / `CronDeleteTool` / `CronListTool` | 定时任务管理 |
| `RemoteTriggerTool` | 远程触发器 |
| `SleepTool` | 定时等待（Proactive 模式） |

#### 交互类
| 工具 | 功能 |
|------|------|
| `AskUserQuestionTool` | 向用户提问等待回复 |
| `SyntheticOutputTool` | 强制结构化输出 |

### 7. Claude Code 独有能力

**1. 流式工具执行器（StreamingToolExecutor）**：支持工具在 API 响应流式到达时即刻开始执行，而非等待所有 tool_call 解析完毕。通过状态机（queued → executing → completed → yielded）管理并发。

**2. 编译期 Feature Flag 死代码消除**：通过 Bun bundler 的 `feature()` 宏实现编译期条件编译，未启用的工具代码在构建时完全消除。

**3. 投机性分类器预检**：Bash 工具在 PreToolUse Hook 执行前就启动安全分类器检查（`startSpeculativeClassifierCheck()`），与 Hook、权限对话框并行执行，减少等待时间。

**4. 工具别名（aliases）向后兼容**：工具重命名后保留旧名作为别名（如 `KillShell` → `TaskStop`），旧会话记录中的 tool_call 仍能正确路由。

**5. MCP 工具深度集成**：MCP 外部工具与内置工具共享完整的权限系统、并发控制、延迟加载和 UI 渲染管线。支持 MCP 服务器级权限规则、`alwaysLoad` opt-out 延迟加载。

**6. 纵深权限防御体系**：六种权限模式 + 多来源规则 + 分类器 + Hook + 拒绝追踪降级。特别是 `yoloClassifier`（bypassPermissions 安全兜底）和 `denialTracking`（连续拒绝自动降级）是独有设计。

**7. 中断行为控制（interruptBehavior）**：每个工具可声明用户中断时的行为：`'cancel'`（立即停止并丢弃结果）或 `'block'`（继续运行，新消息排队等待）。

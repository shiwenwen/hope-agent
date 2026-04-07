# 三项目统一维度对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-06 | 版本：v2.0（统一维度版）
> 替换文档：competitive-analysis.md + 14 份子文档

## 前言

| 项目 | 一句话定位 |
|------|-----------|
| **OpenComputer (OC)** | 本地 AI 桌面助手——Tauri + Rust 核心，GUI/Server/ACP 三模式，28 Provider 模板，12 IM 渠道 |
| **Claude Code (CC)** | Anthropic 官方 CLI 编码助手——TypeScript + Bun，终端 TUI + IDE Bridge，MCP 原生集成 |
| **OpenClaw (OW)** | 多渠道本地 AI 网关——TypeScript + Node.js，WebSocket 控制面，25+ 平台接入，OpenAI 兼容 API |

**评分标准**：5 = 业界领先 / 4 = 完善 / 3 = 可用 / 2 = 基础 / 1 = 缺失 / 0 = 不适用

**阅读指引**：每章统一结构——**能力矩阵表** → **关键差异分析** → **评分行**。第 16 章汇总所有评分，第 18 章给出 OC 可追项。

---

## 第 1 章：项目定位与技术栈

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|:------------|:-----------|:---------|
| 核心语言 | Rust（后端）+ TypeScript（前端） | TypeScript（全栈） | TypeScript（全栈） |
| UI 形态 | 桌面 GUI（Tauri 2 窗口） | 终端 TUI（React/Ink） + IDE 面板 | CLI + macOS/iOS/Android Companion App |
| 前端框架 | React 19 + Tailwind v4 + shadcn/ui | React + Ink（终端渲染） | 无 Web 前端（CLI + 原生 App） |
| 构建工具 | Vite 8（前端）+ Cargo（后端） | Bun bundler + Feature Flag 编译期消除 | pnpm workspaces + Node.js ESM |
| 数据存储 | SQLite（会话/日志/记忆） | 文件系统（~/.claude/sessions/） | 内存（默认）+ 可插拔后端 |
| 桌面框架 | Tauri 2 | 无（终端应用） | 无（Companion App 为原生） |
| 用户模型 | 单用户本地 | 单用户本地 | 单 Operator 多 Agent |
| 部署模式数 | 3（GUI / HTTP Server / ACP stdio） | 2（CLI / IDE Bridge） | 3（Gateway / CLI / Companion App） |
| 代码规模 | ~30 Rust 模块 + React 前端 | ~1,900 TS 文件, 512K+ LOC | 60+ 目录, 318 CLI 子命令 |
| 测试框架 | Cargo test | Vitest | Vitest + 契约测试 |

**设计哲学差异**

- **OC**：重型桌面应用，Rust 性能保障 + GUI 傻瓜操作。核心逻辑零 Tauri 依赖（oc-core），可复用于 Server/ACP。多 Provider 兼容是核心卖点。
- **CC**：开发者工具，终端优先。Anthropic 模型深度集成，MCP 生态连接，IDE Bridge 无缝嵌入编码流。Feature Flag 精细控制功能集。
- **OW**：消息网关，渠道覆盖优先。25+ 平台统一接入，OpenAI 兼容 API 降低集成门槛。DM 配对安全模型保障个人隐私。

---

## 第 2 章：LLM Provider 与模型支持

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| API 类型 | 4（Anthropic / OpenAI Chat / OpenAI Responses / Codex） | 1（Anthropic） + Bedrock 多区域 | 可配置（OpenAI / Anthropic / Gemini / OpenRouter / 本地模型） |
| 预置模板数 | **28** | 0（硬编码 Anthropic） | 按 agent 配置（无模板概念） |
| 预置模型数 | **108** | ~6（Opus/Sonnet/Haiku 各版本） | 按 agent 自行配置 |
| Extended Thinking | 4 种格式（OpenAI/Anthropic/Qwen/Z.AI） | Anthropic 原生（adaptive budget） | 透传（取决于上游 API） |
| Prompt Cache | Anthropic 显式 + OpenAI 自动前缀缓存 | Anthropic 显式（1h TTL） | 无自有缓存（透传上游） |
| 模型链降级 | 5 类错误分类 + 指数退避 2 次 + 跳下一模型 | 指数退避重试 + 非流式降级 | 单 Agent 单模型（无链式降级） |
| 自定义端点 | 任意 base_url | 无（Anthropic 固定） | 任意 OpenAI 兼容端点 |
| 温度配置 | **3 层覆盖**（会话 > Agent > 全局） | 模型固定 | Agent 级配置 |
| Token 计数 | 动态估算 + `TokenEstimateCalibrator` 学习校准 | Anthropic API 精确计数 + 估算 | 透传上游 usage |
| Failover | ContextOverflow→compaction, RateLimit/Overloaded/Timeout→重试, Auth/Billing→跳模型 | 流式超时→非流式降级, 错误→重试 | 无自有降级策略 |
| Side Query 缓存 | 复用 system_prompt + history 前缀，侧查询成本降低 **90%** | 无 | 无 |

**关键差异**

OC 在 Provider 多样性上 **远超** CC 和 OW——28 个预置模板覆盖了主流商业和开源模型 API。CC 专注 Anthropic 生态，深度集成但锁定单一供应商。OW 采用配置驱动，灵活但缺乏预置优化（无降级链、无缓存策略）。

OC 的 Side Query 缓存是独创设计，利用 prompt cache 复用使 Tier 3 摘要和记忆提取成本降低约 90%，这在高频对话场景下有显著成本优势。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| Provider 支持 | **5** | 2 | 3 |

---

## 第 3 章：工具系统

### 3.1 架构对比

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 工具总数 | **37** | **43+** | ~31 + Channel 插件 |
| 编码工具来源 | Rust 自研 | TypeScript 自研 | `@mariozechner/pi-coding-agent` + 自研覆盖 |
| 工具注册 | `get_available_tools()` + 条件注入 | `getAllBaseTools()` + Feature Flag 编译期消除 | `pi-tools.ts` + `openclaw-tools.ts` 组装 |
| 扩展机制 | SKILL.md 系统（frontmatter + 系统提示词注入） | MCP 协议（外部工具统一管理） + Skill 系统 | `/skills/` 目录动态加载 + `resolvePluginTools` |
| 定义格式 | Rust struct（name/description/parameters/concurrent_safe） | `buildTool()` 工厂（Zod v4 schema + isConcurrencySafe） | TypeScript 函数式定义 |

### 3.2 工具清单对比

#### 文件系统 & 执行

| 工具 | OC | CC | OW |
|------|:---|:---|:---|
| read | `read`（支持图片 base64） | `FileReadTool`（PDF 分页 + 图片 + Jupyter） | `read`（context window 自适应截断） |
| write | `write` | `FileWriteTool` | `write` |
| edit | `edit`（参数别名 oldText/newText） | `FileEditTool` | `edit` |
| apply_patch | `apply_patch` | 无 | `apply_patch`（仅 OpenAI provider） |
| ls | `ls` | 无（通过 Bash） | `ls` |
| grep | `grep` | `GrepTool`（ripgrep） | `grep` |
| find | `find` | `GlobTool`（模式匹配） | `find` |
| exec | `exec`（PTY + Docker sandbox） | `BashTool`（160K 行，安全校验） | `exec`（approval + scopeKey 隔离） |
| process | `process`（7 action） | 无（通过 Bash） | `process`（scopeKey 隔离） |
| notebook | 无 | `NotebookEditTool`（Jupyter 单元格） | 无 |

#### Web & 信息

| 工具 | OC | CC | OW |
|------|:---|:---|:---|
| web_search | `web_search`（9 引擎：Brave/Google/Tavily/DDG/Perplexity/Kimi/Grok/SearXNG/Jina） | `WebSearchTool` | `web_search`（runtime 动态切换引擎） |
| web_fetch | `web_fetch`（SSRF 防护） | `WebFetchTool`（15min 缓存 + Markdown 转换） | `web_fetch`（+ Firecrawl JS 渲染） |

#### 记忆

| 工具 | OC | CC | OW |
|------|:---|:---|:---|
| 保存 | `save_memory`（4 种类型 + 2 种作用域） | 自动 CLAUDE.md（memdir） | 文件系统写 MEMORY.md |
| 搜索 | `recall_memory`（FTS5 + 向量） | 无专用工具（文件读取） | `memory_search`（manager.search） |
| 读取 | `memory_get` | 无 | `memory_get`（文件路径 + 行号） |
| 更新 | `update_memory` | 无 | 无 |
| 删除 | `delete_memory` | 无 | 无 |
| 核心记忆 | `update_core_memory` | 无 | 无 |

#### 浏览器 & Canvas

| 工具 | OC | CC | OW |
|------|:---|:---|:---|
| browser | `browser`（CDP 直连，核心工具） | 无内置（通过 MCP 扩展） | `browser`（plugin + sandbox bridge） |
| canvas | `canvas`（11 action，7 种内容类型，版本历史） | 无 | `canvas`（present/hide/navigate/eval/A2UI） |

#### 多模态 / 媒体

| 工具 | OC | CC | OW |
|------|:---|:---|:---|
| image | `image`（多图 10 张，URL/剪贴板/截屏，原始视觉数据直达模型） | 通过 FileReadTool（图片 resize） | `image`（多图 20 张，但转文字描述） |
| image_generate | `image_generate`（7 Provider） | 无 | `image_generate`（按配置推断 Provider） |
| pdf | `pdf`（三模式 auto/text/vision，URL，多 PDF 10 份） | 通过 FileReadTool（文本提取 + 分页） | `pdf`（Anthropic/Google 原生 + 回退） |
| tts | 无 | 无（语音 feature-gated） | `tts`（ElevenLabs/Edge TTS） |

#### 子 Agent & 会话

| 工具 | OC | CC | OW |
|------|:---|:---|:---|
| 子 Agent | `subagent`（9 action 合一） | `AgentTool`（fork 模式 + worktree 隔离） | `sessions_spawn` + `subagents` + `sessions_yield` |
| ACP Agent | `acp_spawn` | 无（自身就是 ACP） | `sessions_spawn(runtime="acp")` |
| Team/Swarm | 无 | `TeamCreateTool` / `TeamDeleteTool` / `SendMessageTool` | 无 |
| 会话列表 | `sessions_list` | `TaskListTool` | `sessions_list` |
| 会话历史 | `sessions_history` | 无专用工具（session 文件） | `sessions_history` |
| 跨会话消息 | `sessions_send` | `SendMessageTool` | `sessions_send` |
| 会话状态 | `session_status` | `TaskGetTool` | `session_status` |

#### 计划 & 调度

| 工具 | OC | CC | OW |
|------|:---|:---|:---|
| 结构化问答 | `plan_question` | 无 | 无 |
| 提交计划 | `submit_plan` | 无 | 无 |
| 步骤追踪 | `update_plan_step` | `TodoWriteTool` | 无 |
| 修改计划 | `amend_plan` | 无 | 无 |
| 进入/退出计划模式 | Plan Mode 六态状态机 | `EnterPlanModeTool` / `ExitPlanModeTool` | 无 |
| Cron | `manage_cron` | `CronCreateTool` / `CronDeleteTool` / `CronListTool` | `cron` |
| 远程触发 | 无 | `RemoteTriggerTool` | 无 |

#### 通知 & 其他

| 工具 | OC | CC | OW |
|------|:---|:---|:---|
| 通知 | `send_notification`（macOS 桌面） | 无 | `message`（多渠道消息发送） |
| 天气 | `get_weather`（Open-Meteo） | 无 | 无 |
| 工具搜索 | `tool_search` | `ToolSearchTool` | 无 |
| 审批 | `approval` | `AskUserQuestionTool` | approval 内置 |
| LSP | 无 | `LSPTool`（goToDefinition/findReferences/hover） | 无 |
| MCP 资源 | 无 | `ListMcpResourcesTool` / `ReadMcpResourceTool` | 无 |
| Git Worktree | 无 | `EnterWorktreeTool` / `ExitWorktreeTool` | 无 |
| 设备控制 | 无 | 无 | `nodes`（摄像头/截屏/定位） |
| 网关运维 | 无 | 无 | `gateway`（restart/config） |

### 3.3 执行引擎对比

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 并发模型 | `concurrent_safe` 标记（16 个只读工具并行，写入串行） | `isConcurrencySafe` 标记（批次分区，最大并发 10） | 无并发控制（单工具串行） |
| 流式执行 | 等待所有 tool_call 解析后执行 | `StreamingToolExecutor`（流式到达即刻执行，4 态状态机） | 等待完整响应 |
| 延迟加载 | opt-in（`deferredTools.enabled`），核心 ~10 工具始终加载 | 自动（工具数超阈值触发），`shouldDefer` / `alwaysLoad` 控制 | 无 |
| 大结果持久化 | 超 50KB 写入磁盘，上下文保留 head+tail 预览 | `maxResultSizeChars` 超限持久化磁盘 | 无（截断） |
| 超时控制 | 每工具可配置 timeout | 全局 + 每工具 timeout，非流式降级 | 每工具 timeout |
| 权限过滤 | `ToolPermissionMode`（auto/ask/deny） | **6 种模式**（default/acceptEdits/plan/bypass/dontAsk/auto）+ 分类器 + Hooks | approval 机制 + sandbox |
| 投机分类器 | 无 | Bash 命令分类器预检（与权限并行） | 无 |
| 工具别名 | 参数别名（oldText/old_string） | 工具名别名（旧名→新名路由） | 无 |

**关键差异**

CC 的工具系统在**执行引擎**上最成熟——流式执行器（工具到达即执行）、投机分类器（安全检查与执行并行）、6 层权限纵深防御。OC 在**工具种类丰富度**上领先（browser/canvas/image_generate/pdf 视觉模式均为自研核心工具），但执行引擎缺乏流式执行和投机分类器。OW 执行引擎最简，但 approval 机制和 scopeKey 隔离是 OC 可借鉴的。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 工具系统 | 4 | **5** | 3 |

---

## 第 4 章：Agent 与 Session 管理

### 4.1 子 Agent 能力

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 触发方式 | `subagent` 工具（9 action 合一） | `AgentTool`（fork + worktree 隔离） | `sessions_spawn` + `subagents` |
| 内置 Agent 类型 | 用户自定义（无预设） | 验证 Agent、Guide Agent 等预设 | 用户配置 Agent（无预设） |
| 最大嵌套深度 | 3（可配置 1-5） | 无限制（由 token 约束） | 无嵌套（扁平 session） |
| 并发上限 | 5 per session | 无硬上限（token 控制） | 无硬上限 |
| 隔离机制 | 独立 session，`skill_allowed_tools` 过滤 | Fork 子进程 + Git Worktree | 独立 session |
| 结果回传 | Mailbox 异步注入 + 前台等待 30s 自动转后台 | 同步返回（fork 结果合并） | `sessions_yield` 回传 |
| 深度感知资源约束 | 按 depth 递减 token/tool 预算 | 无 | 无 |

### 4.2 Team/Swarm 协作

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| Team 创建 | 无 | `TeamCreateTool`（多 Agent 协作） | 无 |
| Coordinator 编排 | 无 | 协调者模式（共享上下文，任务分发） | 无 |
| Agent 间双向消息 | `sessions_send`（单向投递） | `SendMessageTool`（双向实时） | `sessions_send`（通过 label 定位） |
| Git Worktree 隔离 | 无 | `EnterWorktreeTool` / `ExitWorktreeTool` | 无 |
| Remote Agent | 无 | `RemoteTriggerTool` | 无 |

### 4.3 Session 管理

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 持久化 | SQLite（session_id/name/agent_id/token_usage） | 文件系统（~/.claude/sessions/，JSON + JSONL） | 内存（默认）+ 可插拔后端 |
| 会话恢复 | 支持（从 SQLite 加载） | `/resume` 命令 + 状态重建 | 支持（session key） |
| 跨设备 | 通过 HTTP Server 模式 | Direct Connect + Bridge 模式 | SSH tunnel + Tailscale |
| 附件管理 | `~/.opencomputer/attachments/{session_id}/` 归档 | LRU 文件缓存 | Temp 文件 + 过期清理 |
| Token 追踪 | 每模型 input/output/cost | 每轮 budget + 累计 cost | 透传上游 usage |
| 会话搜索 | FTS5 全文搜索 | `searchSessionsByCustomTitle` | 按 key/label 过滤 |

**关键差异**

CC 的 Team/Swarm 能力是三者中最强的——Coordinator 模式、双向消息、Git Worktree 隔离形成了完整的多 Agent 协作栈。OC 的子 Agent 系统在单 Agent 场景下功能完善（深度感知、Mailbox 回传、前台/后台自动切换），但缺乏多 Agent 协作能力。OW 的 Session 模型最灵活（多 Agent 路由 + 渠道映射），但子 Agent 能力最弱。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| Agent 协作 | 3 | **5** | 2 |
| Session 管理 | 4 | **5** | 4 |

---

## 第 5 章：记忆与上下文管理

### 5.1 记忆系统

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 存储后端 | **SQLite + FTS5 + vec0 向量扩展** | 文件系统（CLAUDE.md + memdir） | 文件系统（MEMORY.md）+ memory-core 插件 |
| 记忆类型 | 4 种（facts/preferences/instructions/context）+ 2 种作用域 | user_context/project_notes/team_notes/memories | 无分类（纯文本） |
| 语义搜索 | **向量相似度 + FTS 混合 + MMR 多样性** | 无（文件扫描） | manager.search()（关键词） |
| 全文搜索 | **FTS5** | 无 | 无 |
| 自动提取 | 每轮 inline 执行（最多 5 次/会话） | 自动 memory extraction hooks | 无 |
| LLM 语义选择 | 候选 > 8 时 side_query 选择 ≤5 条注入 | 无 | 无 |
| 作用域 | Global + Agent | Per-project + user-level | Per-workspace + MEMORY.md |
| Team 记忆同步 | 无 | Team memory sync（共享 Agent 定义） | 无 |
| 记忆老化 | 无 | Age tracking + freshness notes | 无 |
| Embedding 提供者 | **8 种**（OpenAI/Google/Jina/Cohere/SiliconFlow/Voyage/Mistral/Local ONNX） | 无（不用 embedding） | 无 |
| Pin 置顶 | 无 | Pinned cache edits（protect 策略工具不清除） | 无 |

### 5.2 上下文管理

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 压缩层级 | **5 层**（Tier 0 微压缩 → Tier 1 截断 → Tier 2 修剪 → Tier 3 LLM 摘要 → Tier 4 紧急） | 3 层（microcompact + snipCompact + context collapse，部分 feature-gated） | 无自有压缩 |
| API-Round 分组 | `_oc_round` 元数据标记，确保 tool_use/tool_result 不被拆散 | 无（按消息级压缩） | 无 |
| 后压缩文件恢复 | Tier 3 后自动注入最近编辑文件内容（5 文件 × 16KB） | 无 | 无 |
| Side Query 缓存 | 复用 prompt 前缀，侧查询成本降低 **90%** | 无 | 无 |
| Reactive Compact | 无 | Token 预警→自动触发压缩 | 无 |
| Tool Use Summary | 无 | 工具调用结果摘要（减少上下文占用） | 无 |
| Token 估算 | 动态估算 + `TokenEstimateCalibrator` 学习 | Anthropic API 精确 + 估算 | 透传上游 |

**关键差异**

OC 的记忆系统是三者中最完善的——SQLite + FTS5 + 向量检索 + 8 种 Embedding 提供者 + LLM 语义选择，形成了工业级的长期记忆能力。CC 采用轻量文件方案但有 Team 记忆同步和记忆老化。OC 的上下文管理也最深（5 层渐进压缩 + API-Round 分组 + 后压缩文件恢复 + Side Query 缓存），但缺少 CC 的 Reactive Compact 和 Tool Use Summary。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 记忆系统 | **5** | 4 | 3 |
| 上下文管理 | **5** | 4 | 2 |

---

## 第 6 章：规划与工作流

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 状态机 | **六态**（Off/Planning/Review/Executing/Paused/Completed） | 二态（Normal/Plan） | 无 |
| 双 Agent 分离 | 支持（`plan_subagent: true` 隔离计划探索） | 无（Plan Mode 仅限制工具） | 无 |
| 执行层权限 | `plan_mode_allowed_tools` 白名单 + schema 过滤双层防护 | 工具子集限制（只读） | 无 |
| 步骤追踪 | `update_plan_step`（in_progress/completed/failed/skipped） | `TodoWriteTool`（pending/in_progress/completed） | 无 |
| Git Checkpoint | 执行前自动 checkpoint | 无 | 无 |
| 交互式问答 | `plan_question`（选项 + 多选 + 自定义 + 推荐标记） | 无 | 无 |
| 暂停/恢复 | Paused 状态 | 无 | 无 |
| 计划修改 | `amend_plan`（执行中插入/删除/更新步骤） | 无 | 无 |
| Plan 文件持久化 | `~/.opencomputer/plans/` | 文件系统（plan file） | 无 |
| Plan 验证 | 无 | 无（依赖外部 Skill） | 无 |

**关键差异**

OC 的 Plan Mode 是三者中最完整的——六态状态机、双 Agent 分离、执行层权限强制、Git Checkpoint、交互式问答、暂停/恢复、执行中修改计划。CC 的 Plan Mode 是轻量级的（仅限制工具为只读子集），但结合 Skill 系统（superpowers:writing-plans 等）可以达到类似效果。OW 无任何规划能力。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| Plan Mode | **5** | 3 | 1 |

---

## 第 7 章：Skill / 扩展系统

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 格式 | SKILL.md（YAML frontmatter + Markdown 内容） | Skill 文件（Markdown + 元数据） | `/skills/` 目录 + frontmatter |
| 发现来源 | 3 层（extra dirs → `~/.opencomputer/skills/` → `.opencomputer/skills/`） | 4 层（bundled + CLAUDE_CODE_SKILLS_DIR + 项目目录 + MCP） | `~/.openclaw/skills/` + ClawHub marketplace |
| 执行模型 | 3 种（Prompt 注入 / Tool 调用 / Slash 命令） | Prompt 注入（通过 `SkillTool`） | Prompt 注入 + 动态工具注册 |
| 内置 Skill 数 | 无预装（用户创建） | 30+ bundled（Slack/GitHub/前端/调试/TDD 等） | 40+ bundled（GitHub/Discord/Himalaya 等） |
| 工具隔离 | `allowed-tools` frontmatter（schema + execution 双层过滤） | 无（Skill 无工具限制） | 无 |
| Fork 模式 | `context: fork`（子 Agent 执行，不污染主对话） | 无 | 无 |
| 安装系统 | `install` frontmatter（命令 + cwd） | 无（bundled 或手动放置） | npm 安装 |
| Effort 级别 | 无 | 支持（fast/balanced/thorough） | 无 |
| 环境要求检测 | `requires` frontmatter（feature + versions 语义版本） | 条件激活（per file path） | 无 |
| MCP Skills | 无 | MCP 服务器→Skill 桥接 | 通过 mcporter 桥接 |
| Managed Skills | 无 | 组织推送（MDM / managed settings） | 无 |
| 远程 Skill | 无 | 无（bundled 分发） | ClawHub marketplace |
| Slash 命令集成 | 内置（`command-dispatch: slash`） | 内置（Skill 注册自定义命令） | 内置 |

**关键差异**

CC 在内置 Skill 丰富度和 MCP 桥接上领先。OC 在 Skill **隔离性**（allowed-tools 双层过滤 + Fork 模式）和**安全性**（环境要求检测 + 安装系统）上领先。OW 有 ClawHub marketplace 远程分发。OC 缺少 MCP Skills 桥接和组织级 Managed Skills。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| Skill 系统 | 4 | **5** | 3 |

---

## 第 8 章：协议支持

| 协议 | OC | CC | OW |
|------|:---|:---|:---|
| **MCP**（Model Context Protocol） | **无** | 原生客户端（119K 行）：工具调用 + 资源访问 + Prompt 模板 + OAuth + 官方注册表 | 通过 mcporter 桥接 |
| **ACP**（Agent Coding Protocol） | 原生 Rust 实现：stdio + NDJSON-RPC 2.0，支持 VS Code/Zed/Cursor | 自身即 ACP 端（IDE 通过 Bridge 连接） | ACP Bridge port（18790） |
| **OpenAI-compatible API** | 无 | 无 | 完整（/v1/chat/completions, /v1/models, /v1/embeddings, /v1/responses） |
| **LSP** | 无 | `LSPTool`（goToDefinition/findReferences/hover/documentSymbol） | 无 |
| **WebSocket 控制面** | EventBus 事件总线（内部） | 无 | 完整（60+ RPC 方法，versioned protocol v3） |
| **HTTP REST API** | axum Router（oc-server） | 无（CLI 直连 Anthropic API） | 同端口 HTTP 端点 |

**关键差异**

OC 在协议支持上有明显短板：**MCP 缺失**是最关键的 gap，这意味着无法接入 MCP 生态中的工具和资源（GitHub、Slack、数据库等 MCP 服务器）。CC 的 MCP 支持最完整（含 OAuth、资源、Prompt 模板）。OW 通过 OpenAI 兼容 API 提供最强的第三方集成能力。OC 的 ACP 实现是原生 Rust（零延迟），而 CC 和 OW 分别通过 Bridge 和端口暴露。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 协议支持 | 2 | **4** | **4** |

---

## 第 9 章：安全与权限

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 权限模式数 | 3（auto/ask/deny） | **6**（default/acceptEdits/plan/bypass/dontAsk/auto） | 3（approval/sandbox/open） |
| 分类器预检 | 无 | Bash 分类器 + yolo 安全兜底 | 无 |
| Hooks 系统 | 无 | **8 种事件**（PreToolUse/PostToolUse/pre-compact/post-compact/session-start 等） | 无 |
| Docker 沙箱 | `exec --sandbox`（容器隔离） | macOS 原生沙箱检测 | opt-in sandbox 模式 |
| 组织策略/MDM | 无 | Windows Registry + macOS plutil + managed-settings.d | 无 |
| 路径限制 | Tauri CSP scope 限制 | 设备路径阻止（/dev/*） + scratchpad 隔离 | 无 |
| macOS TCC 权限 | **15 种检测**（Accessibility/Screen/Automation/Disk/Location/Camera/Mic 等） | 无 | 无 |
| API Key 脱敏 | 首 4 + 末 4 字符 + 日志自动 redact | 无（不存储第三方 key） | SecretRef + 快照 redaction |
| DM 配对安全 | 无（桌面应用无需） | 无（CLI 无需） | **pairing/open/closed** 三策略 + allowlist |
| 设备认证 | 无 | JWT（IDE Bridge） | 设备指纹 + 公钥签名 + 角色作用域 |
| 拒绝追踪降级 | 无 | 连续拒绝自动降级权限模式 | 无 |
| 规则来源层级 | 1（session） | **7 层**（policy > flag > project > local > user > session > cliArg） | 2（config + env） |
| 中断行为控制 | 无 | 每工具 `interruptBehavior`（cancel/block） | 无 |

**关键差异**

CC 的权限系统是三者中最深的——6 种模式、7 层规则来源、Hooks 系统、分类器预检、拒绝追踪降级形成完整纵深防御。OC 在 macOS TCC 权限检测（15 种）上独有，但缺少 Hooks 系统和多层权限模式。OW 的 DM 配对安全是多渠道场景下的必要设计。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 权限安全 | 3 | **5** | 4 |

---

## 第 10 章：多模态能力

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 图片输入 | **10 张多图 + URL + 剪贴板 + 截屏**，原始视觉数据直达模型 | 图片读取 + resize（token 优化） | 20 张多图但转文字描述（丢失视觉细节） |
| 图片生成 | **7 Provider**（OpenAI/Google/FAL/MiniMax/SiliconFlow/Tongyi/Zhipu） | 无 | 按配置推断 Provider |
| PDF | **三模式**（auto/text/vision）+ URL + 多 PDF 10 份，vision 渲染页面为图片 | 文本提取 + 分页 | Anthropic/Google 原生 + 回退 |
| 语音输入 | 无 | STT（feature-gated） | **唤醒词 + 实时转录**（Deepgram） |
| 语音输出 | 无 | 语音合成（feature-gated） | **TTS**（ElevenLabs/Edge TTS） |
| 视频 | Browser CDP 播放 | 无 | MP4/WebM（预览生成） |
| Canvas | **11 action × 7 内容类型**（HTML/MD/SVG/Mermaid/Chart.js/Slides），版本历史 + 导出 | 无 | present/hide/navigate/eval/A2UI |

**关键差异**

OC 在视觉处理上全面领先——图片直达模型（非文字转述）、PDF 三模式视觉渲染、7 个图片生成 Provider、Canvas 版本历史。CC 当前多模态能力最弱（语音 feature-gated 未开放）。OW 在语音能力（唤醒词 + 实时转录 + TTS）上领先。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 多模态 | **5** | 3 | 3 |

---

## 第 11 章：渠道 / 平台集成

### 11.1 渠道清单

| 渠道 | OC | CC | OW |
|------|:--:|:--:|:--:|
| Telegram | 内置 | - | 内置 |
| WeChat | 内置（iLink 协议） | - | npm 插件 |
| Discord | 内置 | - | 内置 |
| Slack | 内置 | - | 内置 |
| Feishu/Lark | 内置 | - | 扩展 |
| QQ Bot | 内置 | - | 扩展 |
| IRC | 内置 | - | 内置 |
| Signal | 内置 | - | 内置 |
| iMessage | 内置 | - | 内置（BlueBubbles） |
| WhatsApp | 内置 | - | 内置 |
| Google Chat | 内置 | - | 内置 |
| LINE | 内置 | - | 内置 |
| Matrix | - | - | 扩展 |
| Mattermost | - | - | 内置 |
| Teams | - | - | 内置 |
| Nostr | - | - | 内置 |
| Twitch | - | - | 内置 |
| Zalo | - | - | 内置 |
| Nextcloud Talk | - | - | 内置 |
| Synology Chat | - | - | 内置 |
| Tlon | - | - | 内置 |
| WebChat | - | - | 内置 |

### 11.2 渠道能力

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 渠道总数 | **12** | **0** | **25+** |
| 入站媒体管道 | 下载→解密（WeChat AES）→Attachment→归档→多模态输入 | 无 | 下载→转码→Attachment→temp 清理 |
| 出站媒体管道 | 加密（WeChat AES-128-ECB）→CDN→发送（3× 重试） | 无 | 媒体验证→大小限制→MIME 检查 |
| Typing 指示器 | WeChat（24h TTL + 5s keepalive）、Telegram | 无 | 多渠道原生 typing |
| DM 配对安全 | 无 | 无 | pairing/open/closed + allowlist |
| 群组策略 | 无（私聊为主） | 无 | open/closed + activation 模式 |
| 语音能力 | 无 | 无 | 唤醒词 + Talk Mode + 实时转录 |
| 原生 App | 无 | 无 | macOS/iOS/Android Companion App |
| 斜杠命令同步 | Telegram `setMyCommands` | 无 | 各渠道原生命令 |

**关键差异**

OW 渠道覆盖最广（25+），且有完整的安全模型（DM 配对、群组策略）和语音能力。OC 覆盖 12 个核心渠道，WeChat 集成最深（AES 加密/解密、QR 登录刷新、typing 指示器）。CC 无任何渠道集成（定位为编码工具）。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 渠道集成 | 4 | 0 | **5** |

---

## 第 12 章：IDE 集成

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| VS Code 插件 | ACP stdio 协议（通过 ACP 支持） | **原生 Bridge**（115K 行 bridgeMain.ts）+ JWT 认证 | 无 |
| JetBrains | ACP 支持 | **原生 Bridge** | 无 |
| Web IDE | 无 | claude.ai/code（Web 版） | 无 |
| Zed | ACP 支持 | 无 | 无 |
| Cursor | ACP 支持 | 无 | 无 |
| Neovim | ACP 支持 | 无 | 无 |
| LSP 集成 | 无 | `LSPTool`（goToDefinition/findReferences/hover/documentSymbol） | 无 |
| REPL Bridge | 无 | `replBridge.ts`（100K 行） | 无 |
| 远程连接 | HTTP Server 模式 | Direct Connect + Session teleportation | SSH tunnel |

**关键差异**

CC 的 IDE 集成最深——原生 Bridge 双向通信、JWT 认证、REPL 模式、LSP 工具、Web IDE。OC 通过 ACP 协议支持更多 IDE（Zed/Cursor/Neovim），但集成深度不如 CC（无 LSP、无 REPL）。OW 无 IDE 集成。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| IDE 集成 | 3 | **5** | 1 |

---

## 第 13 章：CLI 与用户交互

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| GUI 桌面应用 | **Tauri 窗口**（完整 GUI） | 无 | Companion App（macOS/iOS/Android） |
| 终端 TUI | 无 | **React/Ink 渲染**（分组工具、进度条、权限对话框） | 无 |
| 斜杠命令数 | **20+**（6 类：会话/模型/记忆/Agent/Plan/工具） | **103**（commit/review/compact/mcp/config/doctor 等） | 通过 agent 路由 |
| CLI 子命令数 | 3（server start/install/uninstall/status/stop） | 10+（login/logout/mcp/config 等） | **318** |
| i18n 语言数 | **12** | 1（英文） | 1（英文） |
| Dashboard | **8 维度大盘**（Token/Cost/Session/Error/Cron/Subagent/Metrics） | `/cost` 命令（Token + USD） | 无 |
| Hooks 系统 | 无 | **8 种事件**（session-start/pre-compact/PreToolUse/PostToolUse 等） | 无 |
| 键盘快捷键 | 无（GUI 交互） | 自定义（~/.claude/keybindings.json）+ chord binding | 无 |
| Vim 模式 | 无 | `/vim` 命令 | 无 |
| 主题 | 无（跟随系统） | `/theme` 命令 | 无 |
| 环境诊断 | 无 | `/doctor` 命令 | `openclaw doctor` |

**关键差异**

OC 以 GUI 桌面体验见长（Dashboard 大盘、12 语言 i18n），CC 以终端开发者体验见长（103 命令、Hooks、快捷键、Vim），OW 以 CLI 管理能力见长（318 子命令覆盖全部网关操作）。OC 缺少 Hooks 系统是重要 gap。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| CLI/UX | **5** | 4 | 4 |

---

## 第 14 章：部署与基础设施

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 部署模式 | Desktop GUI + HTTP/WS Server + ACP stdio | CLI + IDE Bridge | Gateway + CLI + Companion App |
| 系统服务注册 | macOS launchd + Linux systemd | 无（CLI 手动启动） | macOS launchd + Linux systemd + Windows Task |
| Docker 支持 | SearXNG 容器 + exec sandbox | 无 | **多阶段构建**（bookworm/slim，扩展选择） |
| Docker Compose | 无 | 无 | 完整（gateway + cli + healthcheck） |
| Guardian 心跳 | 统一 keepalive（桌面 + 服务器共用） | 无 | heartbeat tick |
| 热配置重载 | 无 | 无 | **hybrid**（hot + restart 自适应） |
| Feature Flag | 无 | **Bun 编译期消除** + GrowthBook 远程 | 无 |
| 自动更新 | 无 | `autoUpdater.ts` | 无 |
| Companion App | 无 | 无 | macOS/iOS/Android |
| Cron 调度 | SQLite 存储 + Agent 执行 + 指数退避 | Session 持久化 + 7 天过期 | 持久化 + 调度 |

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 部署 | 4 | 4 | **4** |

---

## 第 15 章：监控与可观测性

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 成本追踪 | **每模型 input/output Token × 单价 = USD** | Token budget + cost-tracker.ts | 透传上游 usage |
| Dashboard 维度 | **8**（Token/Cost/Session/Error/Cron/Subagent/Metrics/Model breakdown） | 1（`/cost` 简表） | 1（`/healthz`） |
| TTFT 指标 | 记录并展示 | 无 | 无 |
| 错误分类 | 5 类（RateLimit/Overloaded/Timeout/Auth/ContextOverflow） | 按 HTTP 状态码 | 无分类 |
| 日志脱敏 | API Key + Token 自动 redact + 请求体截断 32KB | 无（不存储敏感数据） | SecretRef redaction |
| 日志后端 | **SQLite + 纯文本双写**（非阻塞） | 文件系统 + OpenTelemetry | 文件系统 |
| 进程指标 | 内存/CPU/运行时 | 无 | 无 |

**关键差异**

OC 的可观测性最完善——8 维度 Dashboard 涵盖成本、性能（TTFT）、错误、子 Agent、Cron 等全链路指标，且日志系统支持 SQLite + 纯文本双写和自动脱敏。CC 有 OpenTelemetry 集成但 Dashboard 简单。OW 仅有健康检查。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 可观测性 | **5** | 3 | 2 |

---

## 第 16 章：综合评分矩阵

| 能力维度 | OC | CC | OW | OC 领先? |
|----------|:--:|:--:|:--:|:--------:|
| Provider 支持 | **5** | 2 | 3 | Y |
| 工具系统 | 4 | **5** | 3 | |
| Agent 协作 | 3 | **5** | 2 | |
| Session 管理 | 4 | **5** | 4 | |
| 记忆系统 | **5** | 4 | 3 | Y |
| 上下文管理 | **5** | 4 | 2 | Y |
| Plan Mode | **5** | 3 | 1 | Y |
| Skill 系统 | 4 | **5** | 3 | |
| 协议支持 | 2 | **4** | **4** | |
| 权限安全 | 3 | **5** | 4 | |
| 多模态 | **5** | 3 | 3 | Y |
| 渠道集成 | 4 | 0 | **5** | |
| IDE 集成 | 3 | **5** | 1 | |
| CLI/UX | **5** | 4 | 4 | Y |
| 部署 | 4 | 4 | 4 | |
| 可观测性 | **5** | 3 | 2 | Y |
| **合计** | **66/80** | **61/80** | **48/80** | |

### 各项目优势象限

- **OC 领先**（7/16）：Provider 支持、记忆系统、上下文管理、Plan Mode、多模态、CLI/UX、可观测性
- **CC 领先**（7/16）：工具系统、Agent 协作、Session 管理、Skill 系统、协议支持、权限安全、IDE 集成
- **OW 领先**（2/16）：渠道集成、协议支持（并列）

### 三项目核心差异化

| 项目 | 核心竞争力 | 薄弱环节 |
|------|-----------|----------|
| OC | 多 Provider 兼容 + 深度上下文管理 + 完整 Plan Mode + GUI 体验 | MCP 缺失、Hooks 缺失、多 Agent 协作弱 |
| CC | 工具执行引擎 + 多层权限 + IDE 深度集成 + MCP 生态 | 单一 Provider、无 GUI、无渠道集成 |
| OW | 25+ 渠道覆盖 + OpenAI 兼容 API + 安全模型 | 无 Plan、记忆弱、上下文管理弱、无 IDE |

---

## 第 17 章：OC 独有优势总结

| 优势领域 | 具体表现 | 竞争影响 |
|----------|---------|---------|
| Side Query 缓存 | 复用 prompt 前缀，侧查询成本降低 90% | CC/OW 无此优化，高频对话场景下 OC 成本优势显著 |
| 5 层上下文压缩 | Tier 0-4 渐进式，API-Round 分组保护 tool_use/tool_result 配对 | CC 仅 3 层（部分 feature-gated），OW 无 |
| 后压缩文件恢复 | Tier 3 后自动注入最近编辑文件内容（5 文件 × 16KB） | CC/OW 无，长对话编码场景减少额外 read 调用 |
| Plan Mode 六态状态机 | 双 Agent 分离 + 执行层权限 + Git Checkpoint + 暂停/恢复 + 执行中修改 | CC 仅二态，OW 无 |
| 交互式 Plan 问答 | `plan_question` 选项 + 多选 + 自定义 + 推荐标记 | CC/OW 无此交互模式 |
| 28 Provider 模板 | 108 预设模型，GUI 一键配置 | CC 锁定 Anthropic，OW 需手动配置 |
| 图片直达模型 | 多图 10 张 + URL + 剪贴板 + 截屏，原始视觉数据不经转述 | OW 图片转文字描述丢失细节 |
| PDF 三模式 | auto/text/vision + 扫描件智能检测 + URL + 多 PDF | CC 仅文本，OW 仅部分 Provider 原生 |
| 8 维度 Dashboard | Token/Cost/Session/Error/Cron/Subagent/Metrics/Model breakdown | CC 仅 `/cost` 简表，OW 仅 healthz |
| 8 种 Embedding 提供者 | OpenAI/Google/Jina/Cohere/SiliconFlow/Voyage/Mistral/Local ONNX | CC 不用 embedding，OW 无 |
| LLM 记忆语义选择 | 候选 > 8 时 side_query 精选 ≤5 条注入 | CC/OW 无，减少无关记忆噪音 |
| WeChat 深度集成 | AES-128-ECB 加密/解密 + QR 登录 + Typing 24h TTL | OW 的 WeChat 是外部 npm 包 |
| macOS TCC 15 种权限检测 | Accessibility/Screen/Automation/Disk/Location/Camera/Mic 等 | CC/OW 无 |

---

## 第 18 章：Actionable 差距清单（OC 待追项）

### P0 — 阻塞性缺失

| 缺失能力 | 参考来源 | 影响分析 | 估计复杂度 |
|----------|---------|---------|-----------|
| **MCP 协议支持** | CC 原生客户端（119K 行：工具调用 + 资源 + Prompt + OAuth + 注册表） | 无法接入 MCP 生态（GitHub/Slack/数据库等 MCP 服务器），与行业标准脱节。MCP 已成为 AI 工具互操作的事实标准 | 高（需实现 MCP Client SDK：传输层 + RPC + 工具代理 + 资源访问 + OAuth 流） |
| **Hooks 系统** | CC 8 种事件（PreToolUse/PostToolUse/pre-compact/post-compact/session-start 等） | 无法实现工具执行前后的外部拦截和自定义逻辑，限制了可扩展性和企业集成能力 | 中（EventBus 已有基础，需定义 Hook 接口 + 配置 schema + 执行管线） |

### P1 — 重要增强

| 缺失能力 | 参考来源 | 影响分析 | 估计复杂度 |
|----------|---------|---------|-----------|
| **Team/Swarm Agent** | CC `TeamCreateTool`/`TeamDeleteTool`/Coordinator 模式 | 无法进行多 Agent 协作（如一个 Agent 写代码、一个 Agent 写测试、一个 Agent 做 review） | 高 |
| **Git Worktree 隔离** | CC `EnterWorktreeTool`/`ExitWorktreeTool` | 子 Agent 无法在隔离分支上工作，存在互相干扰风险 | 中 |
| **多权限模式** | CC 6 种模式（default/acceptEdits/plan/bypass/dontAsk/auto） | 当前仅 3 种模式，无法满足不同信任级别场景（如 acceptEdits 自动接受编辑，bypass 全自动） | 中 |
| **`read` context window 自适应** | OW 的 read 工具根据剩余 token 动态截断输出 | 大文件可能撑爆上下文窗口 | 低 |
| **流式工具执行** | CC `StreamingToolExecutor`（工具到达即执行，4 态状态机） | 当前等待所有 tool_call 解析后才执行，延迟更高 | 中 |
| **投机分类器预检** | CC Bash 命令分类器（与权限检查并行执行） | 安全检查串行执行增加延迟 | 中 |

### P2 — 差异化扩展

| 缺失能力 | 参考来源 | 影响分析 | 估计复杂度 |
|----------|---------|---------|-----------|
| Agent 间双向实时消息 | CC `SendMessageTool` | 当前 `sessions_send` 仅单向投递，无法实现 Agent 间实时对话 | 中 |
| 语音输入/输出 | OW 唤醒词 + 实时转录 + TTS（ElevenLabs/Deepgram） | 语音交互在桌面端有需求（如边看代码边语音指令） | 高 |
| LSP 集成 | CC `LSPTool`（goToDefinition/findReferences/hover） | 无法利用 Language Server 提供精确的代码导航和分析 | 中 |
| DM 配对安全 | OW pairing/open/closed + allowlist | IM 渠道场景下无陌生人消息过滤机制 | 低 |
| MCP Skills 桥接 | CC MCP 服务器→Skill 格式桥接 | Skill 系统与 MCP 生态割裂 | 中（依赖 P0 MCP 完成） |
| 组织级 Managed Skills | CC managed-settings.d 推送 | 无法在团队/企业层面统一推送 Skill | 低 |
| `exec` approval 机制 | OW 敏感命令审批 + scopeKey 隔离 | 危险命令执行缺少结构化审批流程 | 低 |
| `web_fetch` Firecrawl | OW Firecrawl runtime 支持 | JS 重渲染页面抓取能力弱 | 低 |

### P3 — 锦上添花

| 缺失能力 | 参考来源 |
|----------|---------|
| Reactive Compact（Token 预警→自动压缩） | CC |
| Tool Use Summary（工具结果摘要） | CC |
| Structured Output（强制 JSON schema 输出） | CC |
| 记忆老化 + freshness notes | CC |
| Team 记忆同步 | CC |
| Pin 置顶（protect 策略工具不清除） | CC |
| 拒绝追踪降级 | CC |
| 中断行为控制（cancel/block per tool） | CC |
| Effort 级别（fast/balanced/thorough） | CC |
| 热配置重载（hybrid mode） | OW |
| `web_search` runtime 动态切换引擎 | OW |
| OpenAI 兼容 HTTP API | OW |
| 自动更新 | CC |

---

## 第 19 章：建议演进路线图

```
Phase 8: MCP 协议支持
├── MCP Client SDK (传输层 + JSON-RPC)
├── 工具代理 (MCP 工具→内置工具统一执行)
├── 资源访问 (文件/URL 资源读取)
├── OAuth 认证流
└── MCP Skills 桥接

Phase 9: Hooks 系统
├── Hook 接口定义 (PreToolUse/PostToolUse/session-start 等)
├── 配置 schema (settings.json 声明式)
├── 执行管线 (EventBus 订阅→Shell 命令执行)
└── 权限决策集成 (Hook 可 allow/deny/ask/stop)

Phase 10: 多 Agent 协作
├── Team 创建/删除/管理
├── Agent 间双向实时消息
├── Coordinator 编排模式
├── Git Worktree 隔离 (子 Agent 独立分支)
└── 深度感知资源调度

Phase 11: 权限模式扩展
├── 新增模式 (acceptEdits/bypass/dontAsk)
├── 多层规则来源 (project > local > user > session)
├── 分类器预检 (exec 命令语义分析)
├── 拒绝追踪降级
└── exec approval + scopeKey 隔离

Phase 12: 多模态增强
├── 语音输入 (STT: Deepgram/Whisper)
├── 语音输出 (TTS: ElevenLabs/Edge TTS)
├── LSP 集成 (goToDefinition/findReferences)
├── read context window 自适应
└── 流式工具执行 (StreamingToolExecutor)
```

---

> 本文档取代原 `competitive-analysis.md` + 14 份子文档。旧文档已在 git 历史中保留。

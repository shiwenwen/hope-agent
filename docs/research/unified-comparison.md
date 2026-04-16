# 四项目统一维度对比分析：OpenComputer vs Claude Code vs OpenClaw vs Hermes Agent

> 基线对比时间：2026-04-16 | 版本：v2.2（加入 Hermes Agent 自我学习对标版）
> 替换文档：competitive-analysis.md + 14 份子文档
> 演进路线图：见 [`roadmap-2026q2.md`](./roadmap-2026q2.md)

> **v2.2 增量要点**（相对 v2.0 / 2026-04-06）
> - **新增 Hermes Agent (HA)**：Nous Research 的自进化 AI Agent 框架（Python 3.11+，OpenAI SDK，200+ 模型 via OpenRouter），核心差异化是**闭环自我学习**——后台自动 Skill 创建/修补、记忆 nudging、FTS5 跨会话召回、Honcho 辩证用户建模。16 个消息平台、40+ 工具、6 种终端后端、MCP 集成。与 OC 的对标焦点：**自我学习与进化能力是 OC 目前完全缺失的维度**
> - **OC**：异步 Tool 执行（`async_jobs` fire-and-forget + retention）、`/recap` 深度复盘、Dashboard Insights（健康度/热力图/同比 Delta/CSV 导出）、会话级 `task_create/update/list`、通用 `ask_user_question`、跨会话 FTS5 搜索 + Find-in-Chat、SubagentGroup 聚合、pending-interaction 指示器、**Cargo Workspace 三 crate 分离**（`oc-core` / `oc-server` / `src-tauri`）、`opencomputer server` HTTP/WS 守护进程 + 系统服务注册、i18n 12 语言全量补齐
> - **OW**：**Context Engine 可插拔**（`registerContextEngine()` + 可插拔 Compaction Provider）、**Active Memory**（pre-reply 阻塞式记忆子 Agent）、**Dreaming**（light/deep/REM 三阶段离线记忆固化 + Dream Diary）、**SOUL.md**（人格文件与 AGENTS.md 分离）、**Delegate Architecture**（组织内代理 Agent 独立凭证+硬约束）、Honcho/QMD/LanceDB cloud/GitHub Copilot 多种记忆后端、Auth Profile 轮换 failover（两级：profile → 模型）、SSRF 纵深硬化（browser 默认 strict + symlink FD-realpath + constant-time secret 比较 + Feishu webhook 强制 encryptKey）、Codex / LM Studio / Arcee / MLX Talk Mode 新 bundled provider、apply_patch（gated）、localModelLean 弱本地模型模式
> - **评分变化**：OW 上下文管理 2→3、记忆系统 3→4、权限安全 4→5，合计 48→51；新增 HA 合计 57/85；OC 66→66/85、CC 61→61/85（新增"自我学习"维度后满分从 80 调为 85）

## 前言

| 项目 | 一句话定位 |
|------|-----------|
| **OpenComputer (OC)** | 本地 AI 桌面助手——Tauri + Rust 核心，GUI/Server/ACP 三模式，28 Provider 模板，12 IM 渠道 |
| **Claude Code (CC)** | Anthropic 官方 CLI 编码助手——TypeScript + Bun，终端 TUI + IDE Bridge，MCP 原生集成 |
| **OpenClaw (OW)** | 多渠道本地 AI 网关——TypeScript + Node.js，WebSocket 控制面，25+ 平台接入，OpenAI 兼容 API |
| **Hermes Agent (HA)** | 自进化 AI Agent 框架——Python 3.11+ + OpenAI SDK，闭环自我学习（Skill 自动创建/修补 + 记忆 nudging + 跨会话召回），200+ 模型 via OpenRouter，16 消息平台 |

**评分标准**：5 = 业界领先 / 4 = 完善 / 3 = 可用 / 2 = 基础 / 1 = 缺失 / 0 = 不适用

**阅读指引**：每章统一结构——**能力矩阵表** → **关键差异分析** → **评分行**。第 16 章汇总所有评分，第 18 章给出 OC 可追项。**v2.2 新增第 16.5 章专题分析 Hermes Agent 的自我学习闭环。**

---

## 第 1 章：项目定位与技术栈

| 维度 | OpenComputer | Claude Code | OpenClaw | Hermes Agent |
|------|:------------|:-----------|:---------|:------------|
| 核心语言 | Rust（后端）+ TypeScript（前端） | TypeScript（全栈） | TypeScript（全栈） | **Python 3.11+**（全栈） |
| UI 形态 | 桌面 GUI（Tauri 2 窗口） | 终端 TUI（React/Ink） + IDE 面板 | CLI + macOS/iOS/Android Companion App | **终端 TUI**（prompt_toolkit + Rich）+ 16 消息平台网关 |
| 前端框架 | React 19 + Tailwind v4 + shadcn/ui | React + Ink（终端渲染） | 无 Web 前端（CLI + 原生 App） | 无 Web 前端（TUI + Gateway） |
| 构建工具 | Vite 8（前端）+ Cargo（后端） | Bun bundler + Feature Flag 编译期消除 | pnpm workspaces + Node.js ESM | pip + setuptools（纯 Python） |
| 数据存储 | SQLite（会话/日志/记忆） | 文件系统（~/.claude/sessions/） | 内存（默认）+ 可插拔后端 | **SQLite + FTS5**（会话）+ **MEMORY.md / USER.md**（记忆）+ 可插拔后端 |
| 桌面框架 | Tauri 2 | 无（终端应用） | 无（Companion App 为原生） | 无 |
| 用户模型 | 单用户本地 | 单用户本地 | 单 Operator 多 Agent | 单用户本地（多 profile） |
| 部署模式数 | 3（GUI / HTTP Server / ACP stdio） | 2（CLI / IDE Bridge） | 3（Gateway / CLI / Companion App） | **4+**（CLI / Docker / SSH 远程 / Modal 无服务器 / Daytona / Singularity / Termux Android） |
| 代码规模 | ~30 Rust 模块 + React 前端 | ~1,900 TS 文件, 512K+ LOC | 60+ 目录, 318 CLI 子命令 | `run_agent.py` 10,871 行 + 40+ 工具文件 |
| 测试框架 | Cargo test | Vitest | Vitest + 契约测试 | pytest |

**设计哲学差异**

- **OC**：重型桌面应用，Rust 性能保障 + GUI 傻瓜操作。核心逻辑零 Tauri 依赖（oc-core），可复用于 Server/ACP。多 Provider 兼容是核心卖点。
- **CC**：开发者工具，终端优先。Anthropic 模型深度集成，MCP 生态连接，IDE Bridge 无缝嵌入编码流。Feature Flag 精细控制功能集。
- **OW**：消息网关，渠道覆盖优先。25+ 平台统一接入，OpenAI 兼容 API 降低集成门槛。DM 配对安全模型保障个人隐私。
- **HA**：**自进化 Agent 框架**，自我学习优先。闭环 Skill 自动创建/修补 + 记忆 nudging + 跨会话召回，"用得越多越聪明"是核心叙事。200+ 模型支持、极端部署灵活性（$5 VPS 到无服务器到手机 Termux）是次要卖点。

---

## 第 2 章：LLM Provider 与模型支持

| 维度 | OC | CC | OW | HA |
|------|:---|:---|:---|:---|
| API 类型 | 4（Anthropic / OpenAI Chat / OpenAI Responses / Codex） | 1（Anthropic） + Bedrock 多区域 | 可配置 + bundled provider（OpenAI / Anthropic / Gemini / OpenRouter / **Codex / LM Studio / Arcee / GitHub Copilot** / 本地模型） | OpenAI SDK 统一接口，支持 Anthropic / OpenAI / **OpenRouter（200+ 模型）** / xAI Grok / Qwen / MiniMax / Kimi / z.ai / Nous Portal / HuggingFace / 自定义端点 |
| 预置模板数 | **28** | 0（硬编码 Anthropic） | bundled provider 扩展中（无统一模板概念） | 0（通过 OpenRouter 动态列表，无预置模板） |
| 预置模型数 | **108** | ~6（Opus/Sonnet/Haiku 各版本） | 按 agent 配置 + provider catalog（GPT-5.4-pro / Gemma 4 / Arcee Trinity 等） | 200+（via OpenRouter catalog） |
| Extended Thinking | 4 种格式（OpenAI/Anthropic/Qwen/Z.AI） | Anthropic 原生（adaptive budget） | 透传（取决于上游 API） | Claude + OpenAI extended thinking + 轨迹保存（供 RL 训练） |
| Prompt Cache | Anthropic 显式 + OpenAI 自动前缀缓存 | Anthropic 显式（1h TTL） | 无自有缓存（透传上游） | Anthropic 前缀缓存（MEMORY 冻结快照模式） |
| 模型链降级 | 5 类错误分类 + 指数退避 2 次 + 跳下一模型 | 指数退避重试 + 非流式降级 | **两级 failover**：先轮换 Auth Profile（同 provider 换 key）→ 再跳下一模型 + per-session 模型 stickiness + cooldown 追踪 | 无降级链（单模型） |
| 自定义端点 | 任意 base_url | 无（Anthropic 固定） | 任意 OpenAI 兼容端点 | 任意 OpenAI 兼容端点 |
| 温度配置 | **3 层覆盖**（会话 > Agent > 全局） | 模型固定 | Agent 级配置 | 全局配置 |
| Token 计数 | 动态估算 + `TokenEstimateCalibrator` 学习校准 | Anthropic API 精确计数 + 估算 | 透传上游 usage | 透传上游 usage |
| Failover | ContextOverflow→compaction, RateLimit/Overloaded/Timeout→重试, Auth/Billing→跳模型 | 流式超时→非流式降级, 错误→重试 | 无自有降级策略 | 无 |
| Side Query 缓存 | 复用 system_prompt + history 前缀，侧查询成本降低 **90%** | 无 | 无 | 无（用廉价辅助模型 Gemini Flash 做跨会话摘要） |

**关键差异**

OC 在 Provider 多样性上 **仍领先** 其他三者——28 个预置模板覆盖主流商业和开源模型 API，GUI 傻瓜式配置。CC 专注 Anthropic 生态，深度集成但锁定单一供应商。OW v2.1 大量扩充 bundled provider 并引入**两级 failover**。HA 通过 OpenRouter 支持 200+ 模型但无预置模板和降级链——模型覆盖广但工程深度浅。

OC 的 Side Query 缓存是独创设计，利用 prompt cache 复用使 Tier 3 摘要和记忆提取成本降低约 90%，这在高频对话场景下有显著成本优势。HA 用"廉价辅助模型"（Gemini Flash）做跨会话摘要是类似思路但实现方式不同。

| 评分 | OC | CC | OW | HA |
|------|:--:|:--:|:--:|:--:|
| Provider 支持 | **5** | 2 | 3 | 4 |

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
| apply_patch | `apply_patch` | 无 | `apply_patch`（**新增，gated by `tools.exec.applyPatch`**，OpenAI-only） |
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
| 结构化问答 | `ask_user_question` | 无 | 无 |
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
| 异步 Tool 执行 | **`async_capable` + `run_in_background`**：模型可把 `exec`/`web_search`/`image_generate` detach 为后台 job，立即返回 `{job_id, status}`，对话继续，完成后通过 mailbox 注入。三道决策（模型显式 / Agent 策略 / 同步预算自动后台化 30s）+ 独立 `async_jobs.db` + spool 文件 + retention 清理（30 天）+ deferred `job_status(block?)` 工具主动 poll/wait | 无 | 无 |

**关键差异**

CC 的工具系统在**执行引擎**上最成熟——流式执行器（工具到达即执行）、投机分类器（安全检查与执行并行）、6 层权限纵深防御。OC 在**工具种类丰富度**上领先（browser/canvas/image_generate/pdf 视觉模式均为自研核心工具），并在 v2.1 独家引入**异步 Tool 执行（fire-and-forget）** —— 模型可把长任务 detach 成后台 job，对话立即继续，结果通过 mailbox 注入回主对话，CC 和 OW 均无对标物。OC 执行引擎仍缺流式执行和投机分类器。OW v2.1 追平了 `apply_patch`（gated），执行引擎仍最简，但 approval 机制和 scopeKey 隔离是 OC 可借鉴的。

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
| **Delegate 代理 Agent** | 无 | 无 | **组织内代理模式**：命名 Agent 持有独立凭证 + 显式权限层级（read-only / send-on-behalf / proactive）+ 硬约束 block + 工具 allowlist（在 SOUL.md / AGENTS.md 声明） |

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

CC 的 Team/Swarm 能力是三者中最强的——Coordinator 模式、双向消息、Git Worktree 隔离形成了完整的多 Agent 协作栈。OC 的子 Agent 系统在单 Agent 场景下功能完善（深度感知、Mailbox 回传、前台/后台自动切换，v2.1 新增 `SubagentGroup` 批量聚合展示 + 批量 hydration），但仍缺乏多 Agent 协作能力。OW v2.1 新增 **Delegate Architecture** 构建了"组织内代理 Agent"的语义层——每个代理有独立凭证和硬约束权限，配合 SOUL.md 人格文件，指向企业/多用户场景。OW 的 Session 模型最灵活（多 Agent 路由 + 渠道映射 + delegate 身份），但子 Agent 嵌套能力仍最弱。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| Agent 协作 | 3 | **5** | 2 |
| Session 管理 | 4 | **5** | 4 |

---

## 第 5 章：记忆与上下文管理

### 5.1 记忆系统

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 存储后端 | **SQLite + FTS5 + vec0 向量扩展** | 文件系统（CLAUDE.md + memdir） | **可插拔 Memory Backend**：builtin 文件 + **Honcho**（AI-native 跨会话+profiling）+ **QMD**（BM25+向量+reranking 本地 sidecar）+ **LanceDB**（支持 cloud 存储）+ memory-core 插件 |
| 记忆类型 | 4 种（facts/preferences/instructions/context）+ 2 种作用域 | user_context/project_notes/team_notes/memories | Honcho 自动 user/agent profiling；QMD 按文件分类 |
| 语义搜索 | **向量相似度 + FTS 混合 + MMR 多样性** | 无（文件扫描） | **BM25 + 向量 + reranking**（QMD）+ Honcho semantic search |
| 全文搜索 | **FTS5** | 无 | QMD BM25 |
| 自动提取 | **阈值双层触发**（冷却 5min AND (8K token OR 10 条消息)）+ 空闲超时兜底 + inline 执行复用 side_query 缓存 | 自动 memory extraction hooks | Honcho 自动 profiling |
| LLM 语义选择 | 候选 > 8 时 side_query 选择 ≤5 条注入 | 无 | 无 |
| **Active Memory（pre-reply 阻塞注入）** | 无 | 无 | **阻塞式记忆子 Agent**：主回复前先跑一轮 recall 把相关记忆以 system prompt 补丁形式注入，支持 query 模式/prompt 风格/超时配置 |
| **Dreaming（离线记忆固化）** | 无 | 无 | **三阶段**（light/deep/REM）后台固化 + **Dream Diary**（人类可读）：摄取信号 → 打分 → 高置信度候选晋升到 `MEMORY.md`，短期 staging 与长期记忆解耦 |
| 作用域 | Global + Agent | Per-project + user-level | Per-workspace + MEMORY.md + delegate 作用域 |
| Team 记忆同步 | 无 | Team memory sync（共享 Agent 定义） | Honcho 多 Agent 感知 |
| 记忆老化 | 无 | Age tracking + freshness notes | Dreaming 冷热分层 |
| Embedding 提供者 | **8 种**（OpenAI/Google/Jina/Cohere/SiliconFlow/Voyage/Mistral/Local ONNX） | 无（不用 embedding） | OpenAI-compat + **GitHub Copilot**（v2.1 新增，复用 Copilot host helper 含 token refresh） |
| Pin 置顶 | 无 | Pinned cache edits（protect 策略工具不清除） | 无 |
| **SOUL.md 人格文件** | 无 | 无 | **与 AGENTS.md 分离**：专门承载 voice/tone/opinions/行为风格，普通会话注入，不污染操作指令 |

### 5.2 上下文管理

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 压缩层级 | **5 层**（Tier 0 微压缩 → Tier 1 截断 → Tier 2 修剪 → Tier 3 LLM 摘要 → Tier 4 紧急）+ Cache-TTL 节流（默认 300s 冷却保护 prompt cache） | 3 层（microcompact + snipCompact + context collapse，部分 feature-gated） | **Context Engine 可插拔**（ingest/assemble/compact/after-turn 生命周期 hook）+ 会话 pruning（缓存友好的老 tool result 修剪）+ **可插拔 Compaction Provider**（`registerCompactionProvider()`，支持自定义摘要后端，失败 fallback LLM）+ 后台 idle maintenance |
| **Context Engine 架构** | 固定 `context_compact` 管线 | 固定内置管线 | **可插拔 `registerContextEngine()`**：第三方插件可注册自定义 context 装配/压缩策略，resolution 失败优雅回退 legacy engine |
| API-Round 分组 | `_oc_round` 元数据标记，确保 tool_use/tool_result 不被拆散 | 无（按消息级压缩） | 无 |
| 后压缩文件恢复 | Tier 3 后自动注入最近编辑文件内容（5 文件 × 16KB） | 无 | 无 |
| Side Query 缓存 | 复用 prompt 前缀，侧查询成本降低 **90%** | 无 | 无 |
| Reactive Compact | 无 | Token 预警→自动触发压缩 | Context Engine `afterTurn` hook 驱动 |
| Tool Use Summary | 无 | 工具调用结果摘要（减少上下文占用） | 无 |
| Token 估算 | 动态估算 + `TokenEstimateCalibrator` 学习 | Anthropic API 精确 + 估算 | 透传上游 + 上下文 reserve 上限保护小窗口本地模型 |

**关键差异**

OC 的记忆系统在**存储与检索机制**上仍最深——SQLite + FTS5 + 向量检索 + 8 种 Embedding 提供者 + LLM 语义选择，形成工业级的长期记忆能力。但 OW v2.1 在**记忆语义/生命周期**层面快速追上——**Honcho 自动 profiling、QMD 本地 sidecar（BM25+向量+reranking）、Dreaming 离线记忆固化（light/deep/REM）、Active Memory pre-reply 阻塞注入、SOUL.md 人格分离**——这是一整套"主动式记忆"语义，OC 的记忆还停留在"被动召回 + 自动提取"模式。OC 的上下文管理深度仍领先（5 层渐进压缩 + API-Round 分组 + 后压缩文件恢复 + Side Query 缓存 + Cache-TTL 节流），但 OW v2.1 引入的**Context Engine 可插拔架构**在**可扩展性**上反超——第三方插件可替换整条装配/压缩管线，OC 还是固定管线，重构到 trait 是 v2.1 路线图的 Phase A 优先级。CC 的 Reactive Compact 和 Tool Use Summary 仍是 OC 的 backlog。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 记忆系统 | **5** | 4 | 4 ↑ |
| 上下文管理 | **5** | 4 | 3 ↑ |

---

## 第 6 章：规划与工作流

| 维度 | OC | CC | OW |
|------|:---|:---|:---|
| 状态机 | **六态**（Off/Planning/Review/Executing/Paused/Completed） | 二态（Normal/Plan） | 无 |
| 双 Agent 分离 | 支持（`plan_subagent: true` 隔离计划探索） | 无（Plan Mode 仅限制工具） | 无 |
| 执行层权限 | `plan_mode_allowed_tools` 白名单 + schema 过滤双层防护 | 工具子集限制（只读） | 无 |
| 步骤追踪 | `update_plan_step`（in_progress/completed/failed/skipped） | `TodoWriteTool`（pending/in_progress/completed） | 无 |
| Git Checkpoint | 执行前自动 checkpoint | 无 | 无 |
| 交互式问答 | **`ask_user_question` 通用化**（非 Plan 场景也可用，1–4 题 / 单选/多选/自定义/预览 markdown·image·mermaid / header chip / 单题超时 + default_values 自动回退 / 持久化 + 重启重放 / IM 按钮 + 文本降级） | 无 | 无 |
| 暂停/恢复 | Paused 状态 | 无 | 无 |
| 计划修改 | `amend_plan`（执行中插入/删除/更新步骤） | 无 | 无 |
| Plan 文件持久化 | `~/.opencomputer/plans/` | 文件系统（plan file） | 无 |
| **会话级 Task 工具** | **`task_create` / `task_update` / `task_list`**（独立于 Plan Mode 的轻量 todo，任意对话可用） | `TodoWriteTool`（类似能力但不分 session/plan 双轨） | 无 |
| **`/recap` 深度复盘** | **独立 `recap.db` + 11 并行 AI 章节**（含 agent_tool_optimization / memory_skill_recommendations / cost_optimization 三个 OC 特有章节）+ HTML 自包含导出 + facet 缓存 | 无 | 无 |
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
| 权限模式数 | 3（auto/ask/deny）+ **审批超时策略可配**（deny/proceed） | **6**（default/acceptEdits/plan/bypass/dontAsk/auto） | 3（approval/sandbox/open） |
| 分类器预检 | 无 | Bash 分类器 + yolo 安全兜底 | 无 |
| Hooks 系统 | 无 | **8 种事件**（PreToolUse/PostToolUse/pre-compact/post-compact/session-start 等） | 无（但 gateway 有丰富的 allowlist/policy hook 点） |
| Docker 沙箱 | `exec --sandbox`（容器隔离） | macOS 原生沙箱检测 | opt-in sandbox 模式 + 每 Agent scope 隔离 |
| 组织策略/MDM | 无 | Windows Registry + macOS plutil + managed-settings.d | 无 |
| 路径限制 | Tauri CSP scope 限制 | 设备路径阻止（/dev/*） + scratchpad 隔离 | **workspace fs-safe**：`openFileWithinRoot` / FD-based realpath（防 open 与 realpath 之间的 symlink swap 攻击）+ 符号链接别名拒绝 |
| **SSRF 纵深防御** | `web_fetch` 基础 SSRF 防护 | 无（CLI 直连官方 API） | **默认 strict 模式** + snapshot/screenshot/tab 全路径策略 + 三阶段交互导航守护 + per-provider `allowPrivateNetwork` allowlist + 代理模式 DNS 解析 trusted-env 收敛 |
| macOS TCC 权限 | **15 种检测**（Accessibility/Screen/Automation/Disk/Location/Camera/Mic 等） | 无 | 无 |
| API Key 脱敏 | 首 4 + 末 4 字符 + 日志自动 redact | 无（不存储第三方 key） | **SecretRef + inspect-vs-strict 分层** + 快照 redaction + **exec approval 提示 redact** + **constant-time `safeEqualSecret`** 比较（所有 auth 门） |
| DM 配对安全 | 无（桌面应用无需） | 无（CLI 无需） | **pairing/open/closed** 三策略 + allowlist + Slack block-action cross-verification + Feishu webhook 强制 `encryptKey` 拒未签名请求 |
| 设备认证 | 无 | JWT（IDE Bridge） | 设备指纹 + 公钥签名 + 角色作用域 + Android 存储 device auth 优先 + owner downgrade for untrusted `hook:wake` |
| 拒绝追踪降级 | 无 | 连续拒绝自动降级权限模式 | 无 |
| 规则来源层级 | 1（session）+ 全局 `toolPolicies` 三态 | **7 层**（policy > flag > project > local > user > session > cliArg） | 2（config + env） |
| 中断行为控制 | 无 | 每工具 `interruptBehavior`（cancel/block） | 无 |
| **模型面配置提权阻断** | 无 | 无 | gateway-tool `config.patch`/`config.apply` 拒绝 enable 审计敏感 flag（`dangerouslyDisable*`, `allowInsecureAuth` 等） |

**关键差异**

CC 的权限系统在**应用层**仍最深——6 种模式、7 层规则来源、Hooks 系统、分类器预检、拒绝追踪降级形成完整纵深防御。但 OW v2.1 在**网关/基础设施层**密集加固后**全面追平**：SSRF 默认 strict 模式覆盖 browser 全路径、FD-based realpath 防 TOCTOU symlink swap、constant-time secret 比较、exec approval 提示 redact、Feishu webhook 强制 encryptKey、模型面配置提权阻断、untrusted wake 事件的 owner downgrade——这些都是生产级网关不可或缺的。OC 在 macOS TCC 权限检测（15 种）上独有，在网关层的 SSRF/realpath/constant-time 防护上全面落后，**Hooks 系统**和**多层权限模式**仍是主要 gap。OW 的 DM 配对安全在多渠道场景下保持领先。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 权限安全 | 3 | **5** | **5** ↑ |

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
| Dashboard 维度 | **8 + Insights**（Token/Cost/Session/Error/Cron/Subagent/Metrics/Model breakdown + **综合 Insights 面板**） | 1（`/cost` 简表） | 1（`/healthz`）+ 控制 UI **Model Auth Status 卡**（OAuth token 健康 + provider rate-limit pressure） |
| **综合 Insights 查询** | **`query_insights` orchestrator**：健康度 0–100（四维加权）+ 7×24 活跃度热力图 + 0–23 时消息分布 + Top N 会话 + 每模型效率对比（tokens/msg、cost/1k、TTFT）+ 日度费用累计 + 峰值/日均 | 无 | 无 |
| **同比 Delta** | Overview Cards 9 张关键指标卡"较上期"百分比徽章，左移相同时间跨度取 previous baseline | 无 | 无 |
| **自动刷新 + CSV 导出** | 大盘 Header 自动刷新（30s/1m/5m）+ 按 Tab 导出 tokens/tools/sessions/errors/insights 为 CSV | 无 | 无 |
| System 实时资源 | **CPU/内存环形缓冲曲线**（60 个采样点，客户端绘制，省去后端历史库） | 无 | 无 |
| TTFT 指标 | 记录并展示，Insights 按模型对比 | 无 | 无 |
| 错误分类 | 5 类（RateLimit/Overloaded/Timeout/Auth/ContextOverflow） | 按 HTTP 状态码 | **细分 failover reason**（`network_error`→timeout、`no error details`→unknown、billing cooldown、session_expired 分类） |
| 日志脱敏 | API Key + Token 自动 redact + 请求体截断 32KB | 无（不存储敏感数据） | SecretRef redaction + alias 字段 redact |
| 日志后端 | **SQLite + 纯文本双写**（非阻塞） | 文件系统 + OpenTelemetry | 文件系统 |
| 进程指标 | 内存/CPU/运行时 | 无 | 无 |

**关键差异**

OC 的可观测性 v2.1 进一步扩大领先——**综合 Insights 面板**（健康度 0–100 四维加权 + 活跃度热力图 + 模型效率对比 + 日度费用趋势 + Top N 会话）、**同比 Delta 徽章**、**自动刷新 + CSV 导出**、**System 实时 CPU/内存曲线**形成了完整的自助分析闭环，日志系统支持 SQLite + 纯文本双写和自动脱敏。CC 有 OpenTelemetry 集成但 Dashboard 简单。OW v2.1 新增 Model Auth Status 卡和 failover reason 细分，但仍无数据大盘。

| 评分 | OC | CC | OW |
|------|:--:|:--:|:--:|
| 可观测性 | **5** | 3 | 2 |

---

## 第 15.5 章：自我学习与进化能力（v2.2 新增维度）

> 这是 Hermes Agent 加入对比后新增的维度。HA 在这个维度上**开创性领先**，OC/CC/OW 在此维度的得分反映了各自在"Agent 自我改进"方向上的现有积累和潜力。

### 15.5.1 能力矩阵

| 维度 | OC | CC | OW | HA |
|------|:---|:---|:---|:---|
| **自主 Skill 创建** | 无 | 无 | 无 | **后台 daemon 线程自动分析对话**（`_spawn_background_review()`），每 N 轮（默认 10）nudge 模型决定是否创建新 Skill 或修补旧 Skill。Skill 存储为 YAML + Markdown，自动成为 slash 命令 |
| **Skill 使用中修补** | 无 | 无 | 无 | **`_patch_skill()` 模糊匹配替换**——Skill 执行失败时当场修补（不需精确匹配文本），下次调用即生效 |
| **记忆 nudging（主动提取）** | 自动提取（阈值触发，inline） | 自动 memory extraction hooks | 无 | **后台 daemon 定期 nudge 模型**（`_MEMORY_REVIEW_PROMPT`）提取用户偏好到 `USER.md` + 环境知识到 `MEMORY.md`。冻结快照模式保护 prompt cache |
| **跨会话知识召回** | **FTS5 全文搜索 + Find-in-Chat** | `searchSessionsByCustomTitle`（仅标题） | 按 key 过滤 | **FTS5 语义搜索 + 廉价辅助模型（Gemini Flash）摘要**——搜索命中后截取 ~100K chars 送辅助模型摘要，返回按相关性排序的 per-session 总结 |
| **辩证用户建模** | 无 | 无 | Honcho 可插拔 | **Honcho 辩证推理**（optional）——两个 "peer" 交叉观察用户/AI 消息，多轮辩证 Q&A 构建深层用户理解，复杂度自动升级 |
| **反馈闭环** | 无 | 无 | 无 | Skill review + memory review → 下一轮对话受益 → 更好的结果 → 更多高质量 skill/memory → **滚雪球** |
| **轨迹保存（RL 训练）** | 无 | 无 | 无 | extended thinking 轨迹 + 批量生成 + Atropos 环境集成，面向 **RL 研究** |
| **Skill Hub 远程分发** | 无（用户本地创建） | Bundled skills（30+） | ClawHub marketplace | **agentskills.io** 中心化 Hub + Agent 可自主发现并提议安装 |
| **安全扫描** | 无 | 无 | 无 | Memory 内容注入前扫描 prompt injection / invisible unicode / credential 泄漏；Skill 创建时安全验证 |

### 15.5.2 核心机制：HA 的闭环学习架构

```
用户对话 → Agent 完成回复
                          ↘
                   _spawn_background_review() — 后台 daemon 线程
                          ├── Skill Review Prompt → 分析是否有可复用模式
                          │     ├── Yes → create_skill() → ~/.hermes/skills/new_skill.md
                          │     └── 已有类似 → patch_skill() 模糊修补
                          └── Memory Review Prompt → 提取用户洞察
                                ├── 环境知识 → MEMORY.md（§ 分隔条目）
                                └── 用户画像 → USER.md（§ 分隔条目）
                                
下一次会话 →
  ├── 系统提示注入 MEMORY.md + USER.md 冻结快照
  ├── 新 Skill 可用（slash 命令 / 自动发现）
  └── session_search 跨会话 FTS5 召回 → 辅助模型摘要
```

**关键设计选择**：
1. **后台 daemon（非 inline）**：review 线程不阻塞用户对话，零延迟感知
2. **冻结快照**：记忆只在会话开始时注入系统提示，mid-session 写入不改变 prompt，保护 Anthropic 前缀缓存
3. **模糊匹配修补**：`_patch_skill()` 不要求精确文本匹配，实际使用中 Skill 修补成功率远高于 exact-match
4. **廉价辅助模型**：跨会话摘要用 Gemini Flash 而非主模型，成本极低

### 15.5.3 OC 的差距与追齐路径

OC 在自我学习维度**完全缺失**：
- 无自主 Skill 创建/修补
- 无记忆 nudging（有自动提取但不是"反省+学习"语义）
- 无跨会话知识召回摘要（有 FTS5 搜索但无 LLM 摘要整合）
- 无辩证用户建模
- 无反馈闭环

OC 拥有追齐的**基础设施优势**：
- `side_query()` 缓存——Active Memory 和 Skill Review 都可以低成本复用
- SKILL.md 系统——已有 frontmatter + Markdown 格式，创建/修补工具链到位
- `async_jobs` 后台执行——可直接承载 background review
- FTS5 会话搜索——已有，缺的是 LLM 摘要层
- 8 种 Embedding 提供者——辩证用户建模可以接入向量检索

追齐计划已纳入路线图 [`roadmap-2026q2.md`](./roadmap-2026q2.md) 的 **Phase B'**（自我学习闭环）。

| 评分 | OC | CC | OW | HA |
|------|:--:|:--:|:--:|:--:|
| 自我学习 | 1 | 2 | 2 | **5** |

---

## 第 16 章：综合评分矩阵

| 能力维度 | OC | CC | OW | HA | 领先者 | v2.2 变化 |
|----------|:--:|:--:|:--:|:--:|:------:|:---------|
| Provider 支持 | **5** | 2 | 3 | 4 | OC | HA 200+ via OpenRouter 但无模板/降级链 |
| 工具系统 | 4 | **5** | 3 | 3 | CC | OC 独家异步 Tool；HA 40+ 工具 + MCP |
| Agent 协作 | 3 | **5** | 2 | 2 | CC | HA 有 subagent delegation 但无 team/swarm |
| Session 管理 | 4 | **5** | 4 | 3 | CC | HA SQLite + FTS5 但无会话恢复深度 |
| 记忆系统 | **5** | 4 | **4** ↑ | **4** | OC | HA MEMORY.md+USER.md+Honcho 辩证；OW Honcho/QMD/Dreaming |
| 上下文管理 | **5** | 4 | 3 ↑ | 2 | OC | HA 仅自适应压缩+降级警告 |
| Plan Mode | **5** | 3 | 1 | 1 | OC | HA 无 Plan Mode |
| Skill 系统 | 4 | **5** | 3 | 4 | CC | HA Skill 创建/修补/Hub 强，但无工具隔离/Fork |
| 协议支持 | 2 | **4** | **4** | 3 | CC/OW | HA 有 MCP client 但无 ACP/OpenAI-compat |
| 权限安全 | 3 | **5** | **5** ↑ | 3 | CC/OW | HA 有 approval + 注入扫描但无纵深 |
| 多模态 | **5** | 3 | 3 | 2 | OC | HA 有 vision + 图片生成但无 PDF 三模式/Canvas |
| 渠道集成 | 4 | 0 | **5** | 4 | OW | HA 16 平台（含 Email/SMS/DingTalk/WeCom） |
| IDE 集成 | 3 | **5** | 1 | 1 | CC | HA 无 IDE 集成 |
| CLI/UX | **5** | 4 | 4 | 4 | OC | HA Rich TUI 体验好但无 GUI/i18n |
| 部署 | 4 | 4 | 4 | **5** | HA | HA 极端灵活（VPS/Docker/SSH/Modal/Daytona/Termux） |
| 可观测性 | **5** | 3 | 2 | 2 | OC | HA 有 session JSON 日志但无 Dashboard |
| **自我学习** | 1 | 2 | 2 | **5** | **HA** | **v2.2 新增维度**：HA 闭环自进化（Skill 自创/修补 + nudging + 跨会话召回 + 辩证建模） |
| **合计** | **67/85** | **63/85** | **53/85** | **52/85** | **OC** | 新增自我学习维度（满分 85），各项目均加该维度分数 |

### 各项目优势象限

- **OC 领先**（6/17）：Provider 支持、上下文管理、Plan Mode、多模态、CLI/UX、可观测性
- **CC 领先**（5/17）：工具系统、Agent 协作、Session 管理、IDE 集成、Skill 系统
- **HA 领先**（2/17）：**自我学习**、部署灵活性
- **OW 领先**（1/17）：渠道集成
- **CC/OW 并列**（2）：权限安全、协议支持
- **OC/OW/HA 并列**（1）：记忆系统（OC 5 vs OW/HA 4，OC 仍领先但 gap 缩小）

### 四项目核心差异化（v2.2）

| 项目 | 核心竞争力 | 薄弱环节 |
|------|-----------|----------|
| OC | 多 Provider 兼容 + 深度上下文管理 + 完整 Plan Mode + GUI 体验 + **异步 Tool fire-and-forget** + **Insights 可观测闭环** + **三 crate workspace（GUI/Server/ACP 三模式）** | MCP 缺失、Hooks 缺失、多 Agent 协作弱、**自我学习完全缺失**、Context Engine 未插件化、Active Memory/Dreaming 缺失、Auth Profile 轮换缺失、网关层安全硬化缺失 |
| CC | 工具执行引擎 + 多层权限 + IDE 深度集成 + MCP 生态 | 单一 Provider、无 GUI、无渠道集成、自我学习弱 |
| OW | 25+ 渠道覆盖 + OpenAI 兼容 API + **Context Engine 可插拔** + **Honcho/QMD/Dreaming 记忆语义** + **Delegate 组织模式** + **生产级网关安全** | 无 Plan、上下文压缩深度弱、无 IDE、无数据大盘、自我学习弱 |
| HA | **闭环自我学习**（Skill 自创/修补 + nudging + 跨会话召回 + 辩证建模）+ 200+ 模型 + 极端部署灵活性 + RL 研究集成 | 无 Plan Mode、上下文管理弱、无 GUI、无降级链、无 Dashboard、安全性基础 |

---

## 第 17 章：OC 独有优势总结（v2.2）

| 优势领域 | 具体表现 | 竞争影响 |
|----------|---------|---------|
| **异步 Tool fire-and-forget** | `run_in_background` + 独立 `async_jobs.db` + spool 文件 + 三道决策（模型显式/Agent 策略/同步预算自动后台化 30s）+ OS 线程相位迁移 + `job_status(block?)` deferred 工具 + retention 清理 | **CC/OW 均无**，长任务不再卡主对话 |
| **`/recap` 深度复盘** | 独立 `recap.db` + 11 并行 AI 章节（含 agent_tool_optimization / memory_skill_recommendations / cost_optimization 三个 OC 特有章节）+ HTML 自包含导出 | CC/OW 无 |
| **Dashboard 综合 Insights** | 健康度 0–100 四维加权 + 7×24 活跃度热力图 + 模型效率对比 + 日度费用趋势 + Top N 会话 + 同比 Delta + 自动刷新 + CSV 导出 | CC 仅 `/cost` 简表，OW 仅 Model Auth Status 卡 |
| **会话级 Task 工具** | `task_create` / `task_update` / `task_list` 任意对话可用，独立于 Plan Mode | CC `TodoWriteTool` 类似但无会话/plan 双轨 |
| **通用 `ask_user_question`** | 非 Plan 场景可用，1–4 题 / preview markdown·image·mermaid / 单题超时 + default_values / 持久化重放 / IM 按钮 + 文本降级 | CC/OW 无 |
| **跨会话 FTS5 全文搜索 + Find-in-Chat** | 侧边栏全局搜索 + 单会话浮条，高亮跳转 + `load_session_messages_around_cmd` | CC `searchSessionsByCustomTitle`（仅标题），OW 按 key |
| **`SubagentGroup` 批量聚合** | 多个并发子 Agent 合并展示 + 批量 hydration（N=10 从 10 次 IPC 降到 1 次） | CC/OW 无 |
| **pending-interaction 会话指示器** | 会话列表对未回复的 approval / ask_user 显示琥珀底色 + BellRing 徽章，EventBus 广播 | CC/OW 无 |
| **三 crate workspace + 三模式运行** | `oc-core`（零 Tauri 依赖核心）+ `oc-server`（axum HTTP/WS 守护进程 + 系统服务注册）+ `src-tauri`（桌面薄壳），前端 Transport 抽象双实现 | CC 仅 CLI + IDE Bridge，OW 仅 gateway |
| Side Query 缓存 | 复用 prompt 前缀，侧查询成本降低 90% | CC/OW 无此优化 |
| 5 层上下文压缩 + Cache-TTL 节流 | Tier 0-4 渐进式，API-Round 分组保护 tool_use/tool_result 配对，TTL 300s 冷却保护 prompt cache | CC 仅 3 层（部分 feature-gated），OW 可插拔架构但压缩层级浅 |
| 后压缩文件恢复 | Tier 3 后自动注入最近编辑文件内容（5 文件 × 16KB） | CC/OW 无 |
| Plan Mode 六态状态机 | 双 Agent 分离 + 执行层权限 + Git Checkpoint + 暂停/恢复 + 执行中修改 | CC 仅二态，OW 无 |
| 28 Provider 模板 | 108 预设模型，GUI 一键配置 | CC 锁定 Anthropic，OW bundled 扩展中无统一模板 |
| 图片直达模型 | 多图 10 张 + URL + 剪贴板 + 截屏，原始视觉数据不经转述 | OW 图片转文字描述丢失细节 |
| PDF 三模式 | auto/text/vision + 扫描件智能检测 + URL + 多 PDF | CC 仅文本，OW 仅部分 Provider 原生 |
| 8 种 Embedding 提供者 | OpenAI/Google/Jina/Cohere/SiliconFlow/Voyage/Mistral/Local ONNX | CC 不用 embedding，OW v2.1 新增 Copilot |
| LLM 记忆语义选择 | 候选 > 8 时 side_query 精选 ≤5 条注入 | CC/OW 无 |
| WeChat 深度集成 | AES-128-ECB 加密/解密 + QR 登录 + Typing 24h TTL | OW 的 WeChat 是外部 npm 包 |
| macOS TCC 15 种权限检测 | Accessibility/Screen/Automation/Disk/Location/Camera/Mic 等 | CC/OW 无 |
| 12 语言 i18n 全量 | 1100+ keys × 12 locale | CC/OW 仅英文 |

---

## 第 18 章：Actionable 差距清单（OC 待追项 · v2.2）

> **v2.2 优先级变化**：Hermes Agent 加入对比后，**自我学习与进化**成为 OC 最大的 gap（评分 1 vs HA 5）。好消息是 OC 已有 side_query 缓存、SKILL.md、async_jobs、FTS5 搜索等基础设施，追齐路径清晰。同时 OW 在 Context Engine / Active Memory / Dreaming / 网关安全方向的突破仍需追。完整执行计划见 [`roadmap-2026q2.md`](./roadmap-2026q2.md)。

### P0 — 架构级追齐（Phase A）

| 缺失能力 | 参考来源 | 影响分析 | 估计复杂度 |
|----------|---------|---------|-----------|
| **Context Engine trait 抽象** | OW `registerContextEngine()`（ingest/assemble/compact/after-turn 生命周期） | OC 的 `context_compact` 是固定管线，无法被第三方替换；Active Memory/Dreaming/可插拔 Compaction 都依赖这个 trait 存在 | 中高（重构现有 5 层压缩到 trait 后端，保持默认实现行为不变） |
| **可插拔 Compaction Provider** | OW `registerCompactionProvider()`（支持外部 LLM/微调摘要模型，失败回退内置 LLM） | 压缩摘要当前硬绑定主对话模型，无法用更便宜/更快的专用模型 | 低（依赖 Context Engine trait） |
| **Auth Profile 轮换 failover** | OW 两级 failover（先同 provider 换 key → 再跳模型） | OC 失败转移只在模型级工作，同一 provider 多 key 无法自动轮询，rate limit 场景下体验差 | 中（复用现有 5 类错误分类，在 failover 路径前插一级 profile 迭代） |
| **网关 SSRF 纵深防御** | OW browser 默认 strict + snapshot/screenshot/tab 全路径 + per-provider `allowPrivateNetwork` allowlist | OC 只有 `web_fetch` 基础 SSRF，`browser` 工具可被用于探测内网 | 中 |
| **workspace fs-safe 符号链接防护** | OW `openFileWithinRoot` + FD-based realpath（防 open/realpath TOCTOU swap） | OC `read/write/edit` 工具在 sandbox root 外可被 symlink 攻击引出 | 中 |
| **constant-time secret 比较** | OW `safeEqualSecret`（所有 auth 门统一用） | OC 的 API Key 鉴权中间件用普通字符串比较，理论上有侧信道风险 | 低 |

### P1 — 自我学习闭环 + 记忆语义升级（Phase B / B'）

> **v2.2 新增**：HA 的闭环自我学习是 OC 最大 gap。OC 已有 side_query + SKILL.md + async_jobs + FTS5 四块基础设施，追齐成本远低于从零开始。

| 缺失能力 | 参考来源 | 影响分析 | 估计复杂度 |
|----------|---------|---------|-----------|
| **自主 Skill 创建/修补** ⭐ | HA `_spawn_background_review()` + `skill_manager_tool.py`（后台 daemon 分析对话 → 创建 Skill 或模糊修补） | **OC 的 SKILL.md 系统只能人工创建**，无法从使用中自我进化。HA 的"用得越多越聪明"叙事完全依赖此能力 | 中（复用 async_jobs 后台执行 + 现有 SKILL.md CRUD，新增"对话分析→是否创建 Skill"提示词 + 模糊匹配修补逻辑） |
| **记忆 nudging（反省式学习）** | HA `_MEMORY_REVIEW_PROMPT` 后台 daemon 定期提取用户偏好到 `USER.md` + 环境知识到 `MEMORY.md` | OC 的自动提取是"提取事实"，不是"反省学到了什么"。HA 的 nudging 专门问"用户有什么偏好/期望/工作习惯？" | 低（在现有自动提取 side_query 路径里新增一个反省提示词 + USER 类型记忆分离） |
| **跨会话知识召回 + LLM 摘要** | HA `session_search_tool.py`（FTS5 搜索 + Gemini Flash 摘要） | OC 已有 FTS5 搜索但结果是原始片段，无 LLM 摘要整合。HA 的做法是截取命中上下文 → 廉价模型摘要 → 返回结构化总结 | 低（在现有 `search_messages` 结果上叠加 side_query 摘要层） |
| **Active Memory pre-reply 注入** | OW 阻塞式记忆子 Agent + HA 跨会话召回 | OC 当前只做被动召回（系统提示注入）+ 自动提取，没有"主动查相关记忆再回复"的语义 | 中（复用现有 side_query 缓存，零额外成本） |
| **SOUL.md 人格文件** | OW + HA `USER.md`（用户画像分离） | OC 的 Agent 人格和操作指令混在一起，难以独立管理 | 低 |
| **Dreaming 离线记忆固化（light 阶段先行）** | OW 三阶段 light/deep/REM + Dream Diary | OC 自动提取是"在线 inline"，没有离线深加工路径，候选晋升不分冷热 | 中 |
| **Memory Backend plugin 接口** | OW Honcho/QMD/LanceDB 多后端 | 为将来接入 Honcho/QMD 或用户自定义后端预留抽象 | 中 |
| **Reactive Compact** | CC Token 预警自动触发 | Context Engine trait 到位后顺手实现 | 低 |

### P2 — 多 Agent 协作与协议追齐（Phase C）

| 缺失能力 | 参考来源 | 影响分析 | 估计复杂度 |
|----------|---------|---------|-----------|
| **MCP 协议支持** | CC 原生 MCP Client（传输层 + RPC + 工具代理 + 资源访问 + OAuth + 注册表） | 无法接入 MCP 生态，推迟到 Phase C 是权衡——先把 Context Engine 和网关安全打好，MCP 作为最后一块插件系统引入 | 高 |
| **Hooks 系统** | CC 8 种事件 + settings.json 声明式 | EventBus 已有基础，定义 Hook 接口 + 配置 schema + 执行管线即可 | 中 |
| **Team/Swarm Agent + Coordinator** | CC `TeamCreateTool` + Coordinator 模式 | 无法多 Agent 协作（写代码/写测试/review 分工） | 高 |
| **Git Worktree 隔离** | CC `EnterWorktreeTool` / `ExitWorktreeTool` | 子 Agent 无法在独立分支工作 | 中 |
| **Agent 间双向实时消息** | CC `SendMessageTool` | `sessions_send` 单向投递 → 扩双向 | 中 |
| **MCP Skills 桥接** | CC MCP 服务器→Skill 格式 | 依赖 MCP 完成 | 中 |

### P3 — 体验与生态补足（Phase D）

| 缺失能力 | 参考来源 | 估计复杂度 |
|----------|---------|-----------|
| 流式工具执行 + 投机分类器 | CC `StreamingToolExecutor` | 中 |
| 多权限模式（acceptEdits/bypass/dontAsk） | CC 6 模式 | 中 |
| 7 层规则来源（policy > flag > project > local > user > session > cliArg） | CC | 中 |
| LSP 工具 + REPL Bridge | CC `LSPTool` / `replBridge.ts` | 中 |
| `read` context window 自适应 | OW | 低 |
| 语音输入/输出（STT + TTS） | OW Deepgram + ElevenLabs | 高 |
| localModelLean 弱本地模型模式 | OW `agents.defaults.experimental.localModelLean` | 低 |
| Provider 补齐（LM Studio / Arcee / Copilot embedding） | OW bundled | 低 |
| 拒绝追踪降级 | CC | 低 |
| 中断行为控制（cancel/block per tool） | CC | 低 |
| Pin 置顶记忆 | CC protect 策略 | 低 |
| 记忆老化 + freshness notes | CC | 低 |
| Effort 级别（fast/balanced/thorough） | CC Skill | 低 |
| 热配置重载（hybrid mode） | OW | 中 |
| 自动更新 | CC `autoUpdater` | 中 |
| OpenAI 兼容 HTTP API 端点 | OW `/v1/*` | 中 |

---

## 第 19 章：演进路线图

详见独立文档 [`roadmap-2026q2.md`](./roadmap-2026q2.md)，分为 Phase A（架构补课）→ Phase B（记忆升级）→ **Phase B'（自我学习闭环）** → Phase C（多 Agent 与 MCP）→ Phase D（体验与生态），总计约 24–30 周。

关键判断：
1. **Context Engine trait 最值得先做**——A1/A2/B1/B4 全系列能力都依赖这个 trait，越晚做越贵
2. **自我学习闭环是 OC 最大 gap（v2.2 新增）**——HA 在此维度评分 5 vs OC 1，但 OC 已有 side_query + SKILL.md + async_jobs + FTS5 四块基础设施，追齐路径清晰且成本可控。Phase B' 是追齐 HA 的专项阶段
3. **MCP 推到 Phase C 而不是 P0**——投入大（Client SDK + OAuth + 资源协议），先把 Context Engine、自我学习、网关安全打稳
4. **Phase A 的网关安全硬化不能跳过**——OC 的 `opencomputer server` 已进入生产运行路径（launchd/systemd），SSRF/realpath/constant-time 这些是基础要求

---

> 本文档取代原 `competitive-analysis.md` + 14 份子文档。旧文档已在 git 历史中保留。

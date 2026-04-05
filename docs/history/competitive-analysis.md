# OpenComputer vs Claude Code vs OpenClaw — 全面能力对比报告

> 生成日期：2026-04-04
>
> 本报告对 OpenComputer、Claude Code（Anthropic 官方 CLI）、OpenClaw（多渠道 AI 网关平台）三个项目进行全面能力对比，识别各自优势与差距，为后续演进提供决策依据。

---

## 一、项目定位与技术栈

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| **定位** | 本地 AI 助手桌面应用（GUI） | 官方 CLI 编程助手 | 多渠道 AI 网关平台 |
| **核心语言** | Rust + TypeScript | TypeScript（Bun 编译） | TypeScript（Node.js） |
| **UI 形态** | Tauri 2 桌面 GUI（React 19） | 终端 TUI（React + Ink） | Web 控制台 + 多端原生 App |
| **用户模型** | 单用户桌面 | 单用户 CLI | 单用户网关（多渠道汇聚） |
| **代码规模** | ~50 Rust 模块 + 132 TSX 组件 | ~1,900 文件 / 512K+ 行 | ~55 模块 / 48MB+ |
| **构建工具** | Vite 8 + Cargo | Bun | Node.js |
| **UI 框架** | Tailwind CSS v4 + shadcn/ui (Radix) | Ink (终端 React) | React Web |
| **数据存储** | SQLite（多库）+ 文件系统 | 文件系统（Markdown）| SQLite + JSON |
| **桌面框架** | Tauri 2 | 无（纯 CLI） | Electron（macOS menu bar） |

### 设计哲学差异

- **OpenComputer**：GUI-first，强调傻瓜式配置（24+ Provider 模板），核心逻辑在 Rust 后端实现，前端只负责展示
- **Claude Code**：CLI-first，面向开发者，深度 IDE 集成（VS Code/JetBrains），大量 feature-flag 控制实验性功能
- **OpenClaw**：Gateway-first，消息路由中枢，24+ IM 渠道汇聚，Plugin SDK 扩展体系

---

## 二、核心能力逐项对比

### 2.1 工具系统

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **工具总数** | 40+ | 43+ | 通过 Plugin SDK 扩展 |
| **延迟加载（Deferred Loading）** | ✅ opt-in，`tool_search` 发现 | ✅ `shouldDefer` + `ToolSearchTool` | ❌ 全量加载 |
| **并发执行** | ✅ `concurrent_safe` 分组（只读并行/写入串行） | ✅ 并行工具调用 | ✅ keyed async queue |
| **权限过滤** | ✅ `denied_tools` + `skill_allowed_tools` | ✅ `filterToolsByDenyRules` + deny 规则 | ✅ tool whitelisting per session |
| **大结果磁盘持久化** | ✅ 50KB 阈值写磁盘，上下文仅保留 head+tail 预览 | ❌ 内存中截断 | ❌ |
| **Browser 控制** | ✅ 6 模块（导航/交互/快照/渲染/连接/高级） | ✅ `WebBrowserTool`（Playwright） | ✅ Browser tool |
| **Web 搜索引擎数** | ✅ **8 个**（Brave/DDG/Google/Grok/Kimi/Perplexity/SearXNG/Tavily） | ⚠️ 1 个（内置，按订阅层级） | ⚠️ 通过 provider 扩展 |
| **图片生成提供商** | ✅ **7 个**（FAL/Imagen3/DALL-E/MiniMax/SiliconFlow/Zhipu/Tongyi） | ❌ 无内置 | ✅ 有图片生成集成 |
| **PDF 操作** | ✅ 合并/拆分/提取/OCR | ✅ 读取 + OCR | ❌ |
| **MCP 工具代理** | ❌ **缺失** | ✅ 完整 MCP 客户端（OAuth + 工具代理 + 资源访问 + 官方注册表） | ✅ MCP loopback server |
| **LSP 集成** | ❌ | ✅ `LSPTool`（feature-gated，语义级代码分析） | ❌ |
| **Feature Flag 控制** | ❌ | ✅ Bun `feature()` 死代码消除（20+ flag） | ❌ |

#### 关键差距

**MCP（Model Context Protocol）是最大缺口。** Claude Code 有完整的 MCP 客户端实现：
- `client.ts`（119KB）：核心客户端，服务器连接生命周期管理
- `auth.ts`（88KB）：OAuth 2.0 + API Key + 环境变量 + XAA 身份认证
- `config.ts`（51KB）：多来源配置发现（Claude.ai 市场 / 用户设置 / 项目级 / 企业策略）
- `MCPTool`：透传 MCP 服务器工具到 LLM
- `ListMcpResourcesTool` / `ReadMcpResourceTool`：MCP 资源访问
- 官方注册表（`officialRegistry.ts`）+ 企业策略过滤（`filterMcpServersByPolicy`）

缺少 MCP 意味着 OpenComputer 无法接入快速增长的 MCP 工具生态。

#### OpenComputer 优势

- **搜索引擎多样性**（8 vs 1）：用户可按需切换搜索引擎，覆盖中文（Kimi）、隐私（Brave/DDG）、学术（Perplexity）等场景
- **图片生成多样性**（7 个提供商）：唯一内置多图片生成支持的项目
- **工具结果磁盘持久化**：大结果不占上下文，通过 head+tail 预览 + 路径引用保持可访问性

---

### 2.2 Agent / 子 Agent 系统

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **子 Agent 产卵** | ✅ `spawn` / `batch_spawn` | ✅ `AgentTool` + `TeamCreateTool` | ✅ 子 Agent |
| **前后台自动切换** | ✅ `spawn_and_wait`（30s 超时自动转后台） | ✅ 前/后台隔离 | ❌ |
| **Team/Swarm 多 Agent 协作** | ❌ **缺失** | ✅ `TeamCreate` / `TeamDelete` + 团队记忆同步 | ❌ |
| **Coordinator 编排模式** | ❌ **缺失** | ✅ 限制为 Agent/Task/SendMessage/SyntheticOutput 四工具 | ❌ |
| **Git Worktree 隔离** | ❌ **缺失** | ✅ `EnterWorktree` / `ExitWorktree`（Agent 独立仓库副本） | ❌ |
| **Agent 间双向消息** | ❌ 仅 `steer`（单向指令注入） | ✅ `SendMessageTool` 异步消息 | ❌ |
| **远程 Agent** | ❌ | ✅ `RemoteAgentTask` + WebSocket | ❌ |
| **内置 Agent 类型** | ✅ 可配置 Agent（personality/filter/behavior） | ✅ Explore/Plan/Implement/Refactor/Review/Test | ✅ Multi-Agent routing |
| **结果自动注入** | ✅ `inject_and_run_parent`（完成后自动推送回父 Agent） | ✅ `TaskOutput` | ❌ |
| **深度限制** | ✅ 可配置 depth limit | ✅ 层级控制 | ❌ |

#### 关键差距

**Team/Swarm 是重大缺口：**
- Claude Code 支持创建命名的 Agent 团队（`TeamCreateTool`），成员间通过 `teamMemorySync` 服务共享记忆
- Coordinator 模式限制协调者只能使用调度工具（Agent/Task/SendMessage/SyntheticOutput），确保职责分离
- OpenComputer 只有单层 parent-child 关系，无法实现多 Agent 并行协作

**Git Worktree 隔离：**
- Claude Code 每个 Agent 可在独立 git worktree 中工作，避免并发文件冲突
- 对于复杂任务（如多个 Agent 同时编辑不同文件），worktree 隔离是安全保障

**Agent 间通信：**
- Claude Code 有 `SendMessageTool` 做异步消息传递，Agent 可以双向沟通
- OpenComputer 只有 `steer`（单向指令注入），通信能力受限

---

### 2.3 Skill 系统

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **Skill 发现来源** | ✅ SKILL.md 扫描（bundled/managed/project） | ✅ **6 种来源**（bundled/MCP/managed/project/user/plugin） | ✅ Plugin SDK |
| **allowed-tools 白名单** | ✅ frontmatter `allowed-tools:` | ✅ frontmatter `allowedTools` | ❌ |
| **Fork 模式（上下文隔离）** | ✅ `context: fork` → 子 Agent 执行 | ✅ context modes（fork/inline） | ❌ |
| **MCP Skills** | ❌ **缺失** | ✅ 从 MCP 服务器动态注册 Skill（`registerMCPSkillBuilders`） | ❌ |
| **Effort 级别** | ❌ | ✅ quick/moderate/involved（影响 token 预算和策略） | ❌ |
| **语义搜索** | ❌ | ✅ `EXPERIMENTAL_SKILL_SEARCH`（语义索引） | ❌ |
| **Managed Skills（组织推送）** | ❌ | ✅ 组织策略/MDM 推送 | ❌ |
| **命令参数模板** | ✅ `$ARGUMENTS` 扩展 | ✅ argument substitution | ❌ |
| **安装规范** | ✅ brew/node/go/uv/download | ⚠️ 基础 | ❌ |
| **环境要求检测** | ✅ bins/any_bins/env/os/config | ✅ 类似 | ❌ |
| **Skill 预算控制** | ✅ max_count(150)/max_chars(30K)/max_file_bytes(256KB) | ✅ token 预算 | ❌ |

#### 关键差距

- **MCP Skills**：Claude Code 可以从 MCP 服务器动态注册 Skill，极大扩展了 Skill 生态（任何 MCP 服务器都可以变成 Skill）
- **Effort 级别**：quick/moderate/involved 三档影响 token 预算和执行策略，是精细化资源控制

#### OpenComputer 优势

- **安装规范**：内置 brew/node/go/uv/download 安装支持，Skill 可声明依赖并自动安装
- **预算控制**：对 Skill 数量、字符数、文件大小有精细的上限控制

---

### 2.4 Plan Mode

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **状态机** | ✅ **六态**（Off → Planning → Review → Executing → Paused → Completed） | ⚠️ Enter/Exit 二态 | ❌ 无 Plan Mode |
| **双 Agent 分离** | ✅ PlanAgent（只读 + 规划工具）+ BuildAgent（全部工具 + 步骤追踪） | ⚠️ 权限模式切换 | ❌ |
| **执行层权限强制** | ✅ schema 过滤 + execution 白名单双重防护 | ✅ permission mode enforcement | ❌ |
| **步骤追踪** | ✅ PlanStep（Pending/InProgress/Completed/Skipped/Failed） | ⚠️ 较简单 | ❌ |
| **Git Checkpoint** | ✅ `checkpoint_ref`（分支/stash 恢复点） | ❌ | ❌ |
| **交互式问答** | ✅ `PlanQuestionOption`（多选 + 推荐标记） | ❌ | ❌ |
| **Plan 验证** | ❌ | ✅ `VerifyPlanExecutionTool`（feature-gated） | ❌ |
| **暂停/恢复** | ✅ `Paused` 状态 + `paused_at_step` 恢复点 | ❌ | ❌ |
| **版本控制** | ✅ `version` 字段（每次编辑递增） | ❌ | ❌ |

#### OpenComputer 领先

**Plan Mode 是 OpenComputer 最强的差异化能力之一。** 六态状态机、双 Agent 分离、步骤追踪、Git Checkpoint、交互式问答、暂停/恢复 —— 这些在 Claude Code 的简单 Enter/Exit 模式中都不存在。

Claude Code 的 `VerifyPlanExecutionTool`（计划验证）是一个值得学习的方向，可以在执行前验证计划的可行性。

---

### 2.5 记忆系统

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **存储后端** | ✅ SQLite + FTS5 全文搜索 + 向量检索 | ✅ 文件系统（Markdown + frontmatter） | ✅ LanceDB 向量存储 |
| **记忆类型** | ✅ User/Feedback/Project/Reference | ✅ user/feedback/project/reference（完全对齐） | ❌ 无分类 |
| **向量语义搜索** | ✅ embedding + MMR 多样性去重 | ⚠️ 文件扫描 + 语义匹配 | ✅ 多模态 embedding |
| **全文搜索** | ✅ FTS5 分词排名 | ❌ 文件名/内容遍历 | ❌ |
| **自动提取** | ✅ side_query 缓存共享（成本 ↓90%） | ✅ `extractMemories` 服务 | ❌ |
| **LLM 语义选择** | ✅ 候选 >8 时 LLM 筛选 top-5 注入 | ❌ | ❌ |
| **作用域** | ✅ Global / Agent-specific | ✅ Private / Team | ❌ |
| **Team 记忆同步** | ❌ | ✅ `teamMemorySync` 服务（跨 Agent 共享） | ❌ |
| **附件支持** | ✅ 图片/音频附件（`attachment_path` + `attachment_mime`） | ❌ | ❌ |
| **Pin 置顶** | ✅ `pinned` 字段（始终优先注入） | ❌ | ❌ |
| **来源追踪** | ✅ `source`: user/auto/import | ❌ | ❌ |
| **记忆老化** | ❌ | ✅ `memoryAge.ts`（考虑时间衰减） | ❌ |

#### 分析

OpenComputer 在存储层（SQLite + 向量 + FTS5 三引擎）更强，且有 LLM 语义选择、附件支持、Pin 置顶等独有功能。Claude Code 在团队记忆同步和记忆老化上领先。

---

### 2.6 上下文管理

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **压缩层级** | ✅ **5 层**（Tier 0 微压缩 → Tier 1 截断 → Tier 2 裁剪 → Tier 3 LLM 摘要 → Tier 4 紧急） | ✅ 多层（micro/compact/session/reactive） | ⚠️ 基础压缩 |
| **API-Round 分组保护** | ✅ `_oc_round` 元数据标记，切割对齐 round 边界 | ✅ `grouping.ts` 分组策略 | ❌ |
| **后压缩文件恢复** | ✅ 摘要后自动扫描 write/edit 调用，注入最近 5 文件 × 16KB | ❌ **独创** | ❌ |
| **Side Query 缓存** | ✅ 复用 prompt cache 前缀，侧查询成本 ↓90% | ❌ **独创** | ❌ |
| **Reactive Compact** | ❌ | ✅ 动态上下文压缩（feature-gated） | ❌ |
| **Tool Use Summary** | ❌ | ✅ 多工具调用合并摘要（`generateToolUseSummary`） | ❌ |
| **Context Collapse 检查** | ❌ | ✅ `CtxInspectTool`（调试上下文状态） | ❌ |
| **Token 估算** | ✅ 校准的 provider-aware 估算 | ✅ `roughTokenCountEstimation` + `tokenCountWithEstimation` | ❌ |
| **成本追踪** | ✅ Dashboard 集成（详细到模型/日期） | ✅ `cost-tracker.ts`（累计 API 费用） | ❌ |

#### 分析

双方各有创新：
- **OpenComputer 独创**：后压缩文件恢复（压缩后自动恢复关键编辑文件内容）和 Side Query 缓存（侧查询复用 prompt cache 降低 90% 成本）
- **Claude Code 独有**：Reactive Compact（动态压缩）、Tool Use Summary（多工具合并摘要）、Context Collapse（上下文状态检查工具）

---

### 2.7 Provider 支持

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **API 类型** | ✅ **4 种**（Anthropic / OpenAI Chat / OpenAI Responses / Codex） | ⚠️ Anthropic 为主（OAuth 2.0） | ✅ 多 Provider 可配置 |
| **预置模板数** | ✅ **24+**（傻瓜式配置） | ❌ 仅 Anthropic 模型 | ✅ 可配置 |
| **模型链降级** | ✅ 6 种错误分类（RateLimit/Overloaded/Timeout/Auth/Billing/ModelNotFound/ContextOverflow） | ✅ `withRetry` + `FallbackTriggeredError` | ⚠️ 基础重试 |
| **Extended Thinking** | ✅ Anthropic 扩展思考 + OpenAI O1 推理 | ✅ Anthropic 扩展思考 | ❌ |
| **Prompt Cache** | ✅ Anthropic 显式 `cache_control` + OpenAI 自动前缀缓存 | ✅ Anthropic prompt cache | ❌ |
| **自定义端点** | ✅ 任意 OpenAI 兼容端点 | ⚠️ 有限 | ✅ 可配置 |
| **代理配置** | ✅ HTTP/HTTPS 代理 + per-URL 规则 | ⚠️ 环境变量 | ❌ |
| **温度配置** | ✅ 三层覆盖（会话 > Agent > 全局） | ⚠️ 单层 | ❌ |

#### OpenComputer 领先

多 Provider 支持是核心差异化优势。24+ 预置模板让用户无需理解 API 差异就能使用各种模型（Claude、GPT-4、Gemini、DeepSeek、Qwen 等），覆盖国内外主流模型。

---

### 2.8 IM 渠道系统

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **渠道数量** | ✅ **12 个** | ❌ 无 IM 功能 | ✅ **24+** |
| **已支持渠道** | Telegram / WeChat / Discord / Slack / Feishu / QQ Bot / IRC / Signal / iMessage / WhatsApp / Google Chat / LINE | — | 上述全部 + Teams / Matrix / Nostr / Mattermost / Zalo / Twitch / BlueBubbles 等 |
| **DM 配对安全策略** | ❌ **缺失** | ❌ | ✅ pairing（未知发送者需验证码审批）/ open 策略 |
| **入站媒体管道** | ✅ 下载解密（图片/视频/语音/文件） | ❌ | ✅ 完整管道（MIME 嗅探 + FFmpeg 转码 + QR 检测） |
| **出站媒体管道** | ✅ AES-128-ECB 加密上传 CDN（3 次 5xx 重试） | ❌ | ✅ 渠道适配器 + 恢复上传 |
| **Webhook 服务器** | ✅ 嵌入式 | ❌ | ✅ HTTP hooks + 配置路径 |
| **进程管理器** | ✅ | ❌ | ✅ |
| **群组策略** | ⚠️ 基础 | ❌ | ✅ 完善（mention gate / reply mode / thread binding / 群组激活模式） |
| **语音能力** | ❌ **缺失** | ❌ | ✅ **语音唤醒 + Talk Mode + 实时转录**（ElevenLabs/Deepgram） |
| **Typing 指示器** | ✅ WeChat（24h TTL + 5s keepalive + cancel） | ❌ | ✅ 多渠道 |
| **原生 App** | ❌（仅 Tauri 桌面） | ❌（仅 CLI） | ✅ macOS / iOS / Android 原生 App |
| **消息去重** | ❌ | ❌ | ✅ 持久化 + 内存双层去重 |

#### 关键差距

- **语音能力**：OpenClaw 有完整的语音链路（唤醒词检测 → 实时转录 → 对话 → TTS 回复），OpenComputer 完全没有
- **DM 配对安全**：OpenClaw 的 pairing 策略要求未知发送者通过验证码审批，避免未授权消息进入 AI 对话
- **群组策略**：OpenClaw 支持 mention-only 激活、thread binding、reply mode 等精细控制

#### OpenComputer 优势

- 12 个渠道已覆盖主流平台，对于桌面 AI 助手场景足够
- WeChat 通道实现深度最高（typing 指示器、QR 自动刷新、媒体加解密完整链路）

---

### 2.9 权限与安全

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **权限模式** | ⚠️ 工具审批 + Plan 限制 | ✅ **6 种**（default / plan / bypassPermissions / dontAsk / acceptEdits / auto） | ✅ DM pairing + allowlist |
| **Docker 沙箱** | ✅ 可配置（镜像/超时/内存/CPU） | ✅ 通过 BashTool | ⚠️ 可选 sandbox mode |
| **组织策略 / MDM** | ❌ **缺失** | ✅ `policyLimits`（组织级工具限制）+ MDM 集成（macOS） | ❌ |
| **路径限制** | ✅ Plan Agent 路径限制 | ✅ 绝对路径强制 + symlink 安全 | ✅ workspace 隔离 |
| **macOS TCC 权限检查** | ✅ **15 种权限**（辅助功能/屏幕录制/自动化/位置/相机/麦克风等） | ⚠️ 基础 | ❌ |
| **API Key 脱敏** | ✅ `redact_sensitive`（32KB 截断） | ✅ secrets redaction | ✅ `SecretRef` 类型 |
| **速率限制** | ❌ | ❌ | ✅ 多层速率限制（auth failure / control-plane write / preauth） |
| **TLS/HTTPS** | ❌（本地应用） | ❌（本地 CLI） | ✅ Gateway TLS（自定义证书） |

#### 关键差距

- **多权限模式**：Claude Code 的 `dontAsk`（同类工具只问一次）、`acceptEdits`（自动接受文件编辑）、`auto`（基于 transcript 分类自动决策）提供更灵活的审批体验
- **组织策略/MDM**：企业级特性，允许组织管理员统一限制工具使用

#### OpenComputer 优势

- macOS TCC 权限检查（15 种）是独有的，确保 AI 助手不会越权访问系统资源

---

### 2.10 IDE 集成

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **VS Code 插件** | ❌ **缺失** | ✅ 完整 Bridge（`bridgeMain.ts` 115KB，33 文件 ~1MB） | ❌ |
| **JetBrains 插件** | ❌ | ✅ 同架构（PyCharm/WebStorm 等） | ❌ |
| **Web IDE** | ❌ | ✅ 浏览器 IDE 支持 | ❌ |
| **ACP 协议** | ✅ Agent Coding Protocol（运行时 stdio 管理 + 健康检查） | ❌ | ❌ |
| **JWT 认证** | ❌ | ✅ IDE-CLI 安全通信 | ❌ |
| **REPL Bridge** | ❌ | ✅ `replBridge.ts`（100KB，进程内 REPL + 沙箱） | ❌ |
| **Bridge 消息协议** | ❌ | ✅ 编码/解码 + 请求/响应关联 | ❌ |

#### 关键差距

IDE 集成是 Claude Code 投入最大的方向之一（33 文件 ~1MB），包括：
- **Bridge 主循环**（`bridgeMain.ts` 115KB）：双向通信编排
- **REPL Bridge**（`replBridge.ts` 100KB）：进程内 REPL 执行 + 工具沙箱
- **Session 协调**（`sessionRunner.ts`）：每个 IDE tab 独立会话

OpenComputer 的 ACP 是不同思路（直接协议而非 IDE 插件），但缺少对主流 IDE 的插件支持限制了开发者使用场景。

---

### 2.11 Hooks 系统

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **Hook 事件数** | ❌ **完全缺失** | ✅ **10+ 事件** | ✅ Webhook hooks |
| **支持的事件** | — | UserPromptSubmit / PreToolUse / PostToolUse / SessionStart / Setup / FileChanged / SubagentStart 等 | Webhook 触发 |
| **同步 Hook（拦截/批准）** | — | ✅ `approve` / `block` 决策 + `reason` | ❌ |
| **异步 Hook** | — | ✅ `HookJSONOutput` / `PromptRequest` | ✅ |
| **Hook 向用户提问** | — | ✅ `PromptRequest`（选项列表） | ❌ |
| **系统消息注入** | — | ✅ `systemMessage` 字段 | ❌ |

#### 关键差距

**Hooks 是 Claude Code 的强大扩展机制，OpenComputer 完全没有。** Hooks 允许：
- 工具执行前拦截（PreToolUse → block 危险操作）
- 工具执行后审计（PostToolUse → 日志/通知）
- 用户输入预处理（UserPromptSubmit → 内容过滤/增强）
- 文件变更监听（FileChanged → 自动重新分析）
- 自定义工作流编排

缺少 Hooks 限制了 OpenComputer 的自动化和自定义能力。

---

### 2.12 通知系统

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **桌面通知** | ✅ 系统通知（title + body） | ⚠️ 终端通知 | ✅ 推送通知 |
| **优先级队列** | ❌ | ✅ low/medium/high/immediate + 超时自动消失 | ❌ |
| **通知折叠** | ❌ | ✅ `fold` 函数合并同 key 通知 | ❌ |
| **通知失效** | ❌ | ✅ `invalidates` 字段废弃旧通知 | ❌ |

---

### 2.13 Session 管理

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **持久化** | ✅ SQLite（消息历史 + 会话元数据） | ✅ Transcript 文件 + 会话标题缓存 | ✅ SQLite + JSON |
| **会话恢复** | ✅ | ✅ `conversationRecovery`（中断状态恢复） | ✅ |
| **Teleport（远程仓库会话）** | ❌ | ✅ Git bundle + 仓库不匹配检测 | ❌ |
| **子 Agent 追踪** | ✅ `SubagentDB`（父子关系 + 执行结果） | ✅ Task 系统 | ❌ |
| **远程会话** | ❌ | ✅ `RemoteSessionManager` + WebSocket | ❌ |

---

### 2.14 其他独有能力

| 能力 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| **数据大盘** | ✅ **10+ 分析维度**（token/工具/会话/错误/系统/成本/TTFT） | ❌ 仅 cost tracker | ❌ |
| **天气系统** | ✅ CoreLocation 定位 + 天气动效（Canvas 粒子） | ❌ | ❌ |
| **Cron 调度** | ✅ 4 种载荷（Webhook/Message/Command/Trigger）+ 退避重试 | ✅ `ScheduleCronTool`（feature-gated） | ✅ cron |
| **i18n 多语言** | ✅ **10 种语言** | ❌ 仅英文 | ❌ |
| **Canvas 画布** | ✅ HTML/React 画布渲染 | ❌ | ✅ A2UI 可视化工作台（push 框架） |
| **Teleport** | ❌ | ✅ 远程仓库会话桥接（Git bundle） | ❌ |
| **Proactive 模式** | ❌ | ✅ `DreamTask` + `SleepTool`（主动后台工作） | ❌ |
| **Structured Output** | ❌ | ✅ `SyntheticOutputTool` + 强制输出验证 | ❌ |
| **History Snip** | ❌ | ✅ `SnipTool`（会话历史裁剪，feature-gated） | ❌ |
| **UDS Inbox** | ❌ | ✅ Unix Domain Socket 消息路由（feature-gated） | ❌ |
| **Advisor Model** | ❌ | ✅ 独立 advisor 模型做代码审查 | ❌ |

---

## 三、综合评分矩阵

> 评分标准：5 = 业界领先，4 = 完善，3 = 可用，2 = 基础，1 = 缺失，0 = 不适用

| 能力维度 | OpenComputer | Claude Code | OpenClaw |
|----------|:-----------:|:-----------:|:--------:|
| 工具系统 | **4** | **5** | 3 |
| Agent 协作 | 3 | **5** | 2 |
| Skill 系统 | **4** | **5** | 2 |
| Plan Mode | **5** | 3 | 1 |
| 记忆系统 | **5** | 4 | 3 |
| 上下文管理 | **5** | **4** | 2 |
| Provider 支持 | **5** | 2 | 3 |
| IM 渠道 | **4** | 0 | **5** |
| 权限安全 | 3 | **5** | 4 |
| IDE 集成 | 2 | **5** | 1 |
| Hooks 扩展 | 1 | **5** | 3 |
| 通知系统 | 3 | **4** | 3 |
| 会话管理 | 4 | **5** | 4 |
| 数据分析 | **5** | 2 | 1 |
| 多语言 | **5** | 1 | 1 |
| GUI 体验 | **5** | 2 | 3 |
| **综合** | **63/96** | **57/96** | **41/96** |

---

## 四、OpenComputer 独有优势总结

| 优势领域 | 具体表现 | 影响 |
|----------|----------|------|
| **GUI 桌面应用** | Tauri 2 原生桌面，比 CLI 和 Web 控制台更友好 | 降低使用门槛，覆盖非技术用户 |
| **24+ Provider 模板** | 傻瓜式多模型配置，一键切换 | 远超两者的模型覆盖度 |
| **8 种搜索引擎** | 覆盖隐私/中文/学术/实时等场景 | 信息获取多样性最强 |
| **7 种图片生成** | FAL/Imagen3/DALL-E/MiniMax/SiliconFlow/Zhipu/Tongyi | 唯一内置多图生成的项目 |
| **Plan Mode 六态状态机** | 双 Agent 分离 + 步骤追踪 + Git Checkpoint + 交互问答 | 最成熟的计划执行系统 |
| **Side Query 缓存** | 侧查询复用 prompt cache 前缀 | 独创，成本降低 ~90% |
| **后压缩文件恢复** | Tier 3 摘要后自动注入最近编辑文件 | 独创，减少 read tool call |
| **数据大盘** | 10+ 分析维度（token/工具/会话/错误/系统/成本） | 唯一有完整分析仪表板的项目 |
| **10 种语言 i18n** | 中英日韩俄葡土越马 + 繁体中文 | 唯一有多语言支持的项目 |
| **macOS TCC 权限检查** | 15 种系统权限检测 | 独有的系统级安全检查 |
| **工具结果磁盘持久化** | 大结果写磁盘，上下文仅保留预览 | 独创，上下文利用率更高 |
| **LLM 记忆语义选择** | 候选 >8 时 LLM 精选 top-5 | 最精细的记忆注入策略 |
| **记忆附件** | 支持图片/音频附件 | 多模态记忆能力 |

---

## 五、需要补齐的关键差距

### P0 — 阻塞性缺失（影响扩展性和自动化能力）

| 缺失能力 | 参考来源 | 影响分析 | 估计复杂度 |
|----------|----------|----------|-----------|
| **MCP 协议支持** | Claude Code | 无法接入 MCP 工具生态（数百个社区 MCP 服务器），限制了工具可扩展性。需实现 MCP 客户端（连接管理 + 工具代理 + 资源访问 + OAuth）| 高（~3-5K 行 Rust） |
| **Hooks 系统** | Claude Code | 无法自定义工作流、无法做 pre/post 工具拦截、无法扩展事件处理。需实现事件定义 + Hook 注册 + 同步/异步执行引擎 | 中（~1-2K 行 Rust） |

### P1 — 重要增强（影响高级使用场景）

| 缺失能力 | 参考来源 | 影响分析 | 估计复杂度 |
|----------|----------|----------|-----------|
| **Team/Swarm Agent** | Claude Code | 多 Agent 协作受限于单层 parent-child，无法实现 Coordinator 编排模式。需实现 Team 创建/销毁 + Agent 间消息 + 记忆同步 | 高 |
| **Git Worktree 隔离** | Claude Code | 并发 Agent 文件冲突风险。需实现 worktree 创建/清理 + Agent 目录绑定 | 中 |
| **多权限模式** | Claude Code | 缺少 `dontAsk` / `acceptEdits` / `auto` 等便捷审批模式，用户体验较刚性 | 中 |

### P2 — 差异化增强（扩展使用场景）

| 缺失能力 | 参考来源 | 影响分析 | 估计复杂度 |
|----------|----------|----------|-----------|
| **Agent 间双向消息** | Claude Code | 当前仅 `steer` 单向注入，需要 `SendMessage` 式异步消息传递 | 低 |
| **语音能力** | OpenClaw | 无语音输入/唤醒/转录能力 | 高 |
| **VS Code / JetBrains 插件** | Claude Code | ACP 已有基础，但缺少主流 IDE 插件 | 高 |
| **DM 配对安全** | OpenClaw | IM 渠道缺少未知发送者验证机制 | 低 |
| **MCP Skills** | Claude Code | 不能从 MCP 服务器动态注册 Skill | 中（依赖 MCP 实现） |

### P3 — 锦上添花

| 缺失能力 | 参考来源 | 影响分析 |
|----------|----------|----------|
| **Reactive Compact** | Claude Code | 动态上下文压缩可进一步优化上下文利用率 |
| **Tool Use Summary** | Claude Code | 多工具调用缺少合并摘要 |
| **Structured Output** | Claude Code | 无强制输出格式验证 |
| **Proactive 模式** | Claude Code | 不能主动做后台工作（如 DreamTask） |
| **Effort 级别** | Claude Code | Skill 缺少 quick/moderate/involved 精细资源控制 |
| **Plan 验证** | Claude Code | 执行前缺少可行性验证 |
| **Teleport** | Claude Code | 不支持远程仓库会话桥接 |
| **记忆老化** | Claude Code | 记忆没有时间衰减机制 |

---

## 六、建议演进路线图

### Phase 8: MCP 协议支持（P0）

```
目标：接入 MCP 工具生态
范围：MCP 客户端 + 工具代理 + 资源访问 + 配置管理
参考：claude-code/src/services/mcp/（client.ts + auth.ts + config.ts）

关键组件：
├── mcp/client.rs          — MCP 连接生命周期（stdio / HTTP transport）
├── mcp/tool_proxy.rs      — 工具代理（MCP 工具 → 内置工具接口适配）
├── mcp/resource.rs        — 资源发现与读取
├── mcp/config.rs          — 多来源配置（用户 / 项目 / 全局）
├── mcp/auth.rs            — OAuth 2.0 + API Key
└── commands/mcp.rs        — Tauri 命令 + 前端管理面板
```

### Phase 9: Hooks 系统（P0）

```
目标：可扩展的事件钩子机制
范围：事件定义 + 注册 + 同步/异步执行
参考：claude-code/src/hooks/（87 个文件）

关键事件：
├── PreToolUse             — 工具执行前拦截（approve/block）
├── PostToolUse            — 工具执行后审计
├── UserPromptSubmit       — 用户输入预处理
├── SessionStart           — 会话初始化
├── FileChanged            — 文件变更监听
└── SubagentStart          — 子 Agent 启动
```

### Phase 10: Team Agent + Git Worktree（P1）

```
目标：多 Agent 协作 + 文件隔离
范围：Team 创建/销毁 + Agent 间消息 + Worktree 管理 + 记忆同步

关键组件：
├── team/manager.rs        — Team 生命周期管理
├── team/message.rs        — Agent 间异步消息传递
├── team/memory_sync.rs    — 团队记忆同步
├── worktree/manager.rs    — Git worktree 创建/清理
└── worktree/binding.rs    — Agent-Worktree 绑定
```

### Phase 11: 权限模式扩展（P1）

```
目标：灵活的审批模式
新增模式：
├── dontAsk                — 同类工具只问一次
├── acceptEdits            — 自动接受文件编辑
├── auto                   — 基于上下文自动决策
└── bypassPermissions      — 全自动（需 opt-in 确认）
```

### Phase 12: IM 渠道增强（P2）

```
目标：安全性 + 语音能力
范围：DM 配对策略 + 群组策略增强 + 语音输入（可选）

关键增强：
├── DM 配对策略            — pairing/open 模式，未知发送者验证码审批
├── 群组 mention gate      — 群组中仅 @bot 时响应
├── 群组 thread binding    — 回复绑定原始 thread
└── 语音输入（可选）       — 录音 → 转录 → 对话（依赖外部 ASR 服务）
```

---

## 七、总结

**OpenComputer 在以下方面业界领先：**
- Plan Mode 系统设计（六态状态机 + 双 Agent + Git Checkpoint）
- 多 Provider 支持生态（24+ 模板 + 4 种 API 类型）
- 上下文管理创新（Side Query 缓存 + 后压缩文件恢复 + 磁盘持久化）
- 记忆系统深度（三引擎存储 + LLM 语义选择 + 附件）
- 用户体验友好性（GUI + i18n + 数据大盘）

**最需要补齐的两个方向：**
1. **MCP 协议支持** — 这是工具生态的关键门户，没有它就无法利用社区构建的数百个 MCP 服务器
2. **Hooks 系统** — 这是自动化和自定义工作流的基础，也是迈向企业级使用的必要条件

**整体而言**，OpenComputer 在 GUI 体验、Provider 多样性、Plan Mode、记忆系统等方面已经超越 Claude Code，但在开发者生态集成（MCP/IDE/Hooks）和多 Agent 协作上仍有显著差距。建议优先补齐 P0 缺失，这将显著扩展 OpenComputer 的能力边界和用户覆盖面。

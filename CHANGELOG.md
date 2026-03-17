# Changelog

All notable changes to OpenComputer will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Agent 定义系统**：支持创建和管理多个 AI Agent，每个 Agent 可独立配置身份、性格和行为
  - 设置页新增 Agent section，支持列表/新建/编辑/删除
  - Agent 编辑 4 个 Tab：身份（名称/描述/Emoji/头像/角色定位）、性格（气质/语气/特质/准则/边界/个性/沟通方式）、行为（工具轮数/审批工具/沙箱/工具指导）、自定义提示词
  - 结构化配置模式：GUI 表单填写，自动组装系统提示词（PersonalityConfig 8 个字段）
  - 自定义提示词模式：开启后忽略结构化设置，直接编辑 Markdown（agent.md / persona.md）
  - 身份和性格页底部均支持「补充说明」自由文本
  - 首次开启自定义模式自动从模板文件预填内容
  - 新增 `agent_config.rs`：AgentConfig / PersonalityConfig / AgentDefinition / AgentSummary 数据结构
  - 新增 `agent_loader.rs`：Agent 文件 CRUD + 多语言模板（`include_str!` 嵌入 12 种语言）
  - 新增 `system_prompt.rs`：模块化提示词组装，支持结构化/自定义双模式
  - 新增 `user_config.rs`：用户个人配置（昵称/性别/年龄/角色/时区/语言/AI 经验/回复风格）
  - 新增 Tauri 命令：`list_agents` / `get_agent_config` / `get_agent_markdown` / `save_agent_config_cmd` / `save_agent_markdown` / `delete_agent` / `get_agent_template` / `get_user_config` / `save_user_config` / `get_system_timezone`
- **多语言 Agent 模板**：12 种语言的 `agent.*.md`（身份说明）和 `persona.*.md`（人设骨架），编译时嵌入二进制
  - 默认 Agent 按系统语言创建（名称/描述/agent.md 本地化）
  - 空字段加载时自动按当前 UI 语言填充模板
- **Agent 头像支持**：通过 `tauri-plugin-dialog` 文件选择器选择本地图片，使用 `convertFileSrc` 展示
  - `tauri.conf.json` 开启 `assetProtocol`
- **聊天界面 Agent 集成**：
  - 对话列表显示当前 Agent 头像 + 名称 + Emoji
  - 聊天页头部显示 Agent 名称
  - 右上角 Settings 图标可跳转 Agent 设置页
- **用户个人配置 UI**：设置页「个人信息」面板，支持头像/昵称/性别/年龄/角色/AI 经验/时区/语言/回复风格/补充说明
- **Markdown 消息渲染**：用户和 AI 消息均支持完整 Markdown 渲染（基于 Streamdown）
  - 流式场景优化：正确处理未闭合语法（加粗、代码块等），渐进式渲染无闪烁
  - 代码块语法高亮（Shiki）、CJK 中文标点优化
  - KaTeX 数学公式渲染（LaTeX 语法）
  - Mermaid 图表渲染（流程图、时序图等）
  - 新增 `MarkdownRenderer` 组件（`src/components/MarkdownRenderer.tsx`）
  - 新增依赖：`streamdown`、`@streamdown/code`、`@streamdown/cjk`、`@streamdown/math`、`@streamdown/mermaid`、`katex`
- **统一数据存储架构**：所有数据落盘集中到 `~/.opencomputer/` 目录
  - 新增 `paths.rs` 模块：集中管理 root、config、credentials、home、share 等路径
  - 目录结构：`config.json`（通用配置）、`credentials/auth.json`（OAuth 凭证）、`home/`（主 Agent Home）、`share/`（共享目录）
  - 启动时自动创建所有必要目录
  - 启动时自动从旧路径迁移数据（`providers.json` 和 `auth.json`）
- **Provider 品牌 Logo**：所有 24 个内置 Provider 模板和 Provider 管理面板使用官方品牌 SVG 图标（基于 `@lobehub/icons`），替换原来的 emoji 字符
  - 新增 `ProviderIcon` 组件（`src/components/ProviderIcon.tsx`），支持 provider key 直接映射和 provider name 模糊匹配
- **多语言支持 (i18n)**：使用 `i18next` + `react-i18next` 实现完整的国际化支持
  - 支持 12 种语言：简体中文、繁體中文、English、日本語、Türkçe、Tiếng Việt、Português、한국어、Русский、العربية、Español、Bahasa Melayu
  - 自动检测系统语言，无法识别时回退到英文
  - 侧边栏语言切换菜单，切换后立即生效
  - 语言偏好持久化到 localStorage
  - 新增 `src/i18n/` 模块：12 个翻译文件 + i18n 初始化配置
- **Think 等级按 Provider 差异化映射**：不同 API 类型使用各自原生的 thinking 参数格式
  - Anthropic：`thinking: { type: "enabled", budget_tokens: N }`（low→1024 / medium→4096 / high→8192 / xhigh→16384）
  - OpenAI Chat Completions：`reasoning_effort` 字段（low/medium/high，xhigh 自动降级为 high）
  - OpenAI Responses / Codex：保持现有 `reasoning.effort` 格式（支持 xhigh）
- **思考类型（Thinking Style）配置**：Provider 级别的 `thinking_style` 字段，控制向不同 API 发送思考参数的格式
  - 支持 5 种风格：`openai`（reasoning_effort）、`anthropic`（thinking budget）、`zai`（thinking budget）、`qwen`（enable_thinking）、`none`（不发送）
  - 各内置模板自动设置默认值：千问/DashScope → `qwen`，智谱 → `zai`，Anthropic → `anthropic`
  - 新增/编辑 Provider 时可通过下拉菜单选择
- **动态 Think 选项**：前端根据当前模型的 API 类型显示不同的 effort 选项列表
- **切换模型自动修正**：当切换到不支持当前 effort 等级的 Provider 时，自动回退到有效值
- **模型 Provider 管理系统**：支持多个自定义模型服务商，GUI 傻瓜式配置
- **24 个内置 Provider 模板**：选择模板后只需填 API Key，Base URL 和模型列表自动预填
  - 国际：Anthropic、OpenAI (Responses)、OpenAI (Chat)、DeepSeek、Google Gemini、xAI、Mistral、OpenRouter、Groq、NVIDIA、Together AI
  - 国内：Moonshot (Kimi)、Kimi Coding、通义千问、ModelStudio (DashScope)、火山引擎、智谱 AI、MiniMax、小米 MiMo、百度千帆
  - 本地：Ollama、vLLM、LM Studio
- **Provider 三步引导向导** (`ProviderSetup.tsx`)：模板网格 + 自定义入口（API 类型选择 → 连接配置 → 模型配置）
- **Provider 管理面板** (`ProviderSettings.tsx`)：查看/编辑/删除/启用禁用，从侧边栏设置按钮进入
- **自定义 User-Agent**：支持在配置 Provider 时指定 `User-Agent` HTTP 头部（默认 `claude-code/0.1.0`），以兼容特定 WAF（如 DashScope CodingPlan）
- **三种 API 类型支持**：Anthropic Messages API、OpenAI Chat Completions、OpenAI Responses API
- **API Key 可选**：本地服务（Ollama/vLLM/LM Studio）和自定义 Provider 的 API Key 为可选项
- **OpenAI Chat Completions 流式调用**：完整的 SSE 解析和 tool calling 支持
- **OpenAI Responses API 自定义 Base URL**：可用于兼容 OpenAI API 的第三方服务
- **Provider 持久化**：配置保存至 `providers.json`，重启自动恢复
- **模型属性配置**：支持名称、输入类型(文本/图片/视频)、Context Window、Max Tokens、推理支持、成本
- **连通性测试**：添加 Provider 时可验证 API Key 和 Base URL 是否有效
- 新增 `provider.rs` 模块：`ApiType`、`ModelConfig`、`ProviderConfig` 数据结构 + JSON 持久化
- 新增 Tauri 命令：`get_providers`、`add_provider`、`update_provider`、`delete_provider`、`test_provider`、`get_available_models`、`get_active_model`、`set_active_model`、`has_providers`
- **统一 Tool Calling 支持**：Anthropic 和 OpenAI 双 Provider 均支持 tool 调用（exec、read_file、write_file、patch_file、list_dir、web_search、web_fetch）
- **新增 `web_search` 工具**：AI 可搜索网页获取最新信息（基于 DuckDuckGo，无需 API Key）
- **新增 `web_fetch` 工具**：AI 可抓取网页内容，自动提取正文并清理 HTML 标签
- **新增 `patch_file` 工具**：基于搜索替换的精确文件编辑，比 write_file 覆写更安全
- **`exec` 工具全面升级**（对齐 OpenClaw）：
  - 默认超时从 120s 调整为 1800s（30 分钟），最大支持 7200s（2 小时）
  - 新增 `env` 参数支持自定义环境变量
  - 新增 `background` 参数支持后台执行，立即返回 session ID
  - 新增 `yield_ms` 参数支持自动后台化（等待指定毫秒后若未完成则后台）
  - 启动时自动解析 login shell PATH，确保 npm/python 等工具可用
  - 输出截断动态调整：根据模型上下文窗口自动计算（默认 200K chars，最小 8K）
- **新增 `process` 工具**：管理后台执行的 exec 会话
  - `list`：列出所有运行/已结束的会话
  - `poll`：获取会话新输出，支持 timeout 等待
  - `log`：查看完整输出日志，支持 offset/limit 分页
  - `write`：向后台进程 stdin 写入（Phase 3 完善）
  - `kill`：终止后台进程
  - `clear`/`remove`：清理已结束会话
- **新增 `process_registry.rs` 模块**：进程会话注册表，全局单例管理所有 exec 产生的后台进程
- **PTY 支持**：exec 新增 `pty` 参数，基于 `portable-pty` crate 实现伪终端执行
  - 适用于需要 TTY 的交互式命令（REPL、编辑器等）
  - PTY 不可用时自动回退到普通模式
  - 输出自动清理 ANSI 转义序列
- **命令审批系统**：exec 执行前检查命令是否在 allowlist 中
  - 不在 allowlist 中的命令触发审批流程（Tauri `approval_required` 事件）
  - 支持 AllowOnce / AllowAlways / Deny 三种响应
  - AllowAlways 自动将命令前缀加入 allowlist（持久化至 `~/.opencomputer/exec-approvals.json`）
  - 新增 `respond_to_approval` Tauri 命令
  - 全局 `APP_HANDLE` 存储用于事件发射
- **`read_file` 工具增强**（对齐 OpenClaw）：
  - 自适应分页：根据模型 context window 自动计算单页大小（20% 上下文），循环拼接最多 8 页
  - 新增 `offset`/`limit` 参数支持行级分页读取（1-based 行号），大文件可分段读取
  - 自动检测图片文件（PNG/JPEG/GIF/WebP/BMP/TIFF/ICO）并返回 base64 编码数据
  - 图片 MIME 二次校验：base64 编码后解码头部 re-sniff 验证实际类型
  - 超大图片自动缩放（最大 1200px、5MB 限制），渐进 JPEG 质量降级
  - 结构化参数解析：支持 `{type:"text", text:"..."}` 嵌套格式
  - 兼容 `file_path` 参数别名
  - 文本输出带行号格式，截断时提示行范围/字节数/续读偏移量
  - 新增 `image` crate 依赖（v0.25）用于图片解码和缩放
  - 工具名从 `read_file` 改为 `read`（保留 `read_file` 别名兼容）
- **`write` 工具增强**（对齐 OpenClaw）：
  - 工具名从 `write_file` 改为 `write`（保留 `write_file` 别名兼容）
  - 兼容 `file_path` 参数别名
  - 结构化参数解析：`path` 和 `content` 均支持 `{type:"text", text:"..."}` 嵌套格式
- **`edit` 工具增强**（对齐 OpenClaw）：
  - 工具名从 `patch_file` 改为 `edit`（保留 `patch_file` 别名兼容）
  - 兼容 `oldText`/`old_string`/`newText`/`new_string`/`file_path` 参数别名
  - 结构化参数解析：所有参数均支持 `{type:"text", text:"..."}` 嵌套格式
  - `new_text` 参数未提供时默认为空字符串（删除模式）
  - 写后恢复（Post-write Recovery）：两层防护
    - 写入错误恢复：写操作报错后检查文件是否已正确更新，避免假失败
    - 重复编辑恢复：old_text 不存在但 new_text 已存在时视为已应用，避免重试报错
- **`ls` 工具增强**（对齐 OpenClaw）：
  - 工具名从 `list_dir` 改为 `ls`（保留 `list_dir` 别名兼容）
  - 新增 `limit` 参数（默认 500 条）
  - 新增 50KB 输出字节上限，防止超大目录撑爆上下文
  - 支持 `~` 和 `~/` 路径展开
  - 大小写不敏感排序
  - 路径验证：检查路径存在性和是否为目录
  - 跳过无法 stat 的条目（不报错）
  - 空目录返回 "(empty directory)"
  - 兼容 `file_path` 参数别名 + 结构化参数解析
- **新增 `grep` 工具**（对齐 OpenClaw）：搜索文件内容
  - 原生 Rust 实现（`ignore` + `regex` crate），无需系统安装 ripgrep
  - 支持正则和字面量搜索（`literal` 参数）
  - 支持 `glob` 文件过滤、`ignore_case` 大小写、`context` 上下文行
  - 默认 100 条匹配限制，每行最长 500 字符，50KB 输出上限
  - 自动尊重 `.gitignore`，跳过二进制文件
- **新增 `find` 工具**（对齐 OpenClaw）：按 glob 模式查找文件
  - 原生 Rust 实现（`ignore` + `glob` crate），无需系统安装 fd
  - 默认 1000 条结果限制，50KB 输出上限
  - 自动尊重 `.gitignore`，支持 `~` 路径展开
  - 输出相对路径，匹配文件名和完整路径
- **新增 `apply_patch` 工具**（对齐 OpenClaw）：多文件补丁操作
  - 支持 `*** Begin Patch` / `*** End Patch` 格式
  - `*** Add File: <path>` — 创建新文件
  - `*** Update File: <path>` — 修改文件（`@@` 上下文 + `-`/`+` 行）
  - `*** Delete File: <path>` — 删除文件
  - `*** Move to: <path>` — 在 Update 中移动文件
  - 3-pass fuzzy matching（精确 → 去尾空白 → 全 trim），容忍空白差异
  - 不限 Provider（OpenClaw 限 OpenAI only，我们全 Provider 可用）
- **新增依赖**：`regex`、`ignore`、`glob` crate
- **命令审批对话框 UI**：前端 `ApprovalDialog` 组件
  - 监听 Tauri `approval_required` 事件，弹出全屏遮罩审批对话框
  - 显示待执行命令内容和工作目录
  - 三按钮：拒绝（红色）/ 允许一次 / 始终允许
  - 支持多请求队列（FIFO），显示队列指示器
  - 全 12 语言 i18n 支持
- **Docker 沙箱模式**：exec 新增 `sandbox` 参数，支持在 Docker 容器内隔离执行命令
  - 基于 `bollard` crate 异步 Docker API 客户端
  - 新增 `sandbox.rs` 模块：容器生命周期管理（创建 → 启动 → 等待 → 收集日志 → 清理）
  - 自动挂载工作目录到容器 `/workspace`
  - 可配置镜像（默认 `ubuntu:22.04`）、内存限制（默认 512MB）、CPU 限制（默认 1 核）
  - 配置持久化至 `~/.opencomputer/sandbox.json`
  - 支持 `background=true` + `sandbox=true` 组合
  - Docker 不可用时返回清晰错误提示，不崩溃
- **Anthropic Messages API 直接调用**：支持 Claude tool_use 流式响应与多轮 tool 循环
- **OpenAI Tool Loop**：完整的 function_call SSE 事件解析与 agent loop 实现
- **Provider Schema 适配层**：`tools.rs` 引入 `ToolProvider` 枚举，同一套 tool 定义自动转换为 Anthropic / OpenAI 格式
- **微信风格三栏布局**：图标侧边栏 + 可拖拽会话/Agent 列表 + 对话区
- **可拖拽会话面板**：会话列表面板宽度可在 180px ~ 400px 范围内拖拽调整
- **模型选择器重构**：从原生 select 改为定制的**级联菜单**（Cascading Submenu）
  - Provider 列表向上弹出可见，鼠标悬停时从右侧展开该 Provider 下的模型列表
  - 支持单模型 Provider 直接点击选中
  - 增加半透明毛玻璃背景、精致阴影、圆角列表项等对齐参考图的质感设计
- **Think 思考模式选择器优化**：同步升级为向上弹出的自定义弹层，样式与模型选择器保持一致
- **可拖拽多行输入框**：类似微信的 Textarea 输入区域，支持拖拽调整高度（80~400px）
- **图片和文件附件**：输入工具栏新增图片（📷）和文件（📎）选择按钮，支持多选
- **粘贴图片/文件**：输入框支持直接从剪贴板粘贴图片和文件
- **附件预览与删除**：已添加的附件显示在输入框上方，支持图片缩略图预览和单独删除
- **后端多模态支持**：`agent.rs` 新增 `Attachment` 结构体和三种 API 格式的图片内容构建函数（Anthropic base64 source / OpenAI Chat image_url / OpenAI Responses input_image）
- **图片消息发送**：前端读取图片为 base64 传递给 Rust 后端，后端按各 Provider API 格式构建多模态请求

### Changed
- `agent.rs` `LlmProvider` 从 2 种（Anthropic/OpenAI）扩展到 4 种（Anthropic/OpenAIChat/OpenAIResponses/Codex），全部支持自定义 base_url
- `lib.rs` `AppState` 使用 `ProviderStore` 替代独立的 codex_model 字段
- `lib.rs` `initialize_agent` 命令改为自动创建 Anthropic Provider
- `lib.rs` `finalize_codex_auth` 改为自动创建/更新内置 Codex Provider
- `App.tsx` 模型选择器改为显示 `Provider / Model` 组合格式
- `App.tsx` 侧边栏底部新增「设置」按钮，可进入 Provider 管理面板
- `App.tsx` 启动流程改为检查 Provider 列表决定显示引导页或聊天界面
- `App.tsx` 底部输入框从单行 `<Input>` 改为多行 `<textarea>`，默认 Enter 发送，Shift+Enter 换行
- `App.tsx` 顶部 Header 简化为仅显示 Agent 名称
- `agent.rs` Anthropic 调用从 `rig-core` Prompt trait 改为直接 HTTP 调用 Messages API
- `tools.rs` `ToolDefinition` 重构为 provider-agnostic 格式，新增 `to_anthropic_schema()` / `to_openai_schema()` 方法
- `LlmProvider::Anthropic` 从包装 `rig-core::Client` 改为存储 API key 字符串
- 对话界面从单栏改为三栏布局（图标侧边栏 / Agent 列表 / 对话区）

### Fixed
- 修复对话上下文丢失问题：`AssistantAgent` 新增 `conversation_history` 字段保存多轮对话历史
- 修复发送消息时出现两个气泡的问题：将独立 loading 指示器合并到 assistant 气泡中
- 修复三栏顶部分割线高度不对齐问题


## [0.2.0] - 2026-03-14

### Added
- **Codex OAuth 登录**：支持通过 ChatGPT 账号 OAuth 2.0（PKCE）登录，使用 OpenAI Codex 模型
- **多模型选择**：顶栏模型下拉菜单，支持 GPT-5.4 / GPT-5.3 Codex / GPT-5.2 / GPT-5.1 等系列模型
- **流式输出**：基于 Tauri Channel + SSE 的流式响应，实时显示 AI 回复
- **思考力度控制**：支持 None / Low / Medium / High / XHigh 五档 reasoning effort 调节
- **会话持久化与自动恢复**：OAuth token 持久化存储，启动时自动恢复上次登录状态
- **Token 自动刷新**：过期 token 自动使用 refresh_token 刷新
- **登出功能**：支持退出登录并清除本地 token
- 新增 `oauth.rs` 模块：完整的 OAuth 2.0 PKCE 流程、本地回调服务器、token 管理
- 新增 Tauri 命令：`start_codex_auth`、`check_auth_status`、`finalize_codex_auth`、`try_restore_session`、`logout_codex`、`get_codex_models`、`set_codex_model`、`set_reasoning_effort`、`get_current_settings`

### Changed
- `rig-core` 从 0.9 升级至 0.32
- `AssistantAgent` 重构为多 Provider 架构（`LlmProvider::Anthropic` / `LlmProvider::OpenAI`）
- `chat` 命令新增 `on_event: Channel<String>` 参数以支持流式输出
- `AppState` 从 `std::sync::Mutex` 迁移到 `tokio::sync::Mutex`
- `SetupScreen` 新增 Codex OAuth 登录按钮（"使用 ChatGPT 登录"）
- `index.css` 主题变量从 `@layer base :root` 迁移到 Tailwind CSS v4 的 `@theme` 语法
- `vite.config.ts` 固定开发端口为 1420

### Added (Dependencies)
- `sha2`、`base64`、`uuid`、`dirs`、`open`、`rand`、`tiny_http`、`reqwest`、`futures-util`

## [0.1.0] - 2026-03-14

### Added
- Initial scaffold: Tauri 2 + React 19 + TypeScript + Vite
- Setup screen with Anthropic API key input
- Chat screen with message history and streaming-style UX
- Rust backend with `rig-core` for Claude claude-sonnet-4-6 integration
- Tailwind CSS v4 + shadcn/ui component library
- `AssistantAgent` abstraction over Anthropic client
- Tauri commands: `initialize_agent`, `chat`

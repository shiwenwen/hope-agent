# Changelog

All notable changes to OpenComputer will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Think 等级按 Provider 差异化映射**：不同 API 类型使用各自原生的 thinking 参数格式
  - Anthropic：`thinking: { type: "enabled", budget_tokens: N }`（low→1024 / medium→4096 / high→8192 / xhigh→16384）
  - OpenAI Chat Completions：`reasoning_effort` 字段（low/medium/high，xhigh 自动降级为 high）
  - OpenAI Responses / Codex：保持现有 `reasoning.effort` 格式（支持 xhigh）
- **动态 Think 选项**：前端根据当前模型的 API 类型显示不同的 effort 选项列表
- **切换模型自动修正**：当切换到不支持当前 effort 等级的 Provider 时，自动回退到有效值
- **模型 Provider 管理系统**：支持多个自定义模型服务商，GUI 傻瓜式配置
- **24 个内置 Provider 模板**：选择模板后只需填 API Key，Base URL 和模型列表自动预填
  - 国际：Anthropic、OpenAI (Responses)、OpenAI (Chat)、DeepSeek、Google Gemini、xAI、Mistral、OpenRouter、Groq、NVIDIA、Together AI
  - 国内：Moonshot (Kimi)、Kimi Coding、通义千问、ModelStudio (DashScope)、火山引擎、智谱 AI、MiniMax、小米 MiMo、百度千帆
  - 本地：Ollama、vLLM、LM Studio
- **Provider 三步引导向导** (`ProviderSetup.tsx`)：模板网格 + 自定义入口（API 类型选择 → 连接配置 → 模型配置）
- **Provider 管理面板** (`ProviderSettings.tsx`)：查看/编辑/删除/启用禁用，从侧边栏设置按钮进入
- **三种 API 类型支持**：Anthropic Messages API、OpenAI Chat Completions、OpenAI Responses API
- **API Key 可选**：本地服务（Ollama/vLLM/LM Studio）和自定义 Provider 的 API Key 为可选项
- **OpenAI Chat Completions 流式调用**：完整的 SSE 解析和 tool calling 支持
- **OpenAI Responses API 自定义 Base URL**：可用于兼容 OpenAI API 的第三方服务
- **Provider 持久化**：配置保存至 `providers.json`，重启自动恢复
- **模型属性配置**：支持名称、输入类型(文本/图片/视频)、Context Window、Max Tokens、推理支持、成本
- **连通性测试**：添加 Provider 时可验证 API Key 和 Base URL 是否有效
- 新增 `provider.rs` 模块：`ApiType`、`ModelConfig`、`ProviderConfig` 数据结构 + JSON 持久化
- 新增 Tauri 命令：`get_providers`、`add_provider`、`update_provider`、`delete_provider`、`test_provider`、`get_available_models`、`get_active_model`、`set_active_model`、`has_providers`
- **统一 Tool Calling 支持**：Anthropic 和 OpenAI 双 Provider 均支持 tool 调用（exec、read_file、write_file、list_dir）
- **Anthropic Messages API 直接调用**：支持 Claude tool_use 流式响应与多轮 tool 循环
- **OpenAI Tool Loop**：完整的 function_call SSE 事件解析与 agent loop 实现
- **Provider Schema 适配层**：`tools.rs` 引入 `ToolProvider` 枚举，同一套 tool 定义自动转换为 Anthropic / OpenAI 格式
- **微信风格三栏布局**：图标侧边栏 + 可拖拽会话/Agent 列表 + 对话区
- **可拖拽会话面板**：会话列表面板宽度可在 180px ~ 400px 范围内拖拽调整

### Changed
- `agent.rs` `LlmProvider` 从 2 种（Anthropic/OpenAI）扩展到 4 种（Anthropic/OpenAIChat/OpenAIResponses/Codex），全部支持自定义 base_url
- `lib.rs` `AppState` 使用 `ProviderStore` 替代独立的 codex_model 字段
- `lib.rs` `initialize_agent` 命令改为自动创建 Anthropic Provider
- `lib.rs` `finalize_codex_auth` 改为自动创建/更新内置 Codex Provider
- `App.tsx` 模型选择器改为显示 `Provider / Model` 组合格式
- `App.tsx` 侧边栏底部新增「设置」按钮，可进入 Provider 管理面板
- `App.tsx` 启动流程改为检查 Provider 列表决定显示引导页或聊天界面
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

# Changelog

All notable changes to OpenComputer will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **统一 Tool Calling 支持**：Anthropic 和 OpenAI 双 Provider 均支持 tool 调用（exec、read_file、write_file、list_dir）
- **Anthropic Messages API 直接调用**：支持 Claude tool_use 流式响应与多轮 tool 循环
- **OpenAI Tool Loop**：完整的 function_call SSE 事件解析与 agent loop 实现
- **Provider Schema 适配层**：`tools.rs` 引入 `ToolProvider` 枚举，同一套 tool 定义自动转换为 Anthropic / OpenAI 格式
- **微信风格三栏布局**：图标侧边栏 + 可拖拽会话/Agent 列表 + 对话区
- **可拖拽会话面板**：会话列表面板宽度可在 180px ~ 400px 范围内拖拽调整

### Changed
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

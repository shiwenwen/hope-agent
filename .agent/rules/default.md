---
trigger: always_on
---

# OpenComputer

OpenComputer 是一款基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，支持 Anthropic Claude 和 OpenAI Codex 双 Provider。

## 项目结构

```
src/            前端（React + TypeScript）
src-tauri/src/  后端（Rust）
  lib.rs        Tauri 命令注册 & AppState
  agent.rs      AssistantAgent（多 Provider 封装）
  oauth.rs      Codex OAuth 2.0 PKCE 流程 & Token 管理
docs/           产品与技术文档
```

## 开发命令

```bash
# 启动开发模式（前端 + Tauri 热重载）
npm run tauri dev

# 仅启动前端 Vite 开发服务器
npm run dev

# 构建生产包
npm run tauri build

# 前端类型检查
npx tsc --noEmit

# Lint
npm run lint
```

## 技术栈

| 层 | 技术 |
|----|------|
| 前端框架 | React 19 + TypeScript |
| 构建工具 | Vite 8 |
| 样式 | Tailwind CSS v4 |
| 组件库 | shadcn/ui（Radix UI 底层） |
| 桌面框架 | Tauri 2 |
| 后端语言 | Rust (edition 2021) |
| LLM 层 | rig-core 0.32 |
| 异步运行时 | tokio |
| AI Provider | Anthropic Claude（API Key）/ OpenAI Codex（OAuth） |
| 默认模型 | Codex: gpt-5.4 / Anthropic: claude-sonnet-4-6 |

## 架构约定

- **前后端通信**：前端通过 `invoke()` 调用 Tauri 命令，不走 HTTP
- **流式输出**：`chat` 命令通过 Tauri `Channel<String>` 向前端推送 delta 片段
- **状态管理**：全局状态用 Tauri 的 `State<AppState>`（Rust 侧，`tokio::sync::Mutex`），前端保持轻量 React state
- **Agent 封装**：所有 LLM 调用集中在 `agent.rs` 的 `AssistantAgent`，支持 `LlmProvider::Anthropic` 和 `LlmProvider::OpenAI` 两种后端
- **OAuth 封装**：Codex 登录流程集中在 `oauth.rs`，包括 PKCE、本地回调服务器、token 持久化与刷新
- **错误处理**：Rust 命令返回 `Result<T, String>`，前端 `invoke` 用 try/catch 捕获

## 编码规范

### 前端
- 组件用函数式 + hooks，不用 class 组件
- 新 UI 组件优先使用 `src/components/ui/`（shadcn/ui 风格）
- 样式只用 Tailwind utility class，不写行内 style 和自定义 CSS（除非必要）
- 路径别名：`@/` → `src/`

### 后端（Rust）
- 新功能工具/能力放在单独的模块文件，在 `lib.rs` 中注册命令
- 使用 `anyhow::Result` 处理内部错误，在命令边界转为 `String`
- 异步命令加 `async`，tokio 运行时由 Tauri 管理，不要自己 `block_on`

## 注意事项

- `tauri.conf.json` 中 CSP 当前为 `null`，生产版本收紧前不要放行外部域名请求
- API Key 和 OAuth Token 不要在任何日志中打印
- 修改 Tauri 命令后需同步更新 `invoke_handler!` 宏注册列表
- Rust 依赖变更后需重新编译，`cargo check` 先行验证
- OAuth token 持久化在用户 config 目录，登出时调用 `clear_token()` 清除

## 文档维护

当发生以下类型的改动时，**必须同步更新对应文档**：

| 改动类型 | 需更新的文档 |
|---------|------------|
| 新增/删除功能、Tauri 命令、模块 | `CHANGELOG.md`、`docs/product-and-technical-spec.md`、`AGENTS.md` |
| 技术栈变更（依赖升级/替换） | `AGENTS.md`、`docs/product-and-technical-spec.md` |
| 架构/约定变更 | `AGENTS.md`、`docs/product-and-technical-spec.md` |
| 编码规范变更 | `AGENTS.md` |

- `CHANGELOG.md`：按 [Keep a Changelog](https://keepachangelog.com/) 格式，在 `[Unreleased]` 或新版本号下记录 Added / Changed / Removed
- `AGENTS.md`：保持与 `CLAUDE.md` 内容一致，更新后同步复制
- `docs/product-and-technical-spec.md`：更新功能清单、架构图、命令表、依赖表、路线图等
- 提交代码时将文档变更一并 commit，commit message 中注明文档更新

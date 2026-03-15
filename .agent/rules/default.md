---
trigger: always_on
---

# OpenComputer

OpenComputer 是一款基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用（桌面只是降低普通人的使用门槛，但是和系统内核要做无缝打通，底层有强大的cli能力），支持 24+ 内置 Provider 模板（Anthropic、OpenAI、DeepSeek、Moonshot、通义千问、Ollama 等），GUI 傻瓜式配置。

## 项目结构

```
src/            前端（React + TypeScript）
  components/
    MarkdownRenderer.tsx  Markdown 渲染封装（Streamdown + 代码高亮/KaTeX/Mermaid/CJK）
    ApprovalDialog.tsx    命令审批对话框（监听 approval_required 事件）
    ProviderSetup.tsx     Provider 引导向导（24+ 模板 + 自定义 + Codex OAuth）
    ProviderSettings.tsx  Provider 管理面板（查看/编辑/删除）
  i18n/
    i18n.ts               i18n 初始化 & 语言列表
    locales/*.json         12 种语言翻译文件
src-tauri/src/  后端（Rust）
  lib.rs        Tauri 命令注册 & AppState
  agent.rs      AssistantAgent（多 Provider 封装 + Tool Loop）
  tools.rs      统一 Tool 定义 & 执行 & Provider Schema 适配（exec/process/read/write/edit/ls/grep/find/apply_patch/web_search/web_fetch）
  process_registry.rs  进程会话注册表（后台进程管理）
  sandbox.rs      Docker 沙箱执行模块（bollard 异步 Docker 客户端）
  paths.rs      统一路径管理（~/.opencomputer/ 目录结构 & 数据迁移）
  provider.rs   Provider 数据模型 & JSON 持久化
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
| LLM 层 | reqwest 直接调用（Anthropic Messages / OpenAI Chat Completions / OpenAI Responses） |
| 异步运行时 | tokio |
| AI Provider | 24+ 内置模板（Anthropic / OpenAI / DeepSeek / Moonshot / Ollama 等）+ Codex OAuth + 自定义 |
| Markdown 渲染 | Streamdown（流式优化） + Shiki 代码高亮 + KaTeX 数学公式 + Mermaid 图表 + CJK 支持 |
| 多语言 | i18next + react-i18next（12 种语言，自动检测系统语言）|
| 默认模型 | Codex: gpt-5.4 / Anthropic: claude-sonnet-4-6 |

## 架构约定

- **前后端通信**：前端通过 `invoke()` 调用 Tauri 命令，不走 HTTP
- **流式输出**：`chat` 命令通过 Tauri `Channel<String>` 向前端推送 JSON 事件（`text_delta` / `tool_call` / `tool_result`）
- **模型选择器重构**：从原生 select 改为定制的**级联菜单**，支持向上伸展和右侧子菜单。
- **可拖拽多行输入框**：类似微信的 Textarea 输入区域，支持拖拽调整高度（80~400px）
- **状态管理**：全局状态用 Tauri 的 `State<AppState>`（Rust 侧，`tokio::sync::Mutex`），前端保持轻量 React state
- **Agent 封装**：所有 LLM 调用集中在 `agent.rs` 的 `AssistantAgent`，支持 4 种 `LlmProvider`（Anthropic / OpenAIChat / OpenAIResponses / Codex）
- **数据存储**：所有数据统一存储到 `~/.opencomputer/` 目录，`paths.rs` 集中管理路径。目录结构包含 `config.json`（通用配置）、`credentials/`（OAuth 凭证）、`home/`（主 Agent Home）、`share/`（共享目录）、`{name}-home/`（其他 Agent Home）
- **Provider 管理**：`provider.rs` 定义 `ProviderConfig` / `ModelConfig` / `ApiType`，支持自定义 `user_agent` 兼容 WAF，持久化至 `~/.opencomputer/config.json`
- **内置模板**：`ProviderSetup.tsx` 中 `PROVIDER_TEMPLATES` 数组包含 24 个预配置模板（API 类型参照 OpenClaw 源码），均默认使用 `claude-code/0.1.0` 作为 User-Agent。
- **统一 Tool 架构**：所有 tool 定义和执行逻辑集中在 `tools.rs`，通过 `ToolProvider` 枚举 + `to_provider_schema()` 自动适配不同 LLM 的 schema 格式。内置 11 个工具：`exec`（Shell 命令，支持 cwd/timeout/env/background/yield_ms/pty/sandbox，默认超时 1800s，login shell PATH 解析，动态输出截断，Docker 沙箱隔离）、`process`（后台进程管理：list/poll/log/write/kill/clear/remove）、`read`（自适应分页 + offset/limit 行级分页、图片自动检测与 base64 返回、MIME 二次校验、超大图片自动缩放、结构化参数解析、file_path 别名）、`write`（file_path 别名 + 结构化参数解析）、`edit`（搜索替换编辑，支持 oldText/old_string/newText/new_string/file_path 别名 + 结构化参数解析）、`ls`（~ 展开、limit 参数、50KB 输出上限、大小写不敏感排序）、`grep`（正则/字面量搜索文件内容，尊重 .gitignore，支持 glob 过滤/上下文行/大小写/100 条限制/50KB 输出上限）、`find`（按 glob 模式查找文件，尊重 .gitignore，1000 条限制/50KB 输出上限）、`apply_patch`（多文件补丁：Add/Update/Delete/Move，3-pass fuzzy matching）、`web_search`（DuckDuckGo）、`web_fetch`（URL 内容抓取）
- **进程注册表**：`process_registry.rs` 维护全局 `ProcessRegistry`（`tokio::sync::Mutex<HashMap>`），管理所有 exec 产生的后台进程会话的生命周期
- **Docker 沙箱**：`sandbox.rs` 提供 Docker 容器隔离执行，基于 `bollard` crate 异步操作。支持可配置镜像/内存/CPU 限制，配置持久化至 `~/.opencomputer/sandbox.json`
- **Tool Loop**：所有 Provider 均实现「请求 → 解析 tool_call → 执行 tool → 回传结果 → 继续」循环，最多 10 轮
- **OAuth 封装**：Codex 登录流程集中在 `oauth.rs`，包括 PKCE、本地回调服务器、token 持久化与刷新
- **Markdown 渲染**：消息内容通过 `MarkdownRenderer` 组件渲染，基于 Streamdown（专为 AI 流式场景设计），支持 GFM、代码高亮（Shiki）、KaTeX 数学公式、Mermaid 图表、CJK 标点优化。流式生成中的消息启用 `isAnimating` 动画
- **多语言 (i18n)**：使用 `i18next` + `react-i18next`，翻译文件集中在 `src/i18n/locales/`，支持 12 种语言（zh / zh-TW / en / ja / ko / tr / vi / pt / ru / ar / es / ms），默认检测系统语言，回退英文，偏好持久化到 localStorage
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
- OAuth token 持久化在 `~/.opencomputer/credentials/auth.json`，登出时调用 `clear_token()` 清除
- 应用启动时自动执行旧数据迁移（`paths::migrate_legacy_data()`），无需手动操作

## 文档维护

当发生以下类型的改动时，**必须同步更新对应文档**：

| 改动类型 | 需更新的文档 |
|---------|------------|
| 新增/删除功能、Tauri 命令、模块 | `CHANGELOG.md`、`AGENTS.md` |
| 技术栈变更（依赖升级/替换） | `AGENTS.md` |
| 架构/约定变更 | `AGENTS.md` |
| 编码规范变更 | `AGENTS.md` |

- `CHANGELOG.md`：按 [Keep a Changelog](https://keepachangelog.com/) 格式，在 `[Unreleased]` 或新版本号下记录 Added / Changed / Removed
- `AGENTS.md`：保持与 `CLAUDE.md`以及`.agent/rules/default.md` 内容一致，更新后同步复制
- 提交代码时将文档变更一并 commit，commit message 中注明文档更新

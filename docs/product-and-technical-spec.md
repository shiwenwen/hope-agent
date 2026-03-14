# OpenComputer 产品与技术方案

> 版本：0.3.0-dev | 更新日期：2026-03-14

---

## 一、产品概述

**OpenComputer** 是一款运行在本地的跨平台 AI 助手桌面应用，致力于让用户通过自然语言与自己的电脑深度交互。不同于基于浏览器的 AI 工具，OpenComputer 以原生应用形态运行，具备访问本地系统资源的能力，目标是成为用户的「个人电脑大脑」。

### 核心价值主张

- **本地优先**：数据和执行均在用户本机，隐私安全
- **系统集成**：超越对话，能够与操作系统、文件、应用程序交互
- **自带智能**：支持 Claude 和 Codex 双模型，具备强大的推理与任务规划能力
- **轻量原生**：使用 Tauri 构建，安装包小、资源占用低

---

## 二、目标用户

| 用户类型 | 核心需求 |
|---------|---------|
| 开发者 | 快速执行命令、查询文档、代码辅助 |
| 知识工作者 | 文件整理、写作助手、信息检索 |
| 普通用户 | 自然语言操作电脑，降低技术门槛 |

---

## 三、当前功能（v0.3.0-dev）

- **双 Provider 支持**：Anthropic Claude（API Key）和 OpenAI Codex（OAuth 登录）
- **统一 Tool Calling**：两个 Provider 共享同一套 tool 定义和执行逻辑（exec、read_file、write_file、list_dir），通过 schema 适配层自动转换格式
- **Codex OAuth 登录**：通过 ChatGPT 账号 OAuth 2.0 PKCE 流程登录
- **多模型选择**：顶栏下拉菜单切换 GPT-5.4 / GPT-5.3 Codex / GPT-5.2 / GPT-5.1 等模型
- **流式输出**：基于 Tauri Channel + SSE 的实时流式回复
- **思考力度控制**：支持 None / Low / Medium / High / XHigh 五档 reasoning effort
- **会话自动恢复**：OAuth token 持久化，启动时自动恢复登录状态
- **对话界面**：支持多轮对话，用户消息与 AI 回复分列展示

---

## 四、技术架构

### 技术栈总览

```
┌─────────────────────────────────────────┐
│              前端 (WebView)              │
│   React 19 + TypeScript + Vite          │
│   Tailwind CSS v4 + shadcn/ui           │
└──────────────────┬──────────────────────┘
                   │ Tauri IPC (invoke + Channel)
┌──────────────────▼──────────────────────┐
│              后端 (Rust)                 │
│   Tauri 2 + tokio + reqwest             │
│   Anthropic Messages API / Codex API    │
│   统一 Tool 执行 (tools.rs)             │
│   OAuth 2.0 PKCE (oauth.rs)             │
└─────────────────────────────────────────┘
```

### 目录结构

```
OpenComputer/
├── src/                    # 前端源码
│   ├── App.tsx             # 根组件（SetupScreen / ChatScreen）
│   ├── components/ui/      # shadcn/ui 组件库
│   └── lib/utils.ts        # 工具函数
├── src-tauri/              # Rust 后端
│   ├── src/
│   │   ├── lib.rs          # Tauri 命令注册 & AppState
│   │   ├── agent.rs        # AssistantAgent（多 Provider 封装 + Tool Loop）
│   │   ├── tools.rs        # 统一 Tool 定义 & 执行 & Provider Schema 适配
│   │   ├── oauth.rs        # Codex OAuth 2.0 PKCE & Token 管理
│   │   └── main.rs         # 程序入口
│   ├── Cargo.toml          # Rust 依赖
│   └── tauri.conf.json     # Tauri 应用配置
├── docs/                   # 项目文档
├── CHANGELOG.md
└── AGENTS.md
```

### 关键依赖

| 层级 | 依赖 | 用途 |
|------|------|------|
| 前端 | React 19 | UI 框架 |
| 前端 | Tailwind CSS v4 | 样式 |
| 前端 | shadcn/ui | 组件库 |
| 前端 | lucide-react | 图标 |
| 后端 | Tauri 2 | 原生桌面框架 |
| 后端 | reqwest | HTTP 客户端（Anthropic / Codex API） |
| 后端 | tokio | 异步运行时 |
| 后端 | futures-util | SSE 流式解析 |
| 后端 | tiny_http | OAuth 本地回调服务器 |
| 后端 | anyhow | 错误处理 |

### 数据流

```
用户输入
  → React: invoke("chat", { message, onEvent: Channel })
  → Tauri IPC
  → Rust: chat() command
  → AssistantAgent::chat()
  → Anthropic Messages API / Codex Responses API (SSE)
  → 检测 tool_call → execute_tool() → 回传结果 → 继续循环
  → Channel 推送 JSON 事件 (text_delta / tool_call / tool_result)
  → React 实时更新消息列表 + Tool 调用 UI
```

### Tauri 命令

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `initialize_agent` | `api_key: String` | `Result<()>` | 初始化 Anthropic Agent |
| `chat` | `message: String, on_event: Channel` | `Result<String>` | 流式发送消息 |
| `start_codex_auth` | — | `Result<()>` | 发起 Codex OAuth 流程 |
| `check_auth_status` | — | `Result<AuthStatus>` | 轮询 OAuth 完成状态 |
| `finalize_codex_auth` | — | `Result<()>` | 完成 OAuth 并初始化 Agent |
| `try_restore_session` | — | `Result<bool>` | 启动时恢复上次登录 |
| `logout_codex` | — | `Result<()>` | 登出并清除 token |
| `get_codex_models` | — | `Result<Vec<Model>>` | 获取可用模型列表 |
| `set_codex_model` | `model_id: String` | `Result<()>` | 切换当前模型 |
| `set_reasoning_effort` | `effort: String` | `Result<()>` | 设置思考力度 |
| `get_current_settings` | — | `Result<Settings>` | 获取当前模型和力度 |

---

## 五、规划路线图

### v0.3.0 — 工具调用与系统集成
- [x] Agent 工具调用（Tool Use）支持（双 Provider 统一架构）
- [x] 文件系统读写工具（read_file / write_file / list_dir）
- [x] Shell 命令执行工具（exec）
- [ ] 对话历史持久化（本地 SQLite）

### v0.4.0 — 体验优化
- [ ] Markdown 渲染
- [ ] 图片/文件拖拽输入
- [ ] 全局快捷键唤起

### v0.5.0 — 插件与扩展
- [ ] 插件系统设计
- [ ] MCP（Model Context Protocol）集成

### v1.0.0 — 正式版
- [ ] 自动更新
- [ ] 应用签名与公证
- [ ] 多平台安装包（macOS / Windows / Linux）

---

## 六、安全考量

- Anthropic API Key 仅存储于内存中（`AppState` Mutex），不写入磁盘
- Codex OAuth Token 持久化在用户 config 目录，登出时清除
- API Key 和 OAuth Token 不在任何日志中打印
- Tauri CSP 策略待收紧（当前为 `null`）
- Shell 工具执行需实现沙箱与用户确认机制
- 本地数据加密存储为后续版本强制要求

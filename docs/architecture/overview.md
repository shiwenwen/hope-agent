# Hope Agent 系统架构总览

> 返回 [文档索引](../README.md) | 更新时间：2026-04-28

## 系统定位

基于 Rust 的本地 AI 助手，支持三种运行模式：桌面 GUI（Tauri）、HTTP/WS 守护进程、ACP stdio。核心设计目标：**一切复杂逻辑在 ha-core**（零 Tauri 依赖），前端只负责展示和交互，Tauri 和 HTTP 服务都是薄壳。

> 三层架构详细设计见 [前后端分离架构](backend-separation.md)

## 技术栈

| 层 | 技术 |
|---|---|
| 前端 | React 19 + TypeScript, Vite 8, Tailwind CSS v4, shadcn/ui (Radix UI) |
| 前端通信 | Transport 抽象层（Tauri IPC 或 HTTP/WebSocket 双模式） |
| 桌面 | Tauri 2（薄壳，调用 ha-core） |
| 服务器 | axum 0.8（HTTP REST API + WebSocket 流式） |
| 核心 | ha-core（Rust, tokio, reqwest，零 Tauri 依赖） |
| 渲染 | Streamdown + Shiki + KaTeX + Mermaid |
| 存储 | SQLite (WAL) + FTS5 + vec0 向量扩展 |
| 多语言 | i18next (12 种语言) |

## 架构全景

```mermaid
graph TD
    subgraph Frontend["Frontend (React 19)"]
        ChatUI["ChatUI"]
        Settings["Settings"]
        Dashboard_UI["Dashboard"]
        CronUI["CronUI"]
        ChannelUI["ChannelUI"]
    end

    ChatUI & Settings & Dashboard_UI & CronUI & ChannelUI -->|"getTransport()"| Transport["Transport 抽象层"]

    Transport -->|"Tauri 模式"| IPC["Tauri IPC<br/>(invoke + Channel)"]
    Transport -->|"Web 模式"| HTTP["HTTP REST + WS<br/>(:8420)"]

    IPC --> TauriShell
    HTTP --> OcServer

    subgraph TauriShell["src-tauri (桌面薄壳)"]
        Commands["~430 Tauri Commands"]
        TauriSetup["setup.rs<br/>内嵌 HTTP 服务"]
    end

    subgraph OcServer["ha-server (HTTP/WS)"]
        Router["axum Router<br/>~430 REST 端点"]
        WSHandler["WebSocket<br/>/ws/events"]
    end

    Commands --> ChatEngine
    Router --> ChatEngine
    TauriSetup -.->|"spawn"| OcServer

    subgraph OcCore["ha-core (核心业务逻辑，零 Tauri 依赖)"]
        ChatEngine["Chat Engine"]
        ChatEngine --> Agent["Agent (4 种 API)"]
        ChatEngine --> Tools["Tools (~50 个)"]
        ChatEngine --> Memory["Memory"]
        ChatEngine --> PlanMode["Plan Mode"]
        ChatEngine --> Project["Project / Working Dir"]
        EventBus["EventBus<br/>(broadcast)"]
        Channel["Channel (12 渠道)"]
        Cron["Cron"]
        ACP["ACP (stdio)"]
        LocalLLM["Local LLM<br/>(Ollama backend)"]
        Channel & Cron & ACP --> ChatEngine
        LocalLLM -.->|"Provider 注册"| Agent
    end

    EventBus -.->|"subscriber"| IPC
    EventBus -.->|"subscriber"| WSHandler

```

> Tauri 命令、HTTP 端点、工具数量是会增长的活数据，截至 2026-04-28 分别为 ~434 / ~431 / ~49；以 [API 参考](api-reference.md) 为单一真相源，其它文档不重复维护精确数字。

## 核心数据流

### 用户消息 → 模型响应（主流程）

```mermaid
flowchart TD
    A["用户输入"] --> B["ChatEngine.run_chat_engine()"]
    B --> C["1. 构建 Agent<br/>解析 Provider + 模型链"]
    C --> D["2. 从 SessionDB<br/>恢复 conversation_history"]
    D --> E["3. 拼装 System Prompt<br/>(13 段组装)"]
    E --> F["4. Agent.chat()<br/>流式调用 LLM API"]

    F --> G["解析 tool_calls"]
    G --> H{"有 tool_calls?"}
    H -- Yes --> I["Tool Loop (默认最多 20 轮，可配置)"]
    I --> J{"concurrent_safe?"}
    J -- Yes --> K["并发安全组<br/>join_all() 并行执行"]
    J -- No --> L["串行组<br/>for loop 逐个执行"]
    K --> M["每轮结果 →<br/>compact_if_needed()<br/>(5 层渐进压缩)"]
    L --> M
    M --> G

    H -- No --> N["流式事件 → EventSink<br/>→ 前端渲染"]
    N --> O["5. 持久化<br/>assistant 消息 + tool 调用<br/>写入 SessionDB"]
    O --> P["6. 保存 context_json<br/>到 SessionDB (会话恢复)"]
    P --> Q["7. 自动记忆提取<br/>(inline, 复用 prompt cache)"]

```

### Failover 降级链

```mermaid
flowchart TD
    A["主模型请求"] --> B{"请求结果?"}
    B -- "成功" --> C["返回响应"]
    B -- "ContextOverflow" --> D["emergency_compact()"]
    D --> E["重试主模型"]
    B -- "RateLimit /<br/>Overloaded /<br/>Timeout" --> F["指数退避重试<br/>(最多 2 次)"]
    F --> G{"重试成功?"}
    G -- Yes --> C
    G -- "重试耗尽" --> H["下一模型"]
    B -- "Auth / Billing /<br/>ModelNotFound" --> I["跳过，直接下一模型"]
    H --> J{"还有模型?"}
    I --> J
    J -- Yes --> A
    J -- "全部失败" --> K["返回错误"]

```

## 模块依赖关系

```mermaid
graph LR
    ChatEngine["ChatEngine"] --> Agent["Agent"]
    Agent --> Provider["Provider (4 种 API)"]
    Provider --> Failover["Failover"]
    Agent --> ToolLoop["Tool Loop"]
    ToolLoop --> Tools["Tools (~50 个 + MCP 动态)"]
    Agent --> SideQuery["Side Query Cache"]
    Agent --> ContextCompact["Context Compact (5 层)"]

    ChatEngine --> SessionDB["Session DB<br/>(消息持久化 + FTS5)"]
    ChatEngine --> Memory["Memory<br/>(记忆注入 + 自动提取)"]
    ChatEngine --> SystemPrompt["System Prompt<br/>(13 段组装)"]
    ChatEngine --> ProjectCtx["Project / Working Dir"]
    ChatEngine --> Awareness["Behavior Awareness"]
    ChatEngine --> PlanMode["Plan Mode (六态 FSM)"]
    PlanMode --> Subagent["Subagent (spawn + inject)"]
    Subagent --> Team["Agent Team"]

    Channel["Channel"] --> ChatEngine
    Cron["Cron"] --> ChatEngine
    ACP["ACP"] --> ChatEngine
    LocalLLM["Local LLM"] -.->|"Ollama Provider 注册"| Provider
    MCP["MCP 客户端"] -.->|"动态工具命名空间"| ToolLoop
    Dashboard["Dashboard"] --> SessionDB
    Dashboard --> LogDB["Log DB"]
    Dashboard --> CronDB["Cron DB"]
    Logging["Logging"] -.->|"非阻塞双写"| AllModules["全模块"]

```

## 项目（Project）与会话工作目录

侧边栏将「会话」和「项目」并列为一等节点，项目是会话分组容器并承载持久化的项目级上下文：

- **三层文件注入**：项目目录清单恒注入；< 4KB 小文件按 8KB 预算内联；其余通过 `project_read_file` 工具按需读取（强制限制在 `project_extracted_dir` 内）
- **记忆优先级**：Project > Agent > Global，预算紧张时项目记忆最先保留；属项目的会话默认把自动提取的记忆写入 Project scope
- **默认工作目录**：`Project.working_dir` 是该项目下会话的默认工作目录；运行时合并优先级 `session.working_dir > project.working_dir > 不注入`，**lazy resolve**——改项目工作目录立即对未单独设置的已有会话生效。合并入口 `session::helpers::effective_session_working_dir`，被 system prompt、`exec` / `read` / `write` 的相对路径解析共同消费
- **IM 路由（无反向认领，Phase A1）**：项目不再认领 channel-account；IM 入站消息默认归 `project_id = NULL`，要归项目从 IM chat 内 `/project <id>` 显式触发，channel worker 调 `set_session_project` 直接改现有 session 不创建新行。Agent 解析按"显式 → 项目 → topic → group → tg-channel → channel-account → AppConfig → 默认"7 级链 (`agent::resolver::resolve_default_agent_id_full`)
- **`/project [name]` 斜杠命令**：无参列项目选择器，有参直接进入对应项目新会话
- **删除级联**：unassign 会话 → 删项目行 + `project_files`（FK）→ `rm -rf projects/{id}/` → 删项目记忆（跨 `memory.db` 单独执行）

详见 [Project 系统](project.md)。

## 本地模型加载

`local_llm/` 模块通过 Ollama 的 OpenAI 兼容端点（`http://127.0.0.1:11434/v1/chat/completions`）将本地模型注册为 Provider，启用 `allow_private_network`。模型目录硬编码 Qwen3.6 / Gemma 4 默认量化的 on-disk 大小，根据可用内存（macOS 50% 统一内存 / Windows + Linux 优先 dGPU VRAM 50%）从大到小推荐适配模型；Ollama 进程不由 app 接管。安装、模型拉取、Embedding 拉取统一走 `local_model_jobs.rs` 后台任务表，事件通道 `local_model_job:created` / `:updated` / `:log` / `:completed`。详见 [本地模型加载](local-model-loading.md)。

## 存储架构

| 数据库 | 路径 | 用途 |
|--------|------|------|
| sessions.db | `~/.hope-agent/sessions.db` | 会话、消息、Subagent/ACP/Team 运行记录 |
| memory.db | `~/.hope-agent/memory.db` | 记忆条目 + FTS5 + vec0 向量 + embedding cache |
| logs.db | `~/.hope-agent/logs.db` | 结构化日志（可查询/过滤） |
| cron.db | `~/.hope-agent/cron.db` | 定时任务 + 执行日志 |
| async_jobs.db | `~/.hope-agent/async_jobs.db` | 异步工具任务（exec / web_search / image_generate 后台化） |
| local_model_jobs.db | `~/.hope-agent/local_model_jobs.db` | 本地模型安装 / 拉取后台任务 |
| local_llm_library_cache.db | `~/.hope-agent/local_llm_library_cache.db` | Ollama Library 搜索 / Tag 元数据缓存 |
| recap/recap.db | `~/.hope-agent/recap/recap.db` | 会话深度复盘缓存 |
| canvas/canvas.db | `~/.hope-agent/canvas/canvas.db` | Canvas 画布数据 |
| config.json | `~/.hope-agent/config.json` | Provider 配置、模型链、全局设置 |
| agent.json | `~/.hope-agent/agents/{id}/agent.json` | 每 Agent 独立配置 |
| projects/ | `~/.hope-agent/projects/{id}/` | 项目工作目录（含 extracted_dir、project_files、记忆） |
| credentials/ | `~/.hope-agent/credentials/` | OAuth token、MCP server 凭据（0600 原子写） |

所有路径通过 `paths.rs` 集中管理，统一在 `~/.hope-agent/` 目录下。配置读写**强制走** `cached_config()` / `mutate_config()`，禁止重新引入 `Mutex<AppConfig>` 或 load+save 手动克隆模式（详见 [配置系统](config-system.md)）。

## 文档导航

各模块详细架构见对应文档（与 [文档索引](../README.md) 一致）：

### 系统架构

| 模块 | 文档 |
|------|------|
| 三层架构 / EventBus / Transport | [前后端分离架构](backend-separation.md) |
| Tauri / HTTP / ACP 三种入口 | [Transport 运行模式](transport-modes.md) |
| 进程清单 / Guardian 协议 | [进程与并发模型](process-model.md) |
| Tauri ↔ HTTP 命令对照 | [API 参考](api-reference.md) |

### 核心模块

| 模块 | 文档 |
|------|------|
| 对话编排 & 流式输出 | [Chat Engine](chat-engine.md) |
| Provider & Failover | [Provider 系统](provider-system.md) |
| 本地模型加载（Ollama） | [本地模型加载](local-model-loading.md) |
| 提示词 13 段组装 | [提示词系统](prompt-system.md) |
| 工具定义/执行/权限 | [工具系统](tool-system.md) |
| 上下文压缩 5 层 | [上下文压缩](context-compact.md) |
| 会话 & 消息持久化 | [Session 系统](session.md) |
| 项目容器 & 默认工作目录 | [Project 系统](project.md) |
| 记忆检索 & 提取 | [记忆系统](memory.md) |

### Agent 能力

| 模块 | 文档 |
|------|------|
| Plan 六态状态机 | [Plan Mode](plan-mode.md) |
| Ask User 结构化问答 | [Ask User](ask-user.md) |
| 技能发现 & 隔离 | [技能系统](skill-system.md) |
| 子 Agent 系统 | [Subagent](subagent.md) |
| 多 Agent 协作 | [Agent Team](agent-team.md) |
| Side Query 缓存 | [Side Query](side-query.md) |
| 跨会话行为感知 | [行为感知](behavior-awareness.md) |
| 错误分类 & Profile 轮换 | [Failover 系统](failover.md) |

### 接入层

| 模块 | 文档 |
|------|------|
| IM 渠道插件 | [IM Channel](im-channel.md) |
| ACP IDE 直连 | [ACP 协议](acp.md) |
| 斜杠命令 | [斜杠命令](slash-commands.md) |
| MCP 客户端 | [MCP 客户端](mcp.md) |

### 基础设施

| 模块 | 文档 |
|------|------|
| 图像生成 | [图像生成](image-generation.md) |
| 定时任务 | [Cron 调度](cron.md) |
| Docker 沙箱 | [Docker Sandbox](sandbox.md) |
| 数据大盘 | [Dashboard](dashboard.md) |
| Recap 深度复盘 | [Recap](recap.md) |
| 日志系统 | [Logging](logging.md) |
| 可靠性 / Guardian / Crash Journal | [可靠性与崩溃自愈](reliability.md) |
| 配置读写 contract | [配置系统](config-system.md) |
| SSRF / Dangerous Mode / 凭据 | [安全子系统](security.md) |
| 跨平台抽象层 | [跨平台抽象层](platform.md) |

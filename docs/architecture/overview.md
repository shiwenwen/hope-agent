# OpenComputer 系统架构总览

> 返回 [文档索引](../README.md) | 更新时间：2026-04-05

## 系统定位

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用。核心设计目标：**一切复杂逻辑在 Rust 后端**，前端只负责展示和交互。

## 技术栈

| 层 | 技术 |
|---|---|
| 前端 | React 19 + TypeScript, Vite 8, Tailwind CSS v4, shadcn/ui (Radix UI) |
| 桌面 | Tauri 2 (IPC via `invoke()`, 流式输出 via `Channel<String>`) |
| 后端 | Rust, tokio, reqwest |
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

    ChatUI & Settings & Dashboard_UI & CronUI & ChannelUI -->|"invoke() / Channel&lt;String&gt;"| IPC["Tauri IPC"]

    IPC --> ChatEngine

    subgraph Backend["Backend (Rust / Tokio)"]
        ChatEngine["Chat Engine (入口)<br/>请求路由 → Agent 构建 → 模型调用<br/>→ 流式输出 → 持久化"]

        ChatEngine --> ProviderSystem["Provider System<br/>(28 模板)<br/>Failover Chain"]
        ChatEngine --> Agent["Agent<br/>(4 种 API)<br/>Side Query Cache"]
        ChatEngine --> Tools["Tools<br/>(37 个)<br/>Tool Loop<br/>并发/串行"]
        ChatEngine --> ContextCompact["Context Compact<br/>(5 层)<br/>Round Grouping"]
        ChatEngine --> Memory["Memory<br/>(SQLite + FTS5<br/>+ vec0)<br/>Hybrid Search"]

        ChatEngine --> SessionDB["Session<br/>(会话 DB)<br/>FTS5 搜索"]
        ChatEngine --> PlanMode["Plan<br/>(六态 FSM)<br/>双 Agent"]
        ChatEngine --> SkillSystem["Skill<br/>(懒加载)<br/>Fork 模式"]
        ChatEngine --> Subagent["Subagent<br/>(spawn + inject)"]

        SystemPrompt["System Prompt"]
        ChatEngine --> SystemPrompt

        Channel["Channel<br/>(12 渠道)"]
        Cron["Cron<br/>(定时调度)"]
        Sandbox["Sandbox<br/>(Docker)"]
        DashboardBackend["Dashboard<br/>(聚合分析)"]

        Channel --> ChatEngine
        Cron --> ChatEngine

        Logging["Logging<br/>(双写日志)"]
        ACP["ACP<br/>(IDE 直连)"]
        ACP --> ChatEngine
    end

    style Frontend fill:#e3f2fd
    style Backend fill:#fff8e1
    style ChatEngine fill:#c8e6c9
```

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
    H -- Yes --> I["Tool Loop (最多 10 轮)"]
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

    style A fill:#e1f5fe
    style Q fill:#e8f5e9
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

    style C fill:#e8f5e9
    style K fill:#ffcdd2
```

## 模块依赖关系

```mermaid
graph LR
    ChatEngine["ChatEngine"] --> Agent["Agent"]
    Agent --> Provider["Provider (4 种 API)"]
    Provider --> Failover["Failover"]
    Agent --> ToolLoop["Tool Loop"]
    ToolLoop --> Tools["Tools (37 个)"]
    Agent --> SideQuery["Side Query Cache"]
    Agent --> ContextCompact["Context Compact (5 层)"]

    ChatEngine --> SessionDB["Session DB<br/>(消息持久化 + FTS5)"]
    ChatEngine --> Memory["Memory<br/>(记忆注入 + 自动提取)"]
    ChatEngine --> SystemPrompt["System Prompt<br/>(13 段组装)"]
    ChatEngine --> PlanMode["Plan Mode (六态 FSM)"]
    PlanMode --> Subagent["Subagent (spawn + inject)"]

    Channel["Channel"] --> ChatEngine
    Cron["Cron"] --> ChatEngine
    ACP["ACP"] --> ChatEngine
    Dashboard["Dashboard"] --> SessionDB
    Dashboard --> LogDB["Log DB"]
    Dashboard --> CronDB["Cron DB"]
    Logging["Logging"] -.->|"非阻塞双写"| AllModules["全模块"]

    style ChatEngine fill:#c8e6c9
    style Logging fill:#fff9c4
```

## 存储架构

| 数据库 | 路径 | 用途 |
|--------|------|------|
| sessions.db | `~/.opencomputer/sessions.db` | 会话、消息、Subagent/ACP 运行记录 |
| memory.db | `~/.opencomputer/memory.db` | 记忆条目 + FTS5 + vec0 向量 + embedding cache |
| logs.db | `~/.opencomputer/logs.db` | 结构化日志（可查询/过滤） |
| cron.db | `~/.opencomputer/cron.db` | 定时任务 + 执行日志 |
| config.json | `~/.opencomputer/config.json` | Provider 配置、模型链、全局设置 |
| agent.json | `~/.opencomputer/agents/{id}/agent.json` | 每 Agent 独立配置 |

所有路径通过 `paths.rs` 集中管理，统一在 `~/.opencomputer/` 目录下。

## 文档导航

各模块详细架构见对应文档：

| 模块 | 文档 |
|------|------|
| 对话编排 & 流式输出 | [Chat Engine](chat-engine.md) |
| Provider & Failover | [Provider 系统](provider-system.md) |
| 提示词 13 段组装 | [提示词系统](prompt-system.md) |
| 工具定义/执行/权限 | [工具系统](tool-system.md) |
| 上下文压缩 5 层 | [上下文压缩](context-compact.md) |
| 会话 & 消息持久化 | [Session 系统](session.md) |
| 记忆检索 & 提取 | [记忆系统](memory.md) |
| Plan 六态状态机 | [Plan Mode](plan-mode.md) |
| 技能发现 & 隔离 | [技能系统](skill-system.md) |
| IM 渠道插件 | [IM Channel](im-channel.md) |
| 图像生成 | [图像生成](image-generation.md) |
| 斜杠命令 | [斜杠命令](slash-commands.md) |
| Side Query 缓存 | [Side Query](side-query.md) |
| 子 Agent 系统 | [Subagent](subagent.md) |
| 定时任务 | [Cron 调度](cron.md) |
| Docker 沙箱 | [Docker Sandbox](sandbox.md) |
| 数据大盘 | [Dashboard](dashboard.md) |
| 日志系统 | [Logging](logging.md) |
| ACP IDE 直连 | [ACP 协议](acp.md) |

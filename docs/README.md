# Hope Agent 技术文档索引

> 项目开发指南见 [AGENTS.md](../AGENTS.md) | 更新日志见 [CHANGELOG.md](../CHANGELOG.md)

---

## 系统架构


| 文档                                            | 说明                                                                                     |
| --------------------------------------------- | -------------------------------------------------------------------------------------- |
| [系统架构总览](architecture/overview.md)            | 技术栈、架构全景图、核心数据流、模块依赖、存储架构                                                              |
| [前后端分离架构](architecture/backend-separation.md) | 三层架构设计（核心库/HTTP 服务/桌面壳）、运行模式、EventBus、Transport 层、Guardian 保活、HTTP API 端点、初始化流程、多客户端支持 |
| [Transport 运行模式](architecture/transport-modes.md) | Tauri / HTTP / ACP 三种入口、Transport 方法差异、chat streaming 路径、EventBus 事件目录 |
| [进程与并发模型](architecture/process-model.md)      | 四层进程清单：二进制运行模式 · 独立 OS 线程 · 长驻 tokio 任务 · 动态子进程；Guardian 父子协议、退出路径、排查指引 |
| [API 参考](architecture/api-reference.md) | Tauri 命令 ↔ HTTP/WS 完整对照表 + EventBus 事件清单 + Transport 方法对照 + 已知不对齐项 + 新增接口 checklist（具体计数以代码为准，文档自带验证脚本） |


---

## 核心模块


| 文档                                             | 说明                                                  | 关联源码                                           |
| ---------------------------------------------- | --------------------------------------------------- | ---------------------------------------------- |
| [Chat Engine](architecture/chat-engine.md)     | 对话编排入口、流式事件协议、Failover 集成、记忆提取门控                    | `chat_engine/`                                 |
| [Provider 系统](architecture/provider-system.md) | 4 种 API 类型、Provider 模板、Failover 策略、Thinking 系统、Provider Write Contract、Local Backend Catalog | `provider/`, `failover/`, `agent/providers/` |
| [本地模型加载](architecture/local-model-loading.md) | Ollama 本地模型搜索/下载/加载/删除、后台任务、Provider 注册、Embedding 配置与记忆向量重建 | `local_llm/`, `local_model_jobs.rs`, `local_embedding.rs`, `memory/embedding/` |
| [提示词系统](architecture/prompt-system.md)         | System Prompt 多段组装、工具描述、行为指导                        | `system_prompt/`                               |
| [工具系统](architecture/tool-system.md)            | 工具定义、Tool Loop 并发/串行执行、结果持久化、四维权限控制                 | `tools/`                                       |
| [上下文压缩](architecture/context-compact.md)       | 5 层渐进式压缩、API-Round 分组保护、后压缩文件恢复                     | `context_compact/`                             |
| [Session 系统](architecture/session.md)          | 会话 + 消息持久化、FTS5 搜索、无痕会话关闭即焚、会话级工作目录、自动会话标题、Subagent/ACP 运行记录 | `session/`, `session_title.rs`                 |
| [Project 系统](architecture/project.md)          | 会话分组容器、项目记忆/文件/指令、Bound Channel + 5 级 Agent 解析、`/project` 命令、侧边栏树状渲染 | `project/`                                     |
| [记忆系统](architecture/memory.md)                 | SQLite + FTS5 + vec0 混合检索、多模型 Embedding 配置、自动提取、Active Memory、Dreaming、Recall Summary、向量重建 | `memory/`                                      |


## Agent 能力


| 文档                                          | 说明                                | 关联源码                  |
| ------------------------------------------- | --------------------------------- | --------------------- |
| [Plan Mode](architecture/plan-mode.md)      | 六态状态机、双 Agent 模式、计划文件管理、步骤追踪      | `plan/`               |
| [Ask User](architecture/ask-user.md)        | 通用结构化问答工具、preview 并排对比、超时回退、IM 渠道集成    | `tools/ask_user_question.rs`, `plan/questions.rs`, `channel/worker/ask_user.rs` |
| [技能系统](architecture/skill-system.md)        | SKILL.md 发现、懒加载、工具隔离、Fork 模式      | `skills/`             |
| [子 Agent 系统](architecture/subagent.md)      | spawn + 结果注入、Mailbox 实时引导、深度/并发控制 | `subagent/`           |
| [Agent Team](architecture/agent-team.md)     | 多 Agent 协作团队、双向通信、Kanban 任务看板、用户自定义模板（内置模板已移除） | `team/`               |
| [Side Query 缓存](architecture/side-query.md) | 复用 prompt cache 降低侧查询成本 90%       | `agent/side_query.rs` |
| [行为感知](architecture/behavior-awareness.md) | 动态 suffix 注入、三层触发器、LLM Digest、prompt cache 双断点 | `awareness/` |
| [Failover 系统](architecture/failover.md) | 错误分类、Profile 轮换 + Cooldown + Sticky LRU、退避重试、ContextOverflow 上交 | `failover/` |


## 接入层


| 文档                                     | 说明                                            | 关联源码                   |
| -------------------------------------- | --------------------------------------------- | ---------------------- |
| [IM 渠道系统](architecture/im-channel.md)  | 12 个渠道插件（Telegram/WeChat/Discord 等）、消息路由、媒体管道 | `channel/`             |
| [ACP 协议](architecture/acp.md)          | IDE 直连（NDJSON over stdio）、会话生命周期、事件映射         | `acp/`, `acp_control/` |
| [斜杠命令](architecture/slash-commands.md) | 6 类命令、双派发路径（UI/IM）、CommandAction 副作用          | `slash_commands/`      |
| [MCP 客户端](architecture/mcp.md)         | 四种 transport（stdio/HTTP/SSE/WebSocket）、OAuth 2.1+PKCE、Resources/Prompts、凭据 0600、SSRF 硬约束、Learning 埋点 | `mcp/`                 |


## 基础设施


| 文档                                        | 说明                                 | 关联源码                    |
| ----------------------------------------- | ---------------------------------- | ----------------------- |
| [图像生成](architecture/image-generation.md)  | 7 个 Provider、Capabilities 路由、分辨率推断 | `tools/image_generate/` |
| [Cron 调度](architecture/cron.md)           | 定时任务调度、Agent 执行、Failover、指数退避      | `cron/`                 |
| [Docker Sandbox](architecture/sandbox.md) | SearXNG 容器管理、代理注入、网络隔离             | `docker/`, `sandbox.rs` |
| [Dashboard](architecture/dashboard.md)    | 跨 DB 聚合分析、成本估算、系统指标                | `dashboard/`            |
| [Recap 深度复盘](architecture/recap.md)      | 逐会话 LLM facet 提取、量化+语义融合报告、HTML 导出 | `recap/`                |
| [日志系统](architecture/logging.md)           | 非阻塞双写、敏感数据脱敏、文件轮转                  | `logging/`              |
| [可靠性与崩溃自愈](architecture/reliability.md) | Guardian 父子三层保活、退出码协议、Crash Journal、Self-Diagnosis prompt + Auto-Fix 覆盖范围、子系统 watchdog | `guardian.rs`, `crash_journal.rs`, `self_diagnosis.rs`, `service_install.rs` |
| [配置系统](architecture/config-system.md)     | `cached_config` / `mutate_config`、ArcSwap 快照、写锁串行化、`config:changed` 事件 | `config/`               |
| [安全子系统](architecture/security.md)         | SSRF 三档 policy、`trusted_hosts`、Metadata IP 硬拒、Dangerous Mode (YOLO)、HTTP 响应封顶 | `security/`             |
| [跨平台抽象层](architecture/platform.md)       | OS 适配入口集合（进程组 kill、安全文件写、shell 命令、系统代理探测、Chrome 定位、advisory lock、GPU 探测等）、Unix/Windows 双实现 | `platform/`             |


## 平台支持

| 文档                                          | 说明                                                |
| ------------------------------------------- | ------------------------------------------------- |
| [Windows 开发指南](platform/windows-development.md) | 前置环境、第一次构建、server 模式（Task Scheduler）、CI/Release、已知限制 |


---

## 计划与设计（Plans）

设计提案与开放任务跟踪，与代码同 commit 演进。

| 文档 | 说明 |
| --- | --- |
| [Review Followups](plans/review-followups.md) | 所有 code review 识别但当期不修的问题登记表（与 PR 同 commit，AGENTS.md 强制约定） |
| [Hooks System Design](plans/hooks-system-design.md) | Hooks 系统的设计文档（与 Claude Code Hook spec 对齐） |

---

## 发版说明（Release Notes）

| 版本 | 中文 | 英文 |
| --- | --- | --- |
| v0.1.0 | [v0.1.0.md](release-notes/v0.1.0.md) | [v0.1.0.en.md](release-notes/v0.1.0.en.md) |

> 任一改动需在同次提交内中英双份同步（AGENTS.md 强制约定）。

---

## 文档缺口（待补）

下列子系统已有代码 + REST endpoint，但**架构文档暂未单独成篇**，仍可通过相关文档间接了解。后续视优先级单独建文：

| 子系统 | 主源码位置 | 当前可参考的入口 |
| --- | --- | --- |
| 首次启动向导 | `crates/ha-core/src/onboarding/` | [前后端分离架构](architecture/backend-separation.md)、[进程与并发模型](architecture/process-model.md) |
| Agent 配置/解析链 | `crates/ha-core/src/agent_config.rs`、`agent_loader.rs`、`agent/resolver.rs` | [Project 系统](architecture/project.md#agent-解析链5-级)、[提示词系统](architecture/prompt-system.md) |
| Backup / Autosave | `crates/ha-core/src/backup.rs` | [配置系统](architecture/config-system.md)、[可靠性与崩溃自愈](architecture/reliability.md) |
| Canvas 子系统 | `crates/ha-core/src/canvas_db.rs`、`tools/canvas/` | [工具系统](architecture/tool-system.md) |
| Browser 子系统（CDP） | `crates/ha-core/src/browser_state.rs`、`browser_ui.rs`、`tools/browser/` | [工具系统](architecture/tool-system.md)、[跨平台抽象层](architecture/platform.md) |
| 主 LLM OAuth | `crates/ha-core/src/oauth.rs` | [Provider 系统](architecture/provider-system.md)（与 [MCP 客户端](architecture/mcp.md) 的 OAuth 实现互不共用） |
| 系统权限（macOS） | `crates/ha-core/src/permissions.rs` | [跨平台抽象层](architecture/platform.md) |
| OpenClaw 导入 | `crates/ha-core/src/openclaw_import/` | [API 参考](architecture/api-reference.md) |

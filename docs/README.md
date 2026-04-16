# OpenComputer 技术文档索引

> 项目开发指南见 [AGENTS.md](../AGENTS.md) | 更新日志见 [CHANGELOG.md](../CHANGELOG.md)

---

## 系统架构


| 文档                                            | 说明                                                                                     |
| --------------------------------------------- | -------------------------------------------------------------------------------------- |
| [系统架构总览](architecture/overview.md)            | 技术栈、架构全景图、核心数据流、模块依赖、存储架构                                                              |
| [前后端分离架构](architecture/backend-separation.md) | 三层架构设计（核心库/HTTP 服务/桌面壳）、运行模式、EventBus、Transport 层、Guardian 保活、HTTP API 端点、初始化流程、多客户端支持 |


---

## 核心模块


| 文档                                             | 说明                                                  | 关联源码                                           |
| ---------------------------------------------- | --------------------------------------------------- | ---------------------------------------------- |
| [Chat Engine](architecture/chat-engine.md)     | 对话编排入口、流式事件协议、Failover 集成、记忆提取门控                    | `chat_engine/`                                 |
| [Provider 系统](architecture/provider-system.md) | 4 种 API 类型、28 个 Provider 模板、Failover 策略、Thinking 系统 | `provider/`, `failover.rs`, `agent/providers/` |
| [提示词系统](architecture/prompt-system.md)         | System Prompt 13 段组装、32 个工具描述、行为指导                  | `system_prompt/`                               |
| [工具系统](architecture/tool-system.md)            | 工具定义、Tool Loop 并发/串行执行、结果持久化、四维权限控制                 | `tools/`                                       |
| [上下文压缩](architecture/context-compact.md)       | 5 层渐进式压缩、API-Round 分组保护、后压缩文件恢复                     | `context_compact/`                             |
| [Session 系统](architecture/session.md)          | 会话 + 消息持久化、FTS5 搜索、Subagent/ACP 运行记录                | `session/`                                     |
| [记忆系统](architecture/memory.md)                 | SQLite + FTS5 + vec0 混合检索、8 种 Embedding 提供者、自动提取    | `memory/`                                      |


## Agent 能力


| 文档                                          | 说明                                | 关联源码                  |
| ------------------------------------------- | --------------------------------- | --------------------- |
| [Plan Mode](architecture/plan-mode.md)      | 六态状态机、双 Agent 模式、计划文件管理、步骤追踪      | `plan/`               |
| [Ask User](architecture/ask-user.md)        | 通用结构化问答工具、preview 并排对比、超时回退、IM 渠道集成    | `tools/ask_user_question.rs`, `plan/questions.rs`, `channel/worker/ask_user.rs` |
| [技能系统](architecture/skill-system.md)        | SKILL.md 发现、懒加载、工具隔离、Fork 模式      | `skills/`             |
| [子 Agent 系统](architecture/subagent.md)      | spawn + 结果注入、Mailbox 实时引导、深度/并发控制 | `subagent/`           |
| [Agent Team](architecture/agent-team.md)     | 多 Agent 协作团队、双向通信、Kanban 任务看板、4 个内置模板 | `team/`               |
| [Side Query 缓存](architecture/side-query.md) | 复用 prompt cache 降低侧查询成本 90%       | `agent/side_query.rs` |
| [行为感知](architecture/behavior-awareness.md) | 动态 suffix 注入、三层触发器、LLM Digest、prompt cache 双断点 | `cross_session/` |


## 接入层


| 文档                                     | 说明                                            | 关联源码                   |
| -------------------------------------- | --------------------------------------------- | ---------------------- |
| [IM 渠道系统](architecture/im-channel.md)  | 12 个渠道插件（Telegram/WeChat/Discord 等）、消息路由、媒体管道 | `channel/`             |
| [ACP 协议](architecture/acp.md)          | IDE 直连（NDJSON over stdio）、会话生命周期、事件映射         | `acp/`, `acp_control/` |
| [斜杠命令](architecture/slash-commands.md) | 6 类命令、双派发路径（UI/IM）、CommandAction 副作用          | `slash_commands/`      |


## 基础设施


| 文档                                        | 说明                                 | 关联源码                    |
| ----------------------------------------- | ---------------------------------- | ----------------------- |
| [图像生成](architecture/image-generation.md)  | 7 个 Provider、Capabilities 路由、分辨率推断 | `tools/image_generate/` |
| [Cron 调度](architecture/cron.md)           | 定时任务调度、Agent 执行、Failover、指数退避      | `cron/`                 |
| [Docker Sandbox](architecture/sandbox.md) | SearXNG 容器管理、代理注入、网络隔离             | `docker/`, `sandbox.rs` |
| [Dashboard](architecture/dashboard.md)    | 跨 DB 聚合分析、成本估算、系统指标                | `dashboard/`            |
| [Recap 深度复盘](architecture/recap.md)      | 逐会话 LLM facet 提取、量化+语义融合报告、HTML 导出 | `recap/`                |
| [日志系统](architecture/logging.md)           | 非阻塞双写、敏感数据脱敏、文件轮转                  | `logging/`              |


---

## 调研（Research）

竞品分析与技术对比，供设计决策参考。


| 文档                                          | 说明                                                                               |
| ------------------------------------------- | -------------------------------------------------------------------------------- |
| [三项目统一维度对比 v2.1](research/unified-comparison.md) | OpenComputer vs Claude Code vs OpenClaw 全维度对比（16 维度评分 + Actionable 差距清单），基线 2026-04-15 |
| [2026 Q2 演进路线图](research/roadmap-2026q2.md) | 四阶段路线图：Phase A 架构补课 → Phase B 记忆升级 → Phase C 多 Agent 与 MCP → Phase D 体验生态补足，总计 20–26 周 |



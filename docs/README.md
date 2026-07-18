# Hope Agent 技术文档索引

> 项目开发指南见 [AGENTS.md](../AGENTS.md) | 更新日志见 [CHANGELOG.md](../CHANGELOG.md) | 发版流程见 [release-process.md](release-process.md)
>
> **面向用户的使用文档见 [user-guide/](user-guide/README.md)**（中文;English: [user-guide/en/](user-guide/en/README.md);安装上手、每项功能的用法与设置);本索引下的 `architecture/` 面向开发者,记录实现架构。

---

## 规划归档

Memory UX v2 的调研、实施路线与原始召回截图已在 2026-07-13 按任务归档；最终运行时、迁移、Prompt Cache、学习/召回边界和验收契约已沉淀到 [记忆系统架构](architecture/memory.md#memory-ux-v2-最终运行时契约)，并同步更新 Prompt、Project、Agent Config 与 Tool System 文档：

```text
/Users/shiwenwen/Library/Mobile Documents/com~apple~CloudDocs/HopeAI/Hope Agent/Plans/2026-07-13-memory-ux-v2
```

历史调研、RFC、计划方案、roadmap、review packet 和原始参考材料已按任务归档到本机 iCloud：

```text
/Users/shiwenwen/Library/Mobile Documents/com~apple~CloudDocs/HopeAI/Hope Agent/Plans/hope-agent-control-plane-plans-2026-07-05
```

仓库内 `docs/architecture/` 只保留已经实现并稳定下来的最终技术架构。重要设计决策已经沉淀到对应 architecture 文档的实现契约、非目标或后续边界中；历史规划材料、阶段 roadmap、review packet 和原始参考材料进入外部 Plans 归档。

截至 2026-07-09，Agent Control Plane V3 的 strict proof evidence 也已归档到同一 Plans 目录下的 `11-agent-control-plane-v3-claude-parity/`：5 个 required strict proof 全部 passed，最终 audit `14/14 passed`。后续若继续扩展 Goal / Loop / Workflow，应继续遵循“过程资料进入 Plans，稳定实现事实进入 architecture”的边界。

---

## 系统架构


| 文档                                            | 说明                                                                                     |
| --------------------------------------------- | -------------------------------------------------------------------------------------- |
| [系统架构总览](architecture/overview.md)            | 技术栈、架构全景图、核心数据流、模块依赖、存储架构                                                              |
| [前后端分离架构](architecture/backend-separation.md) | 三层架构设计（核心库/HTTP 服务/桌面壳）、运行模式、EventBus、Transport 层、Guardian 保活、HTTP API 端点、初始化流程、多客户端支持 |
| [Transport 运行模式](architecture/transport-modes.md) | Tauri / HTTP / ACP 三种入口、Transport 方法差异、chat streaming 路径、EventBus 事件目录 |
| [命令行接口（CLI）](architecture/cli.md)         | `hope-agent` 二进制运行模式（桌面 / server / knowledge-mcp / acp / auth）的子命令、参数、退出码、环境变量、数据目录速查 |
| [进程与并发模型](architecture/process-model.md)      | 四层进程清单：二进制运行模式 · 独立 OS 线程 · 长驻 tokio 任务 · 动态子进程；Guardian 父子协议、退出路径、排查指引 |
| [API 参考](architecture/api-reference.md) | Tauri 命令 ↔ HTTP/WS 完整对照表 + EventBus 事件清单 + Transport 方法对照 + 已知不对齐项 + 新增接口 checklist（具体计数以代码为准，文档自带验证脚本） |
| [Knowledge Agent Access](integrations/knowledge-agent-access.md) | 外部 agent 接入指南：`hope-agent knowledge-mcp`、只读 HTTP token、curl smoke 示例 |


---

## 核心模块


| 文档                                             | 说明                                                  | 关联源码                                           |
| ---------------------------------------------- | --------------------------------------------------- | ---------------------------------------------- |
| [Chat Engine](architecture/chat-engine.md)     | 对话编排入口、流式事件协议、Failover 集成、记忆提取门控                    | `chat_engine/`                                 |
| [Provider 系统](architecture/provider-system.md) | 4 种 API 类型、Provider 模板、Failover 策略、Thinking 系统、Provider Write Contract、Local Backend Catalog | `provider/`, `failover/`, `agent/providers/` |
| [模型 vs Agent 统一配置](architecture/automation-model.md) | 后台一次性 LLM 调用的统一执行原语 `crate::automation`（纯文本 `run` + 带图片 `run_vision`）、`function_models.automation` 全局默认链、真跨模型降级 + purpose 用量标签、15 个消费者字段对照（Phase 1 的 9 个 + Phase 2 的图片 OCR/知识空间维护/Sprite/笔记三件套/Recall Summary/AI 改写） | `automation/`, `recap/`, `memory/{dreaming,recall_summary}.rs`, `knowledge/{compile,source,service,types}.rs`, `knowledge/maintenance/`, `skills/auto_review/`, `hooks/runner/prompt.rs`, `sprite/`, `tools/note.rs`, `agent/mod.rs`（awareness 提取） |
| [主 LLM OAuth](architecture/llm-oauth.md) | Codex 主对话 OAuth 登录（PKCE + 本地回调）、token 刷新与并发去抖、凭据落 `auth.json`、登出清理、与 MCP OAuth 隔离 | `oauth.rs` |
| [本地模型加载](architecture/local-model-loading.md) | Ollama 本地模型搜索/下载/加载/删除、后台任务、Provider 注册、Embedding 配置与记忆向量重建 | `local_llm/`, `local_model_jobs.rs`, `local_embedding.rs`, `memory/embedding/` |
| [语音转写（STT）](architecture/stt.md)             | 独立配置语音转写引擎：8 wire 协议（OpenAI multipart / chat-completions ASR / 5 种 WebSocket）、桌面 batch + 流式会话、IM 自动转写、4 本地后端一键接入、Failover 链与 size/SSRF 红线 | `stt/`, `commands/stt.rs`, `routes/stt.rs` |
| [提示词系统](architecture/prompt-system.md)         | System Prompt 多段组装、工具描述、行为指导                        | `system_prompt/`                               |
| [工具系统](architecture/tool-system.md)            | 工具定义、Tool Loop 并发/串行执行、结果持久化、四维权限控制                 | `tools/`                                       |
| [文件操作统一](architecture/file-operations.md)     | 三处文件（Markdown 链接 / 下挂文件 / 工作台产物）统一操作策略、本机 vs 远端行为矩阵、右侧内置预览面板、preview-by-path 双壳后端与会话鉴权 | `lib/fileActions.ts`, `lib/fileKind.ts`, `components/chat/files/`, `filesystem/ops.rs` |
| [UI 交互与表面设计系统](architecture/ui-interaction-system.md) | 搜索、选择、数字输入、模型选择、焦点、菜单、悬浮弹层和 Tooltip 的组件入口、视觉 token、动效与无障碍红线 | `components/ui/`, `lib/input-modality.ts`, `lib/focus-indicator-preference.ts`, `index.css` |
| [浏览器自动化](architecture/browser.md)            | 8-action 表面、CDP / chrome-devtools-mcp 双 backend、stale-ref 自恢复、BrowserPanel 实时镜像、SSRF 守卫 | `browser/`, `tools/browser/`, `browser_state.rs`, `components/chat/BrowserPanel.tsx` |
| [macOS 控制](architecture/macos-control.md)        | 原生 macOS GUI 控制子系统：权限 readiness、AX snapshot、display/window 截图、App/窗口/元素/菜单/dialog 操作与审批分类 | `mac_control.rs`, `tools/mac_control.rs`, `src-tauri/src/macos_control.rs` |
| [上下文压缩](architecture/context-compact.md)       | 5 层渐进式压缩、API-Round 分组保护、mid-loop checkpoint、runtime ledger 与文件恢复 | `context_compact/` / `agent/context.rs`        |
| [Session 系统](architecture/session.md)          | 会话 + 消息持久化、FTS5 搜索、无痕会话关闭即焚、会话级工作目录、自动会话标题、Subagent/ACP 运行记录 | `session/`, `session_title.rs`                 |
| [Project 系统](architecture/project.md)          | 会话分组容器、项目记忆/工作目录/指令、7 级 Agent 解析、`/project` 命令、侧边栏树状渲染 | `project/`                                     |
| [Agent 配置与解析链](architecture/agent-config.md) | `agent.json` 磁盘真相源、AgentConfig 能力/记忆/委派模型、运行时装配、7 级默认 Agent 解析链、legacy `default`→`ha-main` 迁移 | `agent_config.rs`, `agent_loader.rs`, `agent/resolver.rs`, `agent/migration.rs` |
| [记忆系统](architecture/memory.md)                 | Core Memory、SQLite + FTS5 + vec0 混合检索、默认关闭的 V2 Fast/Deep Recall、自动提取、Dreaming、Recall Summary、向量重建 | `memory/`                                      |
| [Dreaming 子系统](architecture/dreaming.md)        | 离线固化 + 结构化 claim 长期记忆：数据模型、Light/Deep/Profile pipeline、V2 动态召回接入与 legacy Context Pack、Lucid Review 纠错闭环、确定性评测、owner 平面 API | `memory/dreaming/`, `memory/claims/`           |
| [知识空间（Knowledge Base）](architecture/knowledge-base.md) | 真实 `.md` 双链笔记 + index.db 可重建缓存（chunk FTS+向量 RRF/MMR 检索）、Wikilink/反链/标签/块引用、图谱视图 + transclusion、`note_*` 工具 + `effective_kb_access` 双鉴权平面、外部 vault 绑定（默认只读 / opt-in 可写）+ notify watcher、CM6 五模式编辑器、Layer 2 自主维护 | `knowledge/`, `tools/note.rs`, `components/knowledge/` |


## Agent 能力


| 文档                                          | 说明                                | 关联源码                  |
| ------------------------------------------- | --------------------------------- | --------------------- |
| [Plan Mode](architecture/plan-mode.md)      | 5 状态机、plan = 设计契约 + task = 进度真相双轨分离、enter_plan_mode 模型主动入口、Git Checkpoint 回滚 | `plan/`, `tools/enter_plan_mode.rs`, `tools/submit_plan.rs`, `tools/task.rs` |
| [Workspace Control Panel](architecture/workspace.md) | 主聊天右侧工作台：Environment / Goal / Session / Progress / Workflow / Loop / Background Jobs / Output / Sources / Knowledge / Advanced Diagnostics 的信息架构、输入框联动、多语言、UI 验收契约与 V3 strict proof 证据边界 | `components/chat/workspace/`, `components/chat/input/ChatInput.tsx` |
| [Goal 控制平面](architecture/goal.md) | 长任务顶层目标：objective、completion criteria、状态机、证据链、final audit、Goal v3 Runtime/Runner、completion report、Workflow/Loop 绑定与 Workspace Goal section | `goal/`, `workflow/`, `components/chat/workspace/` |
| [Workflow Mode、Workflow Run 与 Execution Mode](architecture/workflow.md) | Durable `workflow.js` 执行编排、WorkflowRun/Op/Event 三表、QuickJS host API、replay、permission preview、模型自主 create/status/trace/control/followup、阶段注入、pause/resume/cancel、Workspace Workflow section | `workflow/`, `execution_mode.rs`, `components/chat/workspace/` |
| [Domain Workflow 控制平面](architecture/domain-workflow.md) | Phase 7.1-7.16 通用场景模板 registry、workflow draft 预览、General Evidence 持久化、Goal evidence 链接、Context Retrieval、Domain Quality、Domain Learning、Domain Eval、Artifact Export Guard、Connector Action Guard、Phase 8.2 Connector E2E Gate 与 Phase 8.4 Workspace 通用任务工作台衔接，覆盖 Research / Writing / Data Analysis / Meeting Prep / Knowledge Curation / Inbox / Project Ops | `domain_workflow.rs`, `goal/`, `workflow/`, `context_retrieval.rs`, `components/chat/workspace/` |
| [Domain Quality 控制平面](architecture/domain-quality.md) | Phase 7.4 通用领域 review / verification：基于 Domain Workflow template、General Evidence 与 approval gates 生成 durable quality run/check/event，失败或需用户确认时写回 Goal blocking evidence，并在 Workspace「领域复核」区块展示；Phase 7.5 作为 Domain Learning 的输入信号 | `domain_quality.rs`, `goal/`, `components/chat/workspace/` |
| [Domain Eval 与 Quality Gate 控制平面](architecture/domain-eval.md) | Phase 7.6-7.14 通用领域 eval / gate / readiness，Phase 8.1-8.3 运行稳定性 Operational Gate 与跨窗口 Soak Report：基于 Goal、Workflow、Loop、Domain Evidence、Domain Quality、Domain Campaign trace 与 Connector E2E evidence 做 deterministic scoring、质量守门、可交付判断、运行稳定性判断和长期运行审计，并在 Dashboard 独立展示 | `domain_eval.rs`, `domain_quality.rs`, `domain_workflow.rs`, `components/dashboard/learning/` |
| [Loop 控制平面](architecture/loop.md) | 真实 `/loop`：复用 Cron 的可靠调度，按时间/条件/事件或 dynamic self-paced 触发原会话，记录 loop_schedules / loop_runs trace，支持模型侧 `loop_*` 工具、status / pause / resume / stop / run history / progress guard | `loop_control.rs`, `cron/`, `components/chat/workspace/` |
| [Managed Worktree 控制平面](architecture/worktree.md) | Durable git worktree 隔离环境：创建/恢复/归档/交接、Workflow 绑定执行、Subagent 隔离、WorktreeCreate/Remove hooks、Workspace GUI 控制 | `worktree.rs`, `workflow/`, `subagent/`, `components/chat/workspace/` |
| [Session Git 控制平面](architecture/git-control.md) | Session-scoped Git snapshot/diff、stage/unstage/discard、分支、commit/push、GitHub PR checks/review comments 与 Local/Worktree 安全 Handoff | `git_control.rs`, `commands/git_control.rs`, `routes/git_control.rs`, `GitControlCard.tsx`, `DiffPanel.tsx` |
| [LSP 与语义代码智能](architecture/lsp.md) | Language Server Protocol 控制面：语义导航工具、诊断缓存、文件修改后同步、动态 diagnostics prompt 后缀、Workspace 诊断面板 | `lsp.rs`, `tools/lsp.rs`, `components/chat/workspace/` |
| [Review Engine 控制平面](architecture/review-engine.md) | Durable 本地代码审查：review run/finding/event、profiles、Deep Review 降级、symbol/IDE evidence、Goal evidence、Workspace 代码审查面板与 `/review` | `review.rs`, `slash_commands/`, `components/chat/workspace/` |
| [Smart Verification 控制平面](architecture/verification-engine.md) | Durable 智能验证选择：基于 diff/项目规则推荐最小检查、后台执行低风险 step、Goal validation evidence、Workspace 验证区块 | `verification.rs`, `components/chat/workspace/` |
| [Context Retrieval v2](architecture/context-retrieval.md) | 任务感知上下文推荐与行动入口：聚合 Git diff、历史 artifacts、LSP diagnostics/symbols、Review findings、Verification steps、Goal evidence、Tasks、Workflow ops、IDE/ACP context、file search v2、URL 来源与 Phase 7.3 domain context，并在候选行触发 focused review / verification 或展示领域动作 | `context_retrieval.rs`, `lsp.rs`, `components/chat/workspace/` |
| [Coding Eval 控制面评测](architecture/coding-eval.md) | Phase 3.7-6.1 确定性 fixture harness + agent execution runner + task-level scorer + Gold Task Pack 全量自动化 + strategy effect evaluator + release/generalization gates + Benchmark Run Center：临时 git repo + 真实 session/goal/task/workflow/IDE context/improvement state，回归 Review / Smart Verification / Context Retrieval / Improvement Loop 协同质量，并可从 task prompt 执行 agent 后评估候选 diff 与策略改动效果 | `coding_eval.rs`, `coding_improvement.rs`, `tests/fixtures/coding_eval/` |
| [Coding Improvement Loop](architecture/coding-improvement-loop.md) | Phase 4.4-7.5 质量趋势与学习闭环：基于 durable Goal/Workflow/Review/Verification/Eval/transcript/Domain Quality 数据生成 trend report、workflow retro、transcript distillation、failure feedback 与 proposal 队列，可预览/应用/晋升 eval、workflow、guidance、skill、domain workflow、domain guidance、domain review profile、domain eval case 与 connector pattern 草稿，并在 Dashboard Learning 中提供 Benchmark Run Center、release/generalization gates 和全局 / 项目级学习视图 | `coding_improvement.rs`, `dashboard/coding_improvement.rs`, `coding_eval.rs`, `domain_quality.rs`, `components/chat/workspace/`, `components/dashboard/learning/` |
| [权限/审批系统](architecture/permission-system.md) | 统一规则引擎 + Default/Smart/Yolo 三模式、Plan 正交、保护路径/危险命令/编辑命令三 list、Smart judge_model + self_confidence、审批弹窗倒计时 | `permission/`, `tools/approval.rs` |
| [Hooks 系统](architecture/hooks.md)          | 事件 → 可拔插处理器，字段级对齐 Claude Code 协议；28 事件（24 触发 + 4 保留）+ 5 种 handler（command/http/mcp_tool/prompt/agent）+ user/managed/project/local 四 scope UNION + 配置热重载 + JSONL transcript 镜像 | `hooks/`, `agent/preflight.rs` |
| [Ask User](architecture/ask-user.md)        | 通用结构化问答工具、preview 并排对比、超时回退、IM 渠道集成    | `tools/ask_user_question.rs`, `plan/questions.rs`, `channel/worker/ask_user.rs` |
| [技能系统](architecture/skill-system.md)        | SKILL.md 发现、懒加载、工具隔离、Fork 模式      | `skills/`             |
| [子 Agent 系统](architecture/subagent.md)      | spawn + 结果注入、Mailbox 实时引导、深度/并发控制 | `subagent/`           |
| [后台任务（Background Jobs）](architecture/background-jobs.md) | 统一后台任务模型（Tool/Subagent/Group）：`JobManager` 门面、两层并发硬配额 + 公平调度、重试、后台 exec 审批 park、output_tail、完成合并注入、owner 面板与端点、`AsyncToolsConfig` | `async_jobs/`, `runtime_tasks.rs` |
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
| [MCP Server（平台）](architecture/mcp-server.md) | `hope-agent mcp` stdio server：把子系统暴露给外部 agent；共享 host + `ToolProvider` 注册表（design 首个 provider）、默认只读 + `--allow-writes`、active-context | `mcp_server/`, `design/mcp_provider.rs` |


## 基础设施


| 文档                                        | 说明                                 | 关联源码                    |
| ----------------------------------------- | ---------------------------------- | ----------------------- |
| [媒体生成](architecture/media-generation.md)  | 统一图/音生成服务商体系：服务商→多模型→功能默认链、数据驱动能力、统一 failover 执行器（video 模态预留） | `media_gen/` |
| [Canvas 子系统](architecture/canvas.md)     | 7 种内容类型沙盒预览、版本快照、snapshot/eval 双向通道、独立窗口、HTTP 静态托管 | `tools/canvas/`, `canvas_db.rs` |
| [设计空间（Design Space）](architecture/design-space.md) | agent 原生设计工作空间：11 类自包含产物（web/mobile/deck/dashboard/poster/document/email/image/motion/audio/component）、品牌设计系统 + token 编译、稳定单产物预览（无画布）、oid 确定性可视化微调、一键导出 HTML/PNG/PDF/PPTX/MP4/ZIP、5 维质量门 + 反 slop 自查、**工程轴**（多平台 Token 导出 / Figma 导入 / 代码交付包 / 绑定代码工程同步）、与知识空间/项目联动 | `design/`, `tools/design/`, `components/design/` |
| [Artifacts 产物平台](architecture/artifacts.md) | Canvas façade、不可变版本、Gallery、Data Analytics、离线本地导出与未来 Publisher Guard | `artifacts/`, `tools/artifact.rs`, `components/artifacts/` |
| [Cron 调度](architecture/cron.md)           | 定时任务调度、Agent 执行、Failover、指数退避      | `cron/`                 |
| [Sandbox 架构](architecture/sandbox.md) | 会话级 Docker 执行沙箱、权限放松矩阵、Docker 平台引导、SearXNG 容器管理 | `sandbox.rs`, `permission/`, `docker/` |
| [Dashboard](architecture/dashboard.md)    | 跨 DB 聚合分析、成本估算、系统指标、Learning Tab、coding release/generalization gate 与 general domain quality gate | `dashboard/`, `components/dashboard/learning/` |
| [Recap 深度复盘](architecture/recap.md)      | 逐会话 LLM facet 提取、量化+语义融合报告、HTML 导出 | `recap/`                |
| [日志系统](architecture/logging.md)           | 非阻塞双写、敏感数据脱敏、文件轮转                  | `logging/`              |
| [可靠性与崩溃自愈](architecture/reliability.md) | Guardian 父子三层保活、退出码协议、Crash Journal、Self-Diagnosis prompt + Auto-Fix 覆盖范围、子系统 watchdog | `guardian.rs`, `crash_journal.rs`, `self_diagnosis.rs`, `service_install.rs` |
| [自诊断与问题上报](architecture/self-diagnosis-issue-reporting.md) | 对话式自我理解：`ha-self-diagnosis` 技能（fork 隔离的自学习 / 排障流程）+ `issue_report` 工具，用户/会话触发、不跑后台健康扫描 | `tools/issue_report.rs`, `skills/ha-self-diagnosis/` |
| [配置系统](architecture/config-system.md)     | `cached_config` / `mutate_config`、ArcSwap 快照、写锁串行化、`config:changed` 事件 | `config/`               |
| [备份 / 自动快照](architecture/backup-autosave.md) | 配置安全网：写盘前单文件 autosave（保留 50）+ 崩溃阈值全量备份（保留 5）、reason 标签、回滚自快照、与 updater 备份隔离 | `backup.rs`             |
| [首次启动向导](architecture/onboarding.md) | 首启引导状态机、预设 Provider/模型、apply 落地经 provider CRUD + config contract、owner 命令面 | `onboarding/`           |
| [OpenClaw 导入](architecture/openclaw-import.md) | 从 OpenClaw 迁移 agents/providers/memory：扫描预览 → 选择性导入三段式、provider 去重、`MEMORY.md` 合并 + SQLite chunk 入库 | `openclaw_import/`      |
| [安全子系统](architecture/security.md)         | SSRF 三档 policy、`trusted_hosts`、Metadata IP 硬拒、Dangerous Mode (YOLO)、HTTP 响应封顶 | `security/`             |
| [跨平台抽象层](architecture/platform.md)       | OS 适配入口集合（进程组 kill、安全文件写、shell 命令、系统代理探测、Chrome 定位、advisory lock、GPU 探测、原子 binary swap 等）、Unix/Windows 双实现 | `platform/`             |
| [系统权限（macOS TCC）](architecture/macos-permissions.md) | TCC 权限探测/请求（辅助功能/录屏/自动化/麦克风/相机/文件夹）、catalog 元数据、按 id 派发原生 prompt 或设置跳转、跨平台 cfg 桩 | `permissions.rs`        |
| [自升级](architecture/self-update.md)        | 三档路径（Tauri bundle / 包管理器 / 自包含 binary swap）、Minisign 单一 pubkey、`app_update` 工具 + `ha-self-update` skill、bare-binary 发布产物 | `updater/`, `tools/app_update.rs`, `src-tauri/src/commands/update_bridge.rs` |


## 平台支持

| 文档                                          | 说明                                                |
| ------------------------------------------- | ------------------------------------------------- |
| [Windows 开发指南](platform/windows-development.md) | 前置环境、第一次构建、server 模式（Task Scheduler）、CI/Release、已知限制 |


---

## 部署（Deployment）

面向 ops 与自托管用户的部署指南。

| 文档 | 中文 | 英文 |
| --- | --- | --- |
| Docker | [docker.md](deployment/docker.md) | [docker.en.md](deployment/docker.en.md) |


---

## 发版说明（Release Notes）

逐版本发版说明（中英双份 `vX.Y.Z.md` / `vX.Y.Z.en.md`）见 [release-notes/](release-notes/) 目录。

> 任一改动需在同次提交内中英双份同步（AGENTS.md 强制约定）。完整发版流程（PR 工作流、tag 推送、cherry-pick backport、避坑速查）见 [release-process.md](release-process.md)。

> 截至当前，`docs/architecture/` 已覆盖全部有独立代码模块的核心子系统；新增子系统时按 [AGENTS.md](../AGENTS.md) 文档维护约定同步建文并登记到本索引。

# OpenComputer

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，支持 24+ 内置 Provider 模板，GUI 傻瓜式配置。

## 开发命令

```bash
npm run tauri dev      # 启动开发模式（前端 + Tauri 热重载）
npm run dev            # 仅前端 Vite 开发服务器
npm run tauri build    # 构建生产包
npx tsc --noEmit       # 前端类型检查
npm run lint           # Lint
node scripts/sync-i18n.mjs --check   # 检查各语言翻译缺失
node scripts/sync-i18n.mjs --apply   # 从翻译文件补齐缺失翻译
```

## 项目结构

```
src/                    前端（React + TypeScript）
  components/
    chat/               聊天相关组件（消息列表/输入框/审批对话框/思考块/工具调用块/Plan Mode/快捷对话浮层）
      plan-mode/          Plan Mode 组件（PlanBlock 内嵌卡片 + PlanPanel 右侧面板 + usePlanMode hook + planParser 解析器）
    settings/           设置面板（Provider/Agent/外观/语言/模型/技能/用户资料/系统）
    common/             共享组件（导航栏/Markdown 渲染/Provider 图标）
    dashboard/          数据大盘（概览卡片/Token 用量/工具使用/会话分析/异常监控/任务统计/系统监控，recharts 图表）
  lib/logger.ts         前端统一日志工具（写入后端日志系统）
  i18n/locales/         12 种语言翻译文件
  types/chat.ts         共享类型定义
src-tauri/src/          后端（Rust）
  lib.rs                Tauri 命令注册 & AppState
  agent/                AssistantAgent 模块目录（多 Provider 封装 + Tool Loop）
    mod.rs              模块声明 + 公共 API 重导出 + 构造器/setter/chat 分发器
    types.rs            核心类型（AssistantAgent/LlmProvider/Attachment/ChatUsage/CodexModel/ThinkTagFilter）
    config.rs           常量 + 系统提示词构建 + API URL 构建 + thinking 风格映射
    content.rs          多模态内容构建器（Anthropic/OpenAI Chat/Responses 三种格式）
    events.rs           前端事件发射（text_delta/tool_call/tool_result/thinking_delta/usage）
    api_types.rs        SSE/请求/响应 DTO 类型
    context.rs          上下文管理（compaction/summarization/conversation history）
    errors.rs           错误处理与重试判断
    providers/          四种 Provider 独立实现
      anthropic.rs      Anthropic Messages API + SSE 解析
      openai_chat.rs    OpenAI Chat Completions API + SSE 解析
      openai_responses.rs  OpenAI Responses API
      codex.rs          Codex OAuth API + 重试逻辑
  tools/                统一 Tool 定义 & 执行（按工具拆分为子模块，30 个内置工具）
    canvas/             画布工具（HTML/Markdown/Code/SVG/Mermaid/Chart/Slides 7 种类型，iframe 预览 + 截图反馈 + 版本历史）
    image_generate/     图片生成工具（7 个 Provider：OpenAI/Google/Fal/MiniMax/SiliconFlow/ZhipuAI/Tongyi + Capabilities 声明 + 参考图编辑 + aspectRatio/resolution + action=list + 排序降级 + 动态工具描述，条件注入）
    plan_step.rs        Plan 步骤更新工具（update_plan_step，Executing 状态条件注入，实时事件驱动 UI）
    amend_plan.rs       执行中计划修改工具（insert/delete/update 步骤，Executing/Paused 状态条件注入）
  skills.rs             技能系统（SKILL.md 发现 + 懒加载 prompt + 三层预算降级 + anyBins/always/install 等 Requirements + 调用策略 + 健康检查 + 缓存）
  slash_commands/       斜杠命令系统（命令注册表 + 解析器 + handlers 分发 + channel-agnostic 结果 + 动态 skill 命令注册）
  provider.rs           Provider 数据模型 & 持久化
  session.rs            会话持久化（SQLite）
  paths.rs              统一路径管理（~/.opencomputer/）
  failover.rs           模型降级错误分类 & 重试策略
  plan.rs               Plan Mode 系统（双 Agent 架构：Plan Agent 白名单工具 + Build Agent 全量工具 + PlanAgentConfig 声明式配置 + 六态状态机 + Plan 文件持久化 + Markdown Checklist 解析 + 步骤进度 DB 持久化 + 子 Agent 安全继承 + 5 阶段规划流程 + Completed 状态提示词 + 项目本地化存储 + 细粒度路径权限 + 计划版本管理 + Git Checkpoint 回滚）
  system_prompt.rs      系统提示词模块化拼装
  memory.rs             记忆系统（MemoryBackend trait + SQLite/FTS5 实现 + Embedding 配置 + 去重 + 批量操作 + pinned 置顶 + 导入导出）
  memory_extract.rs     自动记忆提取（对话后异步 LLM 提取 + 压缩前 Memory Flush + 去重保存 + 事件通知）
  cron.rs               定时任务系统（调度器 + CronDB + 任务执行 + 日历查询）
  dashboard.rs          数据大盘聚合查询（7 个 Tauri 命令 + SQL 聚合 + 费用估算 + DashboardFilter 多维筛选 + 系统指标采集）
  sandbox.rs            Docker 沙箱系统（安全加固容器执行 + 环境变量过滤 + 挂载路径校验 + 配置持久化 + Tauri 命令）
  tray.rs               系统托盘（菜单栏常驻图标 + 上下文菜单 + 窗口显示/隐藏 + 退出控制）
  browser_state.rs      浏览器连接状态管理（全局单例 + CDP 生命周期 + Profile 隔离）
  permissions.rs        macOS 系统权限检测 & 申请（15 项权限，JXA + 框架 API 检测）
  context_compact.rs    上下文压缩系统（4 层渐进式压缩 + Token 估算校准 + 工具结果截断 + 上下文裁剪 + LLM 摘要 + 溢出恢复）
  subagent.rs           子 Agent 系统（数据模型 + SQLite 持久化 + 异步 spawn + CancelRegistry + SteerMailbox + Tauri 事件）
  crash_journal.rs      崩溃日志（JSON 持久化 + 信号映射 + 诊断结果记录）
  backup.rs             配置备份（创建/恢复/轮转 + 增量文件备份）
  self_diagnosis.rs     自诊断系统（多 Provider Failover LLM 调用 + 基础分析降级 + 保守自动修复）
  acp/                  ACP 协议服务器（Agent Client Protocol，stdio + NDJSON + JSON-RPC 2.0，IDE 直连）
    types.rs            ACP 协议类型（JSON-RPC 基础 + 会话/Prompt/事件全量 DTO）
    protocol.rs         NDJSON 传输层（stdio 读写 + 序列化）
    event_mapper.rs     Agent 事件 → ACP 通知映射
    session.rs          ACP 会话存储（LRU 淘汰 + SessionDB 关联）
    agent.rs            ACP Agent 核心（JSON-RPC 分发 + failover + 历史重放）
    server.rs           ACP 服务器入口
  acp_control/            ACP 控制面客户端（启动/管理外部 ACP Agent）
    types.rs              AcpRuntime trait + 核心数据结构（AcpRun/AcpStreamEvent/AcpHealthStatus）
    config.rs             AcpControlConfig 全局配置 + AgentAcpConfig per-Agent 配置
    registry.rs           AcpRuntimeRegistry 后端注册表 + 自动发现（$PATH 扫描）
    runtime_stdio.rs      StdioAcpRuntime（spawn 子进程 + stdin/stdout NDJSON 交互）
    session_manager.rs    AcpSessionManager 会话生命周期（spawn/check/kill/steer）
    health.rs             后端健康检查 + 二进制探测
    events.rs             AcpStreamEvent → Tauri 全局事件映射
    mod.rs                模块声明 + 公共 re-export
```

## 技术栈

| 层 | 技术 |
|----|------|
| 前端 | React 19 + TypeScript, Vite 8, Tailwind CSS v4, shadcn/ui (Radix UI) |
| 桌面 | Tauri 2 |
| 后端 | Rust, tokio, reqwest |
| 渲染 | Streamdown + Shiki + KaTeX + Mermaid |
| 多语言 | i18next (12 种语言) |

## 架构约定

- **前后端通信**：前端通过 `invoke()` 调用 Tauri 命令，流式输出通过 `Channel<String>` 推送事件
- **状态管理**：后端用 `State<AppState>`（`tokio::sync::Mutex`），前端保持轻量 React state
- **LLM 调用**：集中在 `agent/` 模块，支持 Anthropic / OpenAIChat / OpenAIResponses / Codex 四种 Provider
- **Tool Loop**：请求 → 解析 tool_call → 执行 → 回传 → 继续，最多 10 轮
- **数据存储**：所有数据统一在 `~/.opencomputer/`，`paths.rs` 集中管理
- **降级策略**：ContextOverflow 终止 → RateLimit/Overloaded/Timeout 指数退避重试 2 次 → Auth/Billing/ModelNotFound 跳下一模型
- **连续消息合并**：`push_user_message()` 自动合并连续 user 消息，兼容 Anthropic role 交替要求
- **统一日志**：前后端日志统一写入后端 `logging.rs`（SQLite + 纯文本双写）。前端通过 `src/lib/logger.ts` 调用 `frontend_log` / `frontend_log_batch` 命令，支持批量缓冲（500ms / 20 条）。后端 Agent 执行全链路日志覆盖：chat 入口 → 模型链 → API 请求/响应（含原始 request body + response headers）→ SSE 流 → Tool 执行（参数/结果/耗时）→ 完成总结。API 请求体日志自动脱敏（`redact_sensitive`）并截断（32KB），工具执行日志截断（2KB），工具错误自动提升为 warn 级别
- **记忆系统**：三层记忆架构。**① Core Memory**（`memory.md` 文件）：全局 `~/.opencomputer/memory.md` + Agent 级 `~/.opencomputer/agents/{id}/memory.md`，全文注入系统提示词 Section ⑧（用户和 Agent 均可通过 `update_core_memory` 工具修改，支持 append/replace + global/agent scope），用于长期规则/偏好/指令。**② SQLite 记忆**：`memory.rs` 实现 `MemoryBackend` trait 可插拔架构，SQLite + FTS5 全文搜索 + sqlite-vec 向量混合检索（RRF 融合评分）。4 种记忆类型（user/feedback/project/reference），2 种作用域（global/agent），支持 `pinned` 置顶（pinned 记忆在 `build_prompt_summary` 中优先注入并带 ★ 标记，不受时间排序影响）。摘要注入系统提示词 Section ⑧（在 Core Memory 之后）。6 个工具：`save_memory`（含 pinned 参数）/ `recall_memory`（含 `include_history` 参数搜索历史会话）/ `update_memory` / `delete_memory` / `memory_get` / `update_core_memory`。Embedding 配置支持 API 模式（5 个预设）和本地 ONNX 模型（4 个预设）。去重检测（`find_similar` + `add_with_dedup`），阈值可配置。**③ Memory Flush**：`memory_extract.rs` 的 `flush_before_compact()` 在上下文压缩 Tier 3 摘要前自动提取即将被丢弃消息中的重要信息保存为记忆（后台线程 + 30 秒超时），可配置 `flushBeforeCompact`（默认关闭）。对话后自动记忆提取：异步 LLM 调用 + JSON 解析 + 去重保存 + Tauri 事件通知。**④ 历史会话搜索**：`session/db.rs` 的 `messages_fts` FTS5 虚拟表索引 user/assistant 消息，`search_messages()` 方法支持关键词搜索（排除 cron 和子 Agent 会话），通过 `recall_memory(include_history=true)` 工具调用。前端 MemoryPanel 管理 + CoreMemoryEditor 编辑核心记忆
- **数据大盘**：`dashboard.rs` 实现多维度数据分析大盘。7 个 Tauri 命令（`dashboard_overview` / `dashboard_token_usage` / `dashboard_tool_usage` / `dashboard_sessions` / `dashboard_errors` / `dashboard_tasks` / `dashboard_system_metrics`），从 SessionDB/LogDB/CronDB 聚合查询 + `sysinfo` crate 采集系统指标。`DashboardFilter` 支持时间范围/Agent/Provider/模型多维筛选，自动排除 cron 会话和子 Agent 会话。内置 20+ 模型定价表估算费用。前端 `src/components/dashboard/` 目录：`DashboardView.tsx` 主容器（Tab 切换 + 全局筛选）、`OverviewCards.tsx` 8 个指标卡片、`TokenUsageSection.tsx`（趋势折线图 + 模型饼图 + 费用表格）、`ToolUsageSection.tsx`（频次柱状图 + 耗时排行 + 详情表格）、`SessionSection.tsx`（会话趋势 + Agent 分布）、`ErrorSection.tsx`（错误/警告趋势 + 分类分布）、`TaskSection.tsx`（定时任务 + 子 Agent 统计 + 成功率环形图）、`SystemMetricsSection.tsx`（CPU 每核心柱状图 + RAM/Swap 环形图 + 网络流量柱状图 + 系统信息卡片）。侧边栏 `BarChart3` 图标入口。使用 `recharts` 图表库 + `sysinfo` crate
- **定时任务系统**：`cron.rs` 实现完整定时任务调度。3 种调度类型（At 一次性 / Every 固定间隔 / Cron 表达式），tokio 后台轮询执行，隔离 session + 模型链降级。指数退避重试 + 自动禁用。日历视图页面（侧边栏入口）+ 设置面板列表管理。Agent 工具 `manage_cron` 支持 AI 直接管理定时任务
- **Web 搜索多 Provider**：`tools/web_search.rs` 支持 8 个搜索引擎（DuckDuckGo / SearXNG / Brave / Perplexity / Google / Grok / Kimi / Tavily），enum 派发 + 自动检测。配置存储在 `config.json` 的 `webSearch` 字段，设置面板 `WebSearchPanel` 管理。SearXNG 支持 Docker 一键部署（`docker.rs`：镜像拉取 → 容器启动 → 配置注入 → 健康检查）
- **画布工具**：`tools/canvas/` 实现交互式可视化内容创作工具。统一 `canvas` 工具，11 个 action（create/update/show/hide/snapshot/eval_js/list/delete/versions/restore/export），7 种内容类型（html/markdown/code/svg/mermaid/chart/slides）。前端 `CanvasPanel.tsx` 以 iframe 沙箱渲染，嵌入 ChatScreen 右侧。截图通过 html2canvas + postMessage 实现视觉反馈循环。版本历史 SQLite 持久化（`canvas_db.rs`）。配置存储在 `config.json` 的 `canvas` 字段，设置面板 `CanvasSettingsPanel` 管理
- **图片生成**：`tools/image_generate/` 实现 AI 图片生成工具。`ImageGenProviderImpl` trait 抽象 + `resolve_provider()` 注册表，7 个内置 Provider（OpenAI/Google/Fal/MiniMax/SiliconFlow/ZhipuAI/Tongyi）。**Capabilities 声明系统**：每个 Provider 声明 generate/edit 模式能力（max_count/supports_size/supports_aspect_ratio/supports_resolution）+ 编辑能力（enabled/max_input_images）+ 几何约束（sizes/aspect_ratios/resolutions），工具入口自动校验并跳过不兼容的 Provider。**图片编辑**：`image`/`images` 参数接收参考图路径或 URL（支持本地文件/HTTP URL/data URL），Google 以 inlineData 多图注入、Fal 以 image_url + 自动追加 `/image-to-image` 路径、MiniMax 以 subject_reference 实现。**aspectRatio** 参数支持 10 种比例，**resolution** 参数支持 1K/2K/4K（编辑时从参考图自动推断）。**action=list** 返回所有 Provider 的模型和能力详情。Provider id 为 String（向后兼容自动 normalize）。配置顺序即优先级，自动降级（Failover）循环 + 指数退避重试（复用 `failover.rs`）。工具描述动态生成（列出已启用的模型和编辑能力），`model` 参数替代原 `provider` 参数（默认 auto）。配置存储在 `config.json` 的 `imageGenerate` 字段，设置面板 `ImageGeneratePanel` 管理（含排序 UI）。生成的图片保存到 `~/.opencomputer/generated-images/`，通过 `__MEDIA_URLS__` 机制返回给 LLM 实现视觉反馈
- **Web Fetch 网页抓取**：`tools/web_fetch.rs` 的 `tool_web_fetch` 使用 Mozilla Readability（`readability` crate）提取正文 + `htmd` crate 转 Markdown，支持 markdown/text 双模式。内存缓存（15 分钟 TTL / 100 条上限）、SSRF 防护（DNS 解析 + 私有 IP 拦截）、流式字节限制读取（默认 2MB）、结构化 JSON 响应。配置存储在 `config.json` 的 `webFetch` 字段，设置面板 `WebFetchPanel` 管理
- **上下文压缩系统**：`context_compact.rs` 实现 4 层渐进式上下文压缩。Tier 1 工具结果截断（head+tail，结构感知边界切割）→ Tier 2 上下文裁剪（软裁剪 + 硬替换，age×size 优先级评分）→ Tier 3 LLM 摘要（分块摘要 + 合并 + 3 级 fallback）→ Tier 4 溢出恢复（ContextOverflow 触发紧急压缩 + 自动重试）。Token 估算校准器利用 API 返回的实际 token 数做 EMA 滑动平均。15 个可配置参数存储在 `config.json` 的 `compact` 字段，设置面板 `ContextCompactPanel` 管理
- **系统消息通知**：`tauri-plugin-notification` 实现 macOS 原生桌面通知。三级粒度控制：全局开关（`config.json` 的 `notification` 字段，默认开启）→ 按 Agent 覆盖（`agent.json` 的 `notifyOnComplete`，None/true/false）→ 按定时任务开关（`cron_jobs.notify_on_complete` 列）。通知触发场景：非当前会话模型完成/异常、定时任务成功/失败。Agent 可调用 `send_notification` 工具（`tools/notification.rs`），仅在通知开启时条件注入到工具列表。前端 `src/lib/notifications.ts` 统一管理权限检查和通知发送。设置面板 `NotificationPanel` 管理
- **子 Agent 系统**：`subagent.rs` 实现 Agent 间任务委派。`subagent` 工具支持 spawn/check/list/result/kill/kill_all/steer/batch_spawn/wait_all 九种操作。非阻塞异步 spawn（`tokio::spawn`），子 Agent 在隔离 session 中运行，复用 cron 的 `build_and_run_agent` 模式（load agent → resolve model chain → failover retry）。可配置最大嵌套深度（1-5，默认 3），每个父 session 最多 5 个并发。**Steer 运行中干预**：`SubagentMailbox` 消息邮箱模式，父 Agent 可在子 Agent tool loop 每轮注入消息改变方向。**文件附件传递**：spawn 时可传递 files（utf8/base64），自动转为 Attachment 传入子 Agent。**标签系统**：每个 run 可附带 label 便于追踪定位。**深度分层工具策略**：`SubagentConfig.deniedTools` 可限制子 Agent 可用工具集。**批量操作**：batch_spawn 一次 spawn 多个任务，wait_all 等待多个 run 完成。**Token 统计**：记录 input_tokens/output_tokens 到 DB。`SubagentCancelRegistry`（`AtomicBool`）管理运行时取消。SQLite `subagent_runs` 表持久化运行记录（含 label/attachment_count/input_tokens/output_tokens）。Tauri 全局事件 `subagent_event` 实时通知前端。`SubagentConfig` per-Agent 配置（enabled/allowedAgents/deniedAgents/maxConcurrent/defaultTimeoutSecs/model/deniedTools/maxSpawnDepth/archiveAfterMinutes/announceTimeoutSecs）。系统提示词 section ⑩ 条件注入委派说明（含 steer/files/label/batch 用法）。前端 `SubagentBlock.tsx`（聊天内嵌状态，含 label/model/token 统计展示）+ `SubagentPanel.tsx`（Agent 设置面板，含深度/超时/工具策略配置）
- **技能系统**：`skills.rs` 实现完整技能发现与管理系统。SKILL.md frontmatter 格式定义技能（name/description/requires/install/invocation policy）。三层目录发现（extra dirs → managed `~/.opencomputer/skills/` → project `.opencomputer/skills/`），支持嵌套 skills/ 子目录自动检测。**懒加载 Prompt 注入**：系统提示词仅注入目录（`- name: description (read: ~/path/SKILL.md)`），LLM 按需 read 全文。**三层预算降级**：Full（名称+描述+路径）→ Compact（名称+路径）→ 二分截断，`SkillPromptBudget` 可配置（max_count/max_chars/max_file_bytes/max_candidates_per_root）。**Requirements 增强**：bins（AND）+ anyBins（OR）+ env + os + config 路径 + always 标记 + primaryEnv。**调用策略**：`user-invocable`（默认 true）控制是否注册为斜杠命令，`disable-model-invocation`（默认 false）控制是否注入 prompt。**安装引导**：`install:` 块支持 brew/node/go/uv/download 五种方式，前端一键安装 + 二进制验证。**健康检查**：`check_all_skills_status()` 返回 `SkillStatusEntry`（eligible/disabled/blocked/missing_*），前端状态徽章。**缓存**：`AtomicU64` 版本号 + 30 秒 TTL，配置变更自动 `bump_skill_version()`。**Bundled Allowlist**：`skill_allow_bundled` 限制 bundled 技能可用集。14 个 Tauri 命令（含 `get_skills_status` / `install_skill_dependency`）。前端 `SkillsPanel.tsx` 管理（列表+详情+安装+健康状态）
- **Plan Mode（计划模式）**：`plan.rs` 实现双 Agent 架构的交互式六态 Plan Mode（Off → Planning → Review → Executing → Paused → Completed）。**双 Agent 架构**：`PlanAgentConfig` 声明式配置 Plan Agent 工具白名单（read/grep/exec/plan_question/submit_plan 等），`PlanAgentMode` 枚举（Off/PlanAgent/BuildAgent）统一控制工具注入，`apply_plan_tools()` 共享方法消除 4 个 Provider 的重复代码。Planning/Review 使用 Plan Agent（白名单工具 + 路径受限 write/edit），Executing/Paused 使用 Build Agent（全量工具 + update_plan_step/amend_plan）。**Planning 阶段**：5 阶段规划流程（Deep Exploration → Requirements Clarification → Design & Architecture → Plan Composition → Review & Refinement），推荐子 Agent 并行探索代码库。细粒度路径权限：write/edit 工具在 Planning 阶段仅允许编辑计划文件（`.opencomputer/plans/`），通过 `plan_mode_allow_paths` 在 `ToolExecContext` 中传播路径白名单，其他文件全部禁止。`exec` 需用户审批（`PlanAgentConfig.ask_tools` 激活）。条件注入 `plan_question`（交互式问答，支持 `recommended` 标记 + `template` 模板分类图标）和 `submit_plan`（提交计划）两个内部工具，LLM 通过 `plan_question` 发送结构化问题（含建议选项），前端渲染可视化选择卡片（PlanQuestionBlock），用户选择/自定义输入后通过 oneshot channel 回传。**Review 阶段**：PlanPanel 只读 Markdown 渲染 + 审批/退出按钮 + 版本历史浏览与恢复，消息流中嵌入 PlanCardBlock 摘要卡片（标题/阶段数/步骤数/进度条，Phase 可展开显示步骤）。**协同评论修改**：用户选中计划文本后弹出 CommentPopover 浮层评论框，评论以引用选中内容 + 修改建议的格式发送给 LLM，前端自动转回 Planning 状态进行修订（替代原有的独立"请求修改"按钮）。**Executing 阶段**：注入计划内容 + 条件注入 `update_plan_step` 工具实时报告步骤进度 + `amend_plan` 工具支持执行中修改计划（insert/delete/update 步骤，自动重编号 + 计划文件再生成 + `plan_amended` 事件通知前端），支持暂停（→ Paused）。**Git Checkpoint 回滚**：进入 Executing 状态时自动创建 git 分支 checkpoint（`opencomputer/checkpoint-{id}-{ts}`），步骤失败后 PlanPanel 显示回滚按钮，执行 `git reset --hard` 恢复到执行前状态，成功完成后自动清理 checkpoint 分支。**Paused 阶段**：记录 `paused_at_step`，后端工具集等同 Executing，可恢复执行或回滚。**Completed 阶段**：注入 `PLAN_COMPLETED_SYSTEM_PROMPT`，引导 LLM 总结执行结果、标注失败/跳过步骤、建议后续操作。**步骤进度持久化**：`plan_steps` JSON 列持久化到 SessionDB，每次 `update_step_status` 自动写入 DB，崩溃恢复时优先从 DB 加载步骤状态。**计划版本管理**：编辑/保存计划时自动备份旧版本为 `plan-xxx-v{N}.md`，PlanMeta 维护 `version` 计数器，PlanPanel 支持版本历史浏览与一键恢复。**Plan/Build 独立模型**：`AgentModelConfig.planModel` 配置 Planning 阶段模型覆盖，使用便宜模型探索节省 60-80% 成本，前端 Agent 设置面板提供 Plan Model 选择器。**项目本地化存储**：git 仓库内计划存储到 `.opencomputer/plans/`，非 VCS 回退 `~/.opencomputer/plans/`，支持自定义 `plansDirectory` 配置覆盖，加载时自动查找全局目录兼容旧计划。**子 Agent 安全继承**：`subagent/spawn.rs` 检测父 session 计划模式状态，Planning/Review 状态下子 Agent 自动继承 `PLAN_MODE_DENIED_TOOLS`，防止工具限制逃逸。Per-session 状态管理（内存 HashMap + DB `plan_mode` + `plan_steps` 列）。`/plan` 斜杠命令（enter/exit/approve/show/pause/resume）。11 个 Tauri 命令（含 `respond_plan_question` / `get_plan_versions` / `plan_rollback` / `get_plan_checkpoint`）。Tauri 全局事件驱动前端实时更新（`plan_amended` / `plan_step_updated` / `plan_mode_changed` / `plan_submitted` / `plan_question_request`）。前端：ChatInput 工具栏 Plan 按钮（灰/蓝/紫/绿/黄五色）+ PlanQuestionBlock 交互问答卡片（含 recommended 标记 + template 图标）+ PlanCardBlock 计划摘要卡片（Phase 可展开） + PlanPanel 详情面板（Planning/Review/Executing/Paused/Completed 五种视图 + 只读 Markdown + 选中评论协同编辑 + 版本历史 + Git 回滚，计划面板仅在计划内容生成后自动展示）
- **斜杠命令系统**：`slash_commands/` 模块实现 channel-agnostic 命令系统，17 个内置命令分 6 类（Session/Model/Memory/Agent/Utility/Skill）。后端 `registry.rs` 声明式命令注册表，`parser.rs` 文本解析（`/command args` 格式），`handlers/` 按类别拆分（session.rs/model.rs/memory.rs/agent.rs/utility.rs），dispatch 模式分发执行。**Skill 命令动态注册**：`user-invocable` 的技能自动注册为 Skill 分类的斜杠命令，名称规范化（小写+去重+32 字符截断）。支持 `command-dispatch: tool` + `command-tool` 绑定特定工具直接调用。3 个 Tauri 命令（`list_slash_commands` / `execute_slash_command` / `is_slash_command`），返回 `CommandResult`（content 文本 + `CommandAction` 枚举），各 channel（桌面端/Telegram/Discord 等）根据 action 类型执行对应副作用。前端 `SlashCommandMenu.tsx` 弹出菜单（按分类分组、键盘导航、模糊过滤，支持 `descriptionRaw` 直接展示技能描述），`useSlashCommands.ts` hook 管理输入检测和执行，`ChatInput.tsx` 集成 "/" 按钮和键盘拦截。模型切换支持模糊匹配（exact → prefix → contains），Agent 切换自动创建新 session。`/export` 导出为 Markdown（后端生成内容 + 前端 save dialog + `write_export_file`），`/search` 作为 PassThrough 注入给 LLM
- **Docker 沙箱系统**：`sandbox.rs` 实现安全加固的 Docker 容器沙箱执行。`exec` 工具 `sandbox=true` 参数或 Agent `behavior.sandbox` 配置触发。默认镜像 `debian:bookworm-slim`。安全加固：只读根文件系统（`--read-only`）+ capability 全部移除（`--cap-drop ALL`）+ 禁止新权限（`--no-new-privileges`）+ 网络隔离（`--network none`）+ 进程数限制（`--pids-limit 256`）+ tmpfs 可写临时目录。环境变量过滤：`sanitize_env()` 拦截 20+ 种敏感变量模式（API_KEY/TOKEN/SECRET/PASSWORD 等），白名单放行 PATH/HOME/LANG 等。挂载路径校验：`validate_bind_mount()` 禁止挂载 `/etc`、`/proc`、`/sys`、`/dev`、`/root`、Docker socket 等系统路径，canonicalize 防 symlink 逃逸。`SandboxConfig` 持久化在 `~/.opencomputer/sandbox.json`（8 个可配置参数）。系统提示词 Section ⑪ 条件注入沙箱说明。设置面板 `SandboxPanel` 管理（Docker 可用性检测 + 镜像/资源/安全配置）。3 个 Tauri 命令（`get_sandbox_config` / `set_sandbox_config` / `check_sandbox_available`）
- **ACP 协议支持**：`acp/` 模块实现原生 Agent Client Protocol 服务器，IDE（Zed/VS Code 等）通过 stdio + NDJSON（JSON-RPC 2.0）直连 OpenComputer Agent。`opencomputer acp` 子命令启动（`--verbose`/`--agent-id`）。完整会话生命周期（new/load/list/close）+ prompt 执行（流式事件映射）+ 历史重放（loadSession 从 SessionDB 重建完整对话）+ 多 Agent 模式切换 + failover 降级。共享 SessionDB 实现桌面端与 IDE 会话互通
- **自愈式自动重启**：`main.rs` 实现 Guardian Process 架构，同一二进制通过 `OPENCOMPUTER_CHILD` 环境变量区分 Guardian/Child 模式。Guardian 监控子进程退出码，捕获所有崩溃类型（panic/segfault/OOM/abort），指数退避重启。连续崩溃 5 次触发 `backup.rs` 配置备份 + `self_diagnosis.rs` LLM 自诊断（多 Provider Failover + 基础分析降级），保守自动修复（仅 config/logs.db 损坏）。崩溃记录持久化到 `crash_journal.json`（JSON 格式，最近 50 条）。信号转发确保 Force Quit 不误判。退出码：0=正常、42=请求重启、其他=崩溃。设置面板 `CrashHistoryPanel` 管理崩溃历史和备份
- **系统托盘常驻**：`tray.rs` 实现系统托盘（菜单栏）常驻。关闭主窗口仅隐藏（`on_window_event` 拦截 `CloseRequested` + `prevent_close`），应用在后台持续运行。托盘菜单提供显示主窗口/快捷对话/新建对话/设置/退出五个操作。左键单击托盘图标直接显示主窗口。macOS 点击 Dock 图标通过 `RunEvent::Reopen` 恢复窗口。Tauri 2 内置 `tray-icon` feature，无需额外插件
- **快捷对话快捷键**：全局 Option+Space（Alt+Space）快捷键快速唤起 Spotlight 风格浮动对话框。`tauri-plugin-global-shortcut` 后端注册快捷键，Rust handler 显示/聚焦主窗口并发射 `quick-chat-toggle` 事件。前端 `QuickChatDialog.tsx` 浮层组件（`createPortal` 渲染到 body）+ `useQuickChatSession.ts` 独立会话管理 Hook + `QuickChatMessages.tsx` 简化消息列表。复用 `ChatInput` 完整功能（模型选择/斜杠命令/文件附件）和 `useChatStream` 流式对话。Agent 快捷选择器支持切换 Agent 并自动保存/恢复会话（localStorage 持久化 `quickchat:lastSession:{agentId}`）。连续唤起加载上次会话，支持新建会话和"查看完整对话"跳转

## Plan Mode 前后端时序流程

### 1. Planning 阶段（制定计划）

```
用户                    前端（React）                  后端（Rust）                  LLM
 │                        │                              │                           │
 ├── 点击 Plan 按钮 ──────►│                              │                           │
 │                        ├── invoke("set_plan_mode",    │                           │
 │                        │     {state:"planning"}) ─────►├── set_plan_state()        │
 │                        │                              │   + persist DB             │
 │                        ├── setShowPanel(true)          │                           │
 │                        │                              │                           │
 ├── 输入需求 ────────────►│                              │                           │
 │                        ├── invoke("chat") ────────────►│                           │
 │                        │                              ├── 注入 PLAN_MODE_SYSTEM_PROMPT
 │                        │                              ├── 设置 denied_tools       │
 │                        │                              │   (write/edit 仅允许 plans/ 路径)
 │                        │                              ├── 设置 ask_tools (exec)   │
 │                        │                              ├── 注入 plan_question +    │
 │                        │                              │   submit_plan 工具定义     │
 │                        │                              ├── SSE 请求 ───────────────►│
 │                        │                              │                           │
 │                        │                              │◄── tool_call: plan_question│
 │                        │                              ├── 解析问题 + oneshot channel
 │                        │◄── emit("plan_question_request")│                        │
 │◄── PlanQuestionBlock ──│                              │                           │
 │    (可视化选择卡片)     │                              │                           │
 ├── 选择/输入答案 ───────►│                              │                           │
 │                        ├── invoke("respond_plan_question")──►│                    │
 │                        │                              ├── oneshot.send(answers)    │
 │                        │                              ├── 回传 LLM ──────────────►│
 │                        │                              │   ...（可能多轮 Q&A）...   │
 │                        │                              │◄── tool_call: submit_plan  │
 │                        │                              ├── save_plan_file()         │
 │                        │                              ├── parse_plan_steps()       │
 │                        │                              ├── persist_steps_to_db()    │
 │                        │◄── emit("plan_submitted") ───│                           │
 │                        ├── setPlanState("review")     │                           │
 │                        ├── setPlanSteps(steps)        │                           │
```

### 2. Review 阶段（审查计划）

```
用户                    前端（React）                  后端（Rust）
 │                        │                              │
 │◄── PlanPanel 自动展示 ─│                              │
 │    (只读 Markdown +    │                              │
 │     选中评论 + 审批)    │                              │
 │                        │                              │
 │── [选项 A] 批准 ───────►│                              │
 │                        ├── invoke("set_plan_mode",    │
 │                        │     {state:"executing"}) ────►├── create_git_checkpoint()
 │                        │                              │   → branch: opencomputer/checkpoint-xxx
 │                        │                              ├── set_plan_state(Executing)
 │                        ├── handleSend("执行计划")     │
 │                        │                              │
 │── [选项 B] 选中文本 ──►│                              │
 │   + 弹出评论框         │                              │
 │── 输入评论并发送 ──────►├── 格式化: "> 选中内容\n评论"  │
 │                        ├── invoke("set_plan_mode",    │
 │                        │     {state:"planning"}) ─────►├── set_plan_state(Planning)
 │                        ├── handleSend(feedback) ──────►├── 将反馈发送给 LLM 修订
 │                        │                              │
 │── [选项 C] 版本历史 ──►│                              │
 │                        ├── invoke("get_plan_versions")►├── list_plan_versions()
 │                        │◄── versions[] ───────────────│
 │◄── 版本列表 ───────────│                              │
 │── 恢复旧版本 ──────────►│                              │
 │                        ├── invoke("restore_plan_version")►├── load + save + re-parse
```

### 3. Executing 阶段（执行计划）

```
用户                    前端（React）                  后端（Rust）                  LLM
 │                        │                              │                           │
 │                        │                              ├── 注入 PLAN_EXECUTING_SYSTEM_PROMPT
 │                        │                              │   + 计划内容               │
 │                        │                              ├── 注入 update_plan_step   │
 │                        │                              │   + amend_plan 工具定义    │
 │                        │                              ├── SSE 请求 ──────────────►│
 │                        │                              │                           │
 │                        │                              │◄── tool_call: update_plan_step
 │                        │                              │    (step_index=0, status="in_progress")
 │                        │                              ├── update_step_status()     │
 │                        │                              ├── persist_steps_to_db()    │
 │                        │◄── emit("plan_step_updated")─│                           │
 │◄── 步骤 0 变为进行中 ──│                              │                           │
 │                        │                              │◄── tool_call: write/edit   │
 │                        │                              ├── execute_tool() 正常执行   │
 │                        │                              │◄── tool_call: update_plan_step
 │                        │                              │    (step_index=0, status="completed")
 │                        │                              ├── update_step_status()     │
 │                        │◄── emit("plan_step_updated")─│                           │
 │◄── 步骤 0 变为已完成 ──│                              │                           │
 │                        │                              │   ...（重复直到所有步骤）   │
 │                        │                              │                           │
 │                        │                              │◄── [可选] tool_call: amend_plan
 │                        │                              │    (action="insert", title="新步骤")
 │                        │                              ├── 插入步骤 + 重编号        │
 │                        │                              ├── 再生成计划文件            │
 │                        │◄── emit("plan_amended") ─────│                           │
 │◄── 步骤列表刷新 ───────│                              │                           │
 │                        │                              │                           │
 │                        │                              ├── all_terminal() == true   │
 │                        │                              ├── set_plan_state(Completed)│
 │                        │◄── emit("plan_mode_changed", │                           │
 │                        │     state:"completed") ──────│                           │
 │◄── 显示完成状态 ───────│                              │                           │
```

### 4. 失败回滚流程

```
用户                    前端（React）                  后端（Rust）
 │                        │                              │
 │◄── 步骤 N 失败（红色） │                              │
 │◄── 回滚按钮出现 ───────│                              │
 │                        ├── invoke("get_plan_checkpoint")►├── get_checkpoint_ref()
 │                        │◄── "opencomputer/checkpoint-xxx"│
 │                        ├── setHasCheckpoint(true)     │
 │                        │                              │
 │── 点击"回滚更改" ─────►│                              │
 │                        ├── invoke("plan_rollback") ───►├── rollback_to_checkpoint()
 │                        │                              │   → git reset --hard <checkpoint>
 │                        │                              │   → git branch -D <checkpoint>
 │                        │◄── "Rolled back from abc to  │
 │                        │     checkpoint 'xxx'" ───────│
 │◄── 代码恢复到执行前 ──│                              │
```

### 5. 子 Agent 安全继承

```
主 Agent (Planning 状态)    subagent/spawn.rs             子 Agent
 │                              │                           │
 ├── tool_call: subagent ──────►│                           │
 │   (spawn, task="探索代码")   │                           │
 │                              ├── 检测父 session 状态     │
 │                              │   = Planning              │
 │                              ├── 合并 denied_tools:      │
 │                              │   config.deniedTools       │
 │                              │   + PLAN_MODE_DENIED_TOOLS │
 │                              ├── spawn 子 Agent ─────────►│
 │                              │                           ├── 工具列表不含 write/edit
 │                              │                           ├── 无法修改文件 ✓ 安全
```

## 编码规范

### 通用
- **性能和用户体验是最高优先级**
- **核心逻辑必须在 Rust 后端实现**：业务逻辑、数据处理、文件 IO、状态管理、算法计算等核心逻辑一律放在 `src-tauri/` 后端，通过 Tauri 命令暴露给前端。前端只负责展示和交互，不承载任何业务逻辑。
- 操作即时反馈（乐观更新、loading 态），动效 60fps（优先 CSS transform/opacity）

### 前端
- 函数式组件 + hooks，不用 class 组件
- UI 组件统一用 `src/components/ui/`（shadcn/ui），不直接用 HTML 原生表单组件
- 样式只用 Tailwind utility class，不写行内 style 和自定义 CSS
- 动效优先复用 shadcn/ui、Radix UI、Tailwind 内置 utility，确认不够用才手写
- 路径别名：`@/` → `src/`
- 布局避免硬编码过小的 max-width（如 `max-w-md`），使用 `max-w-4xl` 以上或弹性伸缩
- **i18n 功能实现时只需实现中文（zh）和英文（en）**，其余语言通过单独的任务进行补齐，`scripts/sync-i18n.mjs` 统一补齐（翻译数据在 `scripts/i18n-translations.json`）
- 避免不必要的重渲染（`React.memo`、`useMemo`、`useCallback`）
- **Tooltip 必须使用 `@/components/ui/tooltip`**，禁止用 HTML 原生 `title` 属性（延迟过长，体验不一致）。优先使用 `<IconTip label={...}>` 简洁包裹，`TooltipProvider` 已内置默认延迟参数，无需手动传递
- **保存按钮统一三态交互**：所有设置面板的保存按钮必须实现三个状态——① 点击后 `saving`：显示 `Loader2` 旋转动画 + `t("common.saving")`，按钮 disabled；② 成功 `saved`：按钮变绿色（`bg-green-500/10 text-green-600`）+ `Check` 图标 + `t("common.saved")`，2 秒后恢复；③ 失败 `failed`：按钮变红色（`bg-destructive/10 text-destructive`）+ `t("common.saveFailed")`，2 秒后恢复。使用 `saveStatus: "idle" | "saved" | "failed"` + `saving: boolean` 两个状态变量管理

### 后端（Rust）
- 新功能放单独模块文件，在 `lib.rs` 注册命令
- 内部用 `anyhow::Result`，命令边界转为 `String`
- 异步命令加 `async`，不要自己 `block_on`
- **禁止使用 `log::info!` / `log::warn!` / `log::error!` / `log::debug!` 等 `log` crate 宏**，必须使用项目统一日志宏 `app_info!` / `app_warn!` / `app_error!` / `app_debug!`（定义在 `logging.rs`），以确保日志同时写入 SQLite 和日志文件。`log` crate 只输出到控制台（stderr），不会写入日志文件。唯一例外：`lib.rs` 的 `run()` 函数中 `AppLogger` 初始化之前的启动阶段代码，以及 `main.rs` 的 panic 恢复代码
- 日志宏用法：`app_info!("category", "source", "message {}", arg)`，category 为功能分类（如 `cron`/`tool`/`agent`），source 为具体来源（如 `scheduler`/`exec`/`codex`）
- **禁止对字符串使用字节索引切片**（如 `&s[..80]`），中文等多字节字符会导致 panic。必须使用 `crate::truncate_utf8(s, max_bytes)` 进行安全截断（定义在 `lib.rs`）

## 安全红线

- **API Key 和 OAuth Token 禁止出现在任何日志中**
- `tauri.conf.json` CSP 当前为 `null`，不要放行外部域名
- OAuth token 在 `~/.opencomputer/credentials/auth.json`，登出时必须 `clear_token()`

## 易错提醒

- 修改 Tauri 命令后须同步更新 `invoke_handler!` 宏注册列表
- Rust 依赖变更后 `cargo check` 先行验证

## 文档维护

代码改动时**必须同步更新文档**：

| 改动类型 | 需更新 |
|---------|--------|
| 新增/删除功能、命令、模块 | `CHANGELOG.md`、`AGENTS.md` |
| 技术栈/架构/规范变更 | `AGENTS.md` |

- `CHANGELOG.md`：[Keep a Changelog](https://keepachangelog.com/) 格式
- `AGENTS.md`保持与 `CLAUDE.md` 及 `.agent/rules/default.md` 一致，当任意一个文件更新时，其他两个文件也需要更新

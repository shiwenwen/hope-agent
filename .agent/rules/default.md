---
trigger: always_on
---

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
    chat/               聊天相关组件（消息列表/输入框/审批对话框/思考块/工具调用块）
    settings/           设置面板（Provider/Agent/外观/语言/模型/技能/用户资料/系统）
    common/             共享组件（导航栏/Markdown 渲染/Provider 图标）
  lib/logger.ts         前端统一日志工具（写入后端日志系统）
  i18n/locales/         12 种语言翻译文件
  types/chat.ts         共享类型定义
src-tauri/src/          后端（Rust）
  lib.rs                Tauri 命令注册 & AppState
  agent.rs              AssistantAgent（多 Provider 封装 + Tool Loop）
  tools/                统一 Tool 定义 & 执行（按工具拆分为子模块）
  provider.rs           Provider 数据模型 & 持久化
  session.rs            会话持久化（SQLite）
  paths.rs              统一路径管理（~/.opencomputer/）
  failover.rs           模型降级错误分类 & 重试策略
  system_prompt.rs      系统提示词模块化拼装
  memory.rs             记忆系统（MemoryBackend trait + SQLite/FTS5 实现 + Embedding 配置）
  cron.rs               定时任务系统（调度器 + CronDB + 任务执行 + 日历查询）
  browser_state.rs      浏览器连接状态管理（全局单例 + CDP 生命周期 + Profile 隔离）
  permissions.rs        macOS 系统权限检测 & 申请（15 项权限，JXA + 框架 API 检测）
  context_compact.rs    上下文压缩系统（4 层渐进式压缩 + Token 估算校准 + 工具结果截断 + 上下文裁剪 + LLM 摘要 + 溢出恢复）
  subagent.rs           子 Agent 系统（数据模型 + SQLite 持久化 + 异步 spawn + CancelRegistry + SteerMailbox + Tauri 事件）
  crash_journal.rs      崩溃日志（JSON 持久化 + 信号映射 + 诊断结果记录）
  backup.rs             配置备份（创建/恢复/轮转 + 增量文件备份）
  self_diagnosis.rs     自诊断系统（多 Provider Failover LLM 调用 + 基础分析降级 + 保守自动修复）
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
- **LLM 调用**：集中在 `agent.rs`，支持 Anthropic / OpenAIChat / OpenAIResponses / Codex 四种 Provider
- **Tool Loop**：请求 → 解析 tool_call → 执行 → 回传 → 继续，最多 10 轮
- **数据存储**：所有数据统一在 `~/.opencomputer/`，`paths.rs` 集中管理
- **降级策略**：ContextOverflow 终止 → RateLimit/Overloaded/Timeout 指数退避重试 2 次 → Auth/Billing/ModelNotFound 跳下一模型
- **连续消息合并**：`push_user_message()` 自动合并连续 user 消息，兼容 Anthropic role 交替要求
- **统一日志**：前后端日志统一写入后端 `logging.rs`（SQLite + 纯文本双写）。前端通过 `src/lib/logger.ts` 调用 `frontend_log` / `frontend_log_batch` 命令，支持批量缓冲（500ms / 20 条）。后端 Agent 执行全链路日志覆盖：chat 入口 → 模型链 → API 请求/响应 → SSE 流 → Tool Loop → 完成总结
- **记忆系统**：`memory.rs` 实现 `MemoryBackend` trait 可插拔架构，MVP 使用 SQLite + FTS5 全文搜索。4 种记忆类型（user/feedback/project/reference），2 种作用域（global/agent）。记忆自动注入系统提示词 section ⑧。Embedding 配置支持 API 模式（5 个预设）和本地 ONNX 模型（4 个预设），存储在 `config.json`
- **定时任务系统**：`cron.rs` 实现完整定时任务调度。3 种调度类型（At 一次性 / Every 固定间隔 / Cron 表达式），tokio 后台轮询执行，隔离 session + 模型链降级。指数退避重试 + 自动禁用。日历视图页面（侧边栏入口）+ 设置面板列表管理。Agent 工具 `manage_cron` 支持 AI 直接管理定时任务
- **Web 搜索多 Provider**：`tools/web_search.rs` 支持 8 个搜索引擎（DuckDuckGo / SearXNG / Brave / Perplexity / Google / Grok / Kimi / Tavily），enum 派发 + 自动检测。配置存储在 `config.json` 的 `webSearch` 字段，设置面板 `WebSearchPanel` 管理。SearXNG 支持 Docker 一键部署（`docker.rs`：镜像拉取 → 容器启动 → 配置注入 → 健康检查）
- **Web Fetch 网页抓取**：`tools/web_fetch.rs` 的 `tool_web_fetch` 使用 Mozilla Readability（`readability` crate）提取正文 + `htmd` crate 转 Markdown，支持 markdown/text 双模式。内存缓存（15 分钟 TTL / 100 条上限）、SSRF 防护（DNS 解析 + 私有 IP 拦截）、流式字节限制读取（默认 2MB）、结构化 JSON 响应。配置存储在 `config.json` 的 `webFetch` 字段，设置面板 `WebFetchPanel` 管理
- **上下文压缩系统**：`context_compact.rs` 实现 4 层渐进式上下文压缩。Tier 1 工具结果截断（head+tail，结构感知边界切割）→ Tier 2 上下文裁剪（软裁剪 + 硬替换，age×size 优先级评分）→ Tier 3 LLM 摘要（分块摘要 + 合并 + 3 级 fallback）→ Tier 4 溢出恢复（ContextOverflow 触发紧急压缩 + 自动重试）。Token 估算校准器利用 API 返回的实际 token 数做 EMA 滑动平均。15 个可配置参数存储在 `config.json` 的 `compact` 字段，设置面板 `ContextCompactPanel` 管理
- **系统消息通知**：`tauri-plugin-notification` 实现 macOS 原生桌面通知。三级粒度控制：全局开关（`config.json` 的 `notification` 字段，默认开启）→ 按 Agent 覆盖（`agent.json` 的 `notifyOnComplete`，None/true/false）→ 按定时任务开关（`cron_jobs.notify_on_complete` 列）。通知触发场景：非当前会话模型完成/异常、定时任务成功/失败。Agent 可调用 `send_notification` 工具（`tools/notification.rs`），仅在通知开启时条件注入到工具列表。前端 `src/lib/notifications.ts` 统一管理权限检查和通知发送。设置面板 `NotificationPanel` 管理
- **子 Agent 系统**：`subagent.rs` 实现 Agent 间任务委派。`subagent` 工具支持 spawn/check/list/result/kill/kill_all/steer/batch_spawn/wait_all 九种操作。非阻塞异步 spawn（`tokio::spawn`），子 Agent 在隔离 session 中运行，复用 cron 的 `build_and_run_agent` 模式（load agent → resolve model chain → failover retry）。可配置最大嵌套深度（1-5，默认 3），每个父 session 最多 5 个并发。**Steer 运行中干预**：`SubagentMailbox` 消息邮箱模式，父 Agent 可在子 Agent tool loop 每轮注入消息改变方向。**文件附件传递**：spawn 时可传递 files（utf8/base64），自动转为 Attachment 传入子 Agent。**标签系统**：每个 run 可附带 label 便于追踪定位。**深度分层工具策略**：`SubagentConfig.deniedTools` 可限制子 Agent 可用工具集。**批量操作**：batch_spawn 一次 spawn 多个任务，wait_all 等待多个 run 完成。**Token 统计**：记录 input_tokens/output_tokens 到 DB。`SubagentCancelRegistry`（`AtomicBool`）管理运行时取消。SQLite `subagent_runs` 表持久化运行记录（含 label/attachment_count/input_tokens/output_tokens）。Tauri 全局事件 `subagent_event` 实时通知前端。`SubagentConfig` per-Agent 配置（enabled/allowedAgents/deniedAgents/maxConcurrent/defaultTimeoutSecs/model/deniedTools/maxSpawnDepth/archiveAfterMinutes/announceTimeoutSecs）。系统提示词 section ⑩ 条件注入委派说明（含 steer/files/label/batch 用法）。前端 `SubagentBlock.tsx`（聊天内嵌状态，含 label/model/token 统计展示）+ `SubagentPanel.tsx`（Agent 设置面板，含深度/超时/工具策略配置）
- **自愈式自动重启**：`main.rs` 实现 Guardian Process 架构，同一二进制通过 `OPENCOMPUTER_CHILD` 环境变量区分 Guardian/Child 模式。Guardian 监控子进程退出码，捕获所有崩溃类型（panic/segfault/OOM/abort），指数退避重启。连续崩溃 5 次触发 `backup.rs` 配置备份 + `self_diagnosis.rs` LLM 自诊断（多 Provider Failover + 基础分析降级），保守自动修复（仅 config/logs.db 损坏）。崩溃记录持久化到 `crash_journal.json`（JSON 格式，最近 50 条）。信号转发确保 Force Quit 不误判。退出码：0=正常、42=请求重启、其他=崩溃。设置面板 `CrashHistoryPanel` 管理崩溃历史和备份

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

### 后端（Rust）
- 新功能放单独模块文件，在 `lib.rs` 注册命令
- 内部用 `anyhow::Result`，命令边界转为 `String`
- 异步命令加 `async`，不要自己 `block_on`
- **禁止使用 `log::info!` / `log::warn!` / `log::error!` / `log::debug!` 等 `log` crate 宏**，必须使用项目统一日志宏 `app_info!` / `app_warn!` / `app_error!` / `app_debug!`（定义在 `logging.rs`），以确保日志同时写入 SQLite 和日志文件。`log` crate 只输出到控制台（stderr），不会写入日志文件。唯一例外：`lib.rs` 的 `run()` 函数中 `AppLogger` 初始化之前的启动阶段代码，以及 `main.rs` 的 panic 恢复代码
- 日志宏用法：`app_info!("category", "source", "message {}", arg)`，category 为功能分类（如 `cron`/`tool`/`agent`），source 为具体来源（如 `scheduler`/`exec`/`codex`）

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

# OpenComputer

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，支持 36 个内置 Provider 模板（166 个预设模型），GUI 傻瓜式配置。支持三种运行模式：桌面 GUI（Tauri）、HTTP/WS 守护进程（`opencomputer server`）、ACP stdio（`opencomputer acp`）。

## 开发命令

```bash
npm run tauri dev      # 启动开发模式（前端 + Tauri 热重载）
npm run dev            # 仅前端 Vite 开发服务器
npm run tauri build    # 构建生产包
npx tsc --noEmit       # 前端类型检查
npm run lint           # Lint
node scripts/sync-i18n.mjs --check   # 检查各语言翻译缺失
node scripts/sync-i18n.mjs --apply   # 从翻译文件补齐缺失翻译

# Server 模式（HTTP/WS 守护进程）
opencomputer server start              # 前台启动 HTTP/WS 服务
opencomputer server install            # 注册系统服务（macOS launchd / Linux systemd）
opencomputer server uninstall          # 卸载系统服务
opencomputer server status             # 查看服务运行状态
opencomputer server stop               # 停止服务
```

## 项目结构

### Cargo Workspace

```
Cargo.toml              Workspace 根（members: crates/oc-core, crates/oc-server, src-tauri）
skills/                 内置技能（bundled skills，随应用发行）
  oc-skill-creator/     技能创建/编辑/改进工具（含 agents/references/scripts/eval-viewer）
  oc-settings/          应用设置管理技能（通过 get_settings/update_settings 工具调整配置）
crates/
  oc-core/              核心业务逻辑（零 Tauri 依赖，纯 Rust 库）
  oc-server/            HTTP/WS 服务器（axum，REST API + WebSocket 流式推送）
src-tauri/              Tauri 桌面 Shell（薄壳，调用 oc-core）
```

### 前端

```
src/                    前端（React + TypeScript）
  components/
    chat/               聊天组件
      hooks/            聊天相关自定义 hooks
      input/            输入框组件
      message/          消息渲染组件
      plan-mode/        Plan Mode 组件
      sidebar/          侧边栏组件
      slash-commands/   斜杠命令 UI
    settings/           设置面板
      agent-panel/      Agent 管理面板（含 tabs/ 子目录）
      channel-panel/    渠道管理面板
      general-panel/    通用设置面板
      log-panel/        日志面板
      memory-panel/     记忆管理面板
      profile-panel/    个人资料面板
      provider-setup/   Provider 配置（含 templates/ 子目录）
      skills-panel/     技能面板
      teams-panel/      Team 模板配置面板（index/TemplateListView/TemplateEditView/MemberRow/AgentSelector）
      web-search-panel/ 搜索设置面板
    common/             共享组件（导航栏/Markdown 渲染/Provider 图标）
    ui/                 shadcn/ui 基础组件
    dashboard/          数据大盘（recharts 图表 + 综合 Insights 健康度/热力图/Top 会话）
    cron/               定时任务管理组件
  hooks/                全局自定义 hooks（useClickOutside/useTheme/useUrlPreview）
  lib/
    logger.ts           前端统一日志工具
    transport.ts        Transport 抽象层（统一 invoke/listen 接口）
    transport-tauri.ts  Tauri IPC 实现（invoke + Channel 事件）
    transport-http.ts   HTTP/WS 实现（REST API + WebSocket 流式事件）
    transport-provider.ts Provider Transport 适配
    notifications.ts    通知工具
    urlDetect.ts        URL 检测工具
    utils.ts            通用工具函数
  i18n/locales/         12 种语言翻译文件
  types/
    chat.ts             聊天相关类型定义
    tools.ts            工具相关类型定义
```

### oc-core（核心库）

```
crates/oc-core/src/     核心业务逻辑（零 Tauri 依赖）
  lib.rs                模块导出 & CoreState（替代原 AppState）
  event_bus.rs          EventBus 事件总线（替代 Tauri APP_HANDLE 事件发射）
  globals.rs            全局状态管理
  app_init.rs           应用初始化逻辑
  user_config.rs        用户配置管理
  guardian.rs           Guardian 统一心跳管理
  permissions.rs        权限控制
  process_registry.rs   进程注册表
  util.rs               通用工具函数
  weather.rs            天气缓存系统与 Open-Meteo API
  weather_location_macos.rs macOS 原生 CoreLocation 定位（objc2 delegate + callback 生命周期）
  paths.rs              统一路径管理（~/.opencomputer/）
  failover.rs           模型降级 & 重试策略 + Auth Profile 轮换（per-profile cooldown / session stickiness）
  sandbox.rs            Docker 沙箱
  oauth.rs              OAuth 认证
  url_preview.rs        URL 预览
  file_extract.rs       文件内容提取
  browser_state.rs      浏览器状态管理
  canvas_db.rs          Canvas 数据库
  crash_journal.rs      崩溃日志
  dev_tools.rs          开发者工具
  self_diagnosis.rs     自诊断
  service_install.rs    系统服务注册（macOS launchd / Linux systemd）
  backup.rs             备份管理
  memory_extract.rs     记忆自动提取
  agent/                AssistantAgent（多 Provider + Tool Loop + Side Query 缓存侧查询）
    providers/          Anthropic / OpenAI Chat / OpenAI Responses / Codex
    side_query.rs       缓存友好侧查询（复用 prompt cache，成本降低 ~90%）
  agent_config.rs       Agent 配置管理
  agent_loader.rs       Agent 加载器
  channel/              IM 渠道系统（12 个插件，会话映射、分发 worker）
    discord/            Discord 渠道
    feishu/             飞书渠道
    googlechat/         Google Chat 渠道
    imessage/           iMessage 渠道
    irc/                IRC 渠道
    line/               LINE 渠道
    qqbot/              QQ Bot 渠道
    signal/             Signal 渠道
    slack/              Slack 渠道
    telegram/           Telegram 渠道
    wechat/             微信渠道
    whatsapp/           WhatsApp 渠道
    worker/             消息分发器（dispatcher/streaming/media/slash/approval 拆分）
  tools/                内置工具（按子模块拆分）
    definitions/        工具定义注册（types/core_tools/special_tools/plan_tools/registry 拆分）
    image_generate/     AI 图片生成（types/helpers/generate/output + 7 个 Provider）
    browser/            浏览器工具
    canvas/             Canvas 画布工具
    web_search/         Web 搜索工具
  skills/               技能系统（types/frontmatter/requirements/discovery/prompt/slash 拆分）
  slash_commands/       斜杠命令系统
    handlers/           各命令处理器
  async_jobs/           异步 Tool 执行（types/db/spawn/injection/retention 拆分，独立 ~/.opencomputer/async_jobs.db）
  ask_user/             通用结构化问答（types/questions 独立模块，不依赖 plan）
  plan/                 Plan Mode 六态状态机（types/constants/store/subagent/file_io/parser/git 拆分）
  recap/                `/recap` 深度复盘（types/db/facets/aggregate/sections/report/renderer/api 拆分）
  memory/               记忆系统（SQLite + FTS5 + 向量检索）
    embedding/          Embedding 提供者（config/utils/api_provider/local_provider/fallback_provider/factory）
    sqlite/             SQLite 后端（prompt/backend/trait_impl 拆分）
  context_compact/      上下文压缩（5 层渐进式 + ContextEngine trait 可插拔引擎）
  cross_session/        跨会话行为感知（config/types/registry/dirty/collect/render/awareness/llm_digest/peek_tool/build）
  subagent/             子 Agent 系统
  team/                 Agent Team 多 Agent 协作（coordinator/messaging/tasks/templates/cleanup）
  cron/                 定时任务调度
  acp/                  ACP 协议服务器（IDE 直连）
  acp_control/          ACP 控制面客户端
  config/               AppConfig 根结构 + 持久化（mod/persistence 拆分，load_config/save_config/cached_config）
  provider/             Provider 数据模型（types/proxy/persistence 拆分，persistence 只保留 provider 专属 helpers）
  project/              项目（Project）系统（types/db/files 拆分）
  session/              会话持久化（SQLite）
  system_prompt/        系统提示词模块化拼装（constants/build/sections/helpers 拆分）
  chat_engine/          聊天引擎（types/context/engine 拆分）
  docker/               Docker 服务管理（status/deploy/lifecycle/helpers/proxy 拆分）
  dashboard/            数据大盘聚合查询（types/cost/filters/queries/detail_queries/insights 拆分）
  logging/              统一日志（types/db/file_writer/app_logger/file_ops/config 拆分）
```

### oc-server（HTTP/WS 服务器）

```
crates/oc-server/src/   HTTP/WS 守护进程
  lib.rs                axum Router（路由注册 + 服务启动）
  config.rs             ServerConfig（bind_addr / api_key / cors_origins）
  error.rs              统一错误处理
  middleware.rs          API Key 鉴权中间件（Bearer header + ?token= query param）
  routes/               REST API 路由处理（sessions/chat/providers/models/config/agents/memory/auth/searxng/system/desktop/dev/acp/canvas/... 共 28 个模块）
  ws/                   WebSocket（events 事件推送 + chat_stream 流式聊天）
```

### src-tauri（Tauri 桌面 Shell）

```
src-tauri/src/          Tauri 薄壳（命令层 + 桌面集成）
  lib.rs                Tauri 命令注册 & AppState（委托 oc-core）
  main.rs               入口（桌面 GUI / server / acp 多模式分发）
  setup.rs              Tauri 应用 setup 初始化
  app_init.rs           应用初始化逻辑
  globals.rs            全局状态
  shortcuts.rs          全局快捷键
  tray.rs               系统托盘
  tauri_wrappers.rs     Tauri API 封装
  commands/             Tauri 命令层（~19 个模块）
    provider/           Provider 管理命令（crud/test_provider/test_embedding/test_image/models 拆分）
    chat.rs             聊天命令
    session.rs          会话管理命令
    agent_mgmt.rs       Agent 管理命令
    channel.rs          渠道管理命令
    config.rs           配置命令
    memory.rs           记忆管理命令
    skills.rs           技能命令
    plan.rs             Plan Mode 命令
    subagent.rs         子 Agent 命令
    cron.rs             定时任务命令
    docker.rs           Docker 管理命令
    dashboard.rs        数据大盘命令
    logging.rs          日志命令
    auth.rs             认证命令
    acp_control.rs      ACP 控制命令
    misc.rs             杂项命令
    crash.rs            崩溃报告命令
    url_preview.rs      URL 预览命令
```

## 技术栈

| 层     | 技术                                                                 |
| ------ | -------------------------------------------------------------------- |
| 前端   | React 19 + TypeScript, Vite 8, Tailwind CSS v4, shadcn/ui (Radix UI) |
| 桌面   | Tauri 2                                                              |
| 服务器 | axum (HTTP/WS), clap (CLI)                                          |
| 后端   | Rust, tokio, reqwest（oc-core 库，零 Tauri 依赖）                   |
| 渲染   | Streamdown + Shiki + KaTeX + Mermaid                                 |
| 多语言 | i18next (12 种语言)                                                  |

## 架构约定

- **Cargo Workspace 三 Crate 架构**：`oc-core`（核心业务逻辑，零 Tauri 依赖）、`oc-server`（axum HTTP/WS 守护进程）、`src-tauri`（Tauri 桌面薄壳）。所有业务逻辑在 oc-core，src-tauri 和 oc-server 均为调用方
- **三种运行模式**：`opencomputer`（桌面 GUI，Tauri）、`opencomputer server`（HTTP/WS 守护进程，支持 install/uninstall/status/stop 子命令）、`opencomputer acp`（stdio ACP 协议）
- **前后端通信**：前端通过 Transport 抽象层（`src/lib/transport.ts`）统一调用后端。桌面模式走 Tauri IPC（`invoke()` + `Channel<String>`），服务器模式走 HTTP REST API + WebSocket 流式推送。业务代码无需感知底层传输
- **EventBus 事件总线**：`oc-core` 中的 `EventBus` 替代原 Tauri `APP_HANDLE` 进行事件发射，使核心逻辑脱离 Tauri 依赖。Tauri shell 和 axum server 各自订阅 EventBus 并转发到各自的前端通道
- **状态管理**：后端核心状态在 `oc-core::CoreState`（`tokio::sync::Mutex`），Tauri 端通过 `State<AppState>` 持有引用，Server 端通过 axum `Extension` 注入。前端保持轻量 React state
- **Guardian 统一心跳**：桌面模式和服务器模式共用 Guardian keepalive 机制，确保后台任务（Channel 轮询、Cron 调度等）持续运行
- **系统服务注册**：`opencomputer server install` 在 macOS 注册 launchd plist（`~/Library/LaunchAgents/`），在 Linux 注册 systemd unit（`~/.config/systemd/user/`），实现开机自启
- **API Key 鉴权**：`oc-server/middleware.rs` 实现 axum `from_fn_with_state` 中间件。支持 `Authorization: Bearer <key>` 头和 `?token=<key>` 查询参数（浏览器 WebSocket 不支持自定义头）。`/api/health` 免鉴权。`api_key` 为 `None` 时全部放行。CLI 模式通过 `--api-key` 参数传入，桌面模式从 `config.json` 的 `server.apiKey` 读取
- **内嵌服务器配置**：桌面应用内嵌 HTTP 服务的 bind 地址和 API Key 存储在 `config.json` 的 `server` 字段（`EmbeddedServerConfig`），`setup.rs` 启动时读取。默认 `127.0.0.1:8420` 仅本机访问，设为 `0.0.0.0:8420` 可对外暴露。修改后需重启应用生效
- **LLM 调用**：集中在 `agent/` 模块，四种 Provider（Anthropic / OpenAIChat / OpenAIResponses / Codex）
- **温度配置**：三层覆盖架构（会话 > Agent > 全局）。全局存储在 `config.json` 的 `temperature` 字段，Agent 级存储在 `agent.json` 的 `model.temperature` 字段，会话级通过 `chat` 命令的 `temperatureOverride` 参数传递。`AssistantAgent.temperature` 字段在四种 Provider 的 API 请求中统一注入。范围 0.0–2.0，`None` 表示使用 API 默认值
- **Tool Loop**：请求 → 解析 tool_call → 并发/串行执行 → 回传 → 继续，最多 10 轮。工具按 `concurrent_safe` 标记分组：只读工具（read/grep/ls/find 等）并行执行，写入工具（exec/write/edit 等）串行执行
- **异步 Tool 执行（async_capable）**：`exec` / `web_search` / `image_generate` 标记 `async_capable = true`，模型可在 args 里设 `run_in_background: true` 把整轮 tool call detach 成后台 job，立即返回 `{job_id, status: "started"}` 作为合法 tool_result，对话继续。三道决策：(1) 模型显式 `run_in_background` (2) `AgentConfig.capabilities.async_tool_policy = "always-background" / "never-background" / "model-decide"` (3) 同步预算自动后台化（默认 30s `asyncTools.autoBackgroundSecs`，超时通过 OS 线程 + Mutex 相位机迁移到后台 job 而不丢结果）。结果通过新模块 [crates/oc-core/src/async_jobs/](crates/oc-core/src/async_jobs/) 持久化到独立的 `~/.opencomputer/async_jobs.db`，大结果 spool 到 `~/.opencomputer/async_jobs/{job_id}.txt`；完成后复用 `subagent::injection::inject_and_run_parent` 在会话空闲时把 `<tool-job-result job-id="..." tool="..." status="...">...</tool-job-result>` 作为 user 消息注入回主对话。新增 deferred 工具 `job_status(job_id, block?, timeout_ms?)` 让模型主动 poll/wait。**等待机制**（[async_jobs/wait.rs](crates/oc-core/src/async_jobs/wait.rs)）：per-job `tokio::sync::Notify` 注册表，`finalize_job` 在 `update_terminal` 之后 `notify_waiters()` 唤醒所有等待者，remove-on-notify 保证后到 waiter 拿到全新 Notify；`job_status` 注册后强制重读 DB 关闭 register/finalize race，循环使用 `tokio::select!(notify.notified(), sleep(backoff))`，退避曲线 100ms → ×1.5 → 2s 上限作为 Notify 失效时的防御兜底。EventBus `async_tool_job:completed` 仍然保留 emit 供未来前端订阅，但 `job_status` 不再依赖它。**timeout 策略**：默认 = `min(max_job_secs, 1800)` 秒；硬上限 = `max_job_secs`（或 `asyncTools.jobStatusMaxWaitSecs`，默认 7200s，当 `max_job_secs = 0` 时生效）；请求值超过上限自动 clamp。重启回放：`start_background_tasks` 把残留 `running` 行标记 `interrupted` + 重新入队所有 `injected=0` 的终态行。**保留期清理**（`async_jobs/retention.rs`）：启动时 + 每 24h 自动扫描，按 `asyncTools.retentionSecs`（默认 30 天）删除 `completed_at` 过期的终态行和对应 spool 文件；额外按 `asyncTools.orphanGraceSecs`（默认 24h）清理 spool 目录里"mtime 够老且无 DB 行引用"的孤儿文件，grace window 避免与刚写入还没提交 DB 行的 spool 文件竞态。两个配置 `0` 均表示禁用对应清理路径
- **Agent 工具过滤**：`AgentConfig.capabilities.tools` 在 `system_prompt`、Provider `tool_schemas`、`tool_search` 返回和执行层统一生效，作为 Agent 级基线工具权限。internal 系统工具（UI 隐藏不可关闭）在该层始终保留；更强限制由 `denied_tools`、skill allowlist 和 Plan Mode 继续收紧
- **工具结果磁盘持久化**：工具结果超过阈值（默认 50KB，`config.json` → `toolResultDiskThreshold` 可配置）时写入 `~/.opencomputer/tool_results/{session_id}/`，上下文仅保留 head+tail 预览 + 路径引用
- **工具审批等待超时**：审批等待时长由 `config.json` 的 `approvalTimeoutSecs` 控制，默认 300 秒，`0` 表示不限时。超时后的动作由 `approvalTimeoutAction` 控制：默认 `deny` 阻止执行，也可设为 `proceed` 在记录 warning 后继续执行工具
- **会话列表 pending-interaction 指示器**：`SessionMeta.pendingInteractionCount` 在 `list_sessions_cmd` / `GET /api/sessions` 命令层合并 `tools::approval::pending_approvals_per_session()`（`PendingApprovalEntry` 携带 `session_id`）+ `SessionDB::count_pending_ask_user_groups_per_session()`（按 session 聚合 `ask_user_questions.status='pending'` 行，过滤已超时行）。前端 `SessionItem` 在 `!isActive && !channelInfo && pendingInteractionCount > 0` 时叠加三层视觉提示（琥珀行底色 + 左色条、副标题替换为 `BellRing + 等待回应` 文本、头像左下角 pulse 徽章）。后端在 `submit_approval_response` / `submit_ask_user_question_response` / `cancel_pending_ask_user_question` / approval timeout 路径调用 `emit_pending_interactions_changed()` 广播 EventBus 事件 `session_pending_interactions_changed`，前端 `useChatSession` 同时订阅该事件 + `approval_required` + `ask_user_request` 走 300ms 防抖触发 `reloadSessions()`。`isLoading` 优先级最高，pending 仅在未运行时替换副标题
- **数据存储**：所有数据统一在 `~/.opencomputer/`，`paths.rs` 集中管理
- **数据大盘 Insights**：`dashboard/insights.rs` 提供综合分析查询：`query_overview_with_delta`（同环比 delta，按相同时间跨度左移取 previous baseline）、`query_cost_trend`（日度费用累计 + 峰值/日均）、`query_activity_heatmap`（7×24 活跃度网格）、`query_hourly_distribution`（0–23 时消息 + 峰值时段）、`query_top_sessions`（按 token 消耗的 Top N）、`query_model_efficiency`（每模型 tokens/msg、cost/1k、TTFT 对比）、`query_health_score`（四维加权健康度 0–100）和一次性 `query_insights` orchestrator。前端 Dashboard 默认 Tab 为 `InsightsSection`，同时 Header 提供 `autoRefreshMs` 定时轮询（30s/1m/5m）和 CSV 导出能力；System Tab 客户端持有最多 60 个采样点的环形缓冲绘制 CPU/内存实时曲线
- **IM Channel 架构**：`channel/` 目录统一承载 Telegram / WeChat 等渠道插件；Telegram 走 Bot API 轮询，WeChat 走二维码登录 + iLink HTTP 长轮询协议，渠道状态文件统一落在 `~/.opencomputer/channels/`。入站媒体管道：polling 收集 `InboundMedia`（Telegram/WeChat 入站媒体下载到 channel inbound-temp）→ worker 转为 `Attachment`（图片 base64 / 文件 path）并复制归档到会话目录 `~/.opencomputer/attachments/{session_id}/` → `ChatEngineParams.attachments` → `agent.chat()` 多模态接口。WeChat 通道完整能力：typing 指示器（24h TTL + 5s keepalive + cancel）、入站媒体下载解密（图片/视频/语音/文件）、出站媒体 AES-128-ECB 加密上传 CDN（3 次 5xx 重试）、会话过期暂停 1h、QR 登录自动刷新 3 次。**斜杠命令同步**：Telegram Bot 启动时自动调用 `setMyCommands` 同步内置命令到 Bot 菜单，`SlashCommandDef::description_en()` 提供英文描述
- **IM Channel 工具审批交互**：工具需要审批时，`channel/worker/approval.rs` 监听 EventBus `"approval_required"` 事件，通过 `ChannelDB.get_conversation_by_session()` 反查 IM 渠道信息，按 `ChannelCapabilities.supports_buttons` 决定发送方式：支持按钮的渠道（Telegram/Discord/Slack/飞书/QQ Bot/LINE/Google Chat）发送平台原生交互按钮，不支持的渠道（WeChat/Signal/iMessage/IRC/WhatsApp）发送文本提示（回复 1/2/3）。按钮回调通过各渠道原生机制路由回 `submit_approval_response()`。`ChannelAccountConfig.auto_approve_tools` 为 `true` 时，该渠道的所有工具调用自动审批，通过 `ChatEngineParams` → `AssistantAgent` → `ToolExecContext.auto_approve_tools` 传递到执行层，跳过审批门控
- **SearXNG Docker 代理注入**：`web_search.searxng_docker_use_proxy` 控制是否向 Docker SearXNG 写入 `settings.yml` 的 `outgoing.proxies` 和代理环境变量；适用于系统 VPN 场景，修改后在下次启动或重新部署容器时生效
- **SSRF 统一策略**：所有发起出站 HTTP 请求（接受模型 / 用户 URL 输入）的工具必须在发送前调用 `crate::security::ssrf::check_url(url, policy, &trusted_hosts)`（见 [`crates/oc-core/src/security/ssrf.rs`](crates/oc-core/src/security/ssrf.rs)）；redirect 回调用 `check_host_blocking_sync`。三档 policy 从 `AppConfig.ssrf` 读取：`Strict`（拒 loopback + private + link-local + metadata）、`Default`（允许 loopback，拒其他 private + metadata，当前 browser/web_fetch/image_generate/url_preview 默认档）、`AllowPrivate`（允许 loopback + private，仍拒 metadata / link-local）。Metadata IP 黑名单（`169.254.169.254` / `169.254.170.2` / `100.100.100.200` / `fd00:ec2::254`）在任何 policy 下都拒绝。`trustedHosts` allowlist 优先于 policy（支持 `host` / `host:port` / `*.example.com` 通配）。新增出站入口时必须显式接入该模块，不要绕过写自定义 IP 校验。LLM Provider 的 HTTP 出站当前不走此检查（`ProviderConfig.allow_private_network` 仅落字段 + UI 联动），Phase B 再打通
- **Side Query（缓存侧查询）**：`AssistantAgent.side_query()` 复用主对话的 system_prompt + tool_schemas + conversation_history 前缀，利用 Anthropic 显式 prompt caching / OpenAI 自动前缀缓存，侧查询（Tier 3 摘要、记忆提取）成本降低约 90%。每轮主请求 compaction 后自动快照 `CacheSafeParams`，侧查询构建字节一致的前缀请求。无缓存参数时退化为普通请求
- **降级策略**：ContextOverflow 终止 → RateLimit/Overloaded/Timeout 指数退避重试 2 次 → Auth/Billing/ModelNotFound 跳下一模型
- **Auth Profile 轮换 failover**：`ProviderConfig.auth_profiles: Vec<AuthProfile>` 支持同 Provider 多 API Key。Chat Engine 遇到 RateLimit/Overloaded/Auth/Billing 时先轮换同 Provider 下一个 profile，全部耗尽后再跳模型。每个 profile 持有独立 `api_key` + 可选 `base_url` 覆盖。纯内存 `ProfileCooldownTracker`（按错误类型 30s–600s 冷却）+ `ProfileStickyMap`（session 级亲和）。`AssistantAgent::new_from_provider_with_profile(config, model_id, profile)` 按 profile 构建 agent，现有 `new_from_provider` 委托到 `effective_profiles()[0]` 保持 15+ 调用点零改动。Codex (OAuth) 不参与 profile 轮换。前端 Provider 编辑面板 `AuthProfileEditor` 支持增删改多个 profile
- **连续消息合并**：`push_user_message()` 自动合并连续 user 消息，兼容 Anthropic role 交替要求
- **API-Round 消息分组**：Tool loop 中的 assistant + tool_result 消息通过 `_oc_round` 元数据标记为同一 round，压缩切割（Tier 3/4）对齐到 round 边界，确保 tool_use/tool_result 配对不被拆散。元数据在 API 调用前通过 `prepare_messages_for_api()` 剥离。无标记的旧会话退化为原行为
- **后压缩文件恢复**：Tier 3 摘要后自动扫描被摘要消息中的 write/edit/apply_patch 工具调用，从磁盘读取最近编辑的文件当前内容（最多 5 文件 × 16KB），注入 summary 之后的对话历史，省去额外的 read tool call。预算：释放 token 的 10%，兜底 100K chars
- **Cache-TTL 节流**：`compact.cacheTtlSecs`（默认 300 秒）控制 Tier 2+（裁剪/摘要）的冷却时间，TTL 内跳过 Tier 2+ 保护 API prompt cache（Anthropic/OpenAI/Google 均有 ~5 分钟缓存 TTL）。Tier 0/1 不受限。紧急阈值保护：usage ≥ 95% 时强制覆盖 TTL。`0` = 禁用
- **反应式微压缩（Reactive Microcompact, Phase B5）**：tool loop round 末尾在 Tier 1 `truncate_tool_results` 之后调用 `AssistantAgent::reactive_microcompact_in_loop()`，当估算使用率 ≥ `compact.reactiveTriggerRatio`（默认 0.75，可 clamp 到 0.50–0.95）时调 Tier 0 `microcompact`，清理 `tool_policies = eager` 的旧工具结果（ls/grep/find/web_search/process/tool_search 等），避免多轮 tool call 之间 tool_result 积累触发 `emergency_compact`。Tier 0 cache-safe 不改消息顺序。关闭开关 `compact.reactiveMicrocompactEnabled = false` 可完全退出。新字段在 `get_settings("compact")` 同步暴露
- **Persona 编辑模式（Personality Mode, Phase B2）**：`PersonalityConfig.mode: PersonaMode { Structured, SoulMd }`，默认 `Structured`（行为与旧版一致）。`SoulMd` 模式下 `system_prompt::build` 的结构化分支跳过 `build_personality_section`，改为注入 `~/.opencomputer/agents/{id}/soul.md` 内容 + 共享常量 `SOUL_EMBODIMENT_GUIDANCE`，并在身份行省略 `role_suffix` 避免与 markdown 自述身份双重声明。该 `soul.md` 物理文件与 `openclaw_mode` 兼容模式共用同一份——agent_loader 读取触发条件为 "`openclaw_mode` OR `personality.mode == SoulMd`"。前端 PersonalityTab 顶部提供 `Structured ↔ SOUL.md` segmented 切换；首次切到 SoulMd 且文件为空时调 `render_persona_to_soul_md` 命令 / `POST /api/agents/{id}/persona/render-soul-md` 由后端按结构化字段渲染 markdown 初稿。切回 Structured 不删 `soul.md`，两种模式可来回切换不丢数据
- **ContextEngine 可插拔引擎**：`context_compact/engine.rs` 定义 `ContextEngine` trait（`compact_sync` / `emergency_compact` / `system_prompt_addition`），`AssistantAgent` 持有 `Arc<dyn ContextEngine>`。默认实现 `DefaultContextEngine` 委托现有 5 层函数，行为不变。`CompactionContext` 参数对象封装 system_prompt / context_window / max_output_tokens / config / cache-TTL 状态。4 个 Provider 通过 `apply_engine_prompt_addition()` 共享方法注入引擎的 system prompt 补丁。Tier 3 异步编排（摘要/flush/恢复）保留在 `agent/context.rs`，不进 trait
- **CompactionProvider 可插拔摘要**：同文件定义 `CompactionProvider` async trait（`summarize` / `name`），让 Tier 3 摘要策略可插拔。`AssistantAgent` 持有 `Option<Arc<dyn CompactionProvider>>`，`summarize_with_model()` 优先尝试 dedicated provider，失败自动 fallback 到 side_query 路径。内置 `DedicatedModelProvider`（`agent/context.rs`）使用独立 provider:model 对调用 `summarize_direct()`。`summarization_model` 配置驱动 `DedicatedModelProvider` 在 agent 构造时注入（`chat_engine/context.rs` + `acp/agent.rs`）
- **会话历史搜索**：侧边栏顶部搜索框调用 `search_sessions_cmd` → `SessionDB::search_messages`（FTS5 `messages_fts`，`snippet()` 带 `<mark>` 高亮；`SessionTypeFilter` 支持跨普通/子 Agent/定时/IM 渠道会话筛选）。搜索模式下 filter tabs 在客户端按会话类型二次筛选；结果卡片显示会话类型图标 + Agent 头像/名称 + 高亮 snippet + 相对时间。点击结果触发 `handleSwitchSession(sid, { targetMessageId })` → `load_session_messages_around_cmd` 加载以命中消息为中心的窗口（默认前 40 / 后 20 条）→ `pendingScrollTarget` 驱动 `MessageList` 按 `data-message-id` 滚动定位 + `message-hit-pulse` 脉冲高亮（2 秒）。Snippet 渲染走 HTML escape 再白名单反解 `<mark>` 标签防止 XSS。**会话内搜索（Find in Chat）**：聊天窗口 title bar 的 `Search` 图标 / `Cmd+F` / `Ctrl+F` 唤起非常驻 `SessionSearchBar`（`src/components/chat/SessionSearchBar.tsx`），调用 `search_session_messages_cmd`（HTTP: `GET /api/sessions/:id/messages/search`），后端复用同一个 `SessionDB::search_messages` 加 `session_id: Option<&str>` 过滤参数。结果按 `messageId` 升序重排以对应会话时间轴；`Enter` / `↓` 下一条、`Shift+Enter` / `↑` 上一条、`Escape` 关闭；跳转经 `useChatSession.jumpToMessage(messageId)` —— 目标已在已加载窗口内则直接设 `pendingScrollTarget`，否则 `handleSwitchSession(sid, { targetMessageId })` 重载窗口后再跳。切换会话自动关闭
- **自动记忆提取**：默认开启，冷却 + 阈值双层触发——冷却时间 ≥ 5 分钟 AND（Token 累积 ≥ 8000 OR 消息条数 ≥ 10），inline 执行（非 tokio::spawn），支持 side_query 缓存共享降低成本。互斥保护：检测 save_memory/update_core_memory 工具调用时跳过自动提取。空闲超时兜底：阈值未满足时调度延迟任务（默认 30 分钟），会话空闲后从 DB 执行最终提取；新建会话时立即 flush 所有待提取会话。所有阈值均可在全局和 Agent 级配置
- **Dreaming 离线记忆固化（Phase B3 Light）**：`crates/oc-core/src/memory/dreaming/` 在应用空闲 / cron / 手动触发时跑 bounded side_query，把过去 N 天（`dreaming.scopeDays`，默认 1）非 pinned 候选交给 LLM 评估返回 `promotions{id,score,title,rationale}` + 自然语言 diary 段落；`min_score=0.75` + `max_promote=5` 两道过滤后对入选条目 `toggle_pin=true`，diary markdown 落到 `~/.opencomputer/memory/dreams/{YYYY-MM-DD_HHMMSS}.md`。**三触发**：idle（Guardian 风格的 60s ticker + `reset_chat_flags` 打 `touch_activity`，`idleMinutes=30` 默认）+ cron（默认 opt-out，走用户级 cron job 调 `POST /api/dreaming/run` 实现，避免引入 crontab 解析依赖）+ manual（Dashboard "Dream Diary" Tab 按钮 / Tauri `dreaming_run_now` / HTTP `POST /api/dreaming/run`）。并发保护：`DREAMING_RUNNING: AtomicBool` + `RunningGuard` RAII，重叠触发返回 `already_running`。narrative agent 默认复用 `recap::build_analysis_agent`，`narrativeModel="providerId:modelId"` 可指定独立摘要模型。EventBus 事件 `dreaming:cycle_complete` 驱动前端实时刷新。全局配置 `AppConfig.dreaming: DreamingConfig`，Dashboard `dreaming` Tab 提供日记列表 + markdown 渲染 + Run now 按钮 + last cycle 摘要
- **Active Memory 主动召回（Phase B1）**：每轮 user turn 开始时、compaction 之后、save_cache_safe_params 之前调用 `AssistantAgent::refresh_active_memory_suffix(user_text).await`。流程：(1) 读 `AgentConfig.memory.active_memory`（默认 enabled=true / timeout=3000ms / maxChars=220 / cacheTtlSecs=15 / budgetTokens=512 / candidateLimit=20）(2) 哈希 trim+lower 的 user_text，命中 15s TTL 缓存直接复用，未命中继续 (3) `spawn_blocking` 调 `MemoryBackend::search` 按 Project → Agent → Global scope 顺序取 top N 候选（无候选立即 cache 空值返回）(4) 构造 recall prompt 跑 `tokio::time::timeout(side_query)`，超时/失败/LLM 返回 "NONE" 均 cache 空值 (5) 非空结果格式化为 `## Active Memory\n\n{text}` 存 `active_memory_suffix: Mutex<Option<Arc<String>>>`。Provider 层：Anthropic 作为第三个独立 `cache_control` 系统块；OpenAI Chat 第三个 system 消息；OpenAI Responses / Codex 紧跟 cross_session_suffix 之后插入 input[] system 项。与 `cross_session_suffix` 并列为"独立 cache block"，suffix 变化不作废静态前缀缓存。模块 [`crates/oc-core/src/agent/active_memory.rs`](crates/oc-core/src/agent/active_memory.rs) 提供 `ActiveMemoryState`（LRU + TTL cache）/ `hash_user_text` / `scopes_for_session` / `shortlist_candidates` / `build_recall_prompt` / `format_suffix`。前端 MemoryTab 顶部新增 Active Memory 卡片（switch + timeout/cacheTtl/maxChars/candidateLimit 4 个数字输入）。配置属 Agent 级 (`agent.json`)，不进 `oc-settings` 技能（agent 级设置由 agent 管理 UI 覆盖）
- **LLM 记忆语义选择**：当候选记忆数 > 阈值（默认 8）时，通过 side_query 调用 LLM 从候选列表中选择最相关的 ≤5 条注入系统提示。选择在 compaction 后、cache 快照前执行，确保精简后的系统提示被缓存。opt-in 配置（`memorySelection.enabled`），失败时退化为全量注入
- **反省式记忆 COMBINED_EXTRACT_PROMPT（Phase B'2）**：`memory_extract` 的单次 side_query 同时返回 facts + profile 两个数组（`parse_extraction_response` 优先识别 combined 对象、回退 legacy 纯数组）。profile 项强制 `tags` 含 `"profile"`、`source="auto-reflect"`。`format_prompt_summary` 用 `render_section` helper 在 `About the User` / `Preferences & Feedback` 之前渲染独立的 `## About You` 段，把反省学到的沟通风格 / 工作习惯从事实目录里分离（不新增物理文件）。`MemoryExtractConfig.enable_reflection` / `MemoryConfig.enable_reflection`（Agent 级覆盖）默认 true；关闭回退 legacy facts-only prompt
- **召回 LLM 摘要（Phase B'3）**：`memory::recall_summary` 提供 opt-in 的 `maybe_summarize_recall`——命中数 ≥ `min_hits`（默认 3）时走一次 bounded side_query 把多条 snippet 压成 ≤400 字符的洞察段落；timeout / 失败 / 模型回 NONE 均静默降级到原始 snippet。走 `recap::build_analysis_agent` 选择分析模型（无 prompt cache 共享），hot 路径结果覆盖为 `## Summary of N hits\n\n...`。`AppConfig.recall_summary: RecallSummaryConfig`（默认 `enabled=false`）
- **自主 Skill 创建 + Draft 审核（Phase B'1）**：`skills::author` 模块提供 create/update/patch_fuzzy/set_status/delete 五个 CRUD 接口，模糊匹配走 Jaccard 词袋分段相似度（默认阈值 0.80），容忍 LLM 轻度漂移。`security_scan` 拦截三类：shell pipe 到 sh/bash、不可见 Unicode（`U+200B..200F` / `U+2060..206F` / `U+FEFF` / tag chars）、凭证特征（`sk-ant-` 90+ / `sk-proj-` 40+ / `AKIA` 16 / `ghp_` / `ghs_` 36+）。SKILL.md frontmatter 新增 `status: "active"|"draft"|"archived"` + `authored-by` + `rationale` 字段（缺省视为 active 保持兼容），`build_skills_prompt` / `get_invocable_skills` 面向模型的路径跳过非 Active 项。`skills::auto_review` 子模块串联 per-session `Mutex<HashMap<SessionId, AtomicBool>>` guard + 触发器（冷却 600s + Token 10000 / 消息 15 条双阈值）+ side_query（走 `recap::build_analysis_agent` 回退，不阻塞主对话）+ JSON 解析路由 create/patch/skip。`chat_engine::engine` 在 `run_memory_extraction_inline` 之后挂钩 `touch_turn_stats` + spawn `run_review_cycle(PostTurn)`。`AppConfig.skills.auto_review: SkillsAutoReviewConfig` 默认 enabled + promotion=draft。前端 `SkillListView` 顶部 `DraftReviewSection` 琥珀色卡片提供 Activate/Discard 按钮，订阅 EventBus `skills:auto_review_complete` 自动刷新。Tauri/HTTP 命令 `list_draft_skills` / `activate_draft_skill` / `discard_draft_skill` / `trigger_skill_review_now`
- **Learning Tracker Dashboard（Phase B'4）**：`session.db.learning_events` 表 + 索引，`SessionDB::record_learning_event` / `prune_learning_events` helper。`dashboard::learning` 暴露 `emit` + `query_learning_overview` / `query_skill_timeline` / `query_top_skills` / `query_recall_stats`，覆盖 7 类事件（skill_created/patched/activated/discarded/used + recall_hit/recall_summary_used）。`MemoryBackend::count_profile_memories` 按 `tags LIKE '%"profile"%' AND created_at >= cutoff` 高效计数反省式记忆。埋点点：`skills::author` 各 CRUD 方法、`tools::memory::tool_recall_memory` 非空命中和 summarize 分支。Dashboard 新增 "Learning" Tab（概览 + 时间线 + Top N + 召回效果），支持 7/14/30/60/90 天窗口切换
- **跨会话行为感知（Cross-Session Behavior Awareness）**：`crates/oc-core/src/cross_session/` 为每个会话挂 `SessionAwareness`，在每轮 user turn 开始前通过三层触发器（脏位 / 时间节流 / 语义 hint）决定是否重建一段 "其它会话此刻在做什么" 的 markdown suffix。默认 `structured` 模式零 LLM 成本（读 `recap.session_facets` + `sessions` + `messages` + 内存 `ActiveSessionRegistry`），opt-in 切到 `llm_digest` 模式走 `AssistantAgent::side_query` 生成自然语言 digest（bounded 5 秒、5 分钟节流、候选集合 hash 跳过、失败 fallback 到结构化路径）。**两段 cache 模型**：provider 把 suffix 作为第二个独立 `cache_control` 系统块（Anthropic）或在 instructions 末尾追加（OpenAI/Codex），suffix 变化不作废静态前缀缓存；内容未变时通过 `last_suffix_hash` 比对复用旧 `Arc<String>`。全局配置在 `AppConfig.cross_session`（设置 → 对话设置面板），会话级覆盖存 `sessions.cross_session_config_json` 列（partial merge 到全局默认），全局 `enabled=false` 是硬闸。默认只纳入普通会话，cron/channel/subagent 默认排除，UI 正向勾选纳入。Compaction Tier 2+ 后 `mark_force_refresh()` 搭便车刷新。新增 deferred 工具 `peek_sessions(query?, limit?)`、斜杠命令 `/awareness [on|off|mode <x>|status]`、Tauri/HTTP 命令 `get|save_cross_session_config` + `get|set_session_cross_session_override`
- **统一日志**：前后端日志统一写入 `logging.rs`（SQLite + 纯文本双写），API 请求体自动脱敏（`redact_sensitive`）并截断（32KB）
- **内置技能（Bundled Skills）**：项目根目录 `skills/` 存放随应用发行的内置技能，`discovery.rs` 的 `resolve_bundled_skills_dir()` 按优先级定位：(1) `OPENCOMPUTER_BUNDLED_SKILLS_DIR` 环境变量 (2) 可执行文件同级/上级 `skills/` 目录（release 打包）(3) `CARGO_MANIFEST_DIR` 向上两级的 `skills/`（dev 构建）。内置技能优先级最低（bundled < extra < managed < project），同名技能被高优先级来源覆盖。首个内置技能：`skill-creator`（技能创建/编辑/改进工具，含 agents/references/scripts/eval-viewer 子目录）
- **Skill 工具隔离**：SKILL.md frontmatter 支持 `allowed-tools:` 字段，激活时只保留指定工具 schema。空列表 = 全部工具（向后兼容）。Agent 通过 `skill_allowed_tools` 字段在 Provider 层过滤
- **Plan 执行层权限强制**：`ToolExecContext.plan_mode_allowed_tools` 在执行层白名单检查，与 schema 级过滤形成双重防护（defense-in-depth）
- **`skill` 工具（Skill 激活主入口）**：内置 `skill({name, args?})` 取代"模型 `read SKILL.md`"的老路径（read 仍可用于查看原文，不做硬拦截）。执行层 [`tools/skill/`](crates/oc-core/src/tools/skill/) 统一分发 inline / fork；inline 读 SKILL.md + `$ARGUMENTS` 替换后作为 tool_result 返回；fork 复用 [`skills::spawn_skill_fork`](crates/oc-core/src/skills/fork_helper.rs) + `extract_fork_result`，子 Agent 完成后只把最终摘要字符串塞回主对话 tool_result。`SpawnParams { skip_parent_injection: true, skill_name: Some(...), reasoning_effort: Option<String> }` 保证主对话不被子 Agent transcript 污染。工具标记 `internal + always_load`，在 deferred tool 场景也恒定可见。`/skill-name` 斜杠命令路径改走同一 helper（[`slash_commands/handlers/mod.rs`](crates/oc-core/src/slash_commands/handlers/mod.rs#L240)）避免漂移。System prompt catalog 从 `- name: desc (read: path)` 简化为 `- name: desc`
- **Skill Fork 模式**：SKILL.md frontmatter `context: fork` 在"模型调 `skill` 工具"和"用户 `/skill-name`"两条路径都生效（老版本只在斜杠路径生效）。子 Agent 继承 `allowed_tools`。`context: fork` skill 可额外声明 `agent: <agent-id>` 指定使用的 Agent 身份（失败 fallback 到 parent agent + warn 日志），以及 `effort: low|medium|high|xhigh|none` 指定 reasoning / thinking 强度——`SpawnParams.reasoning_effort` 透传到 `AssistantAgent::chat` 第三参数，4 个 provider 现有 effort 消费路径零改动
- **Skill `paths:` 条件激活**：SKILL.md frontmatter `paths: ["*.py", "docs/**"]` (gitignore-style) 让 Skill 默认**不进 catalog**，直到本会话 read/write/edit/ls/apply_patch 触发匹配文件才动态加入。[`skills::activation`](crates/oc-core/src/skills/activation.rs) 维护进程内缓存（按 session_id）+ SQLite 表 `session_skill_activation(session_id, skill_name, activated_at)` 启动时懒加载 + session 删除时级联清理。`tools/execution.rs::maybe_activate_conditional_skills` 在 dispatch 前扫描路径感知工具 args，命中后 `bump_skill_version()` 刷下一轮 prompt。`build_skills_prompt` 新增 `activated_conditional: &HashSet<String>` 参数，系统提示词装配链 `build_skills_section(... session_id)` → `build(... session_id)` → `build_system_prompt_with_session` 全链路透传。kill switch `AppConfig.conditional_skills_enabled: bool`（默认 true）
- **Skill 进度 UI**：`SubagentEvent` + `SpawnParams` 新增 `skill_name: Option<String>` 可辨别字段（仅 skill fork 路径 emit），`#[serde(default, skip_serializing_if)]` 零新通道 / 零老事件膨胀。前端 [`src/components/chat/SkillProgressBlock.tsx`](src/components/chat/SkillProgressBlock.tsx) 琥珀色 🧩 Puzzle 图标独立渲染，通过 `tool.name === "skill"` 挂到 [`MessageContent.tsx`](src/components/chat/message/MessageContent.tsx)。fork 模式检测走 tool_result 前缀（`Skill 'name' completed.`）
- **子 Agent spawn_and_wait**：`subagent(action="spawn_and_wait")` 前台等待 `foreground_timeout`（默认 30s），超时自动转后台。短任务同步返回，长任务无缝衔接后台注入
- **Agent Team 模板（用户预配 + 模型按需发现）**：内置模板已删除，改为"用户在设置面板 Teams Tab 预配 → 模型通过 `team(action="list_templates")` 按需发现 → 用 `team(action="create", template="<templateId>")` 一键实例化"。`TeamTemplateMember` 支持每成员不同 `agent_id`（绑定具体 Agent）、`model_override`、`default_task_template`、`description`（作为 role identity 注入子 session system prompt 的 `### Your Role Identity` 段）。模板存 `team_templates` 表（`members_json` 单列），`team_members.role_description` 列存模板的角色描述以便 resume 时重建 context 不依赖模板仍存在。系统提示词 `build_team_section()` 精简为"调 list_templates 发现预设"的简介，不再硬编码模板名。CRUD 三条通道：GUI（`TeamsPanel`）、Tauri 命令 `save_team_template` / `delete_team_template`、oc-settings 技能 `update_settings(category="teams", values={action:"save"|"delete", ...})`（**不走 `AppConfig` 流程，直接 DB CRUD + EventBus `template_saved` / `template_deleted`**）。删除模板只影响未来 `list_templates` 返回结果，已从该模板创建的运行中 team 不受影响（`teams.template_id` 悬空保留）
- **延迟工具加载**：opt-in 配置（`deferredTools.enabled`），开启后只发送核心工具 schema（exec/read/write/edit 等 ~10 个），其余通过 `tool_search` 元工具按需发现。execution dispatch 不变，直接调用 deferred 工具仍正常执行（容错）
- **通用 `ask_user_question` 工具**：任意对话中让模型向用户提出 1–4 个结构化问题（单选/多选/自定义输入）。每个选项支持 `preview`（markdown / image URL / mermaid 三种 `previewKind`）用于方案并排对比；每题支持 `header` chip 标签、`timeout_secs` 单题超时和 `default_values` 默认值，到点自动回退答案。Pending 组持久化到 session SQLite `ask_user_questions` 表（`request_id / session_id / payload / status / timeout_at`），App 重启后 `start_background_tasks` 重放未完成的 `ask_user_request` 事件实现断点续答。IM 渠道通过 `channel/worker/ask_user.rs` 监听事件，按 `ChannelCapabilities.supports_buttons` 发送原生按钮（每题一行 option + `Done` 按钮 + `Cancel`）或文本 fallback（回复 `1a` / `2b` / `done` / `cancel`）。各渠道插件调用统一 helper `channel::worker::ask_user::try_dispatch_interactive_callback(data, source)` 同时路由 approval 和 ask_user 两类回调。前端 `AskUserQuestionBlock`（`src/components/chat/ask-user/`）复用 Streamdown（仅 `code` + `cjk` 基础插件，不走 MarkdownRenderer 的流式/rAF 包装层）做轻量 preview 渲染，支持倒计时、header chip、default badge
- **`/recap` 深度复盘**：`crates/oc-core/src/recap/` 模块基于 `side_query` 对每个 session 做 LLM 语义 facet 提取（目标/成果/摩擦/满意度），结合 Dashboard 量化查询生成 11 个并行 AI 章节（含 `agent_tool_optimization` / `memory_skill_recommendations` / `cost_optimization` 三个 OpenComputer 特有章节）。Facet + 完整报告缓存到独立 `~/.opencomputer/recap/recap.db`（按 `last_message_ts` 失效），避免与热路径 session DB 锁争用。触发方式三选一：Dashboard "Recap" Tab（`src/components/dashboard/recap/RecapTab.tsx`）、聊天 `/recap`（Incremental 默认，`--range=7d` / `--full` 切换）、Tauri `recap_export_html` / HTTP `POST /api/recap/reports/{id}/export` 导出独立 HTML（inline SVG 图表，零 JS 依赖，self-contained）。Analysis agent 通过 `config.recap.analysisAgent` 与主对话 Agent 解耦，未配置时回退 `active_model`；`recap.facetConcurrency` 限流 `side_query` 并发（默认 4）避免 rate limit。EventBus `recap_progress` 事件实时推送 `started/extractingFacets/aggregatingDashboard/generatingSections/persisting/done/failed` 阶段。已知限制：cron 定时自动生成和 `opencomputer recap --export` CLI 子命令尚未接入
- **项目（Project）容器**：可选的会话分组，承载项目记忆（`MemoryScope::Project { id }`）、项目指令、共享文件。Session 通过 `sessions.project_id` 挂到项目下，NULL 表示未分配（保持 pre-project 行为）。数据模型 `projects` / `project_files` 两张表落在 `session.db`，`ProjectDB` 复用 `SessionDB` 的连接（参考 `ChannelDB` 模式）。项目文件存储在 `~/.opencomputer/projects/{id}/files/`，通过 `file_extract::extract` 提取的文本写入同目录 `extracted/`，三层注入 system prompt：Layer 1 目录清单永远注入，Layer 2 小文件（<4KB）在 8KB 字节预算内自动内联，Layer 3 新工具 `project_read_file(file_id|name, offset?, limit?)` 按需读取并强制只能访问 `project_extracted_dir`。记忆注入优先级 Project → Agent → Global（`load_prompt_candidates_with_project`），budget 裁剪时项目记忆最先保留；自动记忆提取在 session 属于某个项目时默认写到 Project scope。删除项目的级联顺序：unassign 会话（保留历史）→ 删项目行 + `project_files`（FK 级联）→ `rm -rf projects/{id}/` → 删项目记忆（跨 `memory.db` 单独执行，失败只留不可达孤儿）。上传走 `spawn_blocking` 避免阻塞 runtime，20 MB 大小上限，沿用 `save_attachment` 的 JSON bytes 数组而非引入 multipart。前端侧边栏 `ProjectSection`（位于 `AgentSection` 上方）+ `ProjectOverviewDialog`（四 Tab：Overview / Sessions / Files / Instructions），EventBus 事件 `project:created` / `project:updated` / `project:deleted` / `project:file_uploaded` / `project:file_deleted` 驱动跨窗口实时刷新

## 编码规范

### 通用

- **性能和用户体验是最高优先级**
- **核心逻辑必须在 oc-core 实现**：业务逻辑、数据处理、文件 IO、状态管理等一律放 `crates/oc-core/`，`src-tauri/` 仅做 Tauri 命令薄壳，`crates/oc-server/` 仅做 HTTP 路由薄壳，前端只负责展示和交互
- 操作即时反馈（乐观更新、loading 态），动效 60fps（优先 CSS transform/opacity）

### 前端

- 函数式组件 + hooks，不用 class 组件
- UI 组件统一用 `src/components/ui/`（shadcn/ui），不直接用 HTML 原生表单组件
- 样式只用 Tailwind utility class，不写行内 style 和自定义 CSS
- 动效优先复用 shadcn/ui、Radix UI、Tailwind 内置 utility，确认不够用才手写
- 路径别名：`@/` → `src/`
- 布局避免硬编码过小的 max-width（如 `max-w-md`），使用 `max-w-4xl` 以上或弹性伸缩
- **i18n 功能实现时只需实现中文（zh）和英文（en）**，其余语言通过 `scripts/sync-i18n.mjs` 补齐
- 避免不必要的重渲染（`React.memo`、`useMemo`、`useCallback`）
- **Tooltip 必须使用 `@/components/ui/tooltip`**，禁止用 HTML 原生 `title` 属性。优先使用 `<IconTip label={...}>` 简洁包裹
- **保存按钮统一三态交互**：saving（Loader2 旋转 + disabled）→ saved（绿色 + Check 图标，2 秒恢复）→ failed（红色，2 秒恢复）。使用 `saveStatus: "idle" | "saved" | "failed"` + `saving: boolean` 管理
- **Think / Tool 流式块展示约定**：内容块必须设置合理 `max-height`，超出后内部滚动；流式增量期间需自动滚动至底部，并实时显示耗时（结束后保留最终耗时）

### 后端（Rust）

- 新功能放 `crates/oc-core/` 单独模块文件；Tauri 命令在 `src-tauri/src/lib.rs` 注册，HTTP 路由在 `crates/oc-server/src/router.rs` 注册
- 内部用 `anyhow::Result`，命令边界转为 `String`
- 异步命令加 `async`，不要自己 `block_on`
- **禁止使用 `log` crate 宏**（`log::info!` 等），必须使用 `app_info!` / `app_warn!` / `app_error!` / `app_debug!`（定义在 `logging.rs`）。唯一例外：`lib.rs` 的 `run()` 中 AppLogger 初始化之前，以及 `main.rs` 的 panic 恢复
- 日志宏用法：`app_info!("category", "source", "message {}", arg)`
- **禁止对字符串使用字节索引切片**（如 `&s[..80]`），必须使用 `crate::truncate_utf8(s, max_bytes)` 安全截断

## 安全红线

- **API Key 和 OAuth Token 禁止出现在任何日志中**
- `tauri.conf.json` CSP 当前为 `null`，不要放行外部域名
- OAuth token 在 `~/.opencomputer/credentials/auth.json`，登出时必须 `clear_token()`

## 易错提醒

- 修改 Tauri 命令后须同步更新 `invoke_handler!` 宏注册列表
- 新增 HTTP API 端点后须在 `crates/oc-server/src/router.rs` 注册路由
- 新增核心功能须放 `crates/oc-core/`，禁止在 oc-core 中引入 Tauri 依赖
- Rust 依赖变更后 `cargo check` 先行验证（workspace 级别）
- 前端新增 invoke 调用时须同步实现 Transport 的 Tauri 和 HTTP 两种适配

## 设置（Settings）约定

所有用户可操作的配置必须同时具备 **GUI 入口** 和 **`oc-settings` 技能对应能力**，两者零偏差。新增/修改任何进入 `AppConfig` 或 `UserConfig` 且用户需要调整的字段时，**必须在同一 PR 里完成以下三件事**：

1. **前端 GUI**：在 [`src/components/settings/`](src/components/settings/) 对应面板加入控件（shadcn/ui 组件），带即时反馈和三态保存按钮
2. **Settings 技能能力**：
   - 在 [`crates/oc-core/src/tools/settings.rs`](crates/oc-core/src/tools/settings.rs) 的 `read_category` / `update_app_config` 加读写分支
   - 在 `risk_level()`（LOW/MEDIUM/HIGH）和 `get_all_overview()` 的 `riskLevels` map 里显式分级
   - 如涉及重启/网络暴露/凭据等副作用，在 `side_effect_note()` 补一句提示
   - 同步更新 [`crates/oc-core/src/tools/definitions/core_tools.rs`](crates/oc-core/src/tools/definitions/core_tools.rs) 里 `get_settings` / `update_settings` 两个 tool 的 `category` enum
3. **技能文档**：在 [`skills/oc-settings/SKILL.md`](skills/oc-settings/SKILL.md) 的风险等级表里加该分类和字段说明

### 风险等级判定

- **LOW**：UI 偏好、显示配额、不影响成本/安全（theme / language / notification / canvas 等）
- **MEDIUM**：行为调整，影响上下文、成本或输出质量（compact / memory_* / web_search / approval 等）
- **HIGH**：安全、网络暴露、全局键位、凭据、需要重启（proxy / embedding / shortcuts / server / skill_env / acp_control 等）

HIGH 级别的分类，Settings 技能在调用 `update_settings` 前必须向用户二次确认。

### 强制留在 GUI 的例外

以下三类继续只走 GUI，不进 `update_settings` 工具：**Provider 列表与 API Key**、**IM Channel 配置**、**`active_model` / `fallback_models` 的写入**。原因是凭据安全和运行时稳定性。这些在技能里以只读形式出现或完全排除。

### 配置自动备份 / 回滚（强制）

**所有配置写入必须走 `config::save_config()` 或 `user_config::save_user_config_to_disk()`**，禁止 bypass 直接 `std::fs::write(config_path, ...)`。两个函数会在写入前通过 `backup::snapshot_before_write()` 自动把旧文件快照到 `~/.opencomputer/backups/autosave/`，保留最近 50 份。这保证任何改动（UI / 技能 / CLI）都可回滚。

技能 / Tauri 命令在调用 save 前建议用 `backup::scope_save_reason(category, source)` 给快照打标签，否则会记录为 `unknown/unknown`，用户回滚时难以辨识。

技能通过 `list_settings_backups` / `restore_settings_backup` 两个内置工具暴露给模型；UI 如需暴露同样功能，应调用 `backup::list_autosaves()` / `backup::restore_autosave(id)`，不要重复实现快照逻辑。

## 文档维护

技术文档索引见 [`docs/README.md`](docs/README.md)，分为架构文档（`docs/architecture/`）和调研文档（`docs/research/`）。

代码改动时**必须同步更新文档**：

| 改动类型                  | 需更新                                          |
| ------------------------- | ----------------------------------------------- |
| 新增/删除功能、命令、模块 | `CHANGELOG.md`、`AGENTS.md`                     |
| 技术栈/架构/规范变更      | `AGENTS.md`                                     |
| 子系统架构变更            | `docs/architecture/` 对应文档                   |
| 新增调研/对比分析         | `docs/research/` 新建调研文档                   |

- `CHANGELOG.md`：[Keep a Changelog](https://keepachangelog.com/) 格式
- `docs/README.md`：文档索引，新增/删除文档时同步更新

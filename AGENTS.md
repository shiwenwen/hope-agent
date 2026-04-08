# OpenComputer

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，支持 28 个内置 Provider 模板（108 个预设模型），GUI 傻瓜式配置。支持三种运行模式：桌面 GUI（Tauri）、HTTP/WS 守护进程（`opencomputer server`）、ACP stdio（`opencomputer acp`）。

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
      web-search-panel/ 搜索设置面板
    common/             共享组件（导航栏/Markdown 渲染/Provider 图标）
    ui/                 shadcn/ui 基础组件
    dashboard/          数据大盘（recharts 图表）
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
  failover.rs           模型降级 & 重试策略
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
  plan/                 Plan Mode 六态状态机（types/constants/store/subagent/file_io/parser/git 拆分）
  memory/               记忆系统（SQLite + FTS5 + 向量检索）
    embedding/          Embedding 提供者（config/utils/api_provider/local_provider/fallback_provider/factory）
    sqlite/             SQLite 后端（prompt/backend/trait_impl 拆分）
  context_compact/      上下文压缩（5 层渐进式）
  subagent/             子 Agent 系统
  cron/                 定时任务调度
  acp/                  ACP 协议服务器（IDE 直连）
  acp_control/          ACP 控制面客户端
  provider/             Provider 数据模型（types/proxy/store/persistence 拆分）
  session/              会话持久化（SQLite）
  system_prompt/        系统提示词模块化拼装（constants/build/sections/helpers 拆分）
  chat_engine/          聊天引擎（types/context/engine 拆分）
  docker/               Docker 服务管理（status/deploy/lifecycle/helpers/proxy 拆分）
  dashboard/            数据大盘聚合查询（types/cost/filters/queries/detail_queries 拆分）
  logging/              统一日志（types/db/file_writer/app_logger/file_ops/config 拆分）
```

### oc-server（HTTP/WS 服务器）

```
crates/oc-server/src/   HTTP/WS 守护进程
  lib.rs                axum Router（路由注册 + 服务启动）
  config.rs             ServerConfig（bind_addr / api_key / cors_origins）
  error.rs              统一错误处理
  middleware.rs          API Key 鉴权中间件（Bearer header + ?token= query param）
  routes/               REST API 路由处理（sessions/chat/providers/config/agents/memory/health）
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
- **工具结果磁盘持久化**：工具结果超过阈值（默认 50KB，`config.json` → `toolResultDiskThreshold` 可配置）时写入 `~/.opencomputer/tool_results/{session_id}/`，上下文仅保留 head+tail 预览 + 路径引用
- **数据存储**：所有数据统一在 `~/.opencomputer/`，`paths.rs` 集中管理
- **IM Channel 架构**：`channel/` 目录统一承载 Telegram / WeChat 等渠道插件；Telegram 走 Bot API 轮询，WeChat 走 OpenClaw 兼容的二维码登录 + iLink HTTP 长轮询协议，渠道状态文件统一落在 `~/.opencomputer/channels/`。入站媒体管道：polling 收集 `InboundMedia`（Telegram/WeChat 入站媒体下载到 channel inbound-temp）→ worker 转为 `Attachment`（图片 base64 / 文件 path）并复制归档到会话目录 `~/.opencomputer/attachments/{session_id}/` → `ChatEngineParams.attachments` → `agent.chat()` 多模态接口。WeChat 通道完整能力：typing 指示器（24h TTL + 5s keepalive + cancel）、入站媒体下载解密（图片/视频/语音/文件）、出站媒体 AES-128-ECB 加密上传 CDN（3 次 5xx 重试）、会话过期暂停 1h、QR 登录自动刷新 3 次。**斜杠命令同步**：Telegram Bot 启动时自动调用 `setMyCommands` 同步内置命令到 Bot 菜单，`SlashCommandDef::description_en()` 提供英文描述
- **IM Channel 工具审批交互**：工具需要审批时，`channel/worker/approval.rs` 监听 EventBus `"approval_required"` 事件，通过 `ChannelDB.get_conversation_by_session()` 反查 IM 渠道信息，按 `ChannelCapabilities.supports_buttons` 决定发送方式：支持按钮的渠道（Telegram/Discord/Slack/飞书/QQ Bot/LINE/Google Chat）发送平台原生交互按钮，不支持的渠道（WeChat/Signal/iMessage/IRC/WhatsApp）发送文本提示（回复 1/2/3）。按钮回调通过各渠道原生机制路由回 `submit_approval_response()`。`ChannelAccountConfig.auto_approve_tools` 为 `true` 时，该渠道的所有工具调用自动审批，通过 `ChatEngineParams` → `AssistantAgent` → `ToolExecContext.auto_approve_tools` 传递到执行层，跳过审批门控
- **SearXNG Docker 代理注入**：`web_search.searxng_docker_use_proxy` 控制是否向 Docker SearXNG 写入 `settings.yml` 的 `outgoing.proxies` 和代理环境变量；适用于系统 VPN 场景，修改后在下次启动或重新部署容器时生效
- **Side Query（缓存侧查询）**：`AssistantAgent.side_query()` 复用主对话的 system_prompt + tool_schemas + conversation_history 前缀，利用 Anthropic 显式 prompt caching / OpenAI 自动前缀缓存，侧查询（Tier 3 摘要、记忆提取）成本降低约 90%。每轮主请求 compaction 后自动快照 `CacheSafeParams`，侧查询构建字节一致的前缀请求。无缓存参数时退化为普通请求
- **降级策略**：ContextOverflow 终止 → RateLimit/Overloaded/Timeout 指数退避重试 2 次 → Auth/Billing/ModelNotFound 跳下一模型
- **连续消息合并**：`push_user_message()` 自动合并连续 user 消息，兼容 Anthropic role 交替要求
- **API-Round 消息分组**：Tool loop 中的 assistant + tool_result 消息通过 `_oc_round` 元数据标记为同一 round，压缩切割（Tier 3/4）对齐到 round 边界，确保 tool_use/tool_result 配对不被拆散。元数据在 API 调用前通过 `prepare_messages_for_api()` 剥离。无标记的旧会话退化为原行为
- **后压缩文件恢复**：Tier 3 摘要后自动扫描被摘要消息中的 write/edit/apply_patch 工具调用，从磁盘读取最近编辑的文件当前内容（最多 5 文件 × 16KB），注入 summary 之后的对话历史，省去额外的 read tool call。预算：释放 token 的 10%，兜底 100K chars
- **Cache-TTL 节流**：`compact.cacheTtlSecs`（默认 300 秒）控制 Tier 2+（裁剪/摘要）的冷却时间，TTL 内跳过 Tier 2+ 保护 API prompt cache（Anthropic/OpenAI/Google 均有 ~5 分钟缓存 TTL）。Tier 0/1 不受限。紧急阈值保护：usage ≥ 95% 时强制覆盖 TTL。`0` = 禁用
- **自动记忆提取**：默认开启，每轮对话结束后 inline 执行记忆提取（非 tokio::spawn），支持 side_query 缓存共享降低成本。互斥保护：检测 save_memory/update_core_memory 工具调用时跳过自动提取。频率上限：每会话最多 5 次（可配置）
- **LLM 记忆语义选择**：当候选记忆数 > 阈值（默认 8）时，通过 side_query 调用 LLM 从候选列表中选择最相关的 ≤5 条注入系统提示。选择在 compaction 后、cache 快照前执行，确保精简后的系统提示被缓存。opt-in 配置（`memorySelection.enabled`），失败时退化为全量注入
- **统一日志**：前后端日志统一写入 `logging.rs`（SQLite + 纯文本双写），API 请求体自动脱敏（`redact_sensitive`）并截断（32KB）
- **Skill 工具隔离**：SKILL.md frontmatter 支持 `allowed-tools:` 字段，激活时只保留指定工具 schema。空列表 = 全部工具（向后兼容）。Agent 通过 `skill_allowed_tools` 字段在 Provider 层过滤
- **Plan 执行层权限强制**：`ToolExecContext.plan_mode_allowed_tools` 在执行层白名单检查，与 schema 级过滤形成双重防护（defense-in-depth）
- **Skill Fork 模式**：SKILL.md frontmatter `context: fork` 指定在子 Agent 中执行，tool_call 不污染主对话。子 Agent 继承 `allowed_tools`，结果通过注入系统自动推送回主对话
- **子 Agent spawn_and_wait**：`subagent(action="spawn_and_wait")` 前台等待 `foreground_timeout`（默认 30s），超时自动转后台。短任务同步返回，长任务无缝衔接后台注入
- **延迟工具加载**：opt-in 配置（`deferredTools.enabled`），开启后只发送核心工具 schema（exec/read/write/edit 等 ~10 个），其余通过 `tool_search` 元工具按需发现。execution dispatch 不变，直接调用 deferred 工具仍正常执行（容错）

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

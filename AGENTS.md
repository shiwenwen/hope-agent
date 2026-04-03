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
    chat/               聊天组件（消息列表/输入框/Plan Mode/快捷对话浮层）
    settings/           设置面板
    common/             共享组件（导航栏/Markdown 渲染/Provider 图标）
    dashboard/          数据大盘（recharts 图表）
  lib/logger.ts         前端统一日志工具
  i18n/locales/         12 种语言翻译文件
  types/chat.ts         共享类型定义
src-tauri/src/          后端（Rust）
  lib.rs                Tauri 命令注册 & AppState
  weather.rs            天气缓存系统与 Open-Meteo API
  weather_location_macos.rs macOS 原生 CoreLocation 定位（objc2 delegate + callback 生命周期）
  agent/                AssistantAgent（多 Provider + Tool Loop）
    providers/          Anthropic / OpenAI Chat / OpenAI Responses / Codex
  channel/              IM 渠道系统（12 个插件：Telegram / WeChat / Discord / Slack / Feishu / QQ Bot / IRC / Signal / iMessage / WhatsApp / Google Chat / LINE，会话映射、分发 worker、共享 WebSocket 工具、进程管理器、嵌入式 Webhook 服务器）
  tools/                31 个内置工具（按工具拆分子模块）
  skills.rs             技能系统（SKILL.md 发现 + 懒加载）
  slash_commands/       斜杠命令系统
  plan.rs               Plan Mode（子 Agent 制定计划 + 主 Agent 执行，六态状态机）
  memory.rs             记忆系统（SQLite + FTS5 + 向量检索）
  context_compact.rs    上下文压缩（4 层渐进式）
  subagent.rs           子 Agent 系统
  cron.rs               定时任务调度
  sandbox.rs            Docker 沙箱
  acp/                  ACP 协议服务器（IDE 直连）
  acp_control/          ACP 控制面客户端
  provider.rs           Provider 数据模型 & 持久化
  session.rs            会话持久化（SQLite）
  paths.rs              统一路径管理（~/.opencomputer/）
  failover.rs           模型降级 & 重试策略
  system_prompt.rs      系统提示词模块化拼装（per-tool 描述 + 行为指导）
  dashboard.rs          数据大盘聚合查询
  logging.rs            统一日志（SQLite + 纯文本双写）
```

## 技术栈

| 层     | 技术                                                                 |
| ------ | -------------------------------------------------------------------- |
| 前端   | React 19 + TypeScript, Vite 8, Tailwind CSS v4, shadcn/ui (Radix UI) |
| 桌面   | Tauri 2                                                              |
| 后端   | Rust, tokio, reqwest                                                 |
| 渲染   | Streamdown + Shiki + KaTeX + Mermaid                                 |
| 多语言 | i18next (12 种语言)                                                  |

## 架构约定

- **前后端通信**：前端通过 `invoke()` 调用 Tauri 命令，流式输出通过 `Channel<String>` 推送事件
- **状态管理**：后端用 `State<AppState>`（`tokio::sync::Mutex`），前端保持轻量 React state
- **LLM 调用**：集中在 `agent/` 模块，四种 Provider（Anthropic / OpenAIChat / OpenAIResponses / Codex）
- **温度配置**：三层覆盖架构（会话 > Agent > 全局）。全局存储在 `config.json` 的 `temperature` 字段，Agent 级存储在 `agent.json` 的 `model.temperature` 字段，会话级通过 `chat` 命令的 `temperatureOverride` 参数传递。`AssistantAgent.temperature` 字段在四种 Provider 的 API 请求中统一注入。范围 0.0–2.0，`None` 表示使用 API 默认值
- **Tool Loop**：请求 → 解析 tool_call → 执行 → 回传 → 继续，最多 10 轮
- **数据存储**：所有数据统一在 `~/.opencomputer/`，`paths.rs` 集中管理
- **IM Channel 架构**：`channel/` 目录统一承载 Telegram / WeChat 等渠道插件；Telegram 走 Bot API 轮询，WeChat 走 OpenClaw 兼容的二维码登录 + iLink HTTP 长轮询协议，渠道状态文件统一落在 `~/.opencomputer/channels/`。入站媒体管道：polling 收集 `InboundMedia`（Telegram/WeChat 入站媒体下载到 channel inbound-temp）→ worker 转为 `Attachment`（图片 base64 / 文件 path）并复制归档到会话目录 `~/.opencomputer/attachments/{session_id}/` → `ChatEngineParams.attachments` → `agent.chat()` 多模态接口。WeChat 通道完整能力：typing 指示器（24h TTL + 5s keepalive + cancel）、入站媒体下载解密（图片/视频/语音/文件）、出站媒体 AES-128-ECB 加密上传 CDN（3 次 5xx 重试）、会话过期暂停 1h、QR 登录自动刷新 3 次。**斜杠命令同步**：Telegram Bot 启动时自动调用 `setMyCommands` 同步内置命令到 Bot 菜单，`SlashCommandDef::description_en()` 提供英文描述
- **SearXNG Docker 代理注入**：`web_search.searxng_docker_use_proxy` 控制是否向 Docker SearXNG 写入 `settings.yml` 的 `outgoing.proxies` 和代理环境变量；适用于系统 VPN 场景，修改后在下次启动或重新部署容器时生效
- **降级策略**：ContextOverflow 终止 → RateLimit/Overloaded/Timeout 指数退避重试 2 次 → Auth/Billing/ModelNotFound 跳下一模型
- **连续消息合并**：`push_user_message()` 自动合并连续 user 消息，兼容 Anthropic role 交替要求
- **统一日志**：前后端日志统一写入 `logging.rs`（SQLite + 纯文本双写），API 请求体自动脱敏（`redact_sensitive`）并截断（32KB）

## 编码规范

### 通用

- **性能和用户体验是最高优先级**
- **核心逻辑必须在 Rust 后端实现**：业务逻辑、数据处理、文件 IO、状态管理等一律放 `src-tauri/`，前端只负责展示和交互
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

- 新功能放单独模块文件，在 `lib.rs` 注册命令
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
- Rust 依赖变更后 `cargo check` 先行验证

## 文档维护

代码改动时**必须同步更新文档**：

| 改动类型                  | 需更新                      |
| ------------------------- | --------------------------- |
| 新增/删除功能、命令、模块 | `CHANGELOG.md`、`AGENTS.md` |
| 技术栈/架构/规范变更      | `AGENTS.md`                 |

- `CHANGELOG.md`：[Keep a Changelog](https://keepachangelog.com/) 格式

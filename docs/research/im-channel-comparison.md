# IM 渠道系统对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.8

---

## 一、架构总览

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|---------|
| **语言** | Rust（Tauri 后端） | — | TypeScript（Node.js） |
| **架构模式** | 单一 `ChannelPlugin` trait + 中央 `ChannelRegistry` | 无 IM 渠道 | 适配器组合模式（20+ 独立 Adapter 接口） |
| **渠道数** | 12 个内置插件 | 0 | 23+ 渠道插件（extensions/） |
| **部署形态** | 桌面应用内嵌 | CLI 工具 | 服务端 Node.js 进程 |
| **消息路由** | mpsc channel → 单一 dispatcher | — | 插件注册表 + session 绑定 + 线程绑定 |
| **安全模型** | DM/Group Policy + Allowlist | — | DM Policy + Group Policy + Pairing + Command Gating |
| **流式预览** | Draft（Telegram）/ Edit 回退 | — | Draft Stream Loop + Status Reactions |
| **语音能力** | 无 | 无 | Voice Call（Telnyx/Twilio/Plivo）+ Talk Mode + TTS + ASR |

---

## 二、OpenComputer 实现

### 2.1 渠道插件架构

核心 trait 定义在 `crates/oc-core/src/channel/traits.rs`：

```
ChannelPlugin trait
├── meta()                    → ChannelMeta（ID、名称、版本）
├── capabilities()            → ChannelCapabilities（支持的消息类型、媒体、功能）
├── start_account()           → 启动账户（GatewayAdapter）
├── stop_account()            → 停止账户
├── send_message()            → 发送消息（OutboundAdapter）
├── send_typing()             → 发送输入指示
├── send_draft()              → 发送流式草稿（Telegram 专属）
├── edit_message()            → 编辑消息
├── delete_message()          → 删除消息
├── probe()                   → 健康检查（StatusAdapter）
├── check_access()            → 安全校验（SecurityAdapter）
├── markdown_to_native()      → 格式转换
├── chunk_message()           → 长消息分片
└── validate_credentials()    → 凭证验证（SetupAdapter）
```

设计对标 OpenClaw 的多 Adapter 模式（GatewayAdapter / OutboundAdapter / StatusAdapter / SecurityAdapter / SetupAdapter），但扁平化为单一 trait。

中央注册表 `ChannelRegistry`：
- `HashMap<ChannelId, Arc<dyn ChannelPlugin>>` 存储所有已注册插件
- `Mutex<HashMap<String, ChannelWorkerHandle>>` 跟踪运行中账户
- 通过 `mpsc::Sender<MsgContext>` 统一收集所有渠道的入站消息
- 支持 start / stop / restart / probe 等生命周期操作

### 2.2 已支持渠道（12 个）

| 渠道 | 传输方式 | 聊天类型 | 媒体支持 |
|------|---------|---------|---------|
| **Telegram** | Bot API 长轮询 | DM / Group / Forum | Photo / Video / Audio / Document / Sticker / Voice / Animation |
| **WeChat** | iLink HTTP 长轮询（OpenClaw 兼容） | DM | Photo / Video / Document / Voice |
| **Discord** | Gateway WebSocket | DM / Group | — |
| **Slack** | — | — | — |
| **WhatsApp** | — | — | — |
| **Signal** | — | — | — |
| **iMessage** | — | — | — |
| **LINE** | — | — | — |
| **Feishu** | — | — | — |
| **QQ Bot** | — | — | — |
| **Google Chat** | Webhook | — | — |
| **IRC** | — | — | — |

> Telegram 和 WeChat 为完整实现；Discord 有 Gateway 基础框架；其余渠道为占位模块。

### 2.3 消息路由与分发

分发器 `worker/dispatcher.rs` 核心流程：

```
inbound_rx.recv()
  → Semaphore 限流（MAX_CONCURRENT_INBOUND = 20）
  → handle_inbound_message()
      1. 加载 ProviderStore 配置
      2. check_access() 安全校验
      3. Mention gating（群组默认 requireMention=true）
      4. 解析 agent_id（topic > group > channel > account > global）
      5. resolve_or_create_session()（ChannelDB 映射）
      6. 保存用户消息到 SessionDB
      7. send_typing() 输入指示
      8. 斜杠命令拦截（slash.rs）
      9. 构建 IM Channel Context 注入系统提示
      10. ChatEngine 流式执行
      11. 发送最终格式化回复（分片 + HTML 转换）
```

Agent 路由支持五层覆盖：
- Per-topic `agent_id`
- Per-group `agent_id`
- Per-channel (Telegram Channel) `agent_id`
- Per-account `agent_id`
- 全局 `default_agent_id`

### 2.4 入站/出站媒体管道

**入站管道**（以 Telegram 为例）：

```
polling 接收 Update
  → 识别媒体类型（Photo 取最高分辨率）
  → api.download_file_to_path() 下载到 channel inbound-temp/
  → MsgContext.media: Vec<InboundMedia>
    → worker/media.rs: convert_inbound_media_to_attachments()
      → 图片：读取文件 → base64 编码 → Attachment.data
      → 非图片：传递 file_path → Attachment.file_path
      → 持久化到 ~/.opencomputer/attachments/{session_id}/
```

**WeChat 入站媒体**（AES-128 解密）：

```
polling 接收消息
  → download_and_decrypt_media()
    → HTTP GET 下载加密数据
    → AES-128-ECB 解密（OpenSSL）
    → save_inbound_bytes() 到 inbound-temp/
  → 支持类型：Image / File / Video / Voice（.silk）
```

**出站管道**（WeChat）：

```
send_outbound_media()
  → materialize_media_data() 获取本地文件
  → upload_media_to_wechat()
    → 生成随机 AES-128 密钥
    → AES-128-ECB 加密
    → HTTP POST 上传到 CDN
    → 3 次 5xx 重试
  → build_outbound_item() 构建消息 payload
  → api.send_message_items()
```

### 2.5 WeChat 深度集成

实现路径：`crates/oc-core/src/channel/wechat/`

- **协议**：OpenClaw 兼容的 iLink HTTP 长轮询（`DEFAULT_WECHAT_BASE_URL`）
- **登录**：QR 码扫码登录 + 自动刷新（3 次重试）
- **会话管理**：session 过期（errcode -14）后暂停 1 小时（`SESSION_PAUSE_DURATION`）
- **上下文 Token**：per-user context_token 持久化到磁盘（`{account_id}.context_tokens.json`）
- **同步缓冲**：`getUpdatesBuf` 持久化避免重启后重复消息
- **媒体加解密**：AES-128-ECB 加密上传/解密下载（openssl crate）
- **格式转换**：Markdown → WeChat 纯文本（去除代码块标记、链接、标题等）
- **消息长度限制**：4000 字符

`WeChatSharedState` 管理：
- `typing_tickets` 缓存（24h TTL）
- `typing_keepalives` 任务注册表
- `paused_until` 会话暂停计时
- `context_tokens` 持久化 store

### 2.6 Telegram Bot 集成

实现路径：`crates/oc-core/src/channel/telegram/`

- **传输**：Bot API 长轮询（`getUpdates`，30s timeout）
- **错误重试**：指数退避（2^n 秒，上限 30s），连续超时 10 次暂停 60s
- **Bot 地址检测**：@mention / reply-to-bot / /命令前缀
- **格式转换**：Markdown → Telegram HTML（`format::markdown_to_telegram_html`）
- **命令同步**：启动时自动 `setMyCommands` 同步斜杠命令到 Bot 菜单
- **代理支持**：channel-level proxy → global proxy 两级回退
- **自定义 API Root**：支持自部署 Telegram Bot API 服务器
- **流式草稿**：DM 使用 `sendMessageDraft`（Telegram 专属），不支持时回退到 send+edit
- **能力**：Polls / Reactions / Draft / Edit / Unsend / Reply / Threads / 7 种媒体
- **消息长度限制**：4096 字符
- **回调按钮**：CallbackQuery 支持（`slash:<command>` 格式转换为 `/command`）

**Forum 支持**：
- 自动检测 `is_forum` 超级群组
- `thread_id`（`message_thread_id`）传递到会话映射
- Per-topic 配置：`require_mention` / `enabled` / `allow_from` / `agent_id` / `system_prompt`

**分层群组配置**：
- 账户级：`SecurityConfig.group_policy` + `groups` map
- 群组级：`TelegramGroupConfig`（require_mention / enabled / allow_from / agent_id / system_prompt / topics）
- Topic 级：`TelegramTopicConfig`
- Channel 级：`TelegramChannelConfig`

### 2.7 Typing 指示器

**Telegram**：调用 `sendChatAction("typing")`，单次发送。

**WeChat**：完整的 typing 生命周期管理：
- `get_or_fetch_typing_ticket()`：从 `getConfig` API 获取 ticket，24h TTL 缓存，3 次指数退避重试
- `start_typing_keepalive()`：每 5 秒发送一次 typing 状态
- `stop_typing_keepalive()`：取消 keepalive 任务
- 发送消息前自动取消 typing 并发送 `TYPING_STATUS_CANCEL`（status=2）
- ticket 过期时自动失效缓存

### 2.8 流式预览

`worker/streaming.rs` 实现了双模流式预览：

1. **Draft 模式**（Telegram DM）：使用 `sendMessageDraft` API，无频率限制，渐进渲染
2. **Message 模式**（群组 / 其他渠道）：先 `send_message` 再 `edit_message`，1 秒间隔

预览流程：
- 累积 `text_delta` 事件
- 1 秒定时器触发发送预览
- Markdown → HTML 实时转换
- Draft 失败时自动降级到 Message 模式
- 最终由 `send_final_reply()` 提交正式消息（替换预览或新发）

### 2.9 其他基础设施

- **Webhook Server**（`webhook_server.rs`）：axum 嵌入式 HTTP 服务器，绑定 127.0.0.1:1456，按 `/{channel}/{account_id}` 路由
- **WebSocket 工具**（`ws.rs`）：共享 WebSocket 连接封装（Discord Gateway 使用）
- **Cancel Registry**（`cancel.rs`）：per-session 取消令牌注册表
- **Process Manager**（`process_manager.rs`）：渠道进程管理

---

## 三、Claude Code 实现

Claude Code 是纯 CLI 工具，**不包含任何 IM 渠道系统**。

- 无消息接收/发送能力
- 无多渠道插件架构
- 不支持 Telegram / WeChat / Discord 等任何即时通讯平台
- 交互方式仅限终端命令行

---

## 四、OpenClaw 实现

### 4.1 渠道插件架构

OpenClaw 采用**适配器组合模式**，每个渠道插件由 20+ 个独立适配器接口组合而成：

```
ChannelPlugin
├── meta: ChannelMeta                          — 元数据（ID、别名、Markdown 能力）
├── capabilities: ChannelCapabilities          — 能力声明
├── gateway: ChannelGatewayAdapter             — 入站消息网关
├── outbound: ChannelOutboundAdapter           — 出站消息发送
├── status: ChannelStatusAdapter               — 健康探测
├── security: ChannelSecurityAdapter           — 安全策略
├── setup: ChannelSetupAdapter                 — 初始化向导
├── pairing: ChannelPairingAdapter             — DM 配对
├── config: ChannelConfigAdapter               — 配置解析/序列化
├── auth: ChannelAuthAdapter                   — 认证适配
├── lifecycle: ChannelLifecycleAdapter         — 生命周期钩子
├── heartbeat: ChannelHeartbeatAdapter         — 心跳检测
├── command: ChannelCommandAdapter             — 命令路由
├── group: ChannelGroupAdapter                 — 群组管理
├── elevated: ChannelElevatedAdapter           — 提权操作
├── directory: ChannelDirectoryAdapter         — 用户/频道目录
├── resolver: ChannelResolverAdapter           — 名称解析
├── allowlist: ChannelAllowlistAdapter         — 白名单管理
├── approval: ChannelApprovalAdapter           — 审批流程
├── streaming: ChannelStreamingAdapter         — 流式输出
├── threading: ChannelThreadingAdapter         — 线程绑定
├── messaging: ChannelMessagingAdapter         — 消息操作（reaction/pin/unsend）
├── mention: ChannelMentionAdapter             — @mention 检测
├── agentPrompt: ChannelAgentPromptAdapter     — Agent 提示注入
├── agentTools: ChannelAgentTool[]             — Agent 工具注册
└── configuredBinding: ChannelConfiguredBindingProvider — 会话绑定
```

注册表通过 `src/channels/plugins/registry.ts` 管理，支持动态插件发现。

### 4.2 已支持渠道（23+ 个）

以下为所有声明了 `channel` 配置的 extension 插件：

| 渠道 | 类别 |
|------|------|
| **Telegram** | 完整实现（Bot API / Pairing / Forum / Draft） |
| **Discord** | 完整实现（Gateway + Voice / Reactions / Threads / Components） |
| **Slack** | 完整实现（Events API / Socket Mode） |
| **WhatsApp** | 完整实现 |
| **Signal** | 完整实现 |
| **iMessage** | 完整实现（BlueBubbles 网关） |
| **BlueBubbles** | iMessage 替代网关 |
| **LINE** | 完整实现 |
| **Feishu** | 完整实现 |
| **QQ Bot** | 完整实现 |
| **Google Chat** | 完整实现 |
| **IRC** | 完整实现 |
| **Matrix** | 完整实现 |
| **Mattermost** | 完整实现 |
| **MS Teams** | 完整实现 |
| **Nextcloud Talk** | 完整实现 |
| **Nostr** | 完整实现 |
| **Synology Chat** | 完整实现 |
| **Tlon (Urbit)** | 完整实现 |
| **Twitch** | 完整实现 |
| **Zalo** | 完整实现（OA 版） |
| **Zalo User** | 完整实现（个人版） |
| **Thread Ownership** | 线程管理插件 |

此外还有：
- **Voice Call** 扩展（Telnyx / Twilio / Plivo）
- **Talk Voice** 扩展（Talk Mode 语音交互）
- **Phone Control** 扩展
- **Synthetic** 测试用合成渠道

### 4.3 DM 配对安全策略

OpenClaw 实现了完整的 DM Pairing 系统（`src/pairing/`）：

**三种 DM 策略**：
- `open`：接受所有 DM
- `allowlist`：仅白名单用户
- `pairing`：未授权用户触发配对挑战

**配对流程**：
1. 未授权用户发送 DM
2. `resolveDmGroupAccessDecision()` 返回 `"pairing"` 决策
3. `issuePairingChallenge()` 生成 8 位配对码（`ABCDEFGHJKLMNPQRSTUVWXYZ23456789` 字母表）
4. 配对码发送给用户
5. 管理员在 CLI 或 Web 界面输入 `pairing approve <channel> <code>` 批准
6. 用户 ID 写入 `allowFrom` store（`{channel}-allowfrom-{accountId}.json`）
7. `notifyPairingApproved()` 通知用户已授权

**安全特性**：
- 配对请求 TTL：1 小时
- 最大待处理请求数：3
- 文件锁保护并发写入
- 加密存储配对状态
- Bootstrap Token 用于设备配对（Gateway 认证）

### 4.4 群组策略

**三种群组策略**（`GroupPolicy`）：
- `open`：接受所有群组消息
- `allowlist`：仅白名单群组
- `disabled`：禁止所有群组消息

**Mention Gating**（`src/channels/mention-gating.ts`）：
- `requireMention`：群组中需 @mention 才响应
- `canDetectMention`：渠道是否支持 mention 检测
- `implicitMention`：隐式 mention（如回复机器人消息）
- `shouldBypassMention`：控制命令绕过 mention 门控

**Command Gating**（`src/channels/command-gating.ts`）：
- `allowTextCommands`：是否允许文本命令
- `commandAuthorized`：命令是否已授权
- `useAccessGroups`：是否使用访问组

### 4.5 语音能力（Voice Call + Talk Mode + TTS + ASR）

OpenClaw 是三个项目中唯一支持语音的：

**Voice Call**（`extensions/voice-call/`）：
- 电话通话管理器（`CallManager`）
- 支持 3 个 Provider：Telnyx / Twilio / Plivo
- 完整呼叫生命周期：initiated → ringing → answered → active → speaking → listening → completed
- 入站策略：disabled / allowlist / pairing / open
- Webhook 安全签名验证
- 通话录音/转录
- 最大通话时长计时器
- E.164 电话号码格式

**Talk Mode**（`extensions/talk-voice/`）：
- 实时语音交互模式
- 媒体流处理

**TTS（文字转语音）**（`src/tts/`）：
- Provider 注册表（`provider-registry.ts`）
- 自动模式切换
- 语音映射配置
- 状态管理

**ASR（语音识别）**（`extensions/deepgram/`）：
- Deepgram 音频处理
- 实时转录运行时

**Realtime Voice**（`src/realtime-voice/`）：
- Provider 注册表
- 端到端实时语音管道

### 4.6 入站/出站媒体管道

**入站**：
- 各渠道插件下载媒体到本地
- 统一转换为 `MsgContext` 格式
- 媒体大小限制（`media-limits.ts`）
- 入站防抖（`inbound-debounce-policy.ts`）—— 非控制命令的文本消息可配置防抖

**出站**：
- `outbound/` 目录统一出站逻辑
- `media-payload.ts` 处理媒体 payload 构建
- 各渠道格式适配

### 4.7 消息去重

- **入站防抖**（`inbound-debounce-policy.ts`）：可配置延迟合并连续文本消息
- **状态反应去重**（`status-reactions.ts`）：debounce 700ms，避免频繁 emoji 状态切换
- **WebSocket 事件去重**：`processedEventIds` Set 防止重复处理

### 4.8 Status Reactions

OpenClaw 独有的 Agent 状态反应系统（`src/channels/status-reactions.ts`）：

通过消息 Reaction emoji 实时展示 Agent 状态：
- 排队：👀
- 思考中：🤔
- 工具调用：🔥
- 编码中：👨‍💻
- 搜索中：⚡
- 完成：✅
- 错误：❌
- 停滞（软）：⏳（10s）
- 停滞（硬）：⚠️（30s）
- 压缩中：✍

可配置 debounce（700ms）和 stall 检测时间。

### 4.9 Draft Stream（流式输出）

`src/channels/draft-stream-controls.ts` + `draft-stream-loop.ts`：

- 可终止的草稿流控制器
- throttle 节流发送
- send-or-edit 模式
- 最终提交 / 清理删除逻辑
- 支持 Telegram `sendMessageDraft` 等原生草稿 API

### 4.10 会话绑定与线程

- **Thread Binding**（`thread-binding-id.ts`）：渠道线程 ↔ 会话绑定
- **Conversation Binding**（`conversation-binding-context.ts`）：渠道对话 ↔ 系统会话映射
- **Configured Binding**：运行时配置的绑定规则
- **Thread Ownership** 扩展：专门管理线程归属

### 4.11 Setup Wizard

完整的渠道配置向导系统：
- `setup-wizard.ts`：交互式配置向导
- `setup-wizard-binary.ts`：二进制入口
- `setup-wizard-proxy.ts`：代理配置
- `setup-group-access.ts`：群组权限配置
- 支持 CLI 交互和 Web UI

---

## 五、逐项功能对比

| 功能 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:-------:|
| **渠道插件架构** | 单一 trait | — | 适配器组合（20+ 接口） |
| **Telegram** | 完整 | — | 完整 |
| **WeChat** | 完整 | — | 完整 |
| **Discord** | 基础框架 | — | 完整（含 Voice） |
| **Slack** | 占位 | — | 完整 |
| **WhatsApp** | 占位 | — | 完整 |
| **Signal** | 占位 | — | 完整 |
| **iMessage** | 占位 | — | 完整 |
| **Matrix** | — | — | 完整 |
| **Mattermost** | — | — | 完整 |
| **MS Teams** | — | — | 完整 |
| **Nostr** | — | — | 完整 |
| **Twitch** | — | — | 完整 |
| **渠道总数** | 12（2 完整） | 0 | 23+（全部完整） |
| **DM Policy** | Open / Allowlist / Pairing(占位) | — | Open / Allowlist / Pairing / Disabled |
| **Group Policy** | Open / Allowlist / Disabled | — | Open / Allowlist / Disabled |
| **DM Pairing** | 未实现 | — | 完整（8 位码 + 审批流） |
| **Mention Gating** | 基础（群组默认 require） | — | 高级（bypass / implicit / command gate） |
| **Command Gating** | 无 | — | 完整（access group + text command） |
| **入站防抖** | 无 | — | 可配置延迟合并 |
| **流式预览** | Draft + Edit 双模 | — | Draft Stream Loop + Edit |
| **Status Reactions** | 无 | — | emoji 状态反应（9 种状态） |
| **Typing 指示** | Telegram 单次 / WeChat 完整 | — | 统一 keepalive 循环 + TTL |
| **入站媒体** | 下载 → base64/path | — | 下载 → 统一格式 |
| **出站媒体** | Telegram 原生 / WeChat AES 加密上传 | — | 各渠道原生适配 |
| **WeChat AES 加解密** | 完整（openssl） | — | 完整（Node.js crypto） |
| **消息分片** | 段落边界智能分片 | — | 渠道适配分片 |
| **格式转换** | MD→HTML(TG) / MD→纯文本(WC) | — | 渠道适配转换 |
| **Agent 路由** | 5 层覆盖 | — | 渠道+账户+绑定规则 |
| **并发控制** | Semaphore(20) | — | 无限制（依赖 Node event loop） |
| **Webhook 服务器** | 内嵌 axum | — | 独立 HTTP 服务 |
| **WebSocket** | 共享封装 | — | 各渠道独立 |
| **Voice Call** | 无 | — | Telnyx / Twilio / Plivo |
| **TTS** | 无 | — | 多 Provider |
| **ASR** | 无 | — | Deepgram |
| **Talk Mode** | 无 | — | 实时语音交互 |
| **Setup Wizard** | GUI 配置面板 | — | CLI + Web 向导 |
| **原生 App** | Tauri 桌面应用 | CLI | 无（服务端） |

---

## 六、差距分析与建议

### 6.1 OpenComputer 的优势

1. **桌面原生体验**：Tauri GUI 傻瓜式配置，无需命令行操作
2. **Rust 性能**：单一 trait 设计简洁高效，Semaphore 并发控制精确
3. **WeChat 深度集成**：完整的 AES 加解密、typing ticket 缓存、会话暂停恢复
4. **Telegram 流式草稿**：DM 中使用 `sendMessageDraft` 实现无频率限制的渐进渲染
5. **Agent 路由精细度**：5 层覆盖（topic > group > channel > account > global），Per-topic 系统提示注入
6. **长消息智能分片**：UTF-8 安全的段落/句子边界切割

### 6.2 OpenComputer 的差距

1. **渠道覆盖**：仅 2 个完整实现 vs OpenClaw 23+ 个，10 个占位模块需要实现
2. **DM Pairing 未实现**：类型已定义但回退到 Allowlist，缺少配对码生成和审批流程
3. **无语音能力**：缺少 Voice Call / TTS / ASR / Talk Mode 全部语音栈
4. **无入站防抖**：快速连续消息会分别触发 LLM 调用，浪费资源
5. **无 Status Reactions**：用户无法通过 emoji 看到 Agent 当前状态
6. **Mention Gating 不够灵活**：缺少 implicit mention / command bypass / access group 等高级策略
7. **无 Command Gating**：群组中无法限制特定命令的执行权限
8. **Discord Voice 未实现**：OpenClaw 的 Discord 插件支持语音频道（142 个源文件），OC 仅有基础框架

### 6.3 建议优先级

| 优先级 | 建议 | 理由 |
|-------|------|------|
| **P0** | 完成 Discord 完整实现 | Gateway 框架已有，补全 text/voice/reaction |
| **P0** | 完成 Slack 实现 | 企业用户刚需 |
| **P1** | 实现入站防抖 | 避免 LLM 资源浪费 |
| **P1** | 实现 DM Pairing | 类型已定义，补全生成/验证/审批逻辑 |
| **P1** | 实现 Status Reactions | 提升 UX，代码量小 |
| **P2** | 完成 WhatsApp / Signal | 用户量大的渠道 |
| **P2** | 实现 Command Gating | 群组安全必备 |
| **P3** | Voice Call 基础框架 | 长期差异化功能 |
| **P3** | 完成剩余渠道 | Matrix / MS Teams / Nostr 等 |

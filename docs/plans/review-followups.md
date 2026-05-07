# Review Followups — 审查决定但本期不改的问题

> 本文档登记**已被 code review 识别、但当期 PR 决定不修**的问题。每条记录的目的是：让债务可见、可检索、可调度，避免下一次有人撞上同一个问题再重新发现。
>
> 登记规则见 [AGENTS.md](../../AGENTS.md) "Review Followups 登记"段。

## 文档使用方式

- **新增一条 Follow-up**：在最下方"Open"段追加一个 `### F-XXX` 子节，编号递增（不复用），按下方"条目模板"填写。一次提交一个原子条目；多个 review 想法分开记。
- **关闭一条**：把整段从 "Open" 移到底部 "Closed" 段，附 commit / PR 链接和关闭日期；不要原地删除（保留可检索的历史）。
- **不强制顺序**：可以打散在多个版本里慢慢清。
- **不当作 backlog**：这里只放"review 决定不改"的；功能 backlog 放别处（issue tracker / 其他 plan）。

## 条目模板

每条 Follow-up 至少包含：

```
### F-XXX 简短标题

- **来源**：YYYY-MM-DD `<功能名>` PR / `/simplify` review / 手动审查
- **现象**：一两句描述当前是什么样
- **为什么留**：当期不修的具体理由（范围 / 优先级 / 依赖 / 风险）
- **改的话要做什么**：列出涉及文件、需要的设计决策、可能的迁移路径
- **影响面**：当前是否有用户可见的 bug / 安全 / 性能问题；如果只是"不优雅"就明说
- **触发时机建议**：什么场景下应该顺手收掉（例如 "下一次动这块代码时" / "做某某独立重构 PR 时"）
```

---

## Open

### F-063 `/sessions` 搜索：picker 期 `list_agents()` / `list_sessions(None)` 仍是全量 IO

- **来源**：2026-05-07 `/sessions` 搜索能力 PR / `/simplify` review
- **现象**：[`crates/ha-core/src/slash_commands/handlers/session.rs::handle_sessions`](../../crates/ha-core/src/slash_commands/handlers/session.rs) 每次调用都跑：
  1. [`agent_loader::list_agents`](../../crates/ha-core/src/agent_loader.rs#L359) — 读全部 agent.json + 每个 agent 一次 SQLite `count(memory)`，但 picker 只用 `id → name` 映射
  2. `list_sessions(None)` — 拉全部 SessionMeta 行；no-query 路径接着 `take(30)` 丢弃后续
- **为什么留**：`/sessions` 是用户手动触发，非热点。当前 ≤ 10 agents、≤ 1k sessions 量级感觉不到延迟
- **改的话要做什么**：
  - `agent_loader` 加轻量 `list_agent_names() -> HashMap<String, String>` 跳过 memory_count（参照已有 `list_agent_ids` at `agent_loader.rs:429`）
  - no-query 分支换 `list_sessions_paged(None, All, Some(SESSION_PICKER_LIMIT_OVERFETCH), None, None)` —— 注意 `list_sessions_paged` 不在 SQL 层过滤 `is_cron` / `parent_session_id`，得 over-fetch 后再 filter+truncate（如 limit=100 给 cron/subagent 留头）
- **影响面**：纯性能；当前不会引发 bug
- **触发时机建议**：有用户报告 1k+ sessions 库下 `/sessions` 卡顿时；或重构 `list_sessions_paged` 加入 type filter 时一并处理

### F-064 Project emoji+name 拼接在 `channel/worker/slash.rs` 仍有两处内联

- **来源**：2026-05-07 `/sessions` 搜索能力 PR / `/simplify` review
- **现象**：本期已在 [`Project::display_label()`](../../crates/ha-core/src/project/types.rs) 抽出 emoji+name 格式化 helper，并在 `slash_commands/handlers/session.rs::build_picker_item` 用上。但 [`channel/worker/slash.rs:534-537`](../../crates/ha-core/src/channel/worker/slash.rs)（项目按钮 label）和 [`channel/worker/slash.rs:852-855`](../../crates/ha-core/src/channel/worker/slash.rs)（项目 picker text fallback）仍内联同一格式
- **为什么留**：两处是 pre-existing 代码、与 `/sessions` PR 主题无关，同 PR 改会扩散 diff
- **改的话要做什么**：把两处内联换成 `project.display_label()`（注意：那两处用的是 `ProjectPickerItem`，不是 `Project`，可能要在 `ProjectPickerItem` 上挂同名 helper 或 builder 时套）
- **影响面**：纯代码重复
- **触发时机建议**：下一次动 channel slash worker 项目 picker 渲染时

### F-065 `/sessions` 消息内容 FTS 搜索还有重复显示问题（GUI）

- **来源**：2026-05-07 `/sessions` 搜索能力 PR
- **现象**：[`src/components/chat/ChatScreen.tsx`](../../src/components/chat/ChatScreen.tsx) 的 `case "showSessionPicker"` 把 `result.content`（Rust 端组装好的 markdown body）push 一遍，再用 `action.sessions` 自己拼一遍 markdown push 第二遍 —— 用户看到两条几乎一样的 event 消息。`showProjectPicker` 同款问题（pre-existing）
- **为什么留**：本期是搜索功能 PR，重构事件渲染契约不在主题
- **改的话要做什么**：要么 `result.content` 在 picker action 时不 push；要么 action handler 不再自拼 markdown（直接信任 `result.content`）。后者更彻底但要 i18n 在 server 端做
- **影响面**：UX 视觉冗余，但不影响功能
- **触发时机建议**：做 picker UI / event 消息系统重构时一并处理

### F-057 IM channel 主动消息 / 媒体能力补完（跨 channel）

- **来源**：2026-05-05 IM channel 全量审计 + 2026-05-06 codex review 回归
- **现象**：本批**写过** QQ Bot c2c/group msg_type=7 两步上传 + LINE imageMessage/videoMessage/audioMessage HTTPS URL 路径；但 [`channel/worker/dispatcher.rs::to_outbound_media`](../../crates/ha-core/src/channel/worker/dispatcher.rs#L728) 优先给 `MediaData::FilePath`（hope-agent 本地缓存路径），而 QQ Bot V2 上传 / LINE message object 都只接收公网 HTTPS URL —— 两边在 plugin 内部 `match data { Url(_) => ..., _ => continue }` 把 FilePath 静默跳过。结果声明 `supports_media` 反而让 dispatcher 不再追 link fallback → 用户附件两头不到位。
  - **2026-05-06 已回退**：QQ Bot + LINE `supports_media` 重新设为空，恢复 dispatcher 的链接文本兜底。两套两步上传 helper（`post_*_files` / `send_*_media` / `dispatch_media`、LINE message-object 构造）保留备用，等本地附件中转基建就绪再开
- **剩余 channel 状态**：均降级为下载链接文本：
  - Slack — files v2 流程（`files.getUploadURLExternal` + `files.completeUploadExternal`）
  - Google Chat — `media.upload` + `attachment` resource name（需 Drive scope）
  - Signal — signal-cli `--attachment <path>`
  - iMessage — imsg `send_attachment` 子命令（待 stdio 协议字段）
  - WhatsApp — bridge `media` 字段
  - LINE / QQ Bot — 待本地附件中转 + 公网 HTTPS 暴露基建（hope-agent 自托管端点 / 用户配置 `public_base_url` 转发缓存附件）
  - QQ Bot — channel/dms 端点 V2 不开放原生媒体上传，仍要走链接
- **为什么留**：核心是缺"本地缓存附件 → 公网 HTTPS URL"的中转基建，而不是各 channel 的协议代码；本批已修核心稳定性问题（msg_seq / 心跳 / INVALID_SESSION / 速率限制 / chat_type 等），富媒体不阻塞首发文本
- **改的话要做什么**：参照 [`channel/worker/dispatcher.rs::partition_media_by_channel`](../../crates/ha-core/src/channel/worker/dispatcher.rs) 已有的能力声明驱动下，逐 channel 补 `*/media.rs` 模块；先做 Slack（用户最多）+ Signal（调试方便）
- **影响面**：能力承诺 vs 实际不一致，dispatcher 自动降级为链接文本但用户视觉体验差
- **触发时机建议**：用户报"图片发不出来"时按 channel 优先级排队；新增 OAuth scope 时同步评估

### F-058 IM channel WebSocket / 长连接 + IRCv3 + chat_type 协议层细化（跨 channel）

- **来源**：2026-05-05 IM channel 全量审计
- **现象**：协议层补完短板：
  - **Discord** HEARTBEAT 缺 jitter（[`gateway.rs:205`](../../crates/ha-core/src/channel/discord/gateway.rs#L205)）；IDENTIFY connection properties `os: "macos"` 写死无视实际平台；`MESSAGE_CREATE` thread_id 永远 None（实际 Discord forum 消息走 thread channel_id）；ChatType 只 Dm/Group 缺 Channel/Forum 细化（要求 cache `channel.type`）
  - **Slack** disconnect 信封未触发立即重连（[`socket.rs:265`](../../crates/ha-core/src/channel/slack/socket.rs#L265)）；action_id 长度上限 ≤ 255 校验
  - **QQ Bot** shard `[0,1]` 写死、IDENTIFY `properties` 空；sandbox endpoint 切换；event_id 主动/被动消息区分
  - **Signal** SSE `data:` 多空格剥取不一致；daemon readiness 主动 poll `/api/v1/check` 而非 sleep 2s
  - **IRC** IRCv3 `CAP LS 302` + SASL PLAIN 协商（不接 SASL 在 Libera 等主流网络可能强踢）；IRCv3 message-tags 解析（`@key=value` 前缀）；channel name 用户输入自动补 `#`
  - **iMessage** RPC 方法名（`chats.list` / `watch.subscribe` / `sendTyping`）需对照 [`steipete/imsg`](https://github.com/steipete/imsg) 实际 RPC 暴露面；`is_group` 完全信赖字段而非 participants.len() fallback
- **为什么留**：单实例不可见，规模化或边界场景才暴露；IRCv3 SASL 是单 channel 50-100 行重写，独立 PR 更清楚
- **改的话要做什么**：jitter 用 `interval * rand::random::<f64>()`；`os: std::env::consts::OS`；shard 字段从 capabilities 推断；tungstenite Message::Close.code 路由模板已在 Discord 落地，可参考迁移 Slack/QQ；IRCv3 见 <https://ircv3.net/specs/extensions/sasl-3.1.html>
- **影响面**：稳定性 / 服务端可观测性 / 现代 IRCd 兼容性
- **触发时机建议**：单 channel 大流量场景报"频繁断线"时；接 Libera/OFTC 网络的 IRC 用户报告"被踢"时

### F-059 IM channel 速率限制 / 幂等增强（跨 channel）

- **来源**：2026-05-05 IM channel 全量审计
- **现象**：本批用 [`channel/rate_limit.rs`](../../crates/ha-core/src/channel/rate_limit.rs) 接了 Discord/Slack 429 + Retry-After；剩余：
  - **Telegram** teloxide `Throttle` adaptor 包装 `Bot`（自动尊重 FloodWait `RetryAfter` 错误），需要在 [`Cargo.toml`](../../crates/ha-core/Cargo.toml) telmit feature 加 `throttle`，且持有 Bot 类型从 `Bot` 改 `Throttle<Bot>`，diff 较大
  - **LINE** push API `X-Line-Retry-Key` UUID4 幂等头（避免网络重试重复扣费）+ 429 处理
  - **Feishu** `auth.rs` 并发首次取 token 加 `OnceCell` singleflight 锁
  - **WhatsApp** 连续失败 ≥3 后 `consecutive_failures` 清零，无最终告警；接 [`channel/start_watchdog.rs`](../../crates/ha-core/src/channel/start_watchdog.rs) 同款指数退避
- **为什么留**：边界并发 / 长期失败场景才暴露；teloxide Throttle 改动面较大需独立验证
- **改的话要做什么**：Telegram 切 Throttle；LINE push 生成 UUID4 复用；Feishu auth 用 `tokio::sync::OnceCell` 单飞；WhatsApp 改用 watchdog
- **影响面**：偶发计费重复 / token 配额浪费 / 长期挂掉无告警
- **触发时机建议**：用户报"消息重复" / "Feishu 503" / "Telegram FloodWait 螺旋" 时

### F-060 IM channel 配置 / 错误信息 / 安全细节（跨 channel）

- **来源**：2026-05-05 IM channel 全量审计
- **现象**：纯优雅性 / 非阻塞首发的细节，逐 channel 列：
  - **Telegram** `sendMessageDraft` 4xx fallback 软降级；媒体退到 `sendDocument` 丢类型语义（应按 media_type 分发到 `send_voice` / `send_animation` / `send_sticker`）
  - **WeChat** 长轮询 timeout 1ms 边界 clamp（`next_timeout_ms.clamp(5_000, 60_000)`）；登录 `current_api_base_url` redirect 后持久化复用
  - **Slack** `subtype=file_share` 子类型显式处理；slash command response_action ack 带 payload
  - **Feishu** `auth/v3/tenant_access_token/internal` trailing slash 去除；ack `biz_rt` 写实际处理耗时；ack `payload_encoding`/`payload_type` 透传源帧；`card.action.trigger` ack 带 update payload
  - **QQ Bot** mention space 补空（`strip_mention_tags`）；event_id 主动/被动消息区分；botpy 那边的 sandbox bool 字段
  - **LINE** webhook 失败返回 404 而非 403 oracle；postback origin 校验（chat_id 与 pending approval 比对，防群聊跨 session 伪造审批）
  - **Google Chat** `webhook_server` body limit 1MB → 4-8MB（多附件 message 接近）；message resource name 文档化
  - **WhatsApp** baseUrl 缺失 + bridge HTTP 契约 README/SKILL 文档；empty text 返回 err 而非 ok
  - **IRC** channel name 用户输入自动补 `#`；IRCv3 message-tags 解析；reconnect writer 重建用 `Arc<Mutex<Option<...>>>` 一次替换更直观
  - **Signal** localhost vs 127.0.0.1（**已在本批修**）；readiness `/api/v1/check` 探活；username `u:` / `@` recipient form 完整支持
  - **iMessage** `is_group` 完全信赖字段；JSON-RPC 帧协议探测（NDJSON vs Content-Length）；typing 实际能力验证后调整 `supports_typing`
  - **跨 channel** approval callback `try_dispatch_interactive_callback(data, source)` 应加 `source_chat_id` 参数与 worker pending map chat_id 比对（LINE postback / 群聊场景跨用户伪造审批的根本防御）
- **为什么留**：每条独立改动小（≤10 行），按 channel 维度逐个收效率最高
- **改的话要做什么**：每个 bullet 独立改；可分多次小 PR 收
- **影响面**：用户体验 / 调试体验 / 极端边界 + LINE postback 是潜在安全洞
- **触发时机建议**：下一次动到对应 channel 文件时顺手收

### F-061 IM channel sink 与 stream task 在同一 event 流上重复检测 round 边界

- **来源**：2026-05-06 split 模式 per-round 流式预览 PR `/simplify` review（quality + efficiency 两只 agent 同样标记）
- **现象**：[`ChannelStreamSink::send`](../../crates/ha-core/src/chat_engine/types.rs)（types.rs:229-265）和 [`spawn_channel_stream_task`](../../crates/ha-core/src/channel/worker/streaming.rs)（streaming.rs:151-180）都在同一份 event 流上做 round 边界检测：
  - sink 用 `event.contains("\"type\":\"tool_call\"")` 等 cheap-string 检测维护 `RoundTextAccumulator.in_tool_phase`（私有字段）
  - stream task 自己又一次 `event_str.contains("\"type\":\"tool_call\"")` + 本地 `in_tool_phase: bool` 副本，并用 `extract_text_delta` 重新解析同一份 text_delta JSON（sink 已经用 `serde_json::from_str` 解析过推到 `current.text`）
  - 同一份契约（"BTreeMap key 字母序，type 不在开头"）通过两个独立的 `contains` 检查实现，仅 [`worker/tests.rs::tool_call_event_contains_anchor_for_split_streaming_boundary`](../../crates/ha-core/src/channel/worker/tests.rs) 一处兜底
- **为什么留**：unify 两条路都比 cleanup 重得多——
  - 暴露 `in_tool_phase` 私有字段没用：sink 的 `on_text` 会立刻翻 false，stream task 在事件后读总看到 false
  - 推迟 sink flag 翻转破坏 accumulator 现有 invariant；stream task 用 `completed.len()` 推断边界仍是另一种重复检测
  - 干净方案是把 sink → stream task 的 `mpsc::Sender<String>` 换成 `mpsc::Sender<StreamEvent>`，sink 解析后发结构化变体——但前端 `channel:stream_delta` 事件目前消费 raw JSON string，触及 EventBus 契约 + `extract_text_delta` helper + 4 个测试
  - 实际开销 ≈ 0：text_delta JSON ~40 byte，全 turn 双解析合计 ms 级
- **改的话要做什么**：
  - 干净版：定义 `enum StreamEvent { TextDelta(String), ToolCall { call_id, name }, ToolResult { medias }, Other(String) }`，sink 发结构化（保留 raw `Other` 兜底前端需要 raw 的场景），stream task 直接 match 不再 contains
  - 折中版：保留 raw channel 但 stream task 不再 contains——而是周期性 `lock` accumulator 读 `completed.len() / current.text.len()` 推断状态（多了 mutex 抖动，但少了重复 parse）
- **影响面**：纯 conceptual purity，无用户可见 bug；性能开销极低；潜在风险是字符串契约改了两处都得改
- **触发时机建议**：下次重写 EventBus 事件契约 / 加新 channel-side stream 事件类型时顺手做

### F-062 IM channel `deliver_media_to_chat` / `deliver_split` pre-final 50ms gap 不分渠道 capability

- **来源**：2026-05-06 split 模式 per-round 流式预览 PR `/simplify` review（efficiency agent 标记，两条相关项）
- **现象**：[`deliver_media_to_chat`](../../crates/ha-core/src/channel/worker/dispatcher.rs)（dispatcher.rs:1212-1245）和 [`deliver_split`](../../crates/ha-core/src/channel/worker/dispatcher.rs)（dispatcher.rs:1011-1027）每条媒体 / 每条 pre-final round 之间硬编码 `tokio::time::sleep(50ms)`，注释说"Telegram and LINE flood-protect tight loops"——但实际所有 12 个 channel 一律跟着 sleep，包括 Discord（5 req/s/channel 容量）、Feishu cardkit（独立 RPC 不撞 IM rate limit）、内部 webhook 类（无 rate limit）。一条消息 + 5 张图 = 250ms 纯等待
- **为什么留**：
  - 修要在 [`ChannelCapabilities`](../../crates/ha-core/src/channel/types.rs) 加 `min_send_interval_ms: Option<u64>` 字段，跨 12 个 plugin 都要补声明
  - 还要校准每个渠道的实际限速（默认 None 还是 Some(50)？没人撞过的话只能查文档/做 stress test）
  - 现有 50ms × N 在 IM 场景几乎无感（用户已经看到消息流式出现），没用户报告"图片发太慢"
  - 属于"想到的优化"而非"撞到的问题"
- **改的话要做什么**：
  - `ChannelCapabilities` 加 `min_send_interval_ms: Option<u64>`（serde `default` 让旧持久化兼容）
  - 12 个 plugin 的 `capabilities()` 逐个声明：Telegram/LINE → `Some(50)`；Discord/Feishu/Slack → `None` 或更小；其他逐个查文档
  - `deliver_media_to_chat` / `deliver_split` 读字段决定是否 sleep
- **影响面**：纯效率优化，无用户可见 bug；非限速渠道当前每张媒体多 50ms 等待
- **触发时机建议**：用户实际报"Discord 群发图慢"或"内部 webhook 批量发慢"时；或下次有人在 dispatcher.rs 动 media 路径时顺手收

### F-051 `currentSessionMeta` + `incognitoEnabled` 三处复制可推进 `useQuickChatSession` / `useChatSession`

- **来源**：2026-05-03 QuickChat → MessageList 复用 PR `/simplify` review (reuse + quality + efficiency 三只 agent 同样标记)
- **现象**：[`ChatScreen.tsx:145-159`](src/components/chat/ChatScreen.tsx#L145-L159) / [`QuickChatWindow.tsx:33-48`](src/QuickChatWindow.tsx#L33-L48) / [`QuickChatDialog.tsx:38-50`](src/components/chat/QuickChatDialog.tsx#L38-L50) 三处都复制了：`useMemo(() => sessions.find(s => s.id === currentSessionId))` + `incognitoEnabled = currentSessionId ? meta?.incognito : draftIncognito` 这套派生。ChatScreen 还多一个 `incognitoDisabledReason` 派生；QuickChat 没有 project / channel 概念所以不需要
- **当前选择**：不动。本期 PR 主题是"删 QuickChatMessages 复用 MessageList"，把派生推进 hook 是独立重构（影响 ChatScreen 顶层）；本期接受 2 个新复制点（QuickChatWindow / QuickChatDialog），从 1 处升到 3 处，仍在"小到不值得抽"的阈值附近
- **改的话要做什么**：
  - 在 [`useQuickChatSession.ts`](src/components/chat/useQuickChatSession.ts) 内部计算 `currentSessionMeta` + `incognitoEnabled`（`draftIncognito` 已经在 hook 里），return 出来；两个 quick 入口直接 destructure
  - `useChatSession` 同款追加（ChatScreen 的派生最复杂，含 `incognitoDisabledReason` / `isCronSession` / `isSubagentSession`，可能值得专门一个 `useSessionMeta(session)` 子 hook）
  - 三处 `handleIncognitoChange`（3 行 callback）也可以一并推进 hook，但单独看不值得抽
- **影响面**：纯代码卫生 + 防漂移。当前没有用户可见 bug；性能上 `Array.find` 在 sessions 数 ≤100 量级是 µs 级，无忧
- **触发时机建议**：下次有人为了某个 incognito / session-meta 派生 bug 同时改这三个文件时（说明复制成本开始外溢），把派生推进 hook

### F-050 `_clientId` 下划线前缀挂在 `Message` 公共导出 interface 上

- **来源**：2026-05-03 react-virtuoso 移除后续 `/simplify` review (quality agent)
- **现象**：[`src/types/chat.ts:160`](src/types/chat.ts#L160) 给 `Message` 加了 `_clientId?: string` 运行时字段，用来在 placeholder→DB 转换时维持 React row key 稳定。下划线前缀只是命名约定，TypeScript 不会因此把它从公共类型隔离——所有 `Message` 消费者（前端组件、test、merge 工具、序列化 boundary）都看得到这个字段
- **当前选择**：不动。带上下划线 + 8 行 WHY 注释（讲清楚仅运行时、不持久化、不上 wire）+ `mergeMessagesByDbId` 是唯一写入点，足够防御。改 typing 改动面太大（要拆 `MessageRuntime extends Message` 之类，影响 200+ 处消费）
- **改的话要做什么**：
  - 拆出 `interface MessageRuntime extends Message { _clientId?: string }`，只在 useChatStream / chatScrollKeys / chatUtils.mergeMessagesByDbId 这条 client 链路用 `MessageRuntime`；其它路径继续用纯 `Message`
  - 检查所有把 `Message` 序列化 / 落库 / 跨进程传输的 boundary（Tauri command return、HTTP API、IM channel forward、`SessionDB::insert_message`）有没有不小心把 `_clientId` 带过去——目前应该都没有（runtime ref 不会转 JSON 跨进程），但要写显式 strip 兜底
- **影响面**：纯类型卫生 + 防御性。当前没有用户可见 bug
- **触发时机建议**：下次有人专门动 chat 类型层 / serialize boundary 时；或某次发现 `_clientId` 真的混进了不该混进的地方时



### F-049 ~~三处~~两处消息流 scroll listener / atBottom / ResizeObserver 复制

- **来源**：2026-05-03 react-virtuoso 移除后续 `/simplify` review (reuse + quality agents)
- **2026-05-03 状态更新**：原 3 处复制中 `QuickChatMessages.tsx` 已整体删除——快捷会话浮窗 / dialog 改为直接复用 `MessageList`（详见对应 commit）。剩下 [`MessageList.tsx`](src/components/chat/MessageList.tsx) ↔ [`TeamMessageFeed.tsx`](src/components/team/TeamMessageFeed.tsx) 两处复制
- **现象**：两个组件的滚动跟随逻辑相似度很高：byte-相同的 `el.scrollHeight - el.scrollTop - el.clientHeight < THRESHOLD` 距底判定、近乎相同的 RAF-节流 scroll listener、近乎相同的 ResizeObserver pin-to-bottom 块、相同的 reverse-find-last-user + scrollIntoView 块
- **当前选择**：不动。MessageList 复杂（forceFollow / lastUserKey / pendingScrollTarget / askUser+planCard footer），TeamFeed 极简，差异点仍然大于共性，强行抽 hook 会推高耦合
- **改的话要做什么**（hook 抽不动但纯 helper 函数可以抽）：
  - 新建 `src/components/chat/chatScroll.ts`，提供：
    1. `isNearBottom(el, threshold = 48): boolean` —— 两处都在用的距底判定
    2. `scrollLastUserIntoView(el, messages, opts?)` —— 两个组件字节相同的 reverse-find + `scrollIntoView({ block: "start", behavior: "smooth" })`
  - RAF-scroll listener scaffold + ResizeObserver-pin 块仍然各自留在组件里——它们绑定 ref，抽出来就是隐形 hook 化，违背原决策
  - 顺手把 [`ChatSidebar.tsx`](src/components/chat/ChatSidebar.tsx) 里 `< 100px` 距顶判定改成 `isNearTop` 同款 helper
- **影响面**：纯代码卫生。当前是可控的复制——剩 ~60 行重复，都是原子小块，单点修复不易踩坑
- **触发时机建议**：下次有人为了某个滚动 bug 同时改这两个文件时（说明复制成本开始外溢），顺手把 helper 抽出来



### F-048 ChatScreen tray:focus-session effect 的 react-hooks/exhaustive-deps warning

- **来源**：2026-05-02 react-virtuoso 迁移 `/simplify` 收尾发现（main 预存 warning，不是本次引入）
- **现象**：[`ChatScreen.tsx:550-555`](src/components/chat/ChatScreen.tsx#L550-L555) 的 useEffect deps 写 `[session.handleSwitchSession]`，ESLint `react-hooks/exhaustive-deps` 抱怨「访问 `.handleSwitchSession` 形式上依赖整个 `session` 对象，应该把 `session` 放进 deps」。由 commit `f596e8d0`（refactor tray simplify）引入，至今未修
- **当前选择**：不动。两条朴素「修法」都不可取：
  1. **加 `// eslint-disable-next-line`**——掩盖 lint 提示而不是解决问题，下次同款代码再撞还是要 disable，越积越多
  2. **把 `session` 整个放进 deps**——会让 effect 在 session 对象引用变化时重订阅 tray 监听器，引入 listener churn / 重复订阅风险，是真 regression
- **改的话要做什么**：根因是 `session` 是 `useChatSession` hook 返回的大对象，里面所有方法每次 render 都新建。正确做法：
  - 把 `handleSwitchSession` 在 [`useChatSession`](src/components/chat/hooks/useChatSession.ts) 内部用 `useCallback` 包好（应该已经是 useCallback，确认即可），然后在 ChatScreen 顶部 destructure：`const { handleSwitchSession } = session`
  - effect 改 deps 为 `[handleSwitchSession]`——既消除 warning，也保留「只在 handleSwitchSession 变时重订阅」的语义
  - 同款问题在 ChatScreen 里可能还有其它处（grep `session\.\w+` 在 effect deps 里），一并整理
- **影响面**：纯 lint 卫生 + 防御性。当前是 warning 不是 error，CI 不会红
- **触发时机建议**：下次有人专门做 ChatScreen / useChatSession refactor 时；或单独的 lint 卫生 PR 一并清理同款问题



### F-040 mid-turn plan-state probe 用 AtomicU64 version gate 跳过 RwLock 读

- **来源**：2026-05-02 mid-turn plan mode rebuild 修复 `/simplify` review (efficiency agent)
- **现象**：[`streaming_loop.rs`](crates/ha-core/src/agent/streaming_loop.rs) 每个 round 头都跑一次 `crate::plan::get_plan_state(sid).await`（`tokio::sync::RwLock` read + HashMap lookup）+ 一次 `plan_agent_mode_for_state` 构造 `PlanAgentConfig::default_config()`（~14 个 String alloc）。50 round 上限 ×~50ns RwLock + ~14 个 String alloc/round = ~5μs，相比 LLM round（秒级）当前可忽略
- **为什么留**：当前规模不构成 hot-path 瓶颈；过早优化没收益。改起来要加 [`plan/store.rs`](crates/ha-core/src/plan/store.rs) 全局 `static PLAN_STATE_VERSION: AtomicU64` + `set_plan_state` 写时 `fetch_add`，streaming_loop 缓存上轮 version，相等就 skip。改动小但要确保所有 plan state 变更入口都 bump version
- **改的话要做什么**：
  - `plan/store.rs` 加 version counter
  - `set_plan_state` / `transition_state` 写路径 bump
  - `streaming_loop` round 头先比对 version；version 没变则跳过整个 mode 推导
  - 配套：`PlanAgentConfig::default_config()` 改返回 `&'static PlanAgentConfig`（`OnceLock` lazily 初始化），让全部 turn-start 路径也受益
- **影响面**：纯性能，无功能 / 安全影响
- **触发时机建议**：plan mode 用户量上来后、tool loop 实际 round 数上来时；或者下一次动 plan store 时顺手


### F-039 PlanPanel 在 rapid 连续 submit_plan 场景偶尔不刷新内容（root cause 未定）

- **来源**：2026-05-02 plan inline comment 三件套修复期间发现。用户连续评论 → 模型多次 resubmit_plan → 右侧 PlanPanel 偶尔仍显示旧 plan
- **现象**：理论链路（`submit_plan` emit `plan_submitted` 带 `content` → `usePlanMode` listener `setPlanContent`）应该工作，但用户实际见不到刷新。当时为了赶紧收掉用户痛点，加了"主动 refetch `get_plan_content`"作为 belt-and-suspenders，掩盖了真问题
- **当前状态**：refetch 已经收紧到只在 `payload.content` 缺失时兜底（[`usePlanMode.ts`](src/components/chat/plan-mode/usePlanMode.ts) plan_submitted handler）。正常路径只走 `setPlanContent(payload.content)` 一次
- **待办**：复现并定位真因。可能方向：
  - React state batching 在 EventBus 同步 emit 多个事件时合并掉中间 setPlanContent
  - PlanPanel memoization / props 引用相等导致 skip render
  - listener closure stale（虽然 deps `[currentSessionId, setPlanState]` 看起来对）
  - backend `plan_submitted` 实际未带 content（理论 emit 永远带，但 code path 可能有遗漏）
- **触发条件**：用户报告再次出现"评论后 panel 不刷新"或本人手动复测能稳定复现
- **优先级**：低（refetch 兜底覆盖了，UX 不可见）

### F-038 enter_plan_mode 对 single-deliverable user-facing 创意任务覆盖不足

- **来源**：2026-05-02 plan/task 解耦重构后跑「网页贪吃蛇」实测发现模型不进 plan mode 直接动手，零询问视觉/控制/玩法等用户偏好。当期试过方案 B（激进重写）和方案 C（保守兜底）两版均回滚，决定先观察、保留两套备选方案待后续触发场景再决定
- **现象**：用户输入「我想开发一个简单的网页版的贪吃蛇游戏」，模型 thinking 判断「single-step / can be done in fewer than 3 steps」（命中 enter_plan_mode 当前描述的 "When NOT to Use" 第四条）+ HUMAN_IN_THE_LOOP_GUIDANCE 的 "low-cost reversible just do it" / "pure style detail user has no opinion on" → 直接调 task_create 拆 todo 后开始 write HTML，全程零询问。同类场景预计还有：登录页、dashboard 小组件、深色模式、单页 UI 设计等 user-facing 创意任务
- **根因**：当前 enter_plan_mode 描述偏中立平衡（"trivial / fewer than 3 steps 不进 plan"），HUMAN_IN_THE_LOOP_GUIDANCE 又说"low-cost just do it"——两个独立判断都给"别打扰"信号，**贪吃蛇这种 single-file 创意小项目两边都被劝退**。
- **当前选择**：不动。**用户对过度询问的担忧 > 修复贪吃蛇的收益**。两个被否决的方案登记如下，将来覆盖率不足时再考虑

#### 方案 C（保守兜底，曾上线 commit 0fd75e1c 后回滚）

只在 `When NOT to Use` 后追加一段「Edge Case Tiebreaker」：

> If a single-deliverable task is **user-facing** (the user will run / read / interact with the result, e.g. a small game, login page, dashboard widget) AND has multiple reasonable directions in **visual style**, **control scheme**, or **scope** (MVP vs full-featured), lean toward entering plan mode rather than guessing. Limit this rule to those three dimensions only — do NOT extend it to tone / depth / formatting / naming / phrasing details, which the user typically has no opinion on (translate / summarize / draft email / clean up comments / rename variables stay in normal mode).

两层防御：
1. 限定到 single-deliverable + user-facing + 三个明确维度（visual style / control scheme / scope）
2. 显式 deny list：tone / depth / formatting / naming / phrasing 不算

回滚原因：用户对方案 C 仍有过度询问担忧，决定连保守版也先不上，纯兜底方案存档备用。

#### 方案 B（中庸重写，曾尝试 commit a02f570f 后回滚）

完整重写 enter_plan_mode 描述结构：

1. 顶层语气从中立 "prefer for non-trivial" 改为主动 "use this tool **proactively**" + "**prefer using unless tasks are simple**"
2. 把 5 类触发条件重组为 7 条独立编号条款：
   - New Feature / Working Artifact（producing something user will run/read/interact with）
   - Multiple Valid Approaches（multiple ways with comparable trade-offs）
   - Code / Design Modifications（changes to existing behavior/structure）
   - Multi-File / Multi-Section Changes（3+ files OR 3+ logical sections）
   - Unclear Requirements（need to explore first）
   - User Preferences Matter（visual / controls / palette / scope —— **不含** tone / depth）
   - Non-Code Domains（writing / research / analysis / information organization）
3. 删除「fewer than 3 steps」歧义条款（贪吃蛇 single-file 但是 multi-section，用步数判断会误伤）
4. "When NOT to Use" 收紧到只剩 typo / 单函数清晰需求 / step-by-step 详细指令 / 纯 Q&A / 一次性脚本明确输出
5. 新增 GOOD / BAD examples 段：贪吃蛇 / 登录页 / 深色模式 / 小工具 UI 归 GOOD；改 typo / 加 log / 改名 / 跑测试 / Q&A 归 BAD
6. **不加** "If unsure, err on the side of planning" 兜底（这条最容易引发过度询问，方案 B 故意不加；当时 a02f570f 加了，正是 review 否掉的核心理由之一）
7. **不加**「写文章 / 调研」类 GOOD examples（这类用户经常希望"快速给一版我看了再调"）

回滚原因：方案 B 的 7 条触发条件 + GOOD examples 段叠加后，对边界场景（README / 调研 / 翻译 / 邮件起草 / 总结）的过度询问风险无法量化排除，用户决定先不上。

#### 测试用例对比

| 用例 | 当前不动 | 方案 C 兜底版 | 方案 B 中庸版 |
|---|---|---|---|
| 网页贪吃蛇（原痛点） | 不 plan ✗ | plan ✓ | plan ✓ |
| 修 typo / 加 log / 改名 / 跑测试 | 不 plan ✓ | 不 plan ✓ | 不 plan ✓ |
| 翻译 / 总结 / 邮件 / 注释整理 | 不 plan ✓ | 不 plan ✓ | 不 plan ✓ |
| 做一个登录页 | 可能不 plan ✗ | 可能 plan | plan ✓ |
| 实现深色模式 | 可能不 plan ✗ | 可能不 plan | plan ✓ |
| 写 README / 调研 | 不 plan ✓ | 不 plan ✓ | 可能 plan ⚠️（过度风险） |

#### 触发时机建议

- 如果用户多次反馈"做小游戏/小工具/UI 没问就直接干"，先考虑方案 C
- 如果方案 C 上后仍发现「登录页」「深色模式」「dashboard widget」覆盖率不够，再考虑方案 B
- 用户主动按 Plan 按钮 / `/plan enter` 始终是兜底通道，本期 plan/task 解耦后这条路工作良好——所以这个 followup 优先级不高

#### 影响面

无用户阻塞——用户可以主动按 Plan 按钮 / `/plan enter` 进入 plan mode，模型自行判断的"建议"路径只是 nice-to-have 增强。属于"模型主动性 vs 用户专注度"的取舍，决策权在用户偏好。




### F-028 跨平台兼容性更广扫描：`target_os = "linux"` → `cfg(unix)`、macOS-only 分支审视

- **来源**：2026-05-01 跨平台兼容性修复 PR（`claude/cross-platform-compatibility-check-qBENn`）
- **现象**：本期 PR 只修了"主路径在非 macOS / 非 Tauri 直接走不通"的两个硬伤（Skills 目录选择 + Ollama 失败 UX）。仓库里仍有大量 `#[cfg(target_os = "linux")]` / `#[cfg(target_os = "macos")]` 散落在业务代码而非 `crates/ha-core/src/platform/` 门面下，违反 AGENTS.md「优先用 `#[cfg(unix)]` / `#[cfg(windows)]`，少写 `target_os = "linux"`；新增跨平台原语统一放 `crates/ha-core/src/platform/`」规则。重灾区：
  - [`crates/ha-core/src/service_install.rs`](../../crates/ha-core/src/service_install.rs)：~30 处 `target_os = "macos"` / `"linux"` 分支硬编码（launchd plist / systemd unit 业务逻辑应进 `platform::service` 子模块）
  - [`crates/ha-core/src/weather.rs`](../../crates/ha-core/src/weather.rs)：geo lookup 走 macOS-only `CoreLocation` 分支（line 640+），Linux 走 IP geolocation fallback——能走通但耦合在业务文件里
  - [`crates/ha-core/src/provider/proxy.rs`](../../crates/ha-core/src/provider/proxy.rs) / [`docker/proxy.rs`](../../crates/ha-core/src/docker/proxy.rs)：`scutil --proxy` macOS 系统代理探测，非 macOS 兜底 `None`——Linux 用户的 GNOME / KDE / 环境变量代理设置完全不被识别
  - [`crates/ha-core/src/file_extract.rs:164`](../../crates/ha-core/src/file_extract.rs)：office 文件提取按 `target_os` 选 binary（`textutil` macOS / `libreoffice` Linux / Windows 未实现）——Windows 路径直接报"unsupported on this OS"
  - [`crates/ha-core/src/permissions.rs:56,318`](../../crates/ha-core/src/permissions.rs)：macOS-only TCC 权限申请，非 macOS 全 stub——能走通但应迁到 `platform::permissions` 门面
- **为什么留**：本期 PR 范围聚焦"完全走不通"的两个硬伤（用户已经在反馈），上面这些都不是 blocker：要么 Linux/Windows 已有降级路径（weather / proxy / permissions），要么是 Windows 一开始就不支持的功能（file_extract Windows）。逐个迁到 `platform/` 是大块结构性重构，不能跟修主路径混在一个 PR
- **改的话要做什么**：
  1. 先做 audit：grep 全 `target_os =` 出现位置，按"业务文件 vs platform 门面"分两堆
  2. 对每个业务文件里的分支，判断是该 (a) 整个函数迁到 `platform/{macos,linux,windows}.rs` 然后 `crate::platform::xxx()` 调用，还是 (b) 改成 `cfg(unix)` 让 macOS+Linux+BSD 共享路径
  3. `service_install.rs` 拆成 `platform::service::{install,uninstall,status}` + 各 OS 实现文件——是最大的一块
  4. `provider/proxy.rs` Linux 路径加 `gsettings get org.gnome.system.proxy` + `kreadconfig5` + `http_proxy` env var 三档探测，与 macOS `scutil --proxy` 同结构，落 `platform::system_proxy`
  5. `weather.rs` macOS CoreLocation 分支抽到 `platform::geolocation` 门面，业务层只调 `crate::platform::current_location()` 或 fallback IP geo
  6. `file_extract.rs` Windows 路径加 PowerShell `Word.Application` COM / 或彻底声明 unsupported 不再 panic
- **影响面**：全是"已经能跑但不够 OS-native"——非 macOS 用户拿到的是降级体验或不支持提示。无安全 / 数据正确性问题，但跨平台口碑会被这些细节拖累
- **触发时机建议**：可以拆成 4-5 个独立小 PR 渐进推进（`service_install` / `system_proxy` / `geolocation` / `file_extract` / `permissions` 各一个），每个独立可 review；或者下次有 Windows / Linux 用户报某个具体子系统不能用时，趁势把对应那块迁到 platform 门面


### F-027 `notify()` 每次调用都跑 IPC 查权限，可缓存 first-grant

- **来源**：2026-05-01 桌面后台通知 PR `/simplify` review（efficiency agent #5）
- **现象**：[`src/lib/notifications.ts::notify`](../../src/lib/notifications.ts) 每次调用都 `await isPermissionGranted()`，权限被授予后这个值实际上不会再变（用户去系统设置里手动撤销才会变），但函数体没有缓存，每次后台通知都付一次 Tauri IPC 往返
- **为什么留**：`notify()` 是 pre-existing 代码（不是本期 PR 引入），按 AGENTS.md「don't add features beyond what the task requires」原则不顺手改。当期场景下后台通知频率低（人类操作节奏），重复 IPC 不构成可见性能问题
- **改的话要做什么**：
  1. 在 `cachedConfig` 旁加 `let cachedPermissionGranted = false`（默认 false，避免误以为已授权）
  2. `notify()` 命中 `cachedPermissionGranted=true` 时跳过 `isPermissionGranted` IPC 直接 `sendNotification`
  3. 首次 `requestPermission()` 成功后 set `cachedPermissionGranted=true`
  4. 监听 `visibilitychange` 或在窗口聚焦事件回调里 invalidate cache（用户可能在系统设置里撤销了授权——这种边缘场景下重新查一遍 IPC 即可），或者干脆不 invalidate（撤销后 `sendNotification` 会静默失败，用户重启 app 自然纠正）
- **影响面**：纯性能微优化。当前没有用户可见 bug；IPC 单次开销很小，只是高频通知场景下浪费
- **触发时机建议**：下次动 `notify()` 路径或做通知功能扩展时（比如 click handler、声音、icon override）顺手做；不值得单独开 PR


### F-026 IM 端 `permission:mode_changed` 事件订阅方未补齐

- **来源**：2026-04-30 IM channel 权限模式对齐 v2 PR
- **现象**：`/permission yolo` 在 IM 渠道执行后，[`channel/worker/slash.rs::SetToolPermission`](../../crates/ha-core/src/channel/worker/slash.rs) 已经调用 `SessionDB::update_session_permission_mode` 写入 SQLite 并 emit `permission:mode_changed`，但桌面端 `PermissionModeSwitcher` 没订阅该事件——用户在 IM 改完后回到桌面端打开同一会话，dropdown 显示的还是改前的值，必须切走再切回来才会重新读 DB
- **为什么留**：本期 PR 主题是命令对齐 + IM 写入闭环 + Smart 判官说明可见，事件订阅是桌面端的纯 UX 改进，没有数据正确性问题（DB 已经是新值，下一条工具调用按新模式判定）。补全订阅链路涉及前端 hooks（useChatStream / useSession），属于独立 frontend 改动
- **改的话要做什么**：
  1. 前端某个 hook（`useChatStream` 或新建 `usePermissionModeSync`）订阅 EventBus 事件 `permission:mode_changed`，过滤当前 sessionId 命中后更新 stream 本地的 `permissionMode` 状态
  2. Tauri 侧 `EventBus` 转发到前端事件需在 `src-tauri/src/lib.rs::run` 的 EventBus 订阅器里把 `permission:mode_changed` 加进 forward list（参考 `slash:plan_changed` 的模式）；HTTP 模式 axum WS 端走 `crates/ha-server/` 同样加白名单
  3. 顺便看下 IM 端 `slash:plan_changed` / `slash:effort_changed` / `slash:model_switched` 是否都已正确转发——`/permission` 这条事件名加进来时一起统一
- **影响面**：纯 UX。改前用户切走再切回触发 `get_session` 重读即可纠正，没有持续不一致或安全问题
- **触发时机建议**：下次做 IM ↔ 桌面端会话状态同步类工作时（cron 改动、project / agent 切换事件等）顺手补；或者独立 "EventBus → 前端事件转发完整性" 小 PR


### F-025 IM 工具审批仅渲染 SmartJudge，其它 AskReason kind 待补

- **来源**：2026-04-30 IM channel 权限模式对齐 v2 PR
- **现象**：[`channel/worker/approval.rs::format_approval_text` / `format_text_approval`](../../crates/ha-core/src/channel/worker/approval.rs) 当前只渲染 `ApprovalReasonKind::SmartJudge` 一种 reason 的 detail；`EditCommand`（命中 edit-commands 模式）/ `DangerousCommand`（命中危险命令）/ `ProtectedPath`（命中保护路径）/ `EditTool` / `AgentCustomList` / `PlanModeAsk` 全部不在 IM 端渲染说明文字，IM 用户只看到 command preview + 三个按钮/数字回复，无法判断弹审批的具体原因
- **为什么留**：本期对齐范围是「命令切换 + Smart 判官说明」。其它 AskReason 的 detail 文案需要逐一过 i18n、决定哪些适合在 IM 暴露（保护路径 detail 可能泄露用户隐私目录如 `~/.ssh/id_rsa` 给群成员看）、Smart fallback=Ask 时 `reason=None` 也希望加一句"Smart 未决, fallback=ask"提示——铺得有点宽
- **改的话要做什么**：
  1. 在 `smart_judge_line` 旁新增 `reason_line(reason: Option<&ApprovalReasonPayload>) -> String`，按 kind 分支返回对应 prefix（`💭 Smart Judge:` / `🛡 Protected Path:` / `⚠ Dangerous Command:` / `✏ Edit Command:` / 等）
  2. 决定 `ProtectedPath` / `DangerousCommand` 的 detail 是直接 expose 还是脱敏（保护路径有可能是 `/home/user/.ssh/id_rsa` 之类敏感）
  3. Smart 模式 + `SmartFallback::Ask` + judge 失败导致没有 rationale 的场景，渲染 "Smart Judge timed out — falling back to Ask" 之类提示
  4. 同步加单元测试覆盖每种 kind 的渲染分支
- **影响面**：UX 完整性。当前不影响功能正确性，IM 用户审批决策时少了上下文
- **触发时机建议**：下一次动 IM 审批 UX（按钮文案 / 自动审批 / AllowAlways 多作用域）时一并做；或者独立 "IM AskReason renderer" 小 PR


### F-024 IM 端 AllowAlways 按钮在四作用域 (Project / Session / AgentHome / Global) 上的语义补齐

- **来源**：2026-04-30 IM channel 权限模式对齐 v2 PR（plan 阶段调研发现的存量 gap）
- **现象**：[`permission/allowlist.rs`](../../crates/ha-core/src/permission/allowlist.rs) 已经定义了 `AllowScope ∈ Project | Session | AgentHome | Global` enum 骨架，但 IM 端 [`channel/worker/approval.rs::build_approval_buttons`](../../crates/ha-core/src/channel/worker/approval.rs) 的 `🔓 Always Allow` 按钮和文本 fallback 的 `2 - Always allow` 都仍然走兼容旧 [`is_command_allowed`](../../crates/ha-core/src/security/dangerous.rs) 路径，没有 scope 选择 UI；桌面端 `ApprovalDialog` 同样固定 "Allow Once"。AGENTS.md「AllowAlways 多作用域 v1 部分实现」段落明确这是 v1 待补
- **为什么留**：本期主题是命令对齐与 Smart 判官说明，AllowAlways 多作用域是另一条独立的产品决策路径——IM 端尤其复杂（按钮按一下就要选作用域，需要二级菜单或 callback flow）
- **改的话要做什么**：
  1. 桌面端先把 `AllowScope` 落到 ApprovalDialog UI（4 个 scope chip 或 dropdown），确定 UX 后再迁 IM
  2. IM 端可以走「按 Always Allow → 弹第二组按钮选 scope」的二步骤 callback flow（callback_data 加一阶状态机，因为 Telegram callback 是 stateless）
  3. 4 个 scope 的文件 IO（per-project / per-session / per-agent / global allowlist 文件）需要分别落到 [`~/.hope-agent/permission/allowlist/`](../../crates/ha-core/src/permission/) 子目录
- **影响面**：用户当前在 IM 端按 "Always Allow" 实际行为是 Global allowlist（兼容路径），跟桌面端一致。等于多作用域功能整体未上线，没有"已有功能但 IM 不全"的不一致问题
- **触发时机建议**：等桌面端 AllowAlways 多作用域产品决策落地时一并做


### F-023 SkillsPanel 三个 Switch handler 失败处理风格不一致

- **来源**：2026-04-29 Skill 自动审核 UI 信号 PR `/simplify` review（reuse agent）
- **现象**：[`src/components/settings/skills-panel/index.tsx`](../../src/components/settings/skills-panel/index.tsx) 在同一个面板里有三个 Switch handler，失败处理风格不齐：
  - `handleSetAutoReviewPromotion`（line ~228）和 `handleSetAutoReviewEnabled`（line ~253）：本期新加，**乐观更新 + 失败 rollback** —— `setAutoReview…(v) → call → catch → setAutoReview…(previous)`，后端写失败 UI 自动滚回旧值
  - `handleSetSkillEnvCheck`（line ~217）：早期代码，**乐观更新 + 不 rollback** —— `setSkillEnvCheck(v) → await call(...)`，后端如果抛异常 UI 已经切到新值但 config 没存，状态永久不一致直到下次 reload
- **为什么留**：本期 PR 主题是"auto-review UI 信号 + 自动激活开关"，`handleSetSkillEnvCheck` 是 pre-existing 不相关代码。AGENTS.md 风格"a bug fix doesn't need surrounding cleanup"——本 PR 给新代码加 rollback 是新代码本应做对的事，去补一个 pre-existing handler 算超范围
- **改的话要做什么**：
  1. 把 `handleSetSkillEnvCheck` 改成同样的 try/catch + rollback：
     ```ts
     async function handleSetSkillEnvCheck(v: boolean) {
       const previous = skillEnvCheck
       setSkillEnvCheck(v)
       try {
         await getTransport().call("set_skill_env_check", { enabled: v })
       } catch (e) {
         logger.error("settings", "SkillsPanel::setSkillEnvCheck", "Failed to update", e)
         setSkillEnvCheck(previous)
       }
     }
     ```
  2. 顺手扫一遍 [`src/components/settings/`](../../src/components/settings/) 下其它 panel 的 Switch handler，是否也有同样的"乐观更新无 rollback"模式（特别是 ToolSettingsPanel / ChatSettingsPanel / PlanSettingsPanel 这类直接 await call 的）；如果范围大可以抽 `useOptimisticToggle(value, setter, callback)` 共用 hook
- **影响面**：纯一致性 + 边缘 bug。后端 `set_skill_env_check` 走 `mutate_config` 几乎不会失败（除非配置文件磁盘异常），实际触发概率低；触发时用户看到 Switch 显示开但实际行为没切换，要刷新或重新点才会自愈
- **触发时机建议**：下一次动 SkillsPanel（加新 Switch / Setting）时顺手收掉；或独立"settings panel optimistic-toggle 一致化"小 PR 把全 settings 目录扫一遍

---

### F-021 `acp/agent.rs` 每 RPC 新建 tokio runtime + Codex token 每 retry 重复 load

- **来源**：2026-04-27 chat-engine subagent 收敛 PR `/simplify` review（efficiency agent）
- **现象**：
  - [`crates/ha-core/src/acp/agent.rs::build_agent`](../../crates/ha-core/src/acp/agent.rs) 在每次 RPC 请求里 `tokio::runtime::Builder::new_current_thread().enable_all().build()?` 新建一个 runtime 只为 `block_on` 一次 `try_new_from_provider`。`build_agent` 与 `run_agent_chat` 各自 build 自己的 runtime——同一个 "new session → prompt" 序列会发两次 runtime 分配 / 销毁
  - `run_agent_chat` 的 `model_chain × retry` 循环里每次 attempt 都会跑一次 `try_new_from_provider`，对 Codex 走的是 `oauth::load_fresh_codex_token()`——内部**没有**进程级缓存，每次都是 disk read（可能再叠 token endpoint roundtrip）。N model × M retry 次失败可重试场景下放大很明显
- **为什么留**：
  - 收敛 runtime 需要把 `Runtime` 实例挂到 `AcpAgent` 上，构造 / shutdown 顺序要重排——ACP 入口是 sync stdio 主循环，没有外层 runtime 可借（`Handle::try_current()` / `block_in_place` 都不可行），改动有顺序敏感性
  - `oauth::load_fresh_codex_token` 加 in-memory cache 涉及锁 / TTL 选择 / refresh-when-near-expiry 边界，得跟 `ensure_fresh_codex_token` 已有的"prime 后写盘"路径协调，不是单点替换
  - ACP 是低频调用路径（每个 RPC ~人手速度），实际产线压力低，不阻塞 chat-engine 收敛主目标
- **改的话要做什么**：
  1. 在 [`AcpAgent::new`](../../crates/ha-core/src/acp/agent.rs) 持有 `Arc<tokio::runtime::Runtime>`，`build_agent` / `run_agent_chat` 改成 `&self.rt` 复用；构造在 `new` 里失败也 `Result<Self>` 回报
  2. 在 [`crates/ha-core/src/oauth.rs`](../../crates/ha-core/src/oauth.rs) 加进程级 `OnceCell<Mutex<Option<TokenCache>>>` 缓存，`load_fresh_codex_token` 优先读缓存；写盘路径（`refresh_access_token` / `save_token`）同步 invalidate 缓存。或者在 `AcpAgent::run_agent_chat` 顶部一次性 `load_fresh_codex_token` 然后逐 retry 直接构造 `LlmProvider::Codex { ... }`，绕过外层 `try_new_from_provider`
- **影响面**：纯效率，无可见 bug。runtime 浪费每次 ~ms 级（本地默认 num_workers=1），token reload 在网络抖动期会放大失败 latency。Codex 用户在 ACP 模式失败重试时最容易感知
- **触发时机建议**：下一次动 ACP（新协议字段、prompt routing 改动）或 Codex OAuth 流程（refresh logic / 新 grant）时顺手收掉；或独立 "ACP runtime / OAuth caching" 重构 PR

---

### F-020 `ChatEngineParams` 7 个新 boolean / option 字段应收敛成 `ExecutionMode` 枚举

- **来源**：2026-04-27 chat-engine subagent 收敛 PR `/simplify` review（quality agent）
- **现象**：[`crates/ha-core/src/chat_engine/types.rs::ChatEngineParams`](../../crates/ha-core/src/chat_engine/types.rs) 在本期为统一 subagent / parent injection 路径加了 7 个新字段：`denied_tools`、`subagent_depth`、`steer_run_id`、`follow_global_reasoning_effort`、`post_turn_effects`、`abort_on_cancel`、`persist_final_error_event`。实际只有两个语义轴：
  - **Foreground**（4 处：[`src-tauri/src/commands/chat.rs`](../../src-tauri/src/commands/chat.rs)、[`crates/ha-server/src/routes/chat.rs`](../../crates/ha-server/src/routes/chat.rs)、[`crates/ha-core/src/channel/worker/dispatcher.rs`](../../crates/ha-core/src/channel/worker/dispatcher.rs)、[`crates/ha-core/src/cron/executor.rs`](../../crates/ha-core/src/cron/executor.rs)）— 全部 `follow_global_reasoning_effort: true, post_turn_effects: true, abort_on_cancel: false, persist_final_error_event: true`
  - **Background**（2 处：[`crates/ha-core/src/subagent/spawn.rs`](../../crates/ha-core/src/subagent/spawn.rs)、[`crates/ha-core/src/subagent/injection.rs`](../../crates/ha-core/src/subagent/injection.rs)）— 全部反向：`false, false, true, false`
- **为什么留**：4 个 boolean 完美关联，确实可以收敛成 `enum ExecutionMode { Foreground, Background { abort_on_cancel: bool } }` + `denied_tools / subagent_depth / steer_run_id` 也只在 Background 非默认。但改动要触达 ha-core / ha-server / src-tauri 三个 crate 的 6 个调用点，本期 `/simplify` 已经在做 subagent 收敛 + ChatSource 谓词抽取 + image_gen helper 抽取等多项整理，再叠加 enum 重构会让 PR 进一步膨胀，超出 simplify 单次合理范围
- **改的话要做什么**：
  1. 在 [`crates/ha-core/src/chat_engine/types.rs`](../../crates/ha-core/src/chat_engine/types.rs) 新增 `pub enum ExecutionMode { Foreground, Background { abort_on_cancel: bool, denied_tools: Vec<String>, subagent_depth: u32, steer_run_id: Option<String> } }`
  2. 把 `follow_global_reasoning_effort` / `post_turn_effects` / `persist_final_error_event` 三个固定相关字段从 `ChatEngineParams` 删除，由 `mode.is_foreground()` 推导
  3. 给 `ChatEngineParams` 加 `pub fn foreground(...)` / `pub fn background(...)` 构造函数，6 个调用点全部改成 builder 风格
  4. 同步更新 [`docs/architecture/chat-engine.md`](../../docs/architecture/chat-engine.md) 如果有的话
- **影响面**：纯整洁度。当前所有调用点都正确，但 `false / true` 字面量噪声大，新增第 7 个 caller（例如未来 ACP 走 chat_engine）时容易漏字段（编译错保住但语义对不齐 review 才能抓）
- **触发时机建议**：下次有第 7 个 chat_engine 调用点要新加（例如 ACP 改走 `run_chat_engine` 复用主路径），或下次需要再加第 8 个 mode-related boolean / option 字段时一次性收掉；不要单独立 PR

---

### F-019 SSE 解析器在 4 处 LLM / IM stream 重复实现

- **来源**：2026-04-26 F-004 重新核查时分流出来
- **现象**：4 处 `bytes_stream` SSE 解析各自手写 buffer + `find("\n\n")` / `find('\n')` + `event:` / `data:` 拆解，结构相似但实现细节有出入：
  - [`crates/ha-core/src/agent/providers/anthropic_adapter.rs`](../../crates/ha-core/src/agent/providers/anthropic_adapter.rs)（`\n\n` event boundary，多 `data:` 行 join）
  - [`crates/ha-core/src/agent/providers/openai_chat_adapter.rs`](../../crates/ha-core/src/agent/providers/openai_chat_adapter.rs)
  - [`crates/ha-core/src/agent/providers/openai_responses_adapter.rs`](../../crates/ha-core/src/agent/providers/openai_responses_adapter.rs)
  - [`crates/ha-core/src/channel/signal/client.rs`](../../crates/ha-core/src/channel/signal/client.rs)（line-based + 空行 boundary，结构等价）
- **为什么留**：抽公共 SSE parser 需要先统一 event 数据结构（`SseEvent { event, data, id, retry }`）+ 决定多 `data:` 行 join、`\r\n`、`:` 注释行、`retry` 字段处理。3 个 LLM provider adapter 是聊天热点路径，重构必须有逐 frame 等价测试兜底，独立 PR 范围。
- **改的话要做什么**：
  1. 在 [`crates/ha-core/src/util.rs`](../../crates/ha-core/src/util.rs)（或新建 `util/sse.rs`）加 `pub fn sse_event_stream<S>(stream: S, max_buffer_bytes: usize) -> impl Stream<Item = Result<SseEvent>>`
  2. 用 `tokio_util::io::StreamReader` + `AsyncBufReadExt::lines()` 逐行收 `event:` / `data:` / `id:` / `retry:` / 空行 boundary，多 `data:` 按 SSE 规范 `\n` join
  3. 替换 4 处 inline 解析；保留各 caller 自己的 event-name 分支与 payload 反序列化
- **影响面**：纯整洁度，当前无可见 bug；但 SSE spec 边界条件 4 处实现各有遗漏，新增 SSE 接入点时容易再走偏
- **触发时机建议**：下一次新增 SSE 接入点（OpenAI 新流式模式 / 新 IM channel SSE 入站）时顺手抽；或独立 "SSE parser 统一" 重构 PR

---

### F-013 EventBus 事件名常量散落，应有 events 常量模块

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **现象**：EventBus 事件名当前混合两种风格：
  - **Rust 常量**：[`crates/ha-core/src/chat_engine/stream_broadcast.rs::EVENT_CHAT_STREAM_DELTA`](../../crates/ha-core/src/chat_engine/stream_broadcast.rs)、[`crates/ha-core/src/docker/mod.rs::EVENT_SEARXNG_DEPLOY_PROGRESS`](../../crates/ha-core/src/docker/mod.rs)、[`crates/ha-core/src/local_llm/mod.rs`](../../crates/ha-core/src/local_llm/mod.rs) 的 `EVENT_LOCAL_LLM_*`
  - **前端独立常量 / 字面量**：前端仍各自维护同值（例如本地小模型进度事件、`useChatStreamReattach.ts` 的 `EVENT_CHAT_STREAM_DELTA`），缺少跨 Rust/TS 的单一来源
- **为什么留**：跨前端（TS）/ 后端（Rust）同步常量需要 codegen 或 wire-format 文档约定，引入新约束。本期把刚碰到的 searxng 升成常量已经是最低成本的"按碰到逐步收"。
- **改的话要做什么**：候选方案：
  - **A**：每个子系统在自己 mod 顶部定义 `pub const EVENT_*: &str = "..."`（已经 chat / searxng 在做）；前端继续维护独立常量但加注释指向 Rust 同名定义。Rust 端集中调用，前端只 listen 时用一次，漂移风险低
  - **B**：用 `build.rs` 生成 TS const 文件，从 Rust 单一来源。需要新增 build pipeline 复杂度
- **影响面**：纯整洁度。事件名漂移会被 watchdog 测试快速发现（事件不到达 → UI 不更新），是 "fail loud" 类型的 bug。
- **触发时机建议**：等再积累 2-3 个新事件名（看 local_llm 之外）时一次性把所有 `local_llm:*` / 其它字面量升成常量；不必单独立 PR。

---

### F-016 LocalModelJobsDB 与 AsyncJobsDB 大量重复

- **来源**：2026-04-26 Task Center / Local Model Jobs `/simplify` review
- **现象**：[`crates/ha-core/src/local_model_jobs.rs`](../../crates/ha-core/src/local_model_jobs.rs) 重新实现了与 [`crates/ha-core/src/async_jobs/`](../../crates/ha-core/src/async_jobs/) 几乎一一对应的基础设施：
  - 状态枚举 `LocalModelJobStatus { Running, Cancelling, Completed, Failed, Interrupted, Cancelled }` ↔ `AsyncJobStatus`（多一个 `TimedOut`）
  - `is_terminal()` + `TERMINAL_SQL_LIST`
  - `LocalModelJobsDB::open` 的 PRAGMA WAL/NORMAL + CREATE TABLE 模板
  - `mark_interrupted_running` / `mark_cancelling` 的 lifecycle 逻辑
  - `static CANCELS: Mutex<HashMap<String, CancellationToken>>` 取消注册表（`async_jobs::cancel` 已有）
  - `now_secs()` 时间戳助手（`async_jobs::spawn` 已有）
  - `row_to_job` 行解析模板
- **为什么留**：`local_model_jobs.rs` 顶部注释明确说"故意与 async_jobs 分离：那些是工具调用结果，本模块是用户可见的安装任务"——确实需要不同的 payload schema 与 UI 语义，但 *基础设施层*（DB scaffold / cancel registry / lifecycle）是可以共享的。统一需要把 async_jobs 的相关基元抽到一个 `crate::async_jobs::scaffolding` 层，工程量大且涉及现有 async_jobs 的回归风险，本期 PR 已经过大不再叠加。
- **改的话要做什么**：
  1. 在 `crates/ha-core/src/async_jobs/` 抽出 `lifecycle.rs`：`CommonJobStatus` enum + `is_terminal` + `TERMINAL_SQL_LIST` + `mark_interrupted_running` 通用模板
  2. 把 `cancel.rs::CANCELS` 和 helper（`register_job_token` / `cancel_job` / `remove_job`）改成 generic by job-id 字符串，让 `local_model_jobs` 直接复用而不是另开一份
  3. `local_model_jobs::LocalModelJobsDB::open` 把 PRAGMA + CREATE 步骤拆出 `init_journal_pragmas(&conn)` helper
  4. `now_secs()` 移到 `crate::time` 或 `crate::util`
- **影响面**：纯整洁度，没有 bug。但现状下任何对 async_jobs 基础设施的改动（如新增 status / 改 cancel 协议 / 调 PRAGMA）都需要在 local_model_jobs 平行复制一份，长期维护成本。
- **触发时机建议**：下一次有人需要再加第三类用户可见后台任务（例如"批量索引项目文件"或"长时间 web search"）时一并抽 scaffolding；或独立 "async_jobs scaffolding 抽出" 重构 PR。

---

### F-018 SQLite 写在 tokio worker 上同步串行成为高频进度场景的瓶颈

- **来源**：2026-04-26 Task Center / Local Model Jobs `/simplify` review
- **现象**：[`crates/ha-core/src/local_model_jobs.rs::LocalModelJobsDB`](../../crates/ha-core/src/local_model_jobs.rs) 的 `conn: Mutex<Connection>`（`std::sync::Mutex`）在 pull 进度风暴中由 reqwest stream 回调以同步方式持锁；同一把锁也是 `list_jobs` / `get_job` / `cancel_job` 的读路径锁。多 job 并行时 tokio worker 互相阻塞；本期已加 250 ms / phase-change 节流（`ProgressThrottle`）把帧率压到 ~4 Hz 缓解，但 SQLite IO 仍在 worker 线程上。
- **为什么留**：节流后的 4 Hz 写入 + 100 行上限的 GC 已经远低于会成为瓶颈的水平，本期实测无可见卡顿；改成 `spawn_blocking` 或单线程 writer task 是结构性优化但需要重新设计 read/write 分离与 cancel 路径，工程量与风险与本期收益不匹配。
- **改的话要做什么**：候选两条：
  - **A**：所有 SQL 调用包 `spawn_blocking`，retain `Arc<Mutex<Connection>>` 但避免占 worker
  - **B**：dedicated writer task：`mpsc::UnboundedSender<WriterCmd>` + 独立 thread 持 connection，`update_progress` / `append_log` / `mark_*` 改成发消息；读路径用独立 read-only connection（SQLite WAL 允许并发读）
  - 推荐 B，与 dashboard / session DB 的潜在统一更大
- **影响面**：极端场景（多个并发 GB 级 pull + 大量并发 list_jobs 查询）下可能出现 worker stall；现实中很难触发。
- **触发时机建议**：如果未来要支持"批量预拉模型"（多 job 并行）或观察到 tokio worker stall，再处理。

---

### F-022 Diff 面板缺 Shiki 行级语法高亮 + 大 diff 虚拟列表

- **来源**：2026-04-29 文件操作摘要 + 右侧 Diff 面板 feature 实现
- **现象**：[`src/components/chat/diff-panel/UnifiedDiffView.tsx`](../../src/components/chat/diff-panel/UnifiedDiffView.tsx) / [`SplitDiffView.tsx`](../../src/components/chat/diff-panel/SplitDiffView.tsx) 当前直接渲染纯文本 diff 行，没有按 `metadata.language` 做语法高亮。Plan 文件 [`diff-plan-foamy-wave.md`](../../../.claude/plans/diff-plan-foamy-wave.md) 第 16 条原本要求新建 `diff-panel/diffShiki.ts` 复用项目 Shiki 实例。同时大文件 diff（>1000 行）当前一次性渲染所有行，没有虚拟列表
- **为什么留**：Shiki 行级 token 高亮要先解决"对每行单独高亮 vs 对整文件高亮再按行切"的取舍 + Shiki 实例在 streamdown 内的访问方式不直接 + 大文件性能优化（虚拟列表 / `requestIdleCallback` 分批）。本期 MVP 优先把 diff 面板可用打通，纯文本 diff 已能满足"看改了什么"的最低需求
- **改的话要做什么**：
  1. 找到 streamdown / [`src/lib/`](../../src/lib/) 下现有 Shiki 实例并暴露稳定 API（可能需新建 `src/lib/shiki/highlight-line.ts`）
  2. 新建 `src/components/chat/diff-panel/diffShiki.ts`：`highlightLine(text, language) -> React.ReactNode`，fallback 返回 `<span>{text}</span>`
  3. UnifiedDiffView / SplitDiffView 在渲染单行时先经 `highlightLine`
  4. 对 >1000 行 diff 加虚拟列表（建议 `@tanstack/react-virtual`，当前未在依赖里）
- **影响面**：纯视觉。当前 diff 在多语言大文件场景可读性较差（语法着色缺失）；超大 diff（数千行）一次性渲染可能引起卡顿，但项目内 256KB 截断阈值已经把单边压住，实际触发概率低
- **触发时机建议**：下次有用户报告 diff 阅读体验差 / 大 diff 卡顿时；或独立"diff 面板增强 + 虚拟列表"PR

---

### F-023 `file_read` metadata 已 emit 但前端 grouping UI 未实现

- **来源**：2026-04-29 文件操作摘要 + 右侧 Diff 面板 feature 实现
- **现象**：[`crates/ha-core/src/tools/read.rs`](../../crates/ha-core/src/tools/read.rs) 已 emit `kind: "file_read", path, lines"` 元数据，前端 [`src/types/chat.ts`](../../src/types/chat.ts) 也定义了 `FileReadMetadata` 类型；但 [`src/components/chat/message/MessageContent.tsx`](../../src/components/chat/message/MessageContent.tsx) 中没有 grouping 逻辑——连续相邻 read 仍每条一行展示，没有折叠成截图样式的"已浏览 N 个文件"。Plan 文件第 12 条原本要求新建 `ToolCallList.tsx` 中间组件做 grouping，落地时被砍
- **为什么留**：grouping 涉及 contentBlocks 循环里识别相邻同类 ToolCall + 维持 callId 顺序 + 展开列表交互。本期主要功能（diff 面板）已实现完整；read 聚合是体验优化，单条 read 显示也能接受
- **改的话要做什么**：
  1. 新建 `src/components/chat/message/FileReadAggregate.tsx`：接收 `paths: string[]`，渲染折叠/展开 UI（展开列出每个 path）
  2. 在 [`MessageContent.tsx`](../../src/components/chat/message/MessageContent.tsx) 渲染 contentBlocks 时，把相邻 `metadata.kind === "file_read"` 的 ToolCall 折成 `<FileReadAggregate>`
  3. 注意保留 callId 顺序，避免 streaming 中途插入新 read 时渲染抖动
  4. i18n key `tool.fileRead.aggregateLabel`（本期 plan 第 19 条已计划但未落实，12 语言需补）
- **影响面**：纯视觉。当前连续 read 多个文件占多行显示，chat 偏冗长；用户已习惯当前样式，无 bug
- **触发时机建议**：下次动 MessageContent / ToolCallBlock 渲染层时一并实现；或独立"chat 工具调用紧凑视图"PR

---

### F-024 DiffPanel 与 CanvasPanel 互斥未实现

- **来源**：2026-04-29 文件操作摘要 + 右侧 Diff 面板 feature 实现
- **现象**：[`src/components/chat/ChatScreen.tsx`](../../src/components/chat/ChatScreen.tsx) 在 useEffect 中实现了"DiffPanel 打开自动关 PlanPanel"互斥，但**没有**与 CanvasPanel 的互斥。三面板同时打开会导致主 chat 区被挤压到不可用宽度。Plan 文件第 18 条原本要求三面板互斥
- **为什么留**：CanvasPanel 自管 visibility（不像 PlanPanel 暴露 `setShowPanel` API），改动需要先重构 CanvasPanel 的 state ownership。本期落地时为避免冲击 CanvasPanel 现有契约，权衡后只做了 DiffPanel ↔ PlanPanel
- **改的话要做什么**：
  1. CanvasPanel 把 `showPanel` state 提到 ChatScreen 上层管理（或暴露 imperative `onClose` ref）
  2. ChatScreen 的 useEffect 加入第三向互斥：openDiff → close PlanPanel + close CanvasPanel；openCanvas → close DiffPanel + close PlanPanel；openPlan → close DiffPanel + close CanvasPanel
  3. 或更优：抽 `useExclusivePanel(panelId)` hook 统一管理三面板 mutex（PlanPanel / CanvasPanel / DiffPanel 注册到同 registry）
- **影响面**：可见 bug。极端场景（用户 Plan + Canvas + Diff 全打开）chat 主区被挤压；日常使用罕见
- **触发时机建议**：下次动 CanvasPanel state ownership / 新增第四个 side panel 时一并收掉；或独立"side panel mutex 统一"重构 PR

---

### F-025 `commands/permission.rs` 与 `routes/permission.rs` 镜像重复

- **来源**：2026-04-30 权限系统 v2 Phase 3 `/simplify` review（reuse + quality 双 agent）
- **现象**：[`src-tauri/src/commands/permission.rs`](../../src-tauri/src/commands/permission.rs) 和 [`crates/ha-server/src/routes/permission.rs`](../../crates/ha-server/src/routes/permission.rs) 是 byte-for-byte 镜像，仅 wrapper 类型不同（`Result<T, CmdError>` vs `Result<Json<T>, AppError>`）。两份独立维护：
  - `PatternListPayload` / `SetPatternsBody` / `GlobalYoloStatus` 三个 struct 定义重复
  - 12 个 endpoint 一一对应，业务逻辑同（都调 `protected_paths::current_patterns()` 等）
  - `mutate_config(("permission.smart", "settings-ui"|"http"), …)` 仅 source 标签不同
- **为什么留**：Phase 3 范围是"前端 UI + 后端 file IO + commands/routes"打通，时间紧；这是项目级"Tauri ↔ HTTP 双暴露"的通用模式，单独动一个域意义有限——MCP / config / agents 等其它子系统也都有同样的镜像样板，应当作为"统一模式"独立 PR 推
- **改的话要做什么**：
  1. 在 `crates/ha-core/src/permission/` 新建 `api.rs` 模块，把所有 payload 结构体（`PatternListPayload` / `SetPatternsBody` / `GlobalYoloStatus`）+ thin worker functions（`get_protected_paths_inner() -> PatternListPayload` 等）集中
  2. `commands/permission.rs` 退化成 `#[tauri::command]` 包装：`fn get_protected_paths() -> Result<PatternListPayload, CmdError> { permission::api::get_protected_paths().map_err(Into::into) }`
  3. `routes/permission.rs` 同样退化成 `Json(...)` 包装
  4. `mutate_config` 的 source 标签作为参数传入 worker function：`api::set_smart_mode_config(cfg, "settings-ui")` vs `api::set_smart_mode_config(cfg, "http")`
  5. 顺手考虑把这个模式抽成跨域的 `crate::transport_shim::tauri!()` / `axum_route!()` 宏（或代码生成），扩到 mcp / agents / config 等 4-5 个有同样镜像的子系统
- **影响面**：纯重构，无功能变化。当前 ~200 行重复代码 / 12 个新增 endpoint 的双倍维护成本；未来加新 endpoint 必须记得改两处
- **触发时机建议**：等下一个新增"Tauri ↔ HTTP 双暴露"endpoint 的 PR 时累积痛感再做；或立项"transport-shim 通用化"独立重构 PR，一次清掉 mcp / agents / config / permission 4 个域的镜像

---

### F-026 ApprovalTab `APPROVAL_OPTIN_GROUPS` 17 工具 9 分组硬编码于 TS

- **来源**：2026-04-30 权限系统 v2 Phase 3 `/simplify` review（reuse + quality）
- **现象**：[`src/components/settings/agent-panel/tabs/ApprovalTab.tsx:21-67`](../../src/components/settings/agent-panel/tabs/ApprovalTab.tsx#L21-L67) 把 17 个可勾选审批的工具按 9 个分组（shell / browser / settings / outbound / paid / spawn / network / crossSession / settingsRead）硬编码在 TS 常量里。后端工具注册表（[`tools/definitions/`](../../crates/ha-core/src/tools/definitions/)）虽然已有 `ToolTier` 元数据，但**没有"是否出现在用户审批勾选清单 + 归属哪个分组"的字段**——所以 UI 这份清单只能写在 TS 端。
  漂移风险：今天 Rust 加新工具 `send_email` 进 Tier 2，UI 不会自动显示在审批清单里，必须有人记得去改 ApprovalTab。TS 编译器无法捕获这个不一致
- **为什么留**：Phase 3 simplify 不重构 schema。要做需要：
  1. `ToolDefinition` 加 `approval_opt_in: bool` + `approval_group: Option<&'static str>` 两字段
  2. 53 个工具定义文件每个填这俩字段（含归类决策）
  3. `list_builtin_tools` payload 透传字段
  4. ApprovalTab 改数据驱动（动态分组渲染 + 与现有 i18n key 对齐）
  跨 53 个文件的注释决策不属于"清理 review"范围
- **改的话要做什么**：
  1. 在 `crates/ha-core/src/tools/definitions/types.rs::ToolDefinition` 加 metadata：
     ```rust
     /// 是否出现在 Agent「自定义工具审批」勾选清单里。Tier 2/3 中的部分工具开启。
     pub approval_opt_in: bool,
     /// 审批清单 UI 的分组标签 (i18n key 后缀)。
     pub approval_group: Option<&'static str>,
     ```
  2. 在每个工具的 `register_*_tool()` 函数里设置这两个字段（按当前 ApprovalTab.tsx 的 17/9 分组对照填）
  3. `list_builtin_tools` 添加这两个字段到 payload；`commands/chat.rs::list_builtin_tools` + 对应 HTTP 路由同步
  4. ApprovalTab 改成 `useMemo(() => groupBy(builtinTools.filter(t => t.approval_opt_in), t => t.approval_group))`，删除 `APPROVAL_OPTIN_GROUPS` 常量
  5. 更新 [`docs/architecture/tool-system.md`](../../docs/architecture/tool-system.md) 描述新元数据字段
- **影响面**：纯一致性，无可见 bug。当前 17 个工具固定，添加新 Tier 2/3 工具时如果忘改 TS，用户在 Agent 设置里看不到该工具但功能层面仍然工作（不会崩溃，只是无法 opt-in 审批）
- **触发时机建议**：下次新增可审批工具（Tier 2/3）时如果意识到要同时改两处，就顺手把这套 metadata schema 搭起来；或独立"tool definition metadata 扩展"重构 PR

---

### F-027 9 个语言 `settings.approvalPanel` block 是英文 verbatim fallback

- **来源**：2026-04-30 权限系统 v2 Phase 3 `/simplify` review（reuse agent）
- **现象**：Phase 3.4 新增的 `settings.approvalPanel.*` 文案块（~50 keys）只有 `zh.json` / `en.json` / `zh-TW.json`（部分）有原生翻译；剩下 9 个语言（`ar` / `es` / `ja` / `ko` / `ms` / `pt` / `ru` / `tr` / `vi`）通过 `node -e` 脚本批量 deep-clone 英文 block 写入 —— 这些 locale 的"权限"设置面板会渲染英文标签 / 描述 / 提示。同样的情况也部分发生在 `settings.agentApproval.*`（Phase 3.3）和 `approval.reasons.*`（Phase 3.5），但 zh / en / zh-TW / ja / ko 都已精修
- **为什么留**：英文 fallback 不会让 UI 崩溃 / 不会丢功能；Anthropic 内部不是翻译团队 —— 用机器翻译质量参差不如等母语审稿。提交时 `pnpm i18n:check ✓` 因为 key 数量已对齐，仅文案语言不对
- **改的话要做什么**：
  1. 收集需要翻译的 key 集合：从 `en.json` 提 `settings.approvalPanel.*`、`settings.agentApproval.*`（zh-TW 已部分精修，但 ja / ko 也只有部分）、`approval.reasons.*` 在非 zh / en / zh-TW 的 locale 里全部
  2. 翻译团队 / 母语志愿者按 locale 校对（约 ~70 keys × 9 语言 = 630 条）
  3. 提交时把 zh-TW / ja / ko 的部分英文 fallback 一起替换掉
  4. 顺带清查仓库里其它"批量 deep-clone 英文"的 i18n debt：grep `settings.*` 中相同字符串在多个非 en locale 里完全一致的 key
- **影响面**：UX bug for 9 个语言用户。Settings 中相关 panel 看英文不会崩溃，但显著降低非英语 / 非中文用户的体验
- **触发时机建议**：等收到非英 / 非中文用户反馈，或翻译团队 / 志愿者主动认领；不阻塞功能 PR

### F-033 `recapCard` / `openDashboardTab` / `skillFork` 在 ChatScreen 是空 case

- **来源**：2026-05-01 slash command audit `/simplify` review（quality agent）
- **现象**：[`src/components/chat/ChatScreen.tsx`](../../src/components/chat/ChatScreen.tsx) `handleCommandAction` 把这 3 个 `CommandAction` variant 当 no-op 处理，仅靠 switch 之前 push 的 event 气泡（`result.content`）告诉用户后台在跑。后端 `recap_progress` EventBus 流目前只被 Dashboard Recap tab 订阅，对话内没有渲染 RecapCard；`openDashboardTab` 没有 App 级 navigate 回调，不会跳页；`skillFork` 走 EventBus 注入回 user message，已生效，只是没有运行中状态卡片
- **为什么留**：补这三块需要新组件（RecapCard 流式）+ App 级 prop drilling，不在当期 audit PR 范围；后端事件已经 stable，前端补做不会破坏接口
- **改的话要做什么**：
  1. `recapCard`：在 chat 渲染流抽出一个 `RecapCard` 组件，订阅 `recap_progress` 过滤 `action.reportId`，复用 Dashboard `RecapTab` 的渲染层
  2. `openDashboardTab`：把 `setView("dashboard", { tab })` 挂到 `App.tsx`，`ChatScreen` props 加 `onOpenDashboardTab(tab: string)` 触发
  3. `skillFork`：可选——加个轻量 "skill running" 卡片，订阅 EventBus skill_run_progress；当前 result.content 文本提示已经够用
- **影响面**：3 个 slash command 在 GUI 体验降级（功能正常，反馈不及时），不影响 IM 渠道
- **触发时机建议**：下一次动 `ChatScreen` 或 Recap UI 时顺手收掉

### F-034 Skill 目录扫描没有 `SKILL_CACHE_VERSION`-keyed 缓存

- **来源**：2026-05-01 slash command audit `/simplify` review（efficiency agent）
- **现象**：[`crates/ha-core/src/skills/discovery.rs::load_all_skills_with_budget`](../../crates/ha-core/src/skills/discovery.rs) 每次调用都重新走文件系统扫描 bundled + `~/.agents/skills` + `extra_skills_dirs` + managed + project 五类目录，无任何缓存。 hot 调用方包括：
  - [`slash_commands::im_menu_entries`](../../crates/ha-core/src/slash_commands/mod.rs)（Telegram + Discord 同步、`/help`、`list_slash_commands` 都消费）
  - `system_prompt` 渲染（每次 LLM 请求构造 prompt 时都跑一次）
  - `skill_search` 工具
  - `handle_help`（独立调 `get_invocable_skills`，效率 agent 也提到）
  
  IM menu 自动 re-sync 场景特别痛：debounce 触发后，N 个 running account 串行 sync_commands_for_account 各调一次 `im_menu_entries → list_slash_commands → get_invocable_skills`，等于 N 次完整文件系统扫描背靠背
- **为什么留**：本次 audit 的目标是把"IM 菜单不刷"的功能性 bug 收掉，缓存层属于独立性能优化；现成有 `skills::types::SKILL_CACHE_VERSION: AtomicU64` 全局计数器（`bump_skill_version` 已经在所有 mutate 路径埋好），缓存基础设施齐备，缺的只是消费方
- **改的话要做什么**：
  1. 在 `skills/discovery.rs` 加 `static SKILL_CACHE: OnceLock<RwLock<Option<(u64, Arc<Vec<SkillEntry>>)>>>`，`load_all_skills_with_budget` 入口先 read：`(version, entries)` 的 `version == SKILL_CACHE_VERSION.load(Relaxed)` 直接返回 Arc clone，否则正常扫描后 write 缓存
  2. 缓存 key 还要包含 `extra_skills_dirs` 和 `disabled_skills`（不同输入可能命中相同 version）—— 用 `(SKILL_CACHE_VERSION, hash(extra_skills_dirs + disabled_skills))` 复合 key，或者干脆每次写 `bump_skill_version()` 即可（这两个字段写完都会 bump，存量代码已经如此）
  3. `get_invocable_skills` 跟着改成消费 `Arc<Vec<SkillEntry>>` slice，避免 clone 整个 Vec
  4. 为 `tools/settings.rs::update_app_config` 的 skill 类 category 补 `bump_skill_version()`（当前 audit 的 listener 是通过监听 `config:changed { category: "skills" }` 兜的，但缓存 invalidation 也需要这个 bump，否则缓存看不到变更）
- **影响面**：性能 only，没有正确性问题。N 个 IM account 重 sync 时减少 N-1 次文件系统扫描；每次 LLM 请求 system_prompt 构造也省一次扫描。粗估 50ms × N 节省
- **触发时机建议**：下一次做性能优化 PR 时；或者用户报"启动慢""LLM 第一次响应慢"时


### F-035 `isAbsolutePath` helper 散落 3 处，应抽 `src/lib/pathUtil.ts`

- **来源**：2026-05-01 桌面 markdown 文件路径链接 PR `/simplify` review（reuse agent）
- **现象**：判断"是不是绝对路径"的 windows 盘符正则 `^[A-Za-z]:[\\/]` 在仓内重复了 3 处：
  - [`src/components/common/MarkdownRenderer.tsx::isLocalPath`](../../src/components/common/MarkdownRenderer.tsx)（本期新增，含 `/` / `~/` / `file://` / windows）
  - [`src/components/chat/file-mention/types.ts::joinAbs`](../../src/components/chat/file-mention/types.ts)（`/` + windows）
  - [`src/lib/transport-tauri.ts::resolveAssetUrl`](../../src/lib/transport-tauri.ts)（windows 盘符）
  - 三处都是 **inline regex**，定义略有差异（有的不含 `~/`，有的不含 `file://`），重复风险随 1→3→N 累积
- **为什么留**：本期 PR 主题是 markdown 路径链接化，新增第 3 处时已经把语义最完整的版本（含 `/` / `~/` / `file://` / windows）落到 MarkdownRenderer 里。统一抽 helper 涉及 3 处行为对齐 + 单元测试，超出当期范围；MarkdownRenderer 那一处当下唯一被依赖的特性是"识别 LLM 输出的本地路径链接"，不需要 file-mention / transport-tauri 的额外 case
- **改的话要做什么**：
  1. 新建 `src/lib/pathUtil.ts`，导出 `isAbsolutePath(href: string): boolean`（最完整语义：`/` / `~/` / `file://` / windows 盘符）+ 可选的 `stripFileProtocol(href)` / `stripLineAnchor(href)`
  2. MarkdownRenderer / file-mention/types / transport-tauri 三处 inline regex 统一替换为 `isAbsolutePath`
  3. 加一组单元测试覆盖 unix / `~/` / `file://localhost/...` / `C:\\` / `D:/` / `relative/path` / 空串 / undefined
- **影响面**：纯重构债务；当前三处行为差异极小且各自场景不会撞上对方的 case，没有用户可见 bug
- **触发时机建议**：下一次有人改 file-mention 解析或 transport-tauri 的资产 URL 处理时顺手；或者撞到第 4 处需要写绝对路径判断时再统一

### F-052 `useActiveModel()` 共享 hook，避免多个面板各自 `getTransport().call("get_active_model")`

- **来源**：2026-05-04 `embedding swap panic + UX` PR `/simplify` review (quality agent)
- **现象**：`GlobalModelPanel.tsx:99` 与 `LocalLlmAssistantCard.tsx:155` 各自 mount 时拉一次 `get_active_model`；后续若再加面板要看 active model（model picker / 状态栏 / 等），每加一处就再多一份 round-trip + 局部 state
- **当前选择**：不动。本期只是 LocalLlmAssistantCard 加上"已激活"判定，重复点从 1 处升到 2 处，仍小到不值得抽
- **改的话要做什么**：抽 `src/hooks/useActiveModel.ts`：内部用 `getTransport().call("get_active_model")` + 监听 `config:changed` 事件 / EventBus refresh；返回 `{ activeModel, refresh }`。所有面板替换为 hook
- **影响面**：当前**无 bug**——只是多两次 IPC 调用（mount 一次），冷路径
- **触发时机建议**：第 3 个面板想看 active model 时

### F-053 `set_memory_embedding_default` 等 3 个函数都加了 `parent_job_id`，可包成 `SetMemoryEmbeddingDefaultOpts`

- **来源**：2026-05-04 `embedding swap panic + UX` PR `/simplify` review (quality agent)
- **现象**：[`crates/ha-core/src/local_embedding.rs::pull_and_activate_cancellable`](../../crates/ha-core/src/local_embedding.rs) / `save_and_set_default_for_model` / [`crates/ha-core/src/memory/helpers.rs::set_memory_embedding_default`](../../crates/ha-core/src/memory/helpers.rs) 一条链都加了 optional `parent_job_id`，目前只有 1 个真实 caller (`run_embedding_job`) 传 `Some`，其余 4 个 callsite 全部传 `None`
- **当前选择**：不动。透传 1 个 optional 参数没有真实痛点；引入 options struct 反而要给只关心 source 的 caller 多写一行 builder/struct 字面量
- **改的话要做什么**：定义 `pub struct SetMemoryEmbeddingDefaultOpts { source: &'static str, parent_job_id: Option<String> }` 或者 builder pattern；3 个函数签名收成 1 个 opts 参数
- **影响面**：纯结构层，无 bug
- **触发时机建议**：再加第二个 metadata 字段时（如 `dispatcher_kind` / `cause` 之类），一并改

### F-054 后端 `OllamaEmbeddingModel` / active model 加 `is_active` 字段，避免前端复刻 ID 约定

- **来源**：2026-05-04 `embedding swap panic + UX` PR `/simplify` review (reuse + quality agent 同时标记)
- **现象**：`LocalEmbeddingAssistantCard.tsx:340-343` 用 `currentModel?.source === "ollama" && apiModel === recommended.id` 判断模型是否激活；`LocalLlmAssistantCard.tsx:328` 用 `activeModel?.modelId === recommended.id`。两处都复刻了「ollama 模型 ID 命名规则 / source 字段格式」等后端内部约定，未来若改 ID 体系会静默 break
- **当前选择**：不动。本期注释已经把 contract 说清楚，撞名场景副作用低（按钮变绿但不影响功能）
- **改的话要做什么**：
  - 后端 `local_embedding.rs::list_models_with_status()` 返回值里给 `OllamaEmbeddingModel` 加 `is_active: bool`，由后端用 `memory_embedding.selection.modelConfigId == ollama_embedding_config_id(model.id)` 计算
  - `local_llm/management.rs::UsageIndex::usage_for(...)` 已有 `active_model` flag（[`management.rs:604-606`](../../crates/ha-core/src/local_llm/management.rs)），让 `local_llm_list_models` 直接用 `LocalOllamaModel.usage.activeModel` 而不是 `get_active_model + modelId 比对`
  - 前端两处替换为 `is_active` flag，删除字段对比逻辑
- **影响面**：当前撞名场景按钮误显示「已激活」，但点击只是 noop（idempotent），不会破坏数据
- **触发时机建议**：下次后端改 ollama provider 注册逻辑（如改 ID 命名）时一并改

### F-056 接力到 reembed 任务的 dialog state 重置在两个组件里复制（7 行 setter）

- **来源**：2026-05-04 `embedding swap panic + UX` PR `/simplify` round 2 (reuse + quality agent 同时标记)
- **现象**：[`LocalModelsPanel.tsx::handleSnapshot` 接力分支](../../src/components/settings/local-llm/LocalModelsPanel.tsx) 与 [`LocalEmbeddingAssistantCard.tsx::handleSnapshot` 接力分支](../../src/components/settings/memory-panel/LocalEmbeddingAssistantCard.tsx) 都做相同 7 行：`setDialogFrame(localModelJobToProgressFrame(...))` + `setDialogLogs([])` + `setDialogDone` + `setDialogError` + `setDialogTitle(t("settings.embedding.reembedJob.title"))` + `setDialogSubtitle(job.modelId)` + `void hydrateJobLogs(job.jobId)`
- **当前选择**：不抽。两个 callsite，参数贴身（7 个 setter + t + phaseLabel + hydrateJobLogs callback），抽出来比直接复制更复杂；本期已通过 `isJobSuccessorOf` 工具函数共享判定逻辑
- **改的话要做什么**：抽 `applySuccessorJobToDialog(job, opts)` 到 `src/components/settings/local-llm/job-dialog-helpers.ts`；两处各调一次。或者改成 `useJobSuccessorTransition` hook 返回 `applySuccessor` callback
- **影响面**：纯结构层，无 bug；下次任意一边改 dialog state shape 或 reembed title key 时另一边漂移风险
- **触发时机建议**：第 3 个组件想接力时；或者 dialog state shape 因别的需求要改时一并整顿

### F-055 `transferSummary` `formatJobTransferLine` helper 当前签名要求 caller 传入 `t` 函数

- **来源**：2026-05-04 `embedding swap panic + UX` PR `/simplify` review 自评
- **现象**：[`src/lib/format-job-transfer.ts`](../../src/lib/format-job-transfer.ts) 的 helper 签名 `formatJobTransferLine({ t, ... })` 让 lib 函数依赖 i18next `TFunction`。lib 文件夹通常不耦合 i18next，但本 helper 的存在意义就是封装 i18n key 拼装
- **当前选择**：保留。否则让 caller 传 4 个 string template 进来比传 t 还乱
- **改的话要做什么**：考虑把 helper 移到 `src/components/settings/` 命名空间下（不是 lib），或者改成"返回 i18n 调用 plan"让 caller 自己 t；都不优雅
- **影响面**：纯位置 / 命名规范问题，无 bug
- **触发时机建议**：如果未来要在非 React / 非 i18next 上下文（CLI / Node script）复用 transfer formatter 再说

---

## Closed

> 已修复条目移到此处，附 commit hash + 关闭日期。保留以便后续 grep。

### F-044 / F-045 / F-046 / F-047 react-virtuoso 迁移期登记的 4 条 followup 全部失效

- **关闭于**：2026-05-03，react-virtuoso 整体卸载并改用 `messages.map(slice)` + 浏览器 `overflow-anchor` + `content-visibility: auto` + windowed view (DOM 上限 200) + React.memo，commit 待提交
- **失效原因**：上一期 react-virtuoso 迁移落地后撞了横向滚动条 / 用户消息裁剪 / 会话切换闪烁 / 分页 firstItemIndex 误判等多处虚拟化边界 bug。本期决定整体撤掉虚拟化，改用浏览器原生滚动 + 数据层窗口卸载，[`MessageList.tsx`](../../src/components/chat/MessageList.tsx) / [`QuickChatMessages.tsx`](../../src/components/chat/QuickChatMessages.tsx) / [`TeamMessageFeed.tsx`](../../src/components/team/TeamMessageFeed.tsx) 三个组件全部重写：
  - **F-044（抽 `useChatVirtuoso` hook）**：源头消失——三组件不再使用 react-virtuoso，所谓「重复的 6 块 hook 模板」不存在；剩余的 windowed view + auto-follow 逻辑差异较大（QuickChat 简化版 / Team 极简），不需要抽 hook
  - **F-045（Footer re-mount 风险）**：源头消失——MessageList 已不用 virtuoso 的 `components.Footer` API，askUser / planCard / empty 直接拼在 `messages.map()` 后面，AskUserQuestionBlock 的 React 子树由 React 自身管理，不会被 virtuoso 内部 element-type 变化撕掉
  - **F-046（`createVirtuosoMock()` test util）**：源头消失——MessageList.test.tsx / QuickChatMessages.test.tsx 删除了 ~80 行 `vi.mock("react-virtuoso", ...)` 工厂，改用 `Element.prototype.scrollIntoView` / `scrollTo` spy 直接断言；不再有需要抽 helper 的重复
  - **F-047（MessageBubble React.memo）**：本期一并验证完成——[`MessageBubble.tsx`](../../src/components/chat/message/MessageBubble.tsx) 已 `React.memo` 包装；MessageList 重写后所有 callback (hover / copy / contextMenu / open* / switch*) 都 useCallback'd 稳定；`itemContent` 14 项依赖的 useCallback 已不存在（直接内联 `messages.map()` 渲染），鼠标 hover 不再触发可见区全量 re-render
- **本期附带解决**：横向滚动条根因（virtuoso flow item 撑宽 list）、用户消息裁剪（flex item `min-width: auto`）、会话切换闪烁（virtuoso firstItemIndex 派生 state 1-frame 延迟）三个虚拟化边界 bug 同时消失

### F-036 PlanPanel + PlanDetachedWindow 内联 comment 逻辑重复 ~120 行 × 2，`usePlanComment.ts` hook 是死代码

- **关闭于**：2026-05-02，commit a64bcebb + 643cd2d8
- **如何关闭**：删掉 `usePlanComment.ts` 死 hook（commit a64bcebb），抽 `planCommentMessage.ts` 纯函数 helper `buildPlanCommentMessage(selectedText, comment, t) -> {prompt, displayText, payload}`，PlanPanel + PlanDetachedWindow 的 `handleCommentSubmit` 都调这个 helper 构造请求；`onRequestChanges` 签名后来在 simplify pass (643cd2d8) 进一步收紧成单 `BuiltPlanComment` 对象。两个面板的 highlight / popover state 各自保留（页面级 state，没必要共享）
- **效果**：核心 prompt + display + payload 构造逻辑统一一处，将来改 prompt 模板或 display 文案只动 `planCommentMessage.ts` 一个文件

### F-037 Plan state transition 副作用代码在 5 处复制，缺共享 helper

- **来源**：2026-05-02 Plan Mode 重构 `/codex:review` (codex 主报告 + 第一轮 reuse agent #1) + 修复 `/plan exit` 路径漏 cancel subagent bug 时再次撞到
- **关闭**：2026-05-02
- **修复方式**：抽出 [`crates/ha-core/src/plan/transition.rs`](../../crates/ha-core/src/plan/transition.rs) — `pub async fn transition_state(session_id, target, TransitionOpts) -> anyhow::Result<TransitionOutcome>`，按 review 建议封装 5 件副作用（cancel subagent on Off / cleanup checkpoint on Off+Completed / set_plan_state / create checkpoint on Executing / DB persist / emit `plan_mode_changed`）。`TransitionOpts` 暴露 `reason: &'static str`（每个 caller 必填，落 `plan_mode_changed.reason` 用于前端 / 遥测归因）+ `cancel_subagent_on_off: bool` + `manage_checkpoint: bool`（默认都 true，特殊路径可 opt-out）。6 个 caller 全部迁移：
  - [`tools/enter_plan_mode.rs`](../../crates/ha-core/src/tools/enter_plan_mode.rs) — `reason="tool_enter_plan_mode"`
  - [`tools/submit_plan.rs`](../../crates/ha-core/src/tools/submit_plan.rs) — `reason="plan_submitted"`，额外 emit `plan_submitted` 携带 plan title + content（保留各路径二次 emit）
  - [`slash_commands/handlers/plan.rs`](../../crates/ha-core/src/slash_commands/handlers/plan.rs) — `slash_enter` / `slash_exit` / `slash_approve`，顺手把 `db: &SessionDB` 形参移除（dispatcher 同步收紧）
  - [`src-tauri/src/commands/plan.rs::set_plan_mode`](../../src-tauri/src/commands/plan.rs) — `reason="tauri_set_mode"`，原 60+ 行收敛到 8 行；`tauri::State<AppState>` 形参直接删除（不再需要）
  - [`crates/ha-server/src/routes/plan.rs::set_plan_mode`](../../crates/ha-server/src/routes/plan.rs) — `reason="http_set_mode"`，与 Tauri 完全对齐
  - [`tools/task.rs::maybe_complete_plan`](../../crates/ha-core/src/tools/task.rs) — `reason="all_tasks_completed"`，原 30 行手写收敛
- **白拣的 bug**：原 Tauri / HTTP / slash 三条路径**全部漏发** `plan_mode_changed`（只有 3 个 model-tool 路径发过），意味着用户从 GUI / `/plan` 切换 plan 状态时，detached PlanWindow 等次要订阅者收不到通知；helper 化后这条路径自动统一发，前端 `usePlanMode.ts` listener 已是 idempotent functional update（`prev === next ? prev : next`）所以零回归。原 `/plan exit` 漏 cancel subagent 的 bug 也由 helper 兜底，下次再加新副作用（例如 "Executing 进入时落 timestamp"）只改 transition.rs 一处即可
- **测试覆盖**：[`crates/ha-core/src/plan/tests.rs::test_transition_state_in_memory_contract`](../../crates/ha-core/src/plan/tests.rs) — 跑 `Off→Planning` Applied + `Planning→Completed` Rejected（必须经 Review）+ Rejected 后内存状态保持 Planning 不被污染。Globals 未注册时 DB / event-bus 副作用走 `Option::None` 跳过，单测无 fixture 即可
- **验证**：`cargo fmt --all --check` / `cargo clippy -p ha-core -p ha-server --all-targets --locked -- -D warnings` / `cargo test -p ha-core --locked`（812 passed）/ `cargo check -p hope-agent` 全绿
- **影响面**：纯重构 + 顺手补齐 GUI / HTTP / slash 三条路径的 `plan_mode_changed` emit。无用户可见行为变化，但 detached PlanWindow / 多窗口场景的状态同步路径更稳，后续维护成本下降一档

---

### F-004 NDJSON 流式解析无统一 helper

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / rejected on second look，不实现
- **修复方式**：实现前先核对 5 个候选站点，发现登记前提错误：实际 NDJSON 只有 [`crates/ha-core/src/local_llm/mod.rs::pull_model`](../../crates/ha-core/src/local_llm/mod.rs) 一处，其它 4 处均不属于：
  - [`docker/deploy.rs`](../../crates/ha-core/src/docker/deploy.rs) — `docker pull` 纯文本 stdout 转 log
  - [`mcp/client.rs`](../../crates/ha-core/src/mcp/client.rs) — MCP server stderr 纯文本 tail（rate-limit + truncate）
  - [`channel/process_manager.rs`](../../crates/ha-core/src/channel/process_manager.rs) — 子进程 stdout/stderr 纯文本转 `mpsc::Receiver<String>`
  - [`agent/providers/anthropic_adapter.rs`](../../crates/ha-core/src/agent/providers/anthropic_adapter.rs) — **SSE**（`event:` / `data:` / `\n\n` boundary），不是 NDJSON

  抽 helper 只有一个消费者 (`pull_model`)，且本期已经自带 `MAX_PULL_LINE_BYTES` + 严格末帧 + 单测覆盖，新增一层间接零收益（典型的 premature abstraction）。SSE 那侧的真重复另开 [F-019](#f-019-sse-解析器在-4-处-llm--im-stream-重复实现) 登记。

---

### F-017 旧 `local_llm:install_progress` / `local_llm:pull_progress` / `local_embedding:pull_progress` 事件路径已无前端监听

- **来源**：2026-04-26 Task Center / Local Model Jobs `/simplify` review
- **关闭**：2026-04-26
- **修复方式**：grep 全仓库确认前端 100% 已切到 `local_model_job:*` 事件总线、外部消费面零调用后，删除旧路径所有源码与文档。具体：
  - **ha-core**：删除 `EVENT_LOCAL_LLM_INSTALL_PROGRESS` / `EVENT_LOCAL_LLM_PULL_PROGRESS` / `EVENT_LOCAL_EMBEDDING_PULL_PROGRESS` 三个常量；删除非 cancellable 包装函数 `local_llm::install_ollama_via_script` / `local_llm::pull_and_activate` / `local_embedding::pull_and_activate`；windows stub 合并到 `install_ollama_via_script_cancellable`；`*_cancellable` 版本仅保留给 `local_model_jobs` 调用
  - **ha-server**：[`routes/local_llm.rs`](../../crates/ha-server/src/routes/local_llm.rs) / [`routes/local_embedding.rs`](../../crates/ha-server/src/routes/local_embedding.rs) 删 `install` / `pull` handler 与对应 imports，砍到只剩硬件 / Ollama 状态 / 模型目录探测；[`router 注册`](../../crates/ha-server/src/lib.rs) 去掉 `/local-llm/install` / `/local-llm/pull` / `/local-embedding/pull` 三条路由
  - **src-tauri**：[`commands/local_llm.rs`](../../src-tauri/src/commands/local_llm.rs) / [`commands/local_embedding.rs`](../../src-tauri/src/commands/local_embedding.rs) 删 `local_llm_install_ollama` / `local_llm_pull_and_activate` / `local_embedding_pull_and_activate` 三条命令；[`invoke_handler!`](../../src-tauri/src/lib.rs) 注册表去三行
  - **前端**：[`src/lib/transport-http.ts`](../../src/lib/transport-http.ts) COMMAND_MAP 删除三条路由映射
  - **文档**：[`docs/architecture/api-reference.md`](../../docs/architecture/api-reference.md) 事件表用 `local_model_job:*` 替换，新增「Local model background jobs」表与 8 条 routes / 同时把 Local LLM assistant 表收敛到 5 条探测命令；[`docs/architecture/transport-modes.md`](../../docs/architecture/transport-modes.md) 事件矩阵同步替换；[`AGENTS.md`](../../AGENTS.md) 「本地 LLM 助手」段把"进度走 EventBus"改成"后台任务统一接口"；docker.rs / docker command shim 内残留的旧函数引用注释一并清理
  - 验证：`cargo check -p ha-core -p ha-server` / `cargo check -p hope-agent` / `pnpm typecheck` 全绿
- **影响面**：dead-code 移除，无 runtime 行为变更。

---

### F-003 "local Ollama" 判定逻辑分散在 4 处

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / 本次 F-002 + F-003 修复
- **修复方式**：新增 [`crates/ha-core/src/provider/local.rs`](../../crates/ha-core/src/provider/local.rs) 维护 known local backends catalog（Ollama / LiteLLM / vLLM / LM Studio / SGLang）与 host+port 匹配逻辑，`local_llm::OLLAMA_BASE_URL` 改为复用 `LOCAL_OLLAMA_BASE_URL`。新增 Tauri `local_llm_known_backends` 与 HTTP `GET /api/local-llm/known-backends`，前端 [`provider-detection.ts`](../../src/components/settings/local-llm/provider-detection.ts) 改为消费后端 catalog，不再维护 `LOCAL_OLLAMA_HOST_RE`。ProviderSettings / TemplateGrid 均使用同一 catalog 判定是否展示本地小模型助手。

---

### F-002 Provider 写入路径未单一化（add_provider 缺 upsert 语义）

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / 本次 F-002 + F-003 修复
- **修复方式**：新增 [`crates/ha-core/src/provider/crud.rs`](../../crates/ha-core/src/provider/crud.rs) 作为 Provider 写入单一入口，集中 add / update / delete / reorder / set active / add-and-activate / batch add / Codex ensure / local backend upsert。GUI、HTTP、onboarding、Codex auth/restore/logout、OpenClaw import、CLI onboarding、IM slash active-model 切换和 local LLM 注册路径均改走 ha-core helper。普通 `add_provider` 继续追加并生成新 id；本地模型助手单独通过 known backend upsert 去重。

---

### F-015 `src/components/settings/` 大批原生 `<button>` / `<input>` / `<textarea>` 未走 shadcn

- **来源**：2026-04-26 焦点轮廓视觉降噪手动审查
- **关闭**：2026-04-26 / branch `worktree-settings-shadcn-migration`
- **修复方式**：把 `src/components/settings/` 下 50+ 个文件里所有原生 `<button>`（116 处）/ `<input>`（5 处非 file/checkbox 类型）/ `<textarea>`（2 处）/ `<input type="range">`（2 处）/ `<input type="checkbox">`（4 处）系统替换成 shadcn 等价组件：`<Button>` 各 variant（ghost / outline / secondary / icon）、`<Input>`、`<Textarea>`、`<Slider>`、`<Switch>`。图标按钮统一走 `size="icon"`；原本"看起来像按钮但其实是文字链"的内联点击点（如 SearxngDocker 端口、profile 自定义重置）改 `variant="ghost"` + 行内 className override 保留 baseline + underline。涉及 40+ 文件，主要包括 ProviderEditPage / ProviderSettings / ContextCompactPanel / GlobalModelPanel / AgentEditView / PersonalityTab / CapabilitiesTab / ModelTab / AgentListView / AvatarCropDialog / DangerousModeSection / ProfileForm / MemoryListView / MemoryFormView / EmbeddingModelSection / SkillListView / SkillDetailView / ModelEditor / AddAccountDialog / AllowlistTagInput 等。新代码若再写原生 `<button>` / `<input>` / `<textarea>` 由 code review 打回。`src/index.css` 全局 focus-visible fallback 仍然保留作为防御层。

---

### F-009 EventBus 桥接闭包样板在多处重复

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 F-009 修复
- **修复方式**：在 [`crates/ha-core/src/event_bus.rs`](../../crates/ha-core/src/event_bus.rs) 新增 `EventBusProgressExt::emit_progress`，把 typed progress callback 统一桥接到 EventBus JSON payload。为保留 `EventBus` 的 object-safe 形状（仓库大量使用 `Arc<dyn EventBus>`），实现采用 `Arc<B: EventBus + ?Sized>` 扩展 trait，而不是直接在 `EventBus` 本体加泛型方法。local LLM install / pull、SearXNG deploy、local embedding pull 的 ha-server route 与 Tauri command 均已切换到 helper，事件名与 payload contract 不变。

---

### F-012 `useChatStream.ts::onEvent` 嵌套 try/catch + 多重 if 应 flatten

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 F-012 修复
- **修复方式**：[`useChatStream.ts`](../../src/components/chat/hooks/useChatStream.ts) 的 `onEvent` 现在拆为 `handleSessionCreated`、`shouldDropStreamEvent`、`dispatchStreamEvent`、`appendRawStreamText` 等本地 helper；保留 `__pending__` cache rename、loading session 更新、`_oc_seq` cursor 去重、ended stream 丢弃与 raw fallback 行为。

---

### F-005 前端字节/容量格式化在 6+ 处重复

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / 本次 F-005 修复
- **修复方式**：新增 [`src/lib/format.ts`](../../src/lib/format.ts) 统一 `formatBytes`、`formatBytesFromMb`、`formatGbFromMb`；替换 dashboard、BrowserPanel、FileCard、log panel、SkillDetailView、本地 LLM / embedding 卡片、project 上传与 logo 限制错误文案里的重复容量格式化，并新增 [`src/lib/format.test.ts`](../../src/lib/format.test.ts) 覆盖单位转换。

---

### F-014 `docs/architecture/` 缺中心化 transport mode 文档

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 F-014 修复
- **修复方式**：新增 [`docs/architecture/transport-modes.md`](../architecture/transport-modes.md)，集中说明 Tauri / HTTP / ACP 三种入口、`getTransport()` 选择逻辑、`Transport` 方法矩阵、`chat:stream_delta` 双写与 reattach 角色、`/ws/events` EventBus 桥、主要 EventBus 事件目录，以及 `startChat` 不是通用 `streamCall` 的决策记录。同步回填 [`docs/README.md`](../README.md) 索引。

---

### F-010 HTTP `startChat` 用合成 `session_created` 事件 vs 显式 return shape 的取舍

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 F-010 修复
- **修复方式**：保留 [`src/lib/transport-http.ts::startChat`](../../src/lib/transport-http.ts) 合成 `session_created` 的现有合约，让 [`useChatStream.ts`](../../src/components/chat/hooks/useChatStream.ts) 继续用同一条 `onEvent` 路径完成 `__pending__` cache rename，避免把 HTTP 特例泄漏到 hook。经核实前端已不再消费 `/ws/chat/{session_id}`，HTTP 流式输出完整走 `/ws/events` 上的 `chat:stream_delta`；因此删除 [`crates/ha-server/src/ws/chat_stream.rs`](../../crates/ha-server/src/ws/chat_stream.rs)、`ChatStreamRegistry`、`WsSink` 和 `/ws/chat/{session_id}` 路由，ha-server 改用 `NoopSink` 依赖 Chat Engine 的 EventBus 双写路径。同步更新架构文档中旧的 `openChatStream` / `/ws/chat` 描述。

---

### F-006 Ollama pull 流提前结束时仍会激活模型

- **来源**：2026-04-26 commit `a29a4b27393eb573110e1bafe8f9c0cad11d59c9` review
- **关闭**：2026-04-26 / 本次 Ollama followups 修复
- **修复方式**：[`crates/ha-core/src/local_llm/mod.rs::pull_model`](../../crates/ha-core/src/local_llm/mod.rs) 现在会在流结束时解析残留 buffer 中无换行的最后一帧；若最终状态不是 `success`，或最后残留帧是截断/非法 JSON，则返回 `Err`，阻止后续 provider 注册与 active model 切换。新增单元测试覆盖 final success 有换行、final success 无换行、early EOF、truncated final frame。

---

### F-007 Ollama 安装成功后进度弹窗不会关闭

- **来源**：2026-04-26 commit `a29a4b27393eb573110e1bafe8f9c0cad11d59c9` review
- **关闭**：2026-04-26 / 本次 Ollama followups 修复
- **修复方式**：[`InstallProgressDialog`](../../src/components/settings/local-llm/InstallProgressDialog.tsx) 增加受控 `onOpenChange`，运行中拦截关闭，完成/错误态允许关闭；[`LocalLlmAssistantCard.tsx::installOllama`](../../src/components/settings/local-llm/LocalLlmAssistantCard.tsx) 在一键安装并启动成功后展示完成态约 800ms，然后自动关闭弹窗并刷新 Ollama 状态。

---

### F-008 HTTP 模式下手动下载 Ollama 按钮无效

- **来源**：2026-04-26 commit `a29a4b27393eb573110e1bafe8f9c0cad11d59c9` review
- **关闭**：2026-04-26 / 本次 Ollama followups 修复
- **修复方式**：[`LocalLlmAssistantCard.tsx::openDownloadPage`](../../src/components/settings/local-llm/LocalLlmAssistantCard.tsx) 现在会检查 `open_url` 返回值；当 HTTP/server 模式返回 `{ ok: false }` 时主动 fallback 到 `window.open("https://ollama.com/download")`，Tauri 原生打开失败时也继续走同一 fallback。

---

### F-011 短期 EventBus 订阅 + `try/finally off()` 模式应抽 `withEventListener` helper

- **来源**：2026-04-26 `transport-streaming-unify` `/simplify` review
- **关闭**：2026-04-26 / 本次 Ollama followups 修复
- **修复方式**：新增 [`src/lib/transport-events.ts::withEventListener`](../../src/lib/transport-events.ts)，封装"订阅事件 → 执行长任务 → finally 取消订阅"模式；本地小模型 install / pull 与 SearXNG deploy 三个调用点已切换到该 helper。

---

### F-001 Tauri 命令错误类型未统一

- **来源**：2026-04-26 本地小模型助手 `/simplify` review
- **关闭**：2026-04-26 / branch `worktree-tauri-cmd-error-unify`
- **修复方式**：新增 [`src-tauri/src/commands/error.rs`](../../src-tauri/src/commands/error.rs) 定义 `CmdError(pub String)`，挂 `impl<E: Into<anyhow::Error>> From<E>` + `impl Serialize`（输出纯字符串，IPC wire 与原 `Result<T, String>` 等价）；把 `src-tauri/src/commands/` 下 31 个文件的命令签名统一改成 `Result<T, CmdError>`，291 处 `.map_err(|e| e.to_string())?` 删成 `?`，剩余 `.map_err(|e| format!(...))` 改为 `CmdError::msg(format!(...))`，`Err("..".to_string())` / `.ok_or_else(|| "..".to_string())` 等串字面量误差类全部走 `CmdError::msg(..)`。`tauri_wrappers.rs` 不属于"命令尾巴 boilerplate"范畴，保持 `Result<T, String>` 不动。前端零变化。

---

### F-030 `ResolveContext { ... }` 14 字段构造在 execution.rs / exec.rs 重复

- **来源**：2026-04-30 Phase 4 Smart 模式 `/simplify` review（quality agent）
- **关闭**：2026-04-30 / commit `59a36ab5`
- **修复方式**：新增 [`tools::execution::resolve_tool_permission`](../../crates/ha-core/src/tools/execution.rs) `pub(super)` async helper，统一构造 `permission::engine::ResolveContext` + 跑 `resolve_async` + 保留 "Smart 才 cached_config" hot-path 优化。两处 caller（`tools/execution.rs` 主 dispatch、`tools/exec.rs` exec 命令前置审批）从 11 行字段构造塌缩到 1 行 helper 调用。新增字段时只需改 helper 一处。

---

### F-031 `permission::judge::cache_key` JSON 序列化不规范化对象键序

- **来源**：2026-04-30 Phase 4 Smart 模式 `/simplify` review（quality agent）
- **关闭**：2026-04-30 / commit `2eefc428`
- **修复方式**：[`permission::judge`](../../crates/ha-core/src/permission/judge.rs) 把 `args.to_string().hash(...)` 替换成新的 `hash_value_canonical(args, hasher)` 递归哈希器：对象内按键排序后逐对哈希，数组按位置哈希，每个 `Value` 变体加 1 字节 tag 防跨变体冲突（null vs ""）。同语义但键序不同的 args 现在产生相同 cache key，避免冗余的 ~5s 判官 LLM 调用。新增 3 条单测：键序不变性 / 嵌套对象递归 / null/string/array/object 互不冲突。

---

### F-028 `permission::judge` cache 与 `agent::active_memory` cache 模式重复

- **来源**：2026-04-30 Phase 4 Smart 模式 `/simplify` review（reuse agent）
- **关闭**：2026-04-30 / commit `67c7e1f2`
- **修复方式**：新增 [`crate::ttl_cache::TtlCache<K, V>`](../../crates/ha-core/src/ttl_cache.rs)：TTL 在 `get` 时传入（让 `cache_ttl_secs` 配置即时生效）、溢出时 LRU-by-age 单条 evict（O(n) 但 n ≤ cap）、`get` 命中过期项 lazy 移除、无后台 sweep。`permission::judge` 退化为 `OnceLock<TtlCache<u64, JudgeResponse>>` 删除自带 60 行 cache helper；`agent::active_memory` 把 `Mutex<HashMap<...>>` 字段换成 `TtlCache` 删除手写 evict-oldest 12 行。新增 5 条 ttl_cache 单测；代码净减 ~125 行重复，新增 helper 170 行（含完整 doc + 单测）。

---

### F-029 `SessionMeta.permission_mode` 仍是 `String`，应换 `SessionMode` enum

- **来源**：2026-04-30 Phase 4 Smart 模式 `/simplify` review（quality agent）
- **关闭**：2026-04-30 / commit `0dcddf5a`
- **修复方式**：[`session::types::SessionMeta`](../../crates/ha-core/src/session/types.rs) 的 `permission_mode: String` 改成 `permission_mode: SessionMode`（已带 Default impl + snake_case serde rename）。前端 `SessionMode` union / DB TEXT 列 / JSON 编码完全不变，仅 Rust 内部强类型化。`SessionMode::parse_or_default` 仅在 DB row→struct 边界用一次（[`session/db.rs::row_to_session_meta`](../../crates/ha-core/src/session/db.rs)），消费方（`agent/config.rs` system_prompt 构造、`agent/mod.rs` ToolExecContext 构造）改成 `.map(|m| m.permission_mode)` 直接拷贝 enum (Copy)；`update_session_permission_mode` 参数改成 `SessionMode`，4 处 caller 删掉 `.as_str()` 包装。awareness 测试 fixture `"default".into()` 同步改成 `SessionMode::Default`。ha-core 771 / ha-server 18 单测全绿。

---

### F-032 `SessionMeta.plan_mode` 仍是 `String`，应换 `PlanModeState` enum

- **来源**：2026-04-30 F-029 收尾 `/simplify` review（quality agent）
- **关闭**：2026-04-30 / commit 紧跟 F-028..F-031 收尾
- **修复方式**：发现 [`plan::PlanModeState`](../../crates/ha-core/src/plan/types.rs) enum 已存在并完整支持 `from_str`/`as_str`/serde rename_all snake_case + `is_valid_transition`，直接复用即可（不需要新建 PlanMode）。给 PlanModeState 加 `Copy` 派生（6 个 unit variant，1 字节）让消费方按值传递。`SessionMeta.plan_mode: String` → `PlanModeState`，DB row→struct 边界用 `from_str` 一次性转 enum；`update_session_plan_mode` 参数改成 `PlanModeState`，6 处 caller（slash_commands/handlers/plan.rs 5 处 + tools/plan_step.rs + tools/submit_plan.rs + ha-server/routes/plan.rs 2 处 + src-tauri/commands/plan.rs 3 处 + commands/chat.rs 2 处）改用 enum variant；`should_create_execution_checkpoint(persisted_plan_mode: Option<&str>)` 改成 `Option<PlanModeState>`；`restore_from_db(plan_mode_str: &str)` 改成 `state: PlanModeState`，删除内部 from_str 重复转换。`meta.plan_mode == "off"` 等 stringly compare 全部改成 enum 匹配。ha-core 771 / ha-server 18 单测全绿。

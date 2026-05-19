# Review Followups — 审查决定但本期不改的问题

> 本文档登记**已被 code review 识别、但当期 PR 决定不修**的问题。每条记录的目的是：让债务可见、可检索、可调度，避免下一次有人撞上同一个问题再重新发现。
>
> 登记规则见 [AGENTS.md](../../AGENTS.md) "Review Followups 登记"段。

## 文档使用方式

- **新增一条 Follow-up**：在最下方"Open"段追加一个 `### F-XXX` 子节，编号递增（不复用），按下方"条目模板"填写。一次提交一个原子条目；多个 review 想法分开记。
- **清理一条**：确认已修复、已失效、或决定不再追踪后，直接从 "Open" 删除；历史记录交给 Git。
- **定期清理**：避免把纯重构、微优化、已修复或不再需要处理的条目继续留在本文。
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

### F-057 IM channel 主动消息 / 媒体能力补完（跨 channel）

- **来源**：2026-05-05 IM channel 全量审计 + 2026-05-06 codex review 回归
- **现象**：本批**写过** QQ Bot c2c/group msg_type=7 两步上传 + LINE imageMessage/videoMessage/audioMessage HTTPS URL 路径；但 [`channel/worker/dispatcher.rs::to_outbound_media`](../../crates/ha-core/src/channel/worker/dispatcher.rs#L728) 优先给 `MediaData::FilePath`（hope-agent 本地缓存路径），而 QQ Bot V2 上传 / LINE message object 都只接收公网 HTTPS URL —— 两边在 plugin 内部 `match data { Url(_) => ..., _ => continue }` 把 FilePath 静默跳过。结果声明 `supports_media` 反而让 dispatcher 不再追 link fallback → 用户附件两头不到位。
  - **2026-05-06 已回退**：QQ Bot + LINE `supports_media` 重新设为空，恢复 dispatcher 的链接文本兜底。两套两步上传 helper（`post_*_files` / `send_*_media` / `dispatch_media`、LINE message-object 构造）保留备用，等本地附件中转基建就绪再开
- **剩余 channel 状态**：均降级为下载链接文本：
  - Google Chat — 已核对官方文档：`media.upload` 需要 user authentication 的 `chat.messages.create` / `chat.messages` scope；当前插件是 service account app-auth `chat.bot`，不是 Drive scope 问题，不能直接打开 `supports_media`
  - WhatsApp — bridge `media` 字段
  - LINE / QQ Bot — 待本地附件中转 + 公网 HTTPS 暴露基建（hope-agent 自托管端点 / 用户配置 `public_base_url` 转发缓存附件）
  - QQ Bot — channel/dms 端点 V2 不开放原生媒体上传，仍要走链接
- **2026-05-19 已处理**：Signal 出站附件已接上 signal-cli JSON-RPC `send.attachments`，支持本地 `FilePath`，并把 `Url` / `Bytes` 物化到 `channels/signal/outbound-temp/` 后发送
- **2026-05-19 已处理**：Slack 出站附件已接上官方 files v2 流程（`files.getUploadURLExternal` → POST `upload_url` → `files.completeUploadExternal`），需要 bot token 具备 `files:write`
- **2026-05-19 已处理**：iMessage 出站附件已接上 imsg JSON-RPC `send` 的 `file` 参数，支持本地 `FilePath`，并把 `Url` / `Bytes` 物化到 `channels/imessage/outbound-temp/` 后发送；官方当前不是单独 `send_attachment` RPC
- **为什么留**：Google Chat 还缺 user OAuth credential mode；LINE / QQ Bot 核心是缺"本地缓存附件 → 公网 HTTPS URL"的中转基建；其他 channel 按各自协议补原生发送路径即可。富媒体不阻塞首发文本
- **改的话要做什么**：参照 [`channel/worker/dispatcher.rs::partition_media_by_channel`](../../crates/ha-core/src/channel/worker/dispatcher.rs) 已有的能力声明驱动下，逐 channel 补 `*/media.rs` 模块；Google Chat 先补 user OAuth 认证模式或等官方支持 app-auth 上传，协议改造下一项是 WhatsApp
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

### F-045 接入 `auto_curator_enabled` 后台周期合并扫描

- **来源**：2026-05-15 auto-review 五道闸自查
- **现象**：[`SkillsAutoReviewConfig::auto_curator_enabled`](../../crates/ha-core/src/skills/auto_review/config.rs) 和 `auto_curator_interval_days` 字段已经定义、reset_fields 已覆盖、sanitize 已 clamp，但**没有任何背景任务消费它们**。前端 UI 也未暴露开关。当前 v1 仅支持手动按钮触发 curator scan
- **为什么留**：周期任务涉及 init_runtime 启动期接入 + cfg 变化时重启策略 + 测试覆盖；本 PR 已经 8 commit 38 文件，再加 background lifecycle 风险偏高，单独 PR 做
- **改的话要做什么**：
  - 在 `init_runtime`（或 `lib.rs::run`）启动期 spawn 一个 `tokio::time::interval` 后台 task，tick 时读 `cached_config().skills.auto_review`，按 enabled + interval 决定是否跑 `auto_review::curator::run_curator_pass()`
  - 跑出来的 `CuratorReport` emit 到 EventBus 作为 `skills:curator_proposals_ready`，UI 在 SkillEvolutionView 顶部加一个"有 N 组合并建议"提醒
  - SkillEvolutionView 加 `Auto-curator` 开关 + interval input（已 reserve i18n key）
  - cfg 变化时——简化策略：读 cfg 的循环每 tick 重读 cached_config，自然生效；不重新 spawn
- **影响面**：补完已 ship 的配置项，避免用户开了不生效的体验
- **触发时机建议**：下一个 auto-review 相关 PR 或专门做 background lifecycle 的 PR

### F-074 NSIS 重新加回 SimplifiedChinese installer 语言

- **来源**：2026-05-09 v0.1.0 release run 25567046354 Windows 失败诊断
- **现象**：[`src-tauri/tauri.conf.json`](../../src-tauri/tauri.conf.json) `bundle.windows.nsis.languages` 临时去掉了 `SimplifiedChinese`，只保留 `English`。原因是 GitHub `windows-latest` runner 上 tauri 自动下载的 NSIS 包不含 `SimplifiedChinese.nlf`，bundling 阶段报 "Can't open language file - SimplifiedChinese.nlf"
- **为什么留**：v0.1.0 release blocker；先打通中英文用户都能装的英文 installer，installer UI 语言不影响应用本身的 i18n（应用内 12 种语言完整）
- **改的话要做什么**：两个方向二选一：
  - **方案 A（workflow 层）**：在 [`.github/workflows/release.yml`](../../.github/workflows/release.yml) Windows job 加 step pre-stage 完整 NSIS 到 `%LOCALAPPDATA%\tauri\NSIS`（覆盖 tauri 自动下载的精简版），需注意 NSCurl plugin 是 tauri 自定义补丁，要保留
  - **方案 B（env 层）**：用 `NSIS_DIR` 环境变量指向 chocolatey 装的完整 NSIS（`C:\Program Files (x86)\NSIS`），需先验证 tauri-bundler 是否真的 respect `NSIS_DIR` + 验证 NSCurl plugin 可用性
  - 两方案验证完成后把 `tauri.conf.json` 的 `languages` 改回 `["English", "SimplifiedChinese"]`
- **影响面**：当前 Windows 中文用户首次安装 / 卸载 installer UI 是英文（一次性体验，应用内仍是中文）
- **触发时机建议**：v0.1.1 或下一次动 windows release packaging 时

### F-075 WKWebView release 默认右键菜单含 "Reload" — 需要禁用 webview context menu

- **来源**：2026-05-10 v0.1.0 release build 实测（fix/0.1-check-for-updates-menu 自测）
- **现象**：release 桌面 app 右键 webview 主区域，弹出 macOS WKWebView 内置上下文菜单，里面有 "Reload" 等开发者风味选项；非编辑区域不该出现 reload。Tauri 这边没注册任何 reload 菜单（dev_reload_webview 在 `#[cfg(debug_assertions)]` gate 后），是 WKWebView 默认行为
- **为什么留**：当前 PR 主题是 updater 菜单 + 错误诊断，禁用 context menu 是独立 UI 行为变更，影响所有页面（含输入框系统右键），需要做白名单逻辑（输入框保留 cut/copy/paste/Look up，非编辑区禁用），不在本期 scope
- **改的话要做什么**：在前端入口（[`src/main.tsx`](../../src/main.tsx) 或 [`App.tsx`](../../src/App.tsx)）加全局 contextmenu listener，仅在 release（`import.meta.env.PROD`）+ Tauri 模式下 `e.preventDefault()`；按 `target` 是否 `HTMLInputElement / HTMLTextAreaElement / contenteditable` 决定是否保留默认菜单。或后端方案：用 webview2 / wkwebview API 全局禁 context menu（侵入性更大）
- **影响面**：用户视角的"开发者风味泄露"，无功能问题；纯观感
- **触发时机建议**：下次有人改前端入口 / 做 release UI polish 时

### F-076 plugin-process `relaunch()` 与 single-instance 锁的潜在 race

- **来源**：2026-05-10 updater 菜单 / release 自测
- **现象**：[`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs) 注册了 `tauri_plugin_single_instance`；前端 updater 完成后 [`desktopUpdater.ts::relaunch`](../../src/lib/desktopUpdater.ts) 直接调用 plugin-process `relaunch()`。理论 race 是新进程先启动但老进程还没释放 single-instance 锁，新进程被回调到老进程后退出，老进程随后也退出，最终没有进程在跑。
- **为什么留**：当前只是发布路径的潜在可靠性风险，实测未复现；前端已有手动重启兜底文案。真正修要决定是 fork/配置 single-instance、主动释放锁，还是改成 spawn-with-delay，超出当期修复范围。
- **改的话要做什么**：复现或用户反馈"更新后没重启"时，优先评估三条路：让 single-instance 识别 relaunch 二次启动、在 `relaunch()` 前释放锁、或不用 plugin-process relaunch 而改成 detached delayed spawn。
- **影响面**：平台 / 发布风险。命中时用户看到更新安装完成但 app 没自动重新打开。
- **触发时机建议**：下一次动 updater / single-instance / process relaunch 集成时处理；若 release 测试复现则提高优先级。

### F-089 后端 `ask_user_question` payload 仍是字面量英文，未走前端 i18n

- **来源**：2026-05-15 browser / updater 审查
- **现象**：后端调用 [`ask_user_question::execute`](../../crates/ha-core/src/tools/ask_user_question.rs) 时仍直接拼英文 `context` / `text` / `header` / `options[].label`。当前可确认的 callsite 包括 [`tools/browser/mod.rs::confirm_evaluate`](../../crates/ha-core/src/tools/browser/mod.rs) 和 [`tools/app_update.rs`](../../crates/ha-core/src/tools/app_update.rs) 的 install / rollback / manual prompt；中文或其它 locale 用户会在后端审批弹窗里看到英文。
- **为什么留**：正确修法不是把这些字符串短期翻成中文，而是改 ask_user 协议，让后端发送 i18n key + params，前端按当前 locale 渲染。协议迁移要兼容旧 payload 并批量替换 callsite，适合独立 PR。
- **改的话要做什么**：把 `text` / `header` / `options[].label` / `context` 支持 `{ key, params }` 形态，前端 fallback 兼容旧字符串；随后迁移 browser evaluate、app_update install / rollback / manual prompt 等后端弹窗，并补齐 12 语言 key。可给 `sync-i18n.mjs` 加启发式检查，避免新增 ask_user 字面量英文。
- **影响面**：多语言用户可见 UX 问题；不影响审批功能正确性，但会让本地化体验破功。
- **触发时机建议**：做权限审批 UX、Browser Phase 后续、或 app_update 弹窗整理时一并处理。

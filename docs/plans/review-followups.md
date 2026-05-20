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
  - LINE video — 官方 video message 需要单独 HTTPS `previewImageUrl` 图片；当前没有缩略图 URL 元数据，继续走下载链接文本
  - QQ Bot — channel/dms 端点 V2 不开放原生媒体上传，仍走下载链接文本
- **2026-05-19 已处理**：Signal 出站附件已接上 signal-cli JSON-RPC `send.attachments`，支持本地 `FilePath`，并把 `Url` / `Bytes` 物化到 `channels/signal/outbound-temp/` 后发送
- **2026-05-19 已处理**：Slack 出站附件已接上官方 files v2 流程（`files.getUploadURLExternal` → POST `upload_url` → `files.completeUploadExternal`），需要 bot token 具备 `files:write`
- **2026-05-19 已处理**：iMessage 出站附件已接上 imsg JSON-RPC `send` 的 `file` 参数，支持本地 `FilePath`，并把 `Url` / `Bytes` 物化到 `channels/imessage/outbound-temp/` 后发送；官方当前不是单独 `send_attachment` RPC
- **2026-05-19 已处理**：WhatsApp 出站附件已接上 bridge `POST /api/media`，`media` 字段使用 self-contained data URL（同时带旧 `data` alias），避免 bridge 访问 Hope Agent 本机路径或公网附件 URL
- **2026-05-19 已处理**：LINE / QQ Bot c2c/group 出站媒体已接上 `server.publicBaseUrl`：dispatcher 对 URL-only 渠道只在可生成 HTTPS 公网附件 URL 时走原生媒体，否则继续发送下载链接文本；LINE 支持 image/audio，QQ Bot c2c/group 支持 image/video/voice
- **为什么留**：Google Chat 还缺 user OAuth credential mode；LINE video 还缺独立缩略图 URL；QQ Bot channel/dms 端点本身不支持原生媒体，只能继续链接兜底。富媒体不阻塞首发文本
- **改的话要做什么**：Google Chat 先补 user OAuth 认证模式或等官方支持 app-auth 上传；LINE video 先补 `previewImageUrl` 生成/传递；QQ Bot channel/dms 若官方开放媒体端点再补原生发送
- **影响面**：能力承诺 vs 实际不一致，dispatcher 自动降级为链接文本但用户视觉体验差
- **触发时机建议**：用户报"图片发不出来"时按 channel 优先级排队；新增 OAuth scope 时同步评估

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

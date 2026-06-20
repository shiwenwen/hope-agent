# 浏览器自动化子系统

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/browser/`](../../crates/ha-core/src/browser/)、[`crates/ha-core/src/tools/browser/mod.rs`](../../crates/ha-core/src/tools/browser/mod.rs)、[`src/components/chat/BrowserPanel.tsx`](../../src/components/chat/BrowserPanel.tsx)、[`skills/ha-browser/SKILL.md`](../../skills/ha-browser/SKILL.md)

LLM 看到一个 `browser` 工具，**8 个高层 action**。默认后端是 **Chrome Extension + Native Messaging Host**：扩展运行在用户真实 Chrome profile 内，通过 `chrome.debugger` 控制已打开 tab；当扩展未安装或不可用、且动作不依赖真实 Chrome 状态时，降级到现有 `CdpBackend`（`chromiumoxide` managed / user_attach profile）兜底。

## 8-action 表面

```
status                                           # 当前 backend / extension/native host/CDP fallback 诊断
profile { op: list|launch|connect|disconnect|install_runtime } # CDP 会话生命周期
tabs    { op: list|new|select|close|open_user_tabs|claim|release|finalize } # 标签页 / 真实 Chrome tab claim
navigate { url?, op: go|back|forward|reload }
snapshot { format: role|screenshot|pdf }
act     { kind: click|dblclick|fill|type|hover|drag|select|press|upload } # type 是 fill 的兼容别名
observe { kind: console|network|page_errors|downloads, since? }
control { op: resize|scroll|wait_for|handle_dialog|evaluate|raw_cdp|download_cancel }
```

完整 schema 在 [`tools/definitions/core_tools.rs`](../../crates/ha-core/src/tools/definitions/core_tools.rs)（`TOOL_BROWSER` 段）。工具标记 `default_deferred: true`，常态不进 system prompt，通过 `tool_search` 按需暴露。配套 [`skills/ha-browser/SKILL.md`](../../skills/ha-browser/SKILL.md) 教 agent 标准 loop：`status → tabs → snapshot → act → 必要时 resnapshot`，含登录 / 2FA / captcha / camera prompt / 文件下载等阻塞情形清单（一律 `ask_user_question`）。

## Backend 架构

```
┌─────────────────────────────────────────────────────────┐
│ tools/browser/mod.rs  ←─ 8-action dispatch, URL guard   │
└────────────────┬────────────────────────────────────────┘
                 ▼
        browser::acquire_backend(requirement)
                 │
                 ▼
        backend_select::acquire_backend_with_requirement()
          ┌──────┴──────────────────────────┐
          ▼                                 ▼
  ExtensionBackend                    CdpBackend
  Chrome extension +                  chromiumoxide
  Native Messaging Host               managed/user_attach
          │                                 │
          ▼                                 ▼
  user's real Chrome tabs             Hope Agent CDP Chrome

   observe_buffer ─── ring buffer: console / network / errors
   frame.rs    ───── BROWSER_FRAME event + capture API
```

`backend_select` 按动作要求选择后端：

| Requirement | 用途 | 扩展不可用时 |
| --- | --- | --- |
| `ExtensionRequired` | `tabs.open_user_tabs`、`tabs.claim`、操作 claimed user tab、用户明确要求当前 Chrome / 已登录 tab | fail-closed，发 `browser:extension_required` 并返回安装提示 |
| `ExtensionPreferred` | 普通导航、截图、snapshot、表单填写等一般浏览动作 | 可 fallback 到 `CdpBackend`，结果里标明 `backend=cdp` |
| `CdpAllowed` | `profile.launch/connect/install_runtime`、Docker/headless、显式 CDP 生命周期 | 直接走 `CdpBackend` |

`BrowserBackend` trait 是后端抽象。`profile.launch` / `profile.connect` 只管理 CDP fallback 生命周期；真实用户 Chrome tab 必须经扩展 claim，不能用旧 `profile=system` 思路接管默认 profile。

### `BrowserBackend` trait（[`backend.rs`](../../crates/ha-core/src/browser/backend.rs)）

20 个 async method 覆盖 8-action 全部底层操作。共享类型 `ElementRef` / `Snapshot` / `ActKind` / `ActParams` / `ObserveEntry` / `ScreenshotParams` / `PdfParams` 等保持 backend-agnostic，方便后续接入其他实现。`ElementRef.locator` 是 backend 私有字段（CDP 用 CSS selector）——8-action 层从不读它，只透传 `ref_id`。

### `ExtensionBackend`（[`extension/backend.rs`](../../crates/ha-core/src/browser/extension/backend.rs)）

ExtensionBackend 通过 Core broker 和 Chrome 扩展通信。扩展 `connectNative("com.hope_agent.chrome")` 到 `ha-browser-host`，host 再通过本机 broker 连接 `ha-core`。broker 负责握手、版本诊断、request/response 生命周期、大响应 blob、二进制 `dataBlob`、connection generation、late response 丢弃和权限校验。

Native host 是很薄的本机桥：只做 Chrome Native Messaging stdio frame 和本机 broker socket/pipe 转发，不拥有业务策略。策略真相源全部在 `ha-core`：backend selection、tab lease、SSRF、protected path、tool approval、response/blob 校验、session cleanup 都在 Core 层裁决。

主要能力：

- 真实 Chrome tabs：`tabs.open_user_tabs` / `tabs.claim` / `tabs.select` / `tabs.release` / `tabs.finalize`，claim lease 按 Hope session 隔离，turn-end 和 session cleanup 会 best-effort 释放。`tabs.select` 传入 extension 数字 tab id 时会激活并接管该真实 Chrome tab，走统一审批流；如果只想表达显式接管语义，仍推荐用 `tabs.claim`。
- 8-action 控制：导航、role snapshot、screenshot/PDF、click/fill/hover/press/select/upload/drag、resize/scroll/wait/evaluate/dialog。
- 强 snapshot：DOM refs + AX enrichment + AX-only readable nodes；带 `backendDOMNodeId` 的可操作 AX 节点会生成可操作 ref。
- iframe：同源 iframe 使用 `iframeSelector >>> selector`；跨域 iframe 使用 `chrome.scripting` bridge 和 `chrome.debugger` flat session，`browser.status` 输出 frame tree / matched session 诊断。
- observe：console / network / page errors / downloads ring buffer。console / network / page errors 按当前受控 tab 过滤；downloads 是真实 Chrome 的下载活动流，读取前走统一审批。
- 强能力出口：`control.raw_cdp` 要求 ExtensionBackend 和 active controlled tab，只校验 CDP method 形态，是否执行走统一 tool 审批；`control.download_cancel` 仅 ExtensionBackend 支持，可按 download id 中断 Chrome 下载，也走统一 tool 审批。

扩展不可用时，真实 Chrome 状态相关动作绝不悄悄退回 managed profile；普通浏览动作才可 CDP fallback。

`tabs.finalize` 的关闭语义由 tab owner 决定：claimed user tab 只 release/debugger detach，默认保持打开；Hope-created agent tab 默认关闭，除非调用时把对应 `target_id` 放进 `keep: ["..."]`。

#### 安装 / 发布 / 信任边界

- **Chrome Extension 安装**：主路径是 Chrome Web Store；alpha/dev/self-host/enterprise 继续支持 `Load unpacked`。App 不能静默安装扩展，只能在 Settings 打开 Web Store 或 `chrome://extensions` 向导，最终确认必须发生在 Chrome UI。
- **Native host 安装**：Settings 调 owner 平面命令写 user-level native host manifest。正式桌面包通过 Tauri resource 携带 `ha-browser-host`，启动时把资源路径写入 `HOPE_AGENT_BROWSER_HOST_PATH`；dev/self-host 可显式传 path 或设置同名 env。manifest 的 `allowed_origins` 只写入用户选择/检测到的 extension id，扩展 id 必须是 Chrome 的 32 位 `a-p` 字符串。Windows 额外写 HKCU `Software\Google\Chrome\NativeMessagingHosts\<host>` 指向 manifest。
- **Broker 连接**：Core broker 启动时生成本机 token；`ha-browser-host` 首帧必须是带 token 的 `host.hello`。Unix/macOS socket 校验 peer uid，Windows named pipe 校验当前用户 SID。扩展不接触 Hope Agent HTTP API key。
- **Extension id**：生产 id 由 Web Store 首次上传后产生，进入 `browser.extension.extensionIds`；unpacked dev id 由 `manifest.key` 推导并自动加入状态输出，方便 alpha fallback。
- **Stop 控制**：用户可从页面 overlay、extension popup、Settings Stop 结束控制。Core 会 emit `browser:control_stopped`，并清理 session scoped lease/ref 状态。

### `CdpBackend`（[`cdp_backend.rs`](../../crates/ha-core/src/browser/cdp_backend.rs)）

包装现有 [`browser_state`](../../crates/ha-core/src/browser_state.rs) 全局单例。`browser_state` 维护 chromiumoxide `Browser` handle、`Page` 池、`active_page_id`、`ElementRef` 表、CDP event handler 任务。`CdpBackend` 是 trait 适配薄壳，不持状态。它长期保留为 fallback、Docker/headless、自托管和无插件场景使用。

**Stale-ref 一次自恢复**：`act` 失败且错误匹配 `is_stale_ref_error`（`not found` / `no such element` / `stale` / `detached`）时，内部触发：

1. 取出当前 `ref_id` 对应的 `role` + `text`
2. 重新 `take_snapshot_inner()` 刷新所有 ref
3. 按 `(role, text)` 精确或模糊匹配找新 ref
4. 用新 ref 重试一次 `act_inner`

成功返回字符串末尾追加 `(ref auto-recovered: old → new)` 让 LLM 知道发生过。**只重试一次**，避免死循环。`navigate` / `tabs.*` / `control.*` 不走 recovery。

## 实时 BrowserPanel

桌面 app 独占优势——chat 右侧固定 panel，实时镜像 agent 控制的 Chrome 窗口。**事件驱动 + 1s 兜底轮询**：

- **后端 emit**：[`browser::frame::emit_frame_async`](../../crates/ha-core/src/browser/frame.rs) 在每次 `act` / `navigate` / `tabs.new|select|claim` 完成后 fire-and-forget 一次截图（JPEG quality=70），通过 EventBus 发 `browser:frame`。ExtensionBackend 优先捕获真实 claimed tab，CDP fallback 保持旧路径。
- **前端订阅**：[`BrowserPanel.tsx`](../../src/components/chat/BrowserPanel.tsx) `useEffect` 订阅 `browser:frame` 立即替换帧
- **兜底轮询**：panel 打开期 `setInterval(1000, browser_capture_frame)`，关闭即 clear。覆盖用户在 Chrome 里手动操作的场景
- **互斥**：跟 PlanPanel / DiffPanel / CanvasPanel 互斥（ChatScreen.tsx effect），第一次 `browser:frame` 到来自动开 panel，用户手动关闭后保持关闭

`browser_capture_frame` 同时暴露为 Tauri 命令（[`src-tauri/src/commands/browser.rs`](../../src-tauri/src/commands/browser.rs)）和 HTTP `POST /api/browser/capture-frame`（[`crates/ha-server/src/routes/browser.rs`](../../crates/ha-server/src/routes/browser.rs)），保持 Transport 抽象两端对齐。

## SSRF 守卫

8-action 表面对高层 URL 操作做 SSRF 检查。check 走 [`security::ssrf::check_url`](../../crates/ha-core/src/security/ssrf.rs) `cfg.ssrf.browser()` policy + `trusted_hosts`：

| 入口 | 检查内容 |
| --- | --- |
| `navigate.go` | `url` |
| `tabs.new` | `url`（`about:blank` 跳过）|
| `profile.connect` | CDP endpoint `url`（防 agent 让我们连任意远程 9222）|
| `control.evaluate` | regex 扫脚本里的 `"http://..."` / `'https://...'` / `\`https://...\`` 字面量；任一被 policy 拒绝整个 evaluate 拒绝 |

`control.evaluate` 的扫描是 **best-effort**：base64 编码 URL、模板字符串动态拼接、`window.location.host` 之类无法防。skill 文档明确告诉 LLM 这条边界。

`tabs.open_user_tabs` / `tabs.claim` / 数字 id 的 `tabs.select` / `observe.downloads` / `control.evaluate` / `control.raw_cdp` / `control.download_cancel` 都通过统一权限引擎产生浏览器审批原因；Default 会弹 tool approval，Smart 可由 `_confidence:"high"` 或 judge model 自动放行，Yolo / Global YOLO / `ToolExecContext.auto_approve_tools` 直接放行。异步工具重入的 `external_pre_approved` 只表示外层统一 gate 已经处理过，内层不重复审批。高层 `evaluate` 的 SSRF 扫描不受这些开关影响；raw CDP 作为高级逃生口不做额外 method allow/block policy，也不扫描 `Runtime.evaluate` 表达式内容，风险交给统一 tool 审批。

## 配置

[`AppConfig.browser`](../../crates/ha-core/src/browser/mod.rs) 全 optional：

```jsonc
{
  "browser": {
    "defaultMode": "managed",                // "managed" (默认) | "user_attach"; 仅 UI 偏好,模型路径不读
    "defaultProfile": "managed",             // profile.op=launch 无 profile= 时的回退;默认 "managed"
    "backendPreference": "extension_first",   // 默认 extension 优先，普通动作可 CDP fallback
    "heartbeatIntervalSecs": 120,            // CDP ws idle keepalive 心跳间隔; 0 = 关
    "launchCircuit": { "failureThreshold": 3, "cooldownSecs": 60 },
    "extension": {
      "enabled": true,
      "allowRawCdp": true,                       // 兼容字段；不控制 raw_cdp 是否可用
      "showControlOverlay": true,
      "heartbeatIntervalSecs": 15,
      "extensionIds": ["<prod-or-dev-extension-id>"],
      "storeUrl": "https://chromewebstore.google.com/detail/hope-agent/<id>",
      "nativeHostName": "com.hope_agent.chrome"
    },
    "profiles": {
      "user_attach": { "port": 9222, "headless": false, "color": "#7c5cff" },
      "work":       { "userDataDir": "~/.hope-agent/browser-profiles/work" }
    }
  }
}
```

`browser.defaultMode` 风险等级 **LOW**（仅 UI 偏好），可走 `update_settings`。Profile 字段（`profiles[*]`）也是 **LOW**，settings UI 直接编辑。

`browser.extension.allowRawCdp` 只为旧配置 round-trip 保留；`control.raw_cdp` 的真实 gate 是统一权限引擎和 ExtensionBackend/controlled-tab 前提。把它设为 `false` 不会禁用 raw CDP。需要禁用某类工具能力时，应通过 agent 工具 allow/deny、session permission mode 或全局审批策略表达。

`browser.extension.extensionIds` 是生产/企业分发的显式信任列表；unpacked dev id 会从 repo 内 `extensions/chrome/manifest.json` 的 `key` 推导并追加到状态输出，但生产默认仍应回填 Web Store id 和 `storeUrl`。`showControlOverlay=false` 只隐藏页面 Stop overlay，不取消 toolbar popup / Settings Stop。`extension.heartbeatIntervalSecs` 是 native host / extension 活性诊断心跳，和 top-level `browser.heartbeatIntervalSecs`（CDP websocket keepalive）不是同一个开关。

**老 config 字段静默忽略**（serde default 行为）：
- `backend`（曾在 CDP / chrome-devtools-mcp 之间选；MCP backend 已删）
- `userAttach.lastSpawnedPort`（曾给独立的 "Reconnect" UX 用；user_attach 现在是 `profiles` 里的一等条目，port 固定 9222）

## 双模式 UX（Settings BrowserPanel）

设置面板提供三块互补能力：

- **Chrome Extension**：安装/修复 native host、打开 Chrome Web Store 或 unpacked extension 向导、显示 connected/version/backend 状态、Stop browser control。真实用户 Chrome tab 控制走这条路径。
- **独立浏览器**（`AppConfig.browser.defaultMode = "managed"`，默认）：hope-agent 用 [`browser-profiles/{name}/`](../../crates/ha-core/src/paths.rs) 维护的隔离 Chrome 实例做自动化。Launch / Profiles section 控制这条路径。
- **Hope Agent 持久 profile**（`defaultMode = "user_attach"`）：hope-agent 在 [`browser_user_attach_dir()`](../../crates/ha-core/src/paths.rs)（`~/.hope-agent/browser/user-attach/`）下 spawn 一个**独立 user-data-dir 的 Chrome**，让用户在 Hope Agent 专用浏览器里登录并长期复用 cookies，但**不动**用户真正的 Chrome 用户数据。Connect section 的 "doctor" banner + 一键启动按钮驱动这条路径。

两个 Tauri 命令支撑 doctor UX：

- `browser_doctor` 聚合 `probe_user_chrome`（GET `127.0.0.1:9222/json/version` 2s 超时）/ `chrome_already_running`（`pgrep` / `tasklist`）/ system Chrome 路径 / cached Chromium runtime，一次性返回 banner 所需的全部状态
- `browser_spawn_user_chrome`：在 user_attach profile（port 9222）下 spawn detached Chrome；port 已占时报错让用户先手动关老 Chrome

老的独立命令 `browser_probe_user_chrome` / `browser_check_chrome_running` / `userAttach.lastSpawnedPort` bookkeeping 已合并到 `browser_doctor` + profile 一等公民里，HTTP / Tauri 路由表只暴露上面两个。

## `profile.op=launch profile=` 一等公民

`profile.op=launch` 接受 `profile=<name>` 参数（默认 `managed`）。两个内置 profile + 任意数量用户定义 profile：

| profile | 数据目录 | 持久 | 何时用 |
|---|---|---|---|
| `managed`（内置） | `~/.hope-agent/browser/managed-runner/` | **每次 spawn 前 wipe** | 自动化、爬虫、不需要登录态的任务 |
| `user_attach`（内置） | `~/.hope-agent/browser/user-attach/` | ✓ cookies / 登录态长存 | agent 长期复用的"日常"浏览器；独立于用户真实 Chrome 数据 |
| 用户定义 `<name>` | `~/.hope-agent/browser-profiles/<name>/` | ✓ | 分账号 / 分域名 / 分项目 |

> 注：早期的 `target=managed|user_attach|system` 三档 enum 已删除。`target=system`（用 CDP 接管用户日常 Chrome）从未稳定 —— Chrome 148+ 架构性禁止 `--remote-debugging-port` 落在默认 user-data-dir 上。真实 daily Chrome / 已登录 tab 走 ExtensionBackend claim；`profile=user_attach` 只是 CDP fallback 的 Hope Agent 持久 profile。

## Chromium 运行时自动安装

`profile.op=install_runtime` 工具操作 / settings UI 「Install Chromium runtime」按钮 / `POST /api/browser/install-chromium-runtime` HTTP 路由都进入 [`browser/runtime.rs::ensure_chromium`](../../crates/ha-core/src/browser/runtime.rs)：

- 平台 / 架构 → `RuntimeSpec`（4 个支持目标：Mac/Mac_Arm/Linux_x64/Win_x64）
- pinned revision **每平台独立**（[`browser::runtime::CHROMIUM_REVISION_MAC_ARM` / `_MAC` / `_LINUX_X64` / `_WIN_X64`](../../crates/ha-core/src/browser/runtime.rs)）—— Chromium snapshots 每平台独立 trigger 构建，同一 revision 不保证四平台都存在，所以仿 Playwright / Puppeteer 走 per-platform map。升级按四个 `LAST_CHANGE` 各自取值 + HEAD 200 验证 + `--version` smoke test
- `commondatastorage.googleapis.com/chromium-browser-snapshots/{platform}/{rev}/{archive}` 经 SSRF 检查后流式下载，并复用全局 proxy 配置
- `zip::ZipArchive::by_index` + `mangled_name`（zip-slip 防护） + Unix 解压后 `chmod +x` + 启动 `<bin> --version` smoke-test 确认可执行
- 先解压到同目录 staging，smoke-test 通过后写 `.hope-agent-ready` marker 并原子 promote 到 `~/.hope-agent/browser/runtime/chromium-{revision}/`；后续 `build_launch_config` 三级 fallback 只命中带 ready marker 的 runtime，避免 partial install 污染缓存

下载进度走 EventBus `browser:chromium_download_progress`，stage `downloading` / `ready`，throttle 至每百分位 + 40ms 双限流；settings BrowserPanel 订阅渲染进度条。失败 partial 文件主动清理。

`build_launch_config` fallback 链（当没传 `executable_path` 时）：
1. `platform::find_chrome_executable()`（系统 Chrome）
2. `browser::runtime::cached_binary_path()`（已下载 Chromium runtime）
3. 都没有 → 带三条解决方案的友好错误（装 Chrome / 跑 install_runtime / 设 executable_path）

## Settings UX 与三种 launch target

设置面板的 Mode Radio 仍是**纯 UI 偏好**（[`BrowserMode` doc](../../crates/ha-core/src/browser/mod.rs)），但模型路径升级到三档 target。Settings BrowserPanel 在 Backend Radio 上方新增「Browser runtime」健康行，三态：

- ✓ `{brand}` detected on this system（系统 Chrome 找到，显示路径）
- ✓ Chromium runtime ready (rev XXX)（已下载 runtime）
- ⚠ No Chrome / Chromium found → 黄色 banner + 「Install Chromium runtime」按钮 + 进度条

`browser_doctor` 命令额外返回 `systemChrome: { brand, executable, userDataDir }` / `runtimeChromium: { revision, binaryPath }`。

## Docker 部署内置 Chromium

`Dockerfile` 在 runtime 阶段安装 Debian trixie `chromium` 包 + 字体 / nss / libgbm / libxss 共享库；容器带 `HA_DEPLOYMENT=docker`，所以 profile 未显式设置 `headless` 时默认走 headless，并在 spawn argv 里附加容器 sandbox 兼容参数。镜像体积增加约 250 MB；自建镜像若不需要浏览器能力可移除。无 chromium 包的极简镜像仍可走 runtime 自动下载兜底。详见 [`docs/deployment/docker.md`](../deployment/docker.md)。

## 已落地清单

✅ Backend trait + CdpBackend + ObserveBuffer
✅ 27 → 8 action 收敛 + schema 重写 + ha-browser bundled skill
✅ Stale-ref one-shot 自恢复
✅ 高层 URL 守卫覆盖 navigate / tabs.new / profile.connect / control.evaluate
✅ BROWSER_FRAME 事件 + capture_frame Tauri/HTTP + BrowserPanel 前端 + 12 语言 i18n
✅ AppConfig.browser 字段（defaultMode / defaultProfile / profiles / heartbeatIntervalSecs / launchCircuit）
✅ Settings BrowserPanel：Mode Tabs + doctor banner + 一键启动用户态 Chrome + Runtime status 行
✅ Chromium runtime auto-install（pinned revision + zip 解压 + smoke-test + 进度事件 + UI）
✅ Docker 镜像内置 chromium
✅ ExtensionBackend + Native Messaging Host + broker
✅ Extension-first backend selection + CDP fallback / ExtensionRequired fail-closed
✅ Settings Chrome Extension install/repair/stop flow（Web Store 主路径 + unpacked fallback）
✅ tabs.open_user_tabs / claim / release / finalize + session lease cleanup
✅ DOM/AX snapshot、iframe bridge、annotated screenshot、PDF/dataBlob、downloads observe/cancel
✅ raw CDP 强能力出口 + 统一 tool 审批

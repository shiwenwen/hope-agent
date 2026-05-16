# 浏览器自动化子系统

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/browser/`](../../crates/ha-core/src/browser/)、[`crates/ha-core/src/tools/browser/mod.rs`](../../crates/ha-core/src/tools/browser/mod.rs)、[`src/components/chat/BrowserPanel.tsx`](../../src/components/chat/BrowserPanel.tsx)、[`skills/ha-browser/SKILL.md`](../../skills/ha-browser/SKILL.md)

LLM 看到一个 `browser` 工具，**8 个高层 action**。底层直连 CDP（`chromiumoxide`，零运行时依赖）。`BrowserBackend` trait 保留为未来 Playwright / WebDriver 接入的扩展点，当前只有一个 `CdpBackend` 实现。

## 8-action 表面

```
status                                           # 当前 backend / 连接 / 活动 tab
profile { op: list|launch|connect|disconnect }   # Chrome 会话生命周期
tabs    { op: list|new|select|close }            # 标签页
navigate { url?, op: go|back|forward|reload }
snapshot { format: role|screenshot|pdf }
act     { kind: click|type|hover|drag|select|fill|press|upload }
observe { kind: console|network|page_errors, since? }
control { op: resize|scroll|wait_for|handle_dialog|evaluate }
```

完整 schema 在 [`tools/definitions/core_tools.rs`](../../crates/ha-core/src/tools/definitions/core_tools.rs)（`TOOL_BROWSER` 段）。工具标记 `default_deferred: true`，常态不进 system prompt，通过 `tool_search` 按需暴露。配套 [`skills/ha-browser/SKILL.md`](../../skills/ha-browser/SKILL.md) 教 agent 标准 loop：`status → tabs → snapshot → act → 必要时 resnapshot`，含登录 / 2FA / captcha / camera prompt / 文件下载等阻塞情形清单（一律 `ask_user_question`）。

## Backend 架构

```
┌─────────────────────────────────────────────────────────┐
│ tools/browser/mod.rs  ←─ 8-action dispatch, SSRF guard  │
└────────────────┬────────────────────────────────────────┘
                 ▼
         browser::acquire_backend()
                 │
                 ▼
            CdpBackend  (chromiumoxide)
                 │
                 ▼
            Chrome via CDP

   observe_buffer ─── ring buffer: console / network / errors
   frame.rs    ───── BROWSER_FRAME event + capture API
```

`BrowserBackend` trait 保留作为未来扩展点（Playwright / WebDriver 等），但本仓库当前只有 `CdpBackend` 一个实现。`ACTIVE_BACKEND: Mutex<Option<Arc<dyn BrowserBackend>>>` 持有当前 backend；`profile.launch` / `profile.connect` 调 `reset_backend()` 清空 + 重建（同时 `observe_buffer::clear_all`）。

### `BrowserBackend` trait（[`backend.rs`](../../crates/ha-core/src/browser/backend.rs)）

20 个 async method 覆盖 8-action 全部底层操作。共享类型 `ElementRef` / `Snapshot` / `ActKind` / `ActParams` / `ObserveEntry` / `ScreenshotParams` / `PdfParams` 等保持 backend-agnostic，方便后续接入其他实现。`ElementRef.locator` 是 backend 私有字段（CDP 用 CSS selector）——8-action 层从不读它，只透传 `ref_id`。

### `CdpBackend`（[`cdp_backend.rs`](../../crates/ha-core/src/browser/cdp_backend.rs)）

包装现有 [`browser_state`](../../crates/ha-core/src/browser_state.rs) 全局单例。`browser_state` 维护 chromiumoxide `Browser` handle、`Page` 池、`active_page_id`、`ElementRef` 表、CDP event handler 任务。`CdpBackend` 是 trait 适配薄壳，不持状态。

**Stale-ref 一次自恢复**：`act` 失败且错误匹配 `is_stale_ref_error`（`not found` / `no such element` / `stale` / `detached`）时，内部触发：

1. 取出当前 `ref_id` 对应的 `role` + `text`
2. 重新 `take_snapshot_inner()` 刷新所有 ref
3. 按 `(role, text)` 精确或模糊匹配找新 ref
4. 用新 ref 重试一次 `act_inner`

成功返回字符串末尾追加 `(ref auto-recovered: old → new)` 让 LLM 知道发生过。**只重试一次**，避免死循环。`navigate` / `tabs.*` / `control.*` 不走 recovery。

## 实时 BrowserPanel

桌面 app 独占优势——chat 右侧固定 panel，实时镜像 agent 控制的 Chrome 窗口。**事件驱动 + 1s 兜底轮询**：

- **后端 emit**：[`browser::frame::emit_frame_async`](../../crates/ha-core/src/browser/frame.rs) 在每次 `act` / `navigate` / `tabs.new|select` 完成后 fire-and-forget 一次截图（JPEG quality=70），通过 EventBus 发 `browser:frame`
- **前端订阅**：[`BrowserPanel.tsx`](../../src/components/chat/BrowserPanel.tsx) `useEffect` 订阅 `browser:frame` 立即替换帧
- **兜底轮询**：panel 打开期 `setInterval(1000, browser_capture_frame)`，关闭即 clear。覆盖用户在 Chrome 里手动操作的场景
- **互斥**：跟 PlanPanel / DiffPanel / CanvasPanel 互斥（ChatScreen.tsx effect），第一次 `browser:frame` 到来自动开 panel，用户手动关闭后保持关闭

`browser_capture_frame` 同时暴露为 Tauri 命令（[`src-tauri/src/commands/browser.rs`](../../src-tauri/src/commands/browser.rs)）和 HTTP `POST /api/browser/capture-frame`（[`crates/ha-server/src/routes/browser.rs`](../../crates/ha-server/src/routes/browser.rs)），保持 Transport 抽象两端对齐。

## SSRF 守卫

8-action 表面是 SSRF 检查的统一入口。check 走 [`security::ssrf::check_url`](../../crates/ha-core/src/security/ssrf.rs) `cfg.ssrf.browser()` policy + `trusted_hosts`：

| 入口 | 检查内容 |
| --- | --- |
| `navigate.go` | `url` |
| `tabs.new` | `url`（`about:blank` 跳过）|
| `profile.connect` | CDP endpoint `url`（防 agent 让我们连任意远程 9222）|
| `control.evaluate` | regex 扫脚本里的 `"http://..."` / `'https://...'` / `\`https://...\`` 字面量；任一被 policy 拒绝整个 evaluate 拒绝 |

`control.evaluate` 的扫描是 **best-effort**：base64 编码 URL、模板字符串动态拼接、`window.location.host` 之类无法防。skill 文档明确告诉 LLM 这条边界。

## 配置

[`AppConfig.browser`](../../crates/ha-core/src/browser/mod.rs) 全 optional：

```jsonc
{
  "browser": {
    "defaultMode": "managed",                // "managed" (默认) | "user_attach"; 仅 UI 偏好,模型路径不读
    "defaultProfile": "managed",             // profile.op=launch 无 profile= 时的回退;默认 "managed"
    "heartbeatIntervalSecs": 120,            // CDP ws idle keepalive 心跳间隔; 0 = 关
    "launchCircuit": { "failureThreshold": 3, "cooldownSecs": 60 },
    "profiles": {
      "user_attach": { "port": 9222, "headless": false, "color": "#7c5cff" },
      "work":       { "userDataDir": "~/.hope-agent/browser-profiles/work" }
    }
  }
}
```

`browser.defaultMode` 风险等级 **LOW**（仅 UI 偏好），可走 `update_settings`。Profile 字段（`profiles[*]`）也是 **LOW**，settings UI 直接编辑。

**老 config 字段静默忽略**（serde default 行为）：
- `backend`（曾在 CDP / chrome-devtools-mcp 之间选；MCP backend 已删）
- `userAttach.lastSpawnedPort`（曾给独立的 "Reconnect" UX 用；user_attach 现在是 `profiles` 里的一等条目，port 固定 9222）

## 双模式 UX（Settings BrowserPanel）

设置面板提供两条互斥的"模式"路径：

- **独立浏览器**（`AppConfig.browser.defaultMode = "managed"`，默认）：hope-agent 用 [`browser-profiles/{name}/`](../../crates/ha-core/src/paths.rs) 维护的隔离 Chrome 实例做自动化。Launch / Profiles section 控制这条路径。
- **接管用户态 Chrome**（`defaultMode = "user_attach"`）：hope-agent 在 [`browser_user_attach_dir()`](../../crates/ha-core/src/paths.rs)（`~/.hope-agent/browser/user-attach/`）下 spawn 一个**独立 user-data-dir 的 Chrome**，让用户日常使用 + agent 自动化共存，但**不动**用户真正的 Chrome 用户数据。Connect section 的 "doctor" banner + 一键启动按钮驱动这条路径。

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

> 注：早期的 `target=managed|user_attach|system` 三档 enum 已删除。`target=system`（接管用户日常 Chrome）从未稳定 —— Chrome 148+ 架构性禁止 `--remote-debugging-port` 落在默认 user-data-dir 上 + Google 等强反自动化站点会拒绝登录。要持久化 cookies / 扩展，用 `profile=user_attach`。

## Chromium 运行时自动安装（Phase 2）

`profile.op=install_runtime` 工具操作 / settings UI 「Install Chromium runtime」按钮 / `POST /api/browser/install-chromium-runtime` HTTP 路由都进入 [`browser/runtime.rs::ensure_chromium`](../../crates/ha-core/src/browser/runtime.rs)：

- 平台 / 架构 → `RuntimeSpec`（4 个支持目标：Mac/Mac_Arm/Linux_x64/Win_x64）
- pinned revision **每平台独立**（[`browser::runtime::CHROMIUM_REVISION_MAC_ARM` / `_MAC` / `_LINUX_X64` / `_WIN_X64`](../../crates/ha-core/src/browser/runtime.rs)）—— Chromium snapshots 每平台独立 trigger 构建，同一 revision 不保证四平台都存在，所以仿 Playwright / Puppeteer 走 per-platform map。升级按四个 `LAST_CHANGE` 各自取值 + HEAD 200 验证 + `--version` smoke test
- `commondatastorage.googleapis.com/chromium-browser-snapshots/{platform}/{rev}/{archive}` 经 SSRF 检查后流式下载
- `zip::ZipArchive::by_index` + `mangled_name`（zip-slip 防护） + Unix 解压后 `chmod +x` + 启动 `<bin> --version` smoke-test 确认可执行
- 缓存在 `~/.hope-agent/browser/runtime/chromium-{revision}/`，后续 `build_launch_config` 三级 fallback 命中

下载进度走 EventBus `browser:chromium_download_progress`，stage `downloading` / `ready`，throttle 至每百分位 + 40ms 双限流；settings BrowserPanel 订阅渲染进度条。失败 partial 文件主动清理。

`build_launch_config` fallback 链（当没传 `executable_path` 时）：
1. `platform::find_chrome_executable()`（系统 Chrome）
2. `browser::runtime::cached_binary_path()`（已下载 Chromium runtime）
3. 都没有 → 带三条解决方案的友好错误（装 Chrome / 跑 install_runtime / 设 executable_path）

## 双模式 UX → 三种 launch target（Phase 2 收尾）

设置面板的 Mode Radio 仍是**纯 UI 偏好**（[`BrowserMode` doc](../../crates/ha-core/src/browser/mod.rs)），但模型路径升级到三档 target。Settings BrowserPanel 在 Backend Radio 上方新增「Browser runtime」健康行，三态：

- ✓ `{brand}` detected on this system（系统 Chrome 找到，显示路径）
- ✓ Chromium runtime ready (rev XXX)（已下载 runtime）
- ⚠ No Chrome / Chromium found → 黄色 banner + 「Install Chromium runtime」按钮 + 进度条

`browser_doctor` 命令额外返回 `systemChrome: { brand, executable, userDataDir }` / `runtimeChromium: { revision, binaryPath }`。

## Docker 部署内置 Chromium

`Dockerfile` 在 runtime 阶段安装 Debian trixie `chromium` 包 + 字体 / nss / libgbm / libxss 共享库，让 server 模式开箱即用 `profile.op=launch headless=true`。镜像体积增加约 250 MB；自建镜像若不需要浏览器能力可移除。无 chromium 包的极简镜像仍可走 runtime 自动下载兜底。详见 [`docs/deployment/docker.md`](../deployment/docker.md)。

## 已落地清单

✅ Backend trait + CdpBackend + ObserveBuffer
✅ 27 → 8 action 收敛 + schema 重写 + ha-browser bundled skill
✅ Stale-ref one-shot 自恢复
✅ SSRF 守卫覆盖 navigate / tabs.new / profile.connect / control.evaluate
✅ BROWSER_FRAME 事件 + capture_frame Tauri/HTTP + BrowserPanel 前端 + 12 语言 i18n
✅ AppConfig.browser 字段（defaultMode / defaultProfile / profiles / heartbeatIntervalSecs / launchCircuit）
✅ Settings BrowserPanel：Mode Tabs + doctor banner + 一键启动用户态 Chrome + Runtime status 行
✅ Chromium runtime auto-install（pinned revision + zip 解压 + smoke-test + 进度事件 + UI）
✅ Docker 镜像内置 chromium

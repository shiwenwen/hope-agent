# macOS 控制子系统

> 返回 [文档索引](../README.md)
>
> 状态：桌面 bridge、权限 readiness、snapshot/elements/wait、apps/windows/act/menu/clipboard/dialog、display/window 截图镜像与审批分类已接入

本文是 Hope Agent 原生 macOS 桌面控制能力的技术契约。它描述当前系统的运行边界、模块职责、工具接口、权限审批、事件与前端集成方式。

## 能力边界

macOS 控制能力只在桌面 Tauri 运行模式下真实可用。授权主体必须是 Hope Agent `.app` 进程，所有读取屏幕、读取 Accessibility 树、合成输入和 App/窗口操作都通过该进程执行。

当前支持：

- 查询控制状态、权限 readiness 和系统权限摘要
- 读取前台 App、显示器、窗口、Accessibility 元素树，并提供排序元素候选检索
- 采集显示器或窗口截图帧，并把截图引用与 snapshot 绑定
- 等待 app/window/element 出现或消失
- 枚举、搜索、激活、启动、退出 App
- 枚举、聚焦、移动、缩放、最小化、关闭窗口
- 执行 AX 优先的点击、文本输入、设置值、快捷键、滚动、拖拽、右键、双击
- 枚举和点击菜单栏路径
- 读取、写入、清空 UTF-8 文本剪贴板
- 检查并处理前台 dialog/sheet
- 通过 EventBus 打开聊天右侧 Mac Control 镜像面板
- 接入统一 `permission::engine`、Plan Mode、Agent tool allow/deny、Transport Tauri/HTTP 双实现和日志

当前不支持：

- headless server / ACP 直接控制本机桌面
- 把 Terminal、shell、临时 dev binary 或脚本解释器作为长期授权主体
- 在没有 Accessibility 权限时读取或控制 AX 树
- 在没有 Screen Recording 权限时返回截图帧
- 读取密码字段真实值、在非 `clipboard.get` 结果中记录剪贴板原文或把截图 base64 写入上下文
- 用 AX 后台接口控制 Hope Agent 自己的窗口；自身窗口如需控制，必须走专用 main-thread AppKit bridge

## 架构

```mermaid
graph TD
    Agent["Chat Engine / Tool Loop"]
    Tool["ha-core::tools::mac_control"]
    Core["ha-core::mac_control"]
    Bridge["MacControlBridge trait<br/>OnceLock registry"]
    Desktop["src-tauri macOS bridge<br/>authorized .app process"]
    Native["Accessibility / CoreGraphics / NSWorkspace / NSPasteboard / CGEvent / Apple Events"]
    EventBus["EventBus<br/>mac_control:frame"]
    Frontend["PermissionsPanel / MacControlPanel / Transport"]

    Agent --> Tool
    Tool --> Core
    Core --> Bridge
    Bridge --> Desktop
    Desktop --> Native
    Core --> EventBus
    EventBus --> Frontend
    Frontend --> Core
```

分层规则：

- `ha-core` 定义公共类型、工具分发、权限风险分类、snapshot cache、错误统计、EventBus 事件和 bridge trait；不依赖 Tauri。
- `src-tauri` 在 setup 期间注册 `Arc<dyn MacControlBridge>`，并在 macOS `.app` 进程内调用原生 API。
- `ha-server` 只提供同形状 HTTP 路由；server/headless 没有 bridge，所有结果明确返回 `supported=false`。
- 前端只通过 `Transport` 调用 Tauri/HTTP 命令，不直接调用原生 AX 或系统 API。

## 模块职责

| 路径 | 职责 |
| --- | --- |
| `crates/ha-core/src/mac_control.rs` | 公共类型、bridge 注册、status/permissions/snapshot/elements/wait/apps/windows/act/menu/clipboard/dialog/capture_frame 入口、snapshot cache、截图文件 LRU、错误统计 |
| `crates/ha-core/src/tools/mac_control.rs` | builtin tool 的 `action` 分发，把模型参数映射到 `ha_core::mac_control::*` 请求 |
| `crates/ha-core/src/tools/definitions/core_tools.rs` | `mac_control` tool schema、deferred/tool fate 元数据 |
| `crates/ha-core/src/permission/engine.rs` | `mac_control` 只读、普通/隐私动作、高风险突变的审批分类 |
| `crates/ha-core/src/tools/approval.rs` | `MacControlAction` / `MacControlDangerousAction` 审批 payload |
| `src-tauri/src/macos_control.rs` | macOS bridge 实现，封装 AX、截图、NSWorkspace、CGEvent、菜单、剪贴板、dialog、Apple Events fallback |
| `src-tauri/src/tauri_wrappers.rs` | Tauri command wrapper |
| `crates/ha-server/src/lib.rs` | HTTP `/api/mac-control/*` 路由 |
| `src/lib/transport-http.ts` | HTTP command 映射，保持和 Tauri invoke 同名 |
| `src/components/settings/PermissionsPanel.tsx` | Settings → Permissions 顶部 readiness 摘要 |
| `src/components/chat/MacControlPanel.tsx` | 聊天右侧截图镜像面板 |
| `skills/ha-mac-control/SKILL.md` | 模型使用 `mac_control` 的标准 loop 和恢复策略 |

## 运行模式

| 运行模式 | bridge | 结果 |
| --- | --- | --- |
| macOS Tauri desktop | 已注册 | 真实查询和执行桌面控制 |
| macOS Tauri desktop 但缺权限 | 已注册 | 返回 `supported=true`，`readiness=blocked/limited`，具体 action 按权限失败 |
| HTTP/server/headless | 未注册 | 返回同形状结果，`supported=false` |
| 非 macOS | 未注册 | 返回同形状结果，`supported=false` |

`MacControlStatus` 中几个字段的语义：

- `platform`：当前平台字符串。
- `supported`：当前运行模式是否可以真实控制本机 macOS 桌面。
- `desktop`：是否桌面运行模式。
- `bridgeRegistered`：是否已注册 `MacControlBridge`。
- `readiness`：`ready | limited | blocked | unsupported`。
- `coreReady`：Accessibility + Screen Recording 两个核心权限是否已满足。
- `requiredPermissions` / `optionalPermissions`：权限摘要，来自系统权限 catalog。
- `stats`：snapshot cache、截图文件上限、最近错误统计。

## 权限模型

macOS TCC 权限按进程和 bundle 身份绑定。Hope Agent 的桌面控制能力要求真正调用系统 API 的进程就是已授权的 Hope Agent `.app`。

| 权限 | 用途 | 是否核心 |
| --- | --- | --- |
| Accessibility | 读取 AX 树、AXPress、AXSetValue、窗口操作、菜单和 dialog 控制 | 是 |
| Screen Recording | 截图、右侧镜像面板、视觉定位 | 是 |
| Automation: System Events / per-app | Apple Events fallback，例如部分 close/quit 流程 | 可选 |
| Input Monitoring | 当前未接入；预留给操作录制或全局输入学习 | 可选 |
| System Audio Capture | 当前未接入；预留给音频理解 | 可选 |

readiness 计算规则：

- `ready`：Accessibility 和 Screen Recording 均已授权，且没有可选权限待处理。
- `limited`：核心权限已授权，但可选权限缺失或需手动确认。
- `blocked`：缺 Accessibility 或 Screen Recording。
- `unsupported`：非 macOS、非桌面模式、没有 bridge 或系统权限 catalog 不支持。

运行时防御：

- `snapshot` 读取 AX 树需要 Accessibility；`includeScreenshot=true` 额外需要 Screen Recording。截图可按显示器或前台窗口/指定窗口采集；失败时返回 AX-only snapshot 并附 warning。
- `capture_frame` 只需要 Screen Recording；默认采集主显示器，失败时不伪造 frame。
- `act` / `windows` / `menu` / `dialog` 需要 Accessibility。
- Apple Events fallback 只在系统允许 Automation 时可用；失败结果必须结构化返回。

## Transport 接口

前端 Transport 层提供五个 macOS Control command。Tauri 与 HTTP 必须同名、同形状；HTTP/server 模式不控制本机桌面，返回同形状 `supported=false` 结果。

| Tauri Command | HTTP | 入参 | 出参 |
| --- | --- | --- | --- |
| `mac_control_status` | `GET /api/mac-control/status` | 无 | `MacControlStatus`：readiness、权限摘要、bridge 状态、运行时统计 |
| `mac_control_permissions` | `GET /api/mac-control/permissions` | 无 | `MacControlPermissionsResponse`：`status` + `systemPermissions` 完整系统权限 catalog |
| `mac_control_snapshot` | `POST /api/mac-control/snapshot` | `{ options?: MacControlSnapshotRequest }`；Tauri command 直接接收 `options` | `MacControlSnapshotResponse`：`status`、`snapshot?`、`error?` |
| `mac_control_elements` | `POST /api/mac-control/elements` | `{ options?: MacControlElementsRequest }`；Tauri command 直接接收 `options` | `MacControlElementsResponse`：`status`、`result?`、`error?` |
| `mac_control_capture_frame` | `POST /api/mac-control/capture-frame` | 无 | `MacControlFrameResponse`：`status`、`frame?`、`error?` |

Transport 请求类型：

| 类型 | 字段 |
| --- | --- |
| `MacControlSnapshotRequest` | `includeScreenshot?: boolean`、`screenshotTarget?: "display" \| "window"`、`displayId?: number`、`windowId?: string`、`maxElements?: number`、`maxDepth?: number` |
| `MacControlElementsRequest` | `op?: "find"`、`target?: MacControlTargetQuery`、`limit?: number`、`maxElements?: number`、`maxDepth?: number` |

Transport 结果类型：

| 类型 | 字段 |
| --- | --- |
| `MacControlSnapshotResponse` | `status: MacControlStatus`、`snapshot?: MacControlSnapshot`、`error?: string` |
| `MacControlElementsResponse` | `status: MacControlStatus`、`result?: MacControlElementsResult`、`error?: string` |
| `MacControlFrameResponse` | `status: MacControlStatus`、`frame?: MacControlFramePayload`、`error?: string` |
| `MacControlFramePayload` | `snapshotId`、`mediaId?`、`path?`、`jpegBase64`、`widthPx`、`heightPx`、`target`、`displayId?`、`windowId?`、`windowTitle?`、`boundsPoints?`、`scale?`、`capturedAt`、`frontmostApp?` |

这些接口供设置页和右侧镜像面板使用。聊天模型执行桌面动作时不直接调用这些 Tauri command，而是调用 builtin tool `mac_control`。

## Builtin Tool

`mac_control` 是 Standard 工具：

| 属性 | 值 |
| --- | --- |
| `ToolTier` | `Standard` |
| `default_for_main` | `true` |
| `default_for_others` | `false` |
| `default_deferred` | `true` |
| `internal` | `false` |
| `concurrent_safe` | `false` |
| `async_capable` | `false` |

设计含义：

- 主 Agent 默认可发现和使用；其它 Agent 默认关闭，避免子 Agent 意外操作电脑。
- schema 较大，默认走 deferred tool loading。
- GUI 操作依赖焦点、前台 App 和坐标状态，不允许并发执行。
- 只读动作可直接放行，突变动作进入审批系统。

## Builtin Tool API

`mac_control` 是单工具多 action/op 形态。执行层必须按当前 `action/op` 解释参数；共享 schema 中的其它字段不能改变当前 op 的语义。

通用输入字段：

| 字段 | 类型 | 用途 |
| --- | --- | --- |
| `action` | string | 必填。`status`、`permissions`、`snapshot`、`elements`、`wait`、`apps`、`windows`、`act`、`menu`、`clipboard`、`dialog` |
| `op` | string | 子操作。按 `action` 解释；未传时使用该 request 类型的默认 op |
| `target` | `MacControlTargetQuery` | app/window/element 目标过滤，用于 `wait`、`windows`、`act`、`dialog` |
| `appName` | string | App 名称查询，用于 `apps.*` |
| `appNameMatch` | `"exact" \| "contains"` | App 名称匹配策略，默认 `exact` |
| `bundleId` | string | App bundle id 查询，用于 `apps.*` |
| `pid` | number | App 进程 id 查询，用于 `apps.*` |
| `limit` | number | `apps.list/installed/search` 和 `elements.find` 返回条数上限 |
| `windowScope` | `"frontmost" \| "all"` | `windows.list` 和窗口解析范围，默认 `frontmost` |
| `windowId` | string | 窗口 id，用于 `windows.*` 或 `snapshot` window 截图 |
| `x` / `y` | number | `windows.move` 目标位置、`act.click_point` 点击位置、`act.drag` 终点 |
| `width` / `height` | number | `windows.resize` 目标尺寸 |
| `text` | string | `act.type` / `act.paste` 输入文本、`clipboard.set` 写入文本；目标文本匹配放在 `target.text` |
| `value` | string | `act.set_value` 写入值 |
| `key` / `keys` | string / string[] | `act.hotkey` 单键或组合键 |
| `deltaX` / `deltaY` | number | `act.scroll` 滚动增量 |
| `path` | string[] | `menu.click` 菜单路径 |
| `buttonText` | string | `dialog.accept/dismiss` 指定按钮文案 |
| `scope` | `"app" \| "system"` | `menu.list/click` 菜单范围，默认 `app` |
| `includeScreenshot` | boolean | `snapshot` 是否采集 JPEG |
| `screenshotTarget` | `"display" \| "window"` | `snapshot.includeScreenshot=true` 时选择显示器或窗口 |
| `displayId` | number | `snapshot` display 截图目标显示器 |
| `includeSnapshot` | boolean | `act`、`wait`、`dialog` 是否在结果中带完整 AX snapshot，默认 `false` |
| `maxElements` / `maxDepth` | number | AX 树遍历上限 |
| `timeoutMs` / `pollMs` | number | `wait` 总超时和轮询间隔 |
| `maxChars` | number | `clipboard.get` 返回文本上限 |

通用输出形状：

| action | 输出形状 |
| --- | --- |
| `status` | `MacControlStatus` |
| `permissions` | `{ status: MacControlStatus, systemPermissions: SystemPermissionsResponse }` |
| `snapshot` | `{ status: MacControlStatus, snapshot?: MacControlSnapshot, error?: string }` |
| `wait` | `{ status, op, matched, elapsedMs, attempts, target, matches, snapshot?, error? }` |
| 其它 action | `{ status: MacControlStatus, result?: <ActionResult>, error?: string }` |

### status / permissions / snapshot / elements / wait

| action / op | 入参 | 出参 | 说明 |
| --- | --- | --- | --- |
| `status` | `action="status"` | `MacControlStatus` | 只读 readiness/status；不会触发系统权限请求 |
| `permissions` | `action="permissions"` | `status`、`systemPermissions` | 只读系统权限 catalog |
| `snapshot` | `action="snapshot"`；可选 `includeScreenshot`、`screenshotTarget`、`displayId`、`windowId`、`maxElements`、`maxDepth` | `snapshot?: MacControlSnapshot`、`error?` | 返回 AX 树；`includeScreenshot=true` 时写 JPEG 文件并返回 `snapshot.screenshot` |
| `elements.find` | `action="elements"`、`op="find"`；可选 `target`、`limit`、`maxElements`、`maxDepth` | `result.op`、`target`、`snapshotId`、`createdAt`、`frontmostApp?`、`totalMatches`、`elements[]`、`truncated`、`warnings[]` | `elements[]` 是排序候选，每项含 `element`、`window?`、`score`、`reasons[]` |
| `wait.present` | `action="wait"`、`op="present"`、`target` 至少一个字段；可选 `timeoutMs`、`pollMs`、`includeSnapshot`、`maxElements`、`maxDepth` | `matched`、`elapsedMs`、`attempts`、`target`、`matches`、`snapshot?`、`error?` | 轮询直到目标出现；默认不返回完整 snapshot |
| `wait.gone` | `action="wait"`、`op="gone"`、`target` 至少一个字段；可选同上 | 同 `wait.present` | 轮询直到目标消失；若当前已不存在则立即成功 |

### apps

| action / op | 入参 | 出参 `result` | 说明 |
| --- | --- | --- | --- |
| `apps.list` | `action="apps"`、`op="list"`；可选 `limit` | `op`、`frontmost?`、`apps[]`、`installedApps=[]`、`activated?=null`、`launched?=null`、`quit?=null`、`execution?` | 只读运行中 App 列表 |
| `apps.frontmost` | `action="apps"`、`op="frontmost"` | `frontmost?`、`apps=[]` | 只读前台 App |
| `apps.installed` | `action="apps"`、`op="installed"`；可选 `appName`、`appNameMatch`、`bundleId`、`limit` | `installedApps[]`，并标注 `running`、`pid?`、`active`、`hidden` | 已安装 App 列表/过滤 |
| `apps.search` | `action="apps"`、`op="search"`；可选 `appName`、`appNameMatch`、`bundleId`、`limit` | `installedApps[]` | 已安装 App 检索；名称不确定时先用它找 `bundleId` |
| `apps.activate` | `action="apps"`、`op="activate"`；`pid` / `bundleId` / `appName` 之一 | `activated?`、`execution="NSRunningApplication.activate"` | 激活已运行 App |
| `apps.launch` | `action="apps"`、`op="launch"`；`bundleId` / `appName` 之一 | `launched?`、`execution="NSWorkspace.openApplication"` | 启动已安装 App |
| `apps.quit` | `action="apps"`、`op="quit"`；`pid` / `bundleId` / `appName` 之一 | `quit?`、`execution` | 请求 App 正常退出；高风险 |

### windows

| action / op | 入参 | 出参 `result` | 说明 |
| --- | --- | --- | --- |
| `windows.list` | `action="windows"`、`op="list"`；可选 `windowScope`、`target`、`maxElements`、`maxDepth` | `op`、`windowScope`、`frontmostApp?`、`windows[]`、`actedWindow?=null`、`execution?` | `windowScope="all"` 返回所有运行中 App 窗口，id 为 `win_<pid>_<index>` |
| `windows.focus` | `action="windows"`、`op="focus"`；`windowId` / `target.windowTitle` 之一 | `actedWindow?`、`execution="AXRaise/AXFocused"` | 聚焦窗口 |
| `windows.move` | `action="windows"`、`op="move"`；`windowId` / `target.windowTitle` 之一；`x`、`y` | `actedWindow?`、`execution="AXSetPosition"` | 移动窗口到 macOS point 坐标 |
| `windows.resize` | `action="windows"`、`op="resize"`；`windowId` / `target.windowTitle` 之一；`width`、`height` | `actedWindow?`、`execution="AXSetSize"` | 调整窗口大小 |
| `windows.minimize` | `action="windows"`、`op="minimize"`；`windowId` / `target.windowTitle` 之一 | `actedWindow?`、`execution="AXSetMinimized"` | 最小化窗口 |
| `windows.close` | `action="windows"`、`op="close"`；`windowId` / `target.windowTitle` 之一 | `actedWindow?`、`execution` | 关闭窗口；高风险 |

### act

| action / op | 入参 | 出参 `result` | 说明 |
| --- | --- | --- | --- |
| `act.dry_run` | `action="act"`、`op="dry_run"`、`target` | `op`、`execution="DryRun"`、`target?`、`snapshot=null` | 只解析目标，不执行 UI 操作 |
| `act.click` | `action="act"`、`op="click"`、`target` | `execution="AXPress"` 或 `"CGEventFallback"`、`target?`、`snapshot?` | AX target 点击；不消费裸 `x/y` |
| `act.click_point` | `action="act"`、`op="click_point"`、`x`、`y`，且不能带 `target` | `execution="CGEventClick"`、`target=null`、`snapshot?` | 裸坐标点击，允许 `(0, 0)` |
| `act.double_click` | `action="act"`、`op="double_click"`、`target` | `execution="CGEventDoubleClick"`、`target?`、`snapshot?` | 对目标元素中心双击 |
| `act.right_click` | `action="act"`、`op="right_click"`、`target` | `execution="CGEventRightClick"`、`target?`、`snapshot?` | 对目标元素中心右键 |
| `act.type` | `action="act"`、`op="type"`、`text`；可选 `target` | `execution="AXSetValue"`、`target?`、`snapshot?` | 对文本控件设置文本；未给 target 时使用当前 focused text element |
| `act.paste` | `action="act"`、`op="paste"`、`text`；可选 `target` | `execution` 为 pasteboard 恢复状态、`target?`、`snapshot?` | 临时写 pasteboard 后触发系统粘贴；不回显 text |
| `act.set_value` | `action="act"`、`op="set_value"`、`target`、`value` | `execution="AXSetValue"`、`target?`、`snapshot?` | 对明确 AX 元素设置值 |
| `act.hotkey` | `action="act"`、`op="hotkey"`；`key` 或 `keys` | `execution="CGEventHotkey"`、`target=null`、`snapshot?` | 合成快捷键 |
| `act.scroll` | `action="act"`、`op="scroll"`；`deltaX` / `deltaY` 之一非零 | `execution="CGEventScroll"`、`target=null`、`snapshot?` | 合成滚动 |
| `act.drag` | `action="act"`、`op="drag"`、`target`、`x`、`y` | `execution="CGEventDrag"`、`target?`、`snapshot?` | 从目标元素中心拖拽到目标坐标 |

`act` 默认 `snapshot=null`；显式 `includeSnapshot=true` 时，除 `dry_run` 外会返回完整后置 `snapshot`。

### menu / clipboard / dialog

| action / op | 入参 | 出参 `result` | 说明 |
| --- | --- | --- | --- |
| `menu.list` | `action="menu"`、`op="list"`；可选 `scope`、`maxDepth` | `op`、`scope`、`path=[]`、`items[]`、`clicked=null` | 只读菜单树；`scope="app"` 是前台 App 菜单，`system` 是菜单栏 extras/status items |
| `menu.click` | `action="menu"`、`op="click"`、`path[]` 非空；可选 `scope`、`maxDepth` | `op`、`scope`、`path`、`items[]`、`clicked?` | 按 path 逐级点击菜单项；危险菜单词走高风险审批 |
| `clipboard.get` | `action="clipboard"`、`op="get"`；可选 `maxChars` | `op`、`text?`、`textLen`、`truncated`、`changed=false` | 读取 UTF-8 文本剪贴板；隐私敏感，需审批 |
| `clipboard.set` | `action="clipboard"`、`op="set"`、`text` | `op`、`text=null`、`textLen`、`truncated`、`changed=true` | 写入 UTF-8 文本；结果不回显原文 |
| `clipboard.clear` | `action="clipboard"`、`op="clear"` | `op`、`text=null`、`textLen=0`、`truncated=false`、`changed=true` | 清空剪贴板 |
| `dialog.inspect` | `action="dialog"`、`op="inspect"`；可选 `target`、`includeSnapshot`、`maxElements`、`maxDepth` | `op`、`dialogs[]`、`actedButton=null`、`snapshot?`、`execution=null` | 返回当前前台 App dialog/sheet 摘要、文本和按钮 |
| `dialog.accept` | `action="dialog"`、`op="accept"`；可选 `buttonText` / `target.text`、`target`、`includeSnapshot` | `op`、`dialogs[]`、`actedButton?`、`snapshot?`、`execution="AXPressOrCGEvent"` | 点击 accept 类按钮；高风险 |
| `dialog.dismiss` | `action="dialog"`、`op="dismiss"`；可选 `buttonText` / `target.text`、`target`、`includeSnapshot` | 同 `dialog.accept` | 点击 cancel/close 类按钮 |

`dialog` 默认 `snapshot=null`；需要完整 AX 树时传 `includeSnapshot=true`。

核心输出类型字段：

| 类型 | 字段 |
| --- | --- |
| `MacControlAppSummary` | `pid`、`bundleId?`、`name?` |
| `MacControlRunningApp` | `pid`、`bundleId?`、`name?`、`active`、`hidden`、`activationPolicy` |
| `MacControlInstalledApp` | `name?`、`bundleId?`、`path?`、`executablePath?`、`running`、`pid?`、`active`、`hidden`、`activationPolicy?` |
| `MacControlDisplaySummary` | `id`、`framePoints`、`scale` |
| `MacControlWindowSummary` | `id`、`appPid?`、`role?`、`subrole?`、`title?`、`focused`、`boundsPoints?` |
| `MacControlElementSummary` | `id`、`windowId?`、`role?`、`label?`、`value?`、`enabled?`、`focused`、`boundsPoints?`、`actions[]` |
| `MacControlElementCandidate` | `element`、`window?`、`score`、`reasons[]` |
| `MacControlTargetMatches` | `app?`、`windows[]`、`elements[]` |
| `MacControlMenuItemSummary` | `title?`、`description?`、`value?`、`role?`、`enabled?`、`actions[]`、`children[]` |
| `MacControlClipboardResult` | `op`、`text?`、`textLen`、`truncated`、`changed` |
| `MacControlDialogSummary` | `window`、`text[]`、`buttons[]` |
| `MacControlBounds` | `x`、`y`、`width`、`height`，单位是 macOS point |

参数归一化规则：

- 空字符串按缺省处理。
- `pid <= 0` 按缺省处理。
- `enabled=false` / `focused=false` 在 target 中按缺省处理，避免 provider 自动补齐布尔值导致误筛选。
- `appNameMatch` 默认为 `exact`；只有显式传 `contains` 才允许包含匹配。
- `target.windowTitleMatch` 默认为 `exact`；只有显式传 `contains` 才允许包含匹配。
- `snapshot.includeScreenshot=true` 时，`screenshotTarget` 默认为 `display`；传 `displayId` 可指定 `snapshot.displays[].id`；传 `screenshotTarget="window"` 可截图当前前台窗口，传 `windowId` 可指定当前 snapshot 中的窗口。
- `elements.limit` 默认为 20，硬上限 100；`elements.find` 允许空 target，用于只读列出当前前台 App 的高置信候选。
- `windows.windowScope` 默认为 `frontmost`；传 `all` 会返回所有运行中 App 的窗口，并生成 `win_<pid>_<index>` 形式的跨 App window id。
- `menu.scope` 默认为 `app`；`system` 只访问 macOS 菜单栏 extras/status items，不回退到前台 App 菜单。
- `clipboard.maxChars` 默认为 4000，硬上限 20000；`clipboard.set` 不修剪空白，但会硬截到 200000 字符。
- `includeSnapshot` 默认为 `false`；`act` / `wait` / `dialog` 默认只返回摘要字段，显式传 `includeSnapshot=true` 才返回完整 AX snapshot。`act.dry_run` 始终保持轻量。
- 合法坐标 `0` 不能被全局吞掉；裸坐标点击只能通过 `act.click_point` 表达。

有副作用的 App 操作优先使用 `bundleId` 或 `pid`。当名称匹配失败时，模型应先调用 `apps.search` 或 `apps.installed` 找候选，再用明确标识执行 `activate/launch/quit`。

## Snapshot 与 Frame

`snapshot` 返回短生命周期桌面状态，主要字段：

```jsonc
{
  "snapshotId": "macsnap_...",
  "createdAt": "2026-05-18T...",
  "frontmostApp": { "pid": 1234, "bundleId": "com.apple.finder", "name": "Finder" },
  "displays": [{ "id": 1, "framePoints": { "x": 0, "y": 0, "width": 1512, "height": 982 }, "scale": 2 }],
  "windows": [{ "id": "win_1", "title": "Downloads", "focused": true, "boundsPoints": { "x": 80, "y": 90, "width": 900, "height": 680 } }],
  "elements": [{ "id": "el_7", "windowId": "win_1", "role": "AXButton", "label": "Open", "enabled": true, "boundsPoints": { "x": 824, "y": 710, "width": 70, "height": 28 }, "actions": ["AXPress"] }],
  "screenshot": { "mediaId": "macsnap_....jpg", "path": "~/.hope-agent/mac-control/snapshots/macsnap_....jpg", "widthPx": 3024, "heightPx": 1964, "target": "display", "displayId": 1, "boundsPoints": { "x": 0, "y": 0, "width": 1512, "height": 982 }, "scale": 2 },
  "warnings": []
}
```

约束：

- `element.id` 和 `window.id` 只在当前 snapshot 或短 TTL cache 内可靠。
- macOS API 中 AX / CGWindow 使用 point；截图使用 pixel；bridge 负责 scale 转换。
- display 截图默认取主显示器；window 截图会把 AX `windowId` 重新匹配到当前 CGWindow，匹配失败时返回 warning 而不是伪造图片。
- 元素树默认 `maxElements=120`、`maxDepth=8`，硬上限分别为 `500` 和 `16`。
- snapshot cache 进程内最多保留 20 份。
- 截图文件写入 `~/.hope-agent/mac-control/snapshots/`，最多保留 100 个 JPEG。
- 工具结果只返回截图摘要和路径，不把 base64 放进上下文。
- `capture_frame` 成功后 emit `mac_control:frame`，用于打开或刷新右侧面板。

## Target 查询与匹配

目标查询结构：

```jsonc
{
  "appName": "Finder",
  "bundleId": "com.apple.finder",
  "windowTitle": "Downloads",
  "windowTitleMatch": "exact",
  "elementId": "el_7",
  "text": "Open",
  "role": "AXButton",
  "enabled": true,
  "focused": true
}
```

匹配原则：

- `bundleId` / `pid` / `elementId` 优先于名称和文本。
- 名称和窗口标题默认精确匹配；包含匹配必须显式声明。
- 对多个相似目标，执行层必须返回歧义错误或选择唯一最高置信候选，不应静默随机选择。
- AX 元素 mutation 会收集候选并按聚焦、可用、可执行、可见 bounds、精确文本等信号打分；若最高分并列且没有精确 `elementId`，直接拒绝执行，并提示模型用 fresh `snapshot` 后补充 `elementId`、`target.windowTitle`、`target.role` 或更具体的 `target.text`。
- `elements.find` 使用同一套 AX snapshot 与元素匹配规则，只读返回 `totalMatches`、候选 `element`、所在 `window`、`score` 和 `reasons`。模型应先用它确认候选，再把选中的 `element.id` 传给 `act.*`。
- `act.dry_run` 使用和 `act.click` / `act.set_value` 相同的目标解析、前台 App 校验、歧义拒绝和 stale 检查，但不触发 AX action、CGEvent、键盘、剪贴板或窗口变化；结果 `snapshot=null`，避免把完整 AX 树塞回上下文。
- mutation 前会刷新 snapshot 并按 target 重新解析，降低 stale element 引用风险。
- mutation 成功后默认不返回完整后置 snapshot；模型应优先用 `wait`、`elements.find`、`windows.list` 或 `dialog.inspect` 做小结果验证。只有调试或需要完整树时才传 `includeSnapshot=true`。
- `dialog.inspect/accept/dismiss` 默认返回 dialog/window/button/text 摘要，不返回完整 snapshot；需要调试完整 AX 树时传 `includeSnapshot=true`。
- action target 必须符合当前前台 App 约束；跨 App 误点要被拒绝或要求先激活目标 App。

`wait` 是只读能力，默认 `timeoutMs=10000`、`pollMs=500`；硬上限 `timeoutMs=60000`，`pollMs` 限制在 `100..=5000`。`wait/gone` 在目标当前已不存在时立即成功。默认结果只返回 `matches` 摘要，确需完整命中/超时 snapshot 时传 `includeSnapshot=true`。

## 动作执行模型

执行优先级：

1. Accessibility 原生 action：`AXPress`、菜单项 press、dialog 按钮 press。
2. Accessibility attribute 设置：`AXValue`、`AXFocused`、`AXPosition`、`AXSize`。
3. AppKit / NSWorkspace：运行中 App 枚举、激活、启动、正常退出。
4. CGEvent fallback：点击、右键、双击、拖拽、滚动、快捷键。
5. Apple Events fallback：AX close / NSRunningApplication quit 失败后的受控回退路径。

坐标动作规则：

- `act.dry_run` 用于 mutation 前确认目标元素；它只读返回解析结果，不附带完整 snapshot，不产生 UI 副作用。
- `act.click` 只能点击 AX target，不读取 `x/y`。
- 裸坐标点击必须使用 `act.click_point`。
- `act.drag` 的起点来自 AX target 中心，终点来自 `x/y`。
- 每次坐标动作之后应重新 snapshot 验证结果。

文本输入规则：

- 文本控件优先走 `AXValue`。
- 需要焦点输入时先解析和聚焦目标。
- 长文本可通过 `act.paste` pasteboard fallback；不得记录旧剪贴板内容，工具结果只报告恢复是否成功。
- `act.paste` 会尽量恢复旧 UTF-8 文本剪贴板；若旧剪贴板不是 UTF-8 文本，无法完整恢复原类型，结果会标记 `clipboard_restore=not_restored_no_prior_utf8_text`。
- 密码字段不得回读真实值。

窗口操作规则：

- `windows.list` 默认只列前台 App；需要发现后台窗口时传 `windowScope=all`，可再结合 `target.appName` / `target.bundleId` / `target.windowTitle` 过滤。
- `windowScope=all` 返回的 `win_<pid>_<index>` 可直接用于 `windows.focus/move/resize/minimize/close`。
- `windows.move/resize/minimize/close` 只作用于外部 App 窗口。
- 命中 Hope Agent 自己的窗口时拒绝，避免在非主线程触发 AppKit 崩溃。
- `windows.close` 属于高风险动作，审批中禁用 AllowAlways。

菜单和 dialog 规则：

- `menu.list` 默认返回前台 App 菜单树，可按深度截断；传 `scope=system` 返回系统菜单栏 extras/status items。
- `menu.click` 按 path 逐级解析并点击。App 菜单按 title 匹配；system extras 可按 title、description 或 value 匹配，且优先精确匹配再包含匹配。
- 命中危险菜单词的 `menu.click` 属于高风险动作。
- `clipboard.get/set/clear` 均走普通审批；`clipboard.get` 是隐私敏感读取，不作为只读动作自动放行。
- `clipboard.set` 和 `act.paste` 都不得在结果里回显写入文本；只报告长度、是否截断、是否改变或剪贴板恢复状态。
- `dialog.inspect` 只读返回 dialog/sheet 文本和按钮摘要。
- `dialog.accept` 高风险；`dialog.dismiss` 普通突变。

## 审批与风险分类

`permission::engine` 对 `mac_control` 做 tool-specific 风险分类，不依赖 Agent 自定义审批清单。

| 分类 | action/op | 决策 |
| --- | --- | --- |
| 只读 | `status`、`permissions`、`snapshot`、`elements.find`、`wait`、`apps.list/frontmost/installed/search`、`windows.list`、`act.dry_run`、`menu.list`、`dialog.inspect` | Allow |
| 普通/隐私动作 | `apps.activate/launch`、`windows.focus/move/resize/minimize`、除 `dry_run` 外的 `act.*`、普通 `menu.click`、`clipboard.get/set/clear`、`dialog.dismiss` | Ask，可 AllowAlways |
| 高风险突变 | `apps.quit`、`windows.close`、`dialog.accept`、命中危险词的 `menu.click` | Ask，`forbids_allow_always=true` |

权限模式交互：

- Default：普通/隐私动作和高风险突变均弹审批。
- Smart：只读直接放行；普通/隐私动作仍可被 smart 策略处理；高风险突变保持严格审批。
- YOLO：除 Plan Mode 外放行，但风险命中写 `app_warn!` 审计日志。
- Plan Mode：不在 plan allowlist 的 `mac_control` 调用会被拒绝；即使 YOLO 也不能绕过。

审批 payload：

- `mac_control_action`：普通/隐私动作。
- `mac_control_dangerous_action`：高风险突变，前端显示 strict 样式并禁用 AllowAlways。

审批弹窗应展示 action/op、目标 App、窗口、元素 label、菜单 path、hotkey 或输入摘要。文本输入需要截断和脱敏，不能展示密码字段值。

## EventBus 与前端面板

事件：

| 事件 | payload | 说明 |
| --- | --- | --- |
| `mac_control:frame` | `MacControlFramePayload` | 最新截图帧，来自 `snapshot(includeScreenshot=true)` 或 `capture_frame` |

前端行为：

- Settings → Permissions 调 `mac_control_status`，在权限列表顶部展示 readiness。
- `MacControlPanel` 打开期间轮询 `mac_control_capture_frame`。
- 聊天页监听 `mac_control:frame`，首次收到 frame 时打开右侧 Mac Control 面板。
- Mac Control 面板与 PlanPanel / DiffPanel / CanvasPanel / BrowserPanel 视觉互斥。

## 存储、日志和错误统计

存储：

- 截图目录：`~/.hope-agent/mac-control/snapshots/`
- 文件格式：JPEG
- 保留策略：最多 100 个截图文件，写入新截图后做 LRU 清理
- 进程内 snapshot cache：最多 20 份

日志原则：

- 不记录截图 base64。
- 不记录旧剪贴板内容。
- 文本输入默认截断并脱敏。
- 审批日志记录 action/op、目标 App、窗口、元素 label 和风险类型。
- 原生错误按 operation 聚合到 `MacControlRuntimeStats.recentErrors`，用于 `status` 返回和排查。

失败结果必须结构化返回，常见错误包括：

- 当前运行模式 unsupported
- 缺 Accessibility 或 Screen Recording
- 目标 App 未运行或未安装
- 名称匹配歧义
- 元素 stale 或不可见
- AX action 不支持
- 窗口位于其它 Space 或系统限制导致无法操作
- Apple Events fallback 未获授权

## 模型使用约束

内置 skill：`skills/ha-mac-control/SKILL.md`。

模型应遵循：

```text
status -> snapshot/elements.find/wait -> apps/windows/act/menu/clipboard/dialog -> snapshot 验证
```

关键规则：

- 不要一开始猜坐标。
- 有副作用操作前尽量先确认前台 App 和目标窗口。
- 相似按钮或输入框较多时先用 `elements.find` 选候选，再用 `elementId` 执行。
- 对高不确定性的点击/设置值，先用 `act.dry_run` 验证目标解析结果。
- App 名称找不到时先用 `apps.search` / `apps.installed` 查候选。
- 名称匹配不稳定时改用 `bundleId`、`pid`、`windowId` 或 `elementId`。
- 点击 AX 元素用 `act.click`；点击屏幕坐标用 `act.click_point`。
- 操作后用 snapshot 或 wait 验证结果。

## 测试矩阵

轻量检查：

- `cargo check -p ha-core --tests`
- `cargo check -p hope-agent`
- `pnpm typecheck`
- `git diff --check`

单元测试关注：

- tool schema action/op 覆盖
- request normalization：空字符串、`pid=0`、`enabled=false`、坐标 `0`
- readiness 计算
- `mac_control` 权限风险分类
- dangerous menu pattern
- target query ranking
- ambiguous AX target rejection
- elements.find candidate ordering
- point/pixel scale 转换
- snapshot LRU 和错误统计

手工 QA：

- 首次授权、半授权、全授权状态
- HTTP/server 模式返回 `supported=false`
- Finder、Notes、System Settings、Safari 非网页区域
- App search → bundleId → launch/activate
- 窗口 focus/move/resize/minimize/close
- 菜单 list/click 与危险菜单审批
- dialog inspect/accept/dismiss
- 右侧 Mac Control 面板自动打开和轮询刷新
- 多显示器、Retina scale、窗口跨屏
- Mission Control / Space 导致目标不可见

## 已知系统边界

- TCC 权限绑定到 bundle 身份；开发期二进制和正式 `.app` 的授权不是同一份。
- Accessibility 树质量取决于目标 App。Electron、SwiftUI、自绘 canvas、游戏或网页 canvas 可能需要截图和坐标 fallback。
- CGEvent fallback 依赖当前焦点和坐标，必须在操作后重新读取状态验证。
- macOS Spaces / Mission Control 会影响后台窗口可见性和可操作性。
- Screen capture、AX 行为和 Automation consent 会随 macOS 版本变化，需要在发版说明中标清最低支持版本和已验证版本。
- server/headless 模式不会承诺本机桌面控制；除非另有签名、已授权、常驻的本机 helper 作为 bridge 主体。

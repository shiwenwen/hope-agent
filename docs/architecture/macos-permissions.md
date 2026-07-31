# 系统权限（macOS TCC）

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/permissions.rs`](../../crates/ha-core/src/permissions.rs)、[`crates/ha-core/src/platform/system_permissions.rs`](../../crates/ha-core/src/platform/system_permissions.rs)、[`crates/ha-core/src/platform/mod.rs`](../../crates/ha-core/src/platform/mod.rs)（facade）、Tauri 薄壳在 [`src-tauri/`](../../src-tauri/)、前端面板 [`src/components/settings/PermissionsPanel.tsx`](../../src/components/settings/PermissionsPanel.tsx)

## 概述

本子系统是 macOS **TCC（Transparency, Consent, and Control）系统权限的底层探测与引导层**：它维护一张 28 项权限的静态目录（`PERMISSION_DEFS`），向桌面 Settings → Permissions 面板回答两个问题——「这项系统权限当前是什么状态」「点这个按钮时该怎么把用户引导去授权」。

定位上有三条边界要先讲清楚：

- **只读探测 + 引导，不持久化**：TCC 同意状态由 macOS 系统按进程 + bundle 身份持有，本子系统自己不落任何库、不写 `AppConfig`/`UserConfig`。除**录屏的「待重启」进程内探针记忆**（下节，纯内存、随进程消亡）外一律实时查询。
- **Tauri-only**：能力仅经 **6 条 Tauri 命令**暴露给桌面 Shell，**无 HTTP 路由**、不进 `transport.ts` 的 `COMMAND_MAP`——HTTP/server 模式没有系统托盘进程，TCC 概念不适用。
- **非 macOS 严禁伪造 granted**：Windows / Linux / 其它平台一律收敛到 `unsupported` / `NotApplicable`，绝不假装已授权（单测红线，见安全章节）。

与上层桌面控制能力 [`ha-mac-control`](macos-control.md) 是两个子系统：本文是底层 TCC 探测/引导，`ha-mac-control` 是 macOS 桌面控制能力的 readiness 编排，复用本目录的 catalog 但走独立命令/路由（边界详见末章）。

## 模块结构

| 文件 | 职责 |
|---|---|
| [`permissions.rs`](../../crates/ha-core/src/permissions.rs) | 子系统根：`PermissionDef` 静态目录 `PERMISSION_DEFS`（28 项）、v2/v1 双层 API、数据类型枚举、v1↔v2 legacy 映射纯函数、`blocking_with_timeout` 超时包装 |
| [`platform/system_permissions.rs`](../../crates/ha-core/src/platform/system_permissions.rs) | 按 `target_os` 分 `macos` / `windows` / `linux` / `other` 四套 `mod imp`，仅 macOS 给出 framework 原生实现；非 macOS 的 `imp` 一律 `supported()=false` |
| [`platform/mod.rs`](../../crates/ha-core/src/platform/mod.rs) | facade（`pub(crate)`）：`system_permissions_supported` / `system_permissions_platform_name` / `check_system_permission_item` / `request_system_permission_item` / `system_permission_raw_probe`（探针答复侧），把上层 `permissions.rs` 与平台 `imp` 解耦 |

`permissions.rs` 是领域层（权限目录 + 状态语义 + API），`platform/system_permissions.rs` 是平台原生实现层（framework 链接 + 探测）。上层永远经 `platform/mod.rs` 的 facade 调下层，不直接 `cfg` 进 imp。

## 核心数据结构

### 权限目录：`PermissionDef` 与 28 项静态表

`PermissionDef` 是单条权限的**静态元数据**，`PERMISSION_DEFS` 常量数组是这张目录的**单一真相源**，目前 28 项：

| 字段 | 含义 |
|---|---|
| `id` | 稳定字符串标识（如 `full_disk_access` / `automation_system_events` / `desktop_folder`） |
| `group` | 所属分组 `SystemPermissionGroup` |
| `request_mode` | 请求时的引导方式 `SystemPermissionRequestMode` |
| `settings_pane` | 对应的「系统设置」面板锚点（`x-apple.systempreferences:` 深链） |
| `usage` / `note` | 面向 UI 的用途说明与备注 |
| （响应侧 `resettable`） | 非 def 字段，由 `platform::system_permission_supports_reset(id)` 现算后写进 `SystemPermissionItem`：本平台 / 本构建能否重置该项 TCC 记录。**只驱动 UI 是否出按钮，非安全边界**——`reset_system_permission` 会再过同一份白名单 |
| `troubleshoot_note` | 请求后仍 `NotGranted` 时**替换** `note` 的排障文案（附带 `SystemPermissionItem.troubleshoot=true` 标志）。挂在 def 上而非另开 id-match，避免第二张注册表静默漏挂；前端用**独立** i18n key `permissionItems.<id>.troubleshootNote`（复用 `note` key 会显示语义完全不同的译文） |

新增权限项是有契约的：**新增项须同步 `platform` 层 `check_item` / `request_item` 的 `match` 分支**，否则该 id 落 `NotApplicable`；并须考虑 v1 兼容层映射（见 v1 章节）。

### 分组 / 状态 / 请求模式枚举

- **`SystemPermissionGroup`**（snake_case 序列化，5 分组）：`ControlCapture`（控制与采集）/ `FileAccess`（文件访问）/ `PersonalData`（个人数据）/ `DeviceNetwork`（设备与网络）/ `SystemServices`（系统服务）。
- **`SystemPermissionStatus`**（8 态）：`Granted` / `GrantedPendingRestart` / `NotGranted` / `NotDetermined` / `Restricted` / `ManualCheck` / `NotApplicable` / `NotUsed`。`ManualCheck` 表示「无可靠原生 API、需用户自查或探测式判定」，`NotApplicable` 表示「本平台不适用」，`NotUsed` 表示「这项被定义但当前不实际使用」。`GrantedPendingRestart` 表示「TCC 已授权、但本进程要重启才能用」（见下「录屏待重启探针」）——**它对一切能力门控等价于未授权**（`legacy_state_for_status` 映射 `not_granted`、`mac_control` 全部 `== Granted` 比较），只影响给用户/模型的措辞。
- **`SystemPermissionRequestMode`**（4 态）：`NativePrompt`（弹系统原生授权框）/ `OpenSettings`（跳转系统设置面板）/ `TriggerProbe`（触发一次探测以诱发同意弹窗）/ `None`（不主动请求）。

### v2 响应类型

- **`SystemPermissionItem`**（camelCase 序列化）：v2 单项响应，承载某一 `id` 的当前状态 + 元数据，供前端面板逐项渲染。
- **`SystemPermissionsResponse`**：v2 顶层响应 `{ platform, supported, items }`——`supported=false` 时 `items` 为空，前端据此隐藏整个面板或显示「本平台不适用」。

### v1 兼容类型

- **`PermissionStatus`**：v1 单项状态 `{ id, status: String }`（旧字符串态）。
- **`AllPermissions`**：v1 兼容聚合结构，**15 个固定权限字段**，`Default` 实现把全部字段置为 `unknown`。这是早期前端契约，与 v2 的 28 项目录**不一一对应**（见 v1 章节）。

## 数据流 / 状态机

### v2 查询：`check_system_permissions`

桌面面板加载时调 `check_system_permissions`（v2 查询入口）：

1. 经 `blocking_with_timeout` 进 `spawn_blocking`，挂 **6 秒 `CHECK_TIMEOUT`**——framework 查询偶发卡顿不阻塞 UI，超时回 fallback。**这一预算被 28 项串行共享**，须同时容纳最慢两项（录屏探针 ≤1.5s + `notifications` XPC ≤2s）；曾为 3s 会被击穿，而超时 fallback 是 `unsupported_response()`——真 Mac 上整个面板会退化成「仅支持 macOS」页、`mac_control` 误报 unsupported。**新增慢检查项须重算此预算**。
2. 先看 `system_permissions_supported()`：非 macOS 直接回 `supported=false` + 空 `items`。
3. macOS 下遍历 `PERMISSION_DEFS`，逐项调 `platform::check_system_permission_item`，下沉到 `imp::check_item`。
4. `imp::check_item` 按 `id` `match` 派发到对应 framework 的 `authorizationStatus` 查询，把原生枚举映射成 `SystemPermissionStatus`。

`check_item` 的几条特殊分支：

- `automation_system_events` / `automation_messages`：**无可靠 per-target 状态 API**，永远返回 `ManualCheck`。
- `full_disk_access` / `desktop_folder` / `documents_folder` / `downloads_folder`：**无原生 API，走文件系统探测式检测**（`full_disk_access` 读 `~/Library/Safari/Bookmarks.plist` / `~/Library/Messages/chat.db`；folder 三项 `read_dir ~/Desktop` 等）——成功 = `Granted`、失败 = `ManualCheck`（注意**不是** `NotGranted`，因为探测失败可能是别的原因）。
- `system_audio_capture` / `homekit`：返回 `NotUsed`。
- `notifications`：非 bundle 进程查询会抛 `NSException`（Rust 无法 catch），故在非 bundle 进程**降级 `ManualCheck`**（见红线）。
- `screen_recording`：进程内 preflight 为假时**再经新进程探针**判定是否「已授权待重启」，见下节。

### 录屏「待重启」探针（`--tcc-probe`）

macOS 把录屏能力**固定在进程启动时建立的 WindowServer 连接上**：应用运行期间用户在系统设置里打开开关，本进程 `CGPreflightScreenCaptureAccess()` 仍恒为假，直到重启。为区分「已授权待重启」与「真未授权」，`screen_recording_status` 在 preflight 为假时 spawn **同一 exe 的短命子进程** `hope-agent --tcc-probe screen_recording`（新进程 → 看到实时 TCC 状态），据其结果回 `GrantedPendingRestart` 或 `NotGranted`。

契约与红线：

- **判据是 stdout token 而非退出码**：子进程打印一行 `hope-agent-tcc-probe:granted=1|0|unknown`（前缀常量 `permissions::TCC_PROBE_OUTPUT_PREFIX` 为跨 crate 单一真相源）。**不认 token 一律 unknown、绝不当已授权**——自升级回滚后磁盘上的旧二进制不认识该 flag，会落到别的分派路径，其退出码含义完全不同（single-instance 转发即 exit 0）。
- **`--tcc-probe` 分派必须早于 guardian / child 分派**（`src-tauri/src/main.rs`，另见 [cli](cli.md)）：落到 guardian 会**每次探针拉起一个完整 GUI**，且 1.5s 超时 kill 只杀直接子进程、孙进程成孤儿。探针分支也**不得初始化任何运行时状态**（无 `ensure_dirs` / `init_runtime` / 日志）。
- **答复侧 `raw_probe` 永不再走探针**（只调 preflight），否则子进程再 spawn 子进程无限递归。
- **进程内记忆（非 keyed TTL 缓存，故刻意不用 `ttl_cache`）**：`SCREEN_PROBE` 持锁**跨越** spawn 实现单飞行——面板 `Promise.all` 会并发触发两次全目录检查，否则各 spawn 一个子进程；**正向结果（待重启）进程内终身有效**（该事实到重启前不可逆，故不再探测、也不被后续瞬时失败覆盖），**只有负向结果**按 `PROBE_RETRY_TTL`（5s）防抖，代价是用户开完开关后最多 5 秒盲窗（点「去授权」绕过防抖）。
- **仅桌面**（`is_desktop()`）：其余运行模式的宿主二进制未必实现该 flag。

### v2 请求：`request_system_permission`

用户在面板点某项的「请求」按钮时调 `request_system_permission`（v2 请求入口）：

1. 挂 **65 秒 `REQUEST_TIMEOUT`**（`blocking_with_timeout`）——原生授权框需要等用户操作，故远大于查询超时。
2. `find_def` 按 `id` 在 `PERMISSION_DEFS` 找到定义，下沉 `platform::request_system_permission_item` → `imp::request_item`。
3. `imp::request_item` **按 `def.id` `match` 派发**（**不是**按 `request_mode`——`request_mode` 是 catalog 给前端的元数据，平台层落地走 id-match + 一个 `_` 兜底分支）。落地分三类行为：

| 行为 | 哪些 id 走这条 |
|---|---|
| 触发原生授权框（framework `request*` 调用，内部多含 60s `wait_for_prompt` 等待用户决策；已非 `NotDetermined` 的项先 `open_settings_pane` 跳过弹框） | `accessibility` / `screen_recording` / `input_monitoring` / `camera` / `microphone` / `location` / `contacts` / `calendar` / `reminders` / `photos` / `bluetooth` / `speech_recognition` / `notifications`（即 catalog 里 `NativePrompt` 那批） |
| `trigger_automation_probe`：`osascript` 触发一次 Apple Events 诱发「自动化」同意弹窗 → `open_settings_pane` 打开设置 → re-check（`check_item`） | `automation_system_events` / `automation_messages` |
| `_` 兜底分支：`open_settings_pane`（用 `open` 跳 `x-apple.systempreferences:` 深链）→ re-check（`check_item`） | 其余全部 id（catalog 里 `OpenSettings` 与 `None` 那批，含 `system_audio_capture`） |

automation 两项的 request 路径：osascript 触发同意 → 打开设置 → re-check（因为 check 永远 `ManualCheck`，request 后也只能让用户在设置里确认）。

`accessibility` 的 request 路径特殊：走 **`AXIsProcessTrustedWithOptions({kAXTrustedCheckOptionPrompt: YES})`**——**这个调用本身才是把应用注册进「系统设置 → 隐私与安全性 → 辅助功能」列表的动作**（此前只 `open_settings_pane`，用户跳过去发现列表里根本没有 Hope Agent 这一行、无从开启）。两点须知：① 该调用**同步返回当前（仍为假的）信任状态**、弹窗异步等用户操作，且 macOS 每应用只弹一次，故失败分支**照常 `open_settings_pane`**——刻意双 UI，因为反面（信这个同步 false 而不做事）就是「点了没反应」的死路；② 运行在 tokio blocking 线程上**须套 `objc2::rc::autoreleasepool`**（否则 autoreleased 字典无池可归、泄漏并打 runtime 警告）。

### v2 重置 TCC 记录：`reset_system_permission`

旧版本（v0.8.0 / #298 稳定签名之前）留下的 TCC 记录会让系统设置里开关照旧可见、却对当前二进制恒拒——从本子系统看与「未授权」不可区分，用户唯一出路是删掉记录重新授权。此入口把这件事从终端命令搬进面板。

落地是 `tccutil reset <service> <bundle-id>`（**无公开 API 可做重置，`tccutil` 是唯一受支持途径；这三个服务不需要 sudo**），四条约束：

- **服务名是编译期闭合白名单**（`accessibility→Accessibility` / `screen_recording→ScreenCapture` / `input_monitoring→ListenEvent`）：调用方只递权限 id，先经 `find_def` 校验存在，再经白名单换服务名。**服务字符串永不来自模块外**，否则这个动作就退化成「抹掉任意 TCC 服务」。参数走 `Command::args` 不经 shell。
- **bundle id 运行时取 `NSBundle.mainBundle.bundleIdentifier`**，不硬编码、不读 `tauri.conf.json`。**`None` 是承重的**：裸开发二进制没有稳定 TCC 身份，此时 `supports_reset()=false`、`SystemPermissionItem.resettable=false`，UI 不出按钮，后端也拒绝——否则 `tccutil` 会去动某个别的 bundle。故**此功能在 `pnpm tauri dev` 下不可见**，验证须用打包应用。
- **重置录屏后必须 `forget_screen_probe_memory()`**：探针的「待重启」正向结果是进程内终身有效的（前提是授权在重启前不可逆），而重置恰好打破该前提——不清记忆，面板会继续声称「已授权 · 重启生效」，而授权已被抹掉。
- **owner / GUI-only（红线）**：不是配置字段，故不进设置三件套；**刻意不给模型工具面、无 `ha-settings` category**——模型能重置 TCC 就等于能随时剥掉用户已授的系统权限、或反复制造授权弹窗，风险等级与 Provider 凭据同级。

UI 侧只在 `not_granted` / `not_determined` / `restricted` 出按钮：**`granted` 不出**（等于给用户自毁按钮），**`granted_pending_restart` 也不出**——那种状态记录是健康的、只差重启，重置会白扔掉用户刚给的授权。走 `AlertDialog` 二次确认，成功后提示并提供重启入口（复用既有 `request_app_restart`，exit code 42 由 Guardian 接管；dev / 关闭 Guardian 时只退出不重启）。

### v1 兼容包装

`check_all_permissions` / `check_permission` / `request_permission` 是 v1 兼容入口，**内部全部委托 v2** 再做 legacy 映射，由四个纯函数承担 id 与状态的翻译：

- `legacy_request_id` / `legacy_status_for_id`：v1 id ↔ v2 id 与状态的双向映射。
- `legacy_files_and_folders`：v1 的 `files_and_folders` 字段由 v2 的 `desktop_folder` / `documents_folder` / `downloads_folder` **三项聚合**而成（三项全 `Granted` → `granted`；任一 `NotGranted`/`NotDetermined`/`Restricted` → `not_granted`；否则 `unknown`）。`legacy_request_id("files_and_folders")` 则映射到 `desktop_folder` 触发请求。
- `legacy_state_for_status`：v2 `SystemPermissionStatus` → v1 字符串态。

`AllPermissions`（v1，15 字段）与 `PERMISSION_DEFS`（v2，28 项）**不一一对应**——典型如 `automation` → `automation_system_events` 的映射、`files_and_folders` 的三合一聚合。**新增权限项时须同步考虑 v1 映射是否需要更新**。

## 持久化

本子系统**不落任何库、不占任何配置字段、不写 `~/.hope-agent`**：

- **无 DB 表**——TCC 状态实时查询；唯一例外是录屏探针的**进程内内存记忆**（`SCREEN_PROBE`，随进程消亡，见上），不落盘。
- **无 config 字段**——不进 `AppConfig` / `UserConfig`，每次面板加载现查。
- **无 `~/.hope-agent` 文件**——注意 `paths.rs::permission_dir`（`~/.hope-agent/permission/`）持有的是**权限引擎 v2**（`protected_paths` / `dangerous_commands`，见 [`permission-system.md`](permission-system.md)）的列表，**与本子系统无关**，两者只是名字里都有「permission」。
- **TCC 同意状态由 macOS TCC 数据库按进程 + bundle 身份持有**，属系统外部状态，非本仓库管理。

## 对外接口面

### Tauri 命令（6 条，Desktop-only）

6 条命令经 Tauri 薄壳（`tauri_wrappers`）注册到 `invoke_handler`，**无对应 HTTP 路由**：

| 命令 | 层 | 作用 |
|---|---|---|
| `check_system_permissions` | v2 | 查询全部 28 项状态，回 `SystemPermissionsResponse` |
| `request_system_permission` | v2 | 请求单项授权（按 `def.id` 派发） |
| `reset_system_permission` | v2 | 重置单项 TCC 记录（见下「重置 TCC 记录」），失败回 `CmdError` |
| `check_all_permissions` | v1 | 兼容聚合查询，回 `AllPermissions` |
| `check_permission` | v1 | 兼容单项查询 |
| `request_permission` | v1 | 兼容单项请求 |

这 6 条全部登记在 [`api-reference.md`](api-reference.md) §7.3 的 **Desktop-only** 表，计入合法的 Tauri-only 差集（当前 23 条，脚本口径见该文末）。

### HTTP 路由

**无**——不进 `build_router_with_cors`，不进 `transport.ts` 的 `COMMAND_MAP`。HTTP transport 对这 6 条命令没有对应实现。

### 事件

**无**——本子系统不 emit EventBus 事件。

### 前端面板

[`src/components/settings/PermissionsPanel.tsx`](../../src/components/settings/PermissionsPanel.tsx)（Settings → Permissions）经 `getTransport().call` 调 `check_system_permissions` / `request_system_permission`。因这两条仅 Tauri 实现，**HTTP transport 下无对应能力**——面板在 server 模式不可用。

## macOS 原生实现细节

`imp`（macOS 分支）的关键内部函数：

- `check_item`：按 `id` `match` 派发——多数项查对应 framework 的 `authorizationStatus`（经 `map_standard_auth_status` / `map_speech_auth_status` / `map_notification_auth_status` 映射），FDA / folder 走探测，automation / app_management / developer_tools / 各 volumes / media_library / focus_status / local_network 直接 `ManualCheck`，`system_audio_capture` / `homekit` 直接 `NotUsed`，未知 id `NotApplicable`。
- `request_item`：按 `def.id` `match` 派发原生 prompt / automation 探测 / `_` 兜底（打开设置 + re-check）。
- `open_settings_pane`：`open` 跳 `x-apple.systempreferences:` 设置深链。
- `trigger_automation_probe`：`osascript` 触发 Apple Events 同意弹窗。
- `full_disk_access_status` / `folder_status`：文件系统探测式检测（metadata / `read_dir`），成功 = `Granted`、失败 = `ManualCheck`。

非 macOS 的 `imp`：`supported()=false`，`check_item` / `request_item` 一律返回 `NotApplicable`。

## 安全 / 红线

- **非 macOS 严禁伪造 granted**（单测 `non_macos_system_permissions_are_not_fake_granted` 锁此红线）：Windows / Linux / other 的 `imp::supported()=false`，`check_item` / `request_item` 返回 `NotApplicable`；`check_system_permissions` 在 `supported=false` 时回空 `items`；v1 包装回 `AllPermissions::default()`（全 `unknown`）。**绝不假装已授权**。
- **Tauri-only 边界**：6 条命令仅在 src-tauri `invoke_handler` 注册（经 `tauri_wrappers` 薄壳），无 HTTP 路由、不进 `COMMAND_MAP`，是 [`api-reference.md`](api-reference.md) §7.3 Desktop-only 之一。
- **TCC 绑定进程 + bundle 身份**：开发期 bare binary（`target/debug/hope-agent`）与正式 `.app` 的授权**不是同一份**——`running_from_app_bundle` 判定身份；`notifications` 在非 bundle 进程查询会抛 `NSException`（Rust 无法 catch），故**降级 `ManualCheck`**。
- **两层超时**：`request_system_permission` 的 **65s `REQUEST_TIMEOUT`** 是外层，macOS 原生回调内部 `wait_for_prompt` 是 **60s** 内层——**外层须 > 内层**，否则外层先超时、内层等待白做。查询侧 `CHECK_TIMEOUT` 为 **6s，被 28 项串行共享**（须容纳录屏探针 1.5s + notifications 2s，超时即整目录退化 `unsupported`，见 v2 查询节）。
- **`GrantedPendingRestart` 对门控等价未授权**：`legacy_state_for_status` / `legacy_files_and_folders` 映射 `not_granted`，`mac_control` 一切判定用 `== Granted`——**新增消费 `SystemPermissionStatus` 的分支须显式处理该变体**，只在文案层区分「重启生效」与「去授权」。
- **探针 token 不可退化为退出码**：见「录屏待重启探针」节；`--tcc-probe` 分派须早于 guardian/child，答复侧 `raw_probe` 永不递归。
- **重置是 owner / GUI-only，服务名白名单编译期闭合**：见「v2 重置 TCC 记录」节——不给模型工具面、不进 `ha-settings`；重置录屏须同步清探针记忆；裸二进制（无 bundle id）一律拒绝。
- **`request_mode=None`**：此类项（如 `system_audio_capture`）在 v2 请求时**不触发原生 prompt**，只走 fallback（`open_settings` / re-check）。
- **automation 永远 `ManualCheck`**：`automation_system_events` / `automation_messages` 无可靠 per-target 状态 API——`check_item` 恒回 `ManualCheck`，`request` 经 `osascript` 触发同意弹窗 + 打开设置后让用户自查。
- **探测式检测的状态语义**：`full_disk_access` / `desktop_folder` / `documents_folder` / `downloads_folder` 走文件系统探测，**失败 = `ManualCheck` 而非 `NotGranted`**（探测失败有多种原因，不能武断判成「未授权」）。
- **v1↔v2 映射须同步**：`AllPermissions`（15 字段）与 `PERMISSION_DEFS`（28 项）不一一对应（`files_and_folders` 三合一聚合、`automation` 映射等）；新增权限项须同步评估 v1 映射。
- **`PERMISSION_DEFS` 是单一真相源**：新增项须同步 `platform` 层 `check_item` / `request_item` 的 `match` 分支，否则落 `NotApplicable`。

## 与相邻子系统的关系

| 子系统 | 关系 |
|---|---|
| [Platform 抽象层](platform.md) | facade 视角：`platform.md` 列了 `system_permissions_*` facade 与 `system_permissions.rs` 文件；本文是 TCC 领域视角，两文互链 |
| [ha-mac-control（macOS 桌面控制）](macos-control.md) | **边界**：本文是底层 TCC 探测/引导，`ha-mac-control` 是上层桌面控制能力 readiness；`mac_control_permissions` 命令**复用本目录 catalog**（`systemPermissions` 字段）但走**独立命令/HTTP 路由**。`PermissionsPanel` 在两文都出现 |
| [权限引擎 v2](permission-system.md) | **同名不同物**：本子系统 ≠ 工具审批权限引擎；`~/.hope-agent/permission/`（`protected_paths` / `dangerous_commands`）属权限引擎，与 TCC 无关 |
| [API 参考](api-reference.md) | §7.3 Desktop-only 表登记全部 6 条命令；新增/改命令须与此对齐 |

## 关键文件索引

| 文件 | 角色 |
|---|---|
| [`crates/ha-core/src/permissions.rs`](../../crates/ha-core/src/permissions.rs) | 子系统根 + `PERMISSION_DEFS`（28 项）+ v2/v1 API + 枚举 + legacy 映射 + 超时包装 |
| [`crates/ha-core/src/platform/system_permissions.rs`](../../crates/ha-core/src/platform/system_permissions.rs) | 四套 `imp`（macos/windows/linux/other），macOS framework 原生检查/请求/探测 |
| [`crates/ha-core/src/platform/mod.rs`](../../crates/ha-core/src/platform/mod.rs) | facade：`system_permissions_*`（`pub(crate)`） |
| [`src/components/settings/PermissionsPanel.tsx`](../../src/components/settings/PermissionsPanel.tsx) | Settings → Permissions 面板（Tauri-only，HTTP transport 无能力） |

# 系统权限（macOS TCC）

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/permissions.rs`](../../crates/ha-core/src/permissions.rs)、[`crates/ha-core/src/platform/system_permissions.rs`](../../crates/ha-core/src/platform/system_permissions.rs)、[`crates/ha-core/src/platform/mod.rs`](../../crates/ha-core/src/platform/mod.rs)（facade）、Tauri 薄壳在 [`src-tauri/`](../../src-tauri/)、前端面板 [`src/components/settings/PermissionsPanel.tsx`](../../src/components/settings/PermissionsPanel.tsx)

## 概述

本子系统是 macOS **TCC（Transparency, Consent, and Control）系统权限的底层探测与引导层**：它维护一张 28 项权限的静态目录（`PERMISSION_DEFS`），向桌面 Settings → Permissions 面板回答两个问题——「这项系统权限当前是什么状态」「点这个按钮时该怎么把用户引导去授权」。

定位上有三条边界要先讲清楚：

- **只读探测 + 引导，不持久化、不缓存**：TCC 同意状态由 macOS 系统按进程 + bundle 身份持有，本子系统**每次实时查询**，自己不落任何库、不写 `AppConfig`/`UserConfig`。
- **Tauri-only**：能力仅经 **5 条 Tauri 命令**暴露给桌面 Shell，**无 HTTP 路由**、不进 `transport.ts` 的 `COMMAND_MAP`——HTTP/server 模式没有系统托盘进程，TCC 概念不适用。
- **非 macOS 严禁伪造 granted**：Windows / Linux / 其它平台一律收敛到 `unsupported` / `NotApplicable`，绝不假装已授权（单测红线，见安全章节）。

与上层桌面控制能力 [`ha-mac-control`](macos-control.md) 是两个子系统：本文是底层 TCC 探测/引导，`ha-mac-control` 是 macOS 桌面控制能力的 readiness 编排，复用本目录的 catalog 但走独立命令/路由（边界详见末章）。

## 模块结构

| 文件 | 职责 |
|---|---|
| [`permissions.rs`](../../crates/ha-core/src/permissions.rs) | 子系统根：`PermissionDef` 静态目录 `PERMISSION_DEFS`（28 项）、v2/v1 双层 API、数据类型枚举、v1↔v2 legacy 映射纯函数、`blocking_with_timeout` 超时包装 |
| [`platform/system_permissions.rs`](../../crates/ha-core/src/platform/system_permissions.rs) | 按 `target_os` 分 `macos` / `windows` / `linux` / `other` 四套 `mod imp`，仅 macOS 给出 framework 原生实现；非 macOS 的 `imp` 一律 `supported()=false` |
| [`platform/mod.rs`](../../crates/ha-core/src/platform/mod.rs) | facade（`pub(crate)`）：`system_permissions_supported` / `system_permissions_platform_name` / `check_system_permission_item` / `request_system_permission_item`，把上层 `permissions.rs` 与平台 `imp` 解耦 |

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

新增权限项是有契约的：**新增项须同步 `platform` 层 `check_item` / `request_item` 的 `match` 分支**，否则该 id 落 `NotApplicable`；并须考虑 v1 兼容层映射（见 v1 章节）。

### 分组 / 状态 / 请求模式枚举

- **`SystemPermissionGroup`**（snake_case 序列化，5 分组）：`ControlCapture`（控制与采集）/ `FileAccess`（文件访问）/ `PersonalData`（个人数据）/ `DeviceNetwork`（设备与网络）/ `SystemServices`（系统服务）。
- **`SystemPermissionStatus`**（7 态）：`Granted` / `NotGranted` / `NotDetermined` / `Restricted` / `ManualCheck` / `NotApplicable` / `NotUsed`。`ManualCheck` 表示「无可靠原生 API、需用户自查或探测式判定」，`NotApplicable` 表示「本平台不适用」，`NotUsed` 表示「这项被定义但当前不实际使用」。
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

1. 经 `blocking_with_timeout` 进 `spawn_blocking`，挂 **3 秒 `CHECK_TIMEOUT`**——framework 查询偶发卡顿不阻塞 UI，超时回 fallback。
2. 先看 `system_permissions_supported()`：非 macOS 直接回 `supported=false` + 空 `items`。
3. macOS 下遍历 `PERMISSION_DEFS`，逐项调 `platform::check_system_permission_item`，下沉到 `imp::check_item`。
4. `imp::check_item` 按 `id` `match` 派发到对应 framework 的 `authorizationStatus` 查询，把原生枚举映射成 `SystemPermissionStatus`。

`check_item` 的几条特殊分支：

- `automation_system_events` / `automation_messages`：**无可靠 per-target 状态 API**，永远返回 `ManualCheck`。
- `full_disk_access` / `desktop_folder` / `documents_folder` / `downloads_folder`：**无原生 API，走文件系统探测式检测**（`full_disk_access` 读 `~/Library/Safari/Bookmarks.plist` / `~/Library/Messages/chat.db`；folder 三项 `read_dir ~/Desktop` 等）——成功 = `Granted`、失败 = `ManualCheck`（注意**不是** `NotGranted`，因为探测失败可能是别的原因）。
- `system_audio_capture` / `homekit`：返回 `NotUsed`。
- `notifications`：非 bundle 进程查询会抛 `NSException`（Rust 无法 catch），故在非 bundle 进程**降级 `ManualCheck`**（见红线）。

### v2 请求：`request_system_permission`

用户在面板点某项的「请求」按钮时调 `request_system_permission`（v2 请求入口）：

1. 挂 **65 秒 `REQUEST_TIMEOUT`**（`blocking_with_timeout`）——原生授权框需要等用户操作，故远大于查询超时。
2. `find_def` 按 `id` 在 `PERMISSION_DEFS` 找到定义，下沉 `platform::request_system_permission_item` → `imp::request_item`。
3. `imp::request_item` **按 `def.id` `match` 派发**（**不是**按 `request_mode`——`request_mode` 是 catalog 给前端的元数据，平台层落地走 id-match + 一个 `_` 兜底分支）。落地分三类行为：

| 行为 | 哪些 id 走这条 |
|---|---|
| 触发原生授权框（framework `request*` 调用，内部多含 60s `wait_for_prompt` 等待用户决策；已非 `NotDetermined` 的项先 `open_settings_pane` 跳过弹框） | `screen_recording` / `input_monitoring` / `camera` / `microphone` / `location` / `contacts` / `calendar` / `reminders` / `photos` / `bluetooth` / `speech_recognition` / `notifications`（即 catalog 里 `NativePrompt` 那批） |
| `trigger_automation_probe`：`osascript` 触发一次 Apple Events 诱发「自动化」同意弹窗 → `open_settings_pane` 打开设置 → re-check（`check_item`） | `automation_system_events` / `automation_messages` |
| `_` 兜底分支：`open_settings_pane`（用 `open` 跳 `x-apple.systempreferences:` 深链）→ re-check（`check_item`） | 其余全部 id（catalog 里 `OpenSettings` 与 `None` 那批，含 `system_audio_capture`） |

automation 两项的 request 路径：osascript 触发同意 → 打开设置 → re-check（因为 check 永远 `ManualCheck`，request 后也只能让用户在设置里确认）。

### v1 兼容包装

`check_all_permissions` / `check_permission` / `request_permission` 是 v1 兼容入口，**内部全部委托 v2** 再做 legacy 映射，由四个纯函数承担 id 与状态的翻译：

- `legacy_request_id` / `legacy_status_for_id`：v1 id ↔ v2 id 与状态的双向映射。
- `legacy_files_and_folders`：v1 的 `files_and_folders` 字段由 v2 的 `desktop_folder` / `documents_folder` / `downloads_folder` **三项聚合**而成（三项全 `Granted` → `granted`；任一 `NotGranted`/`NotDetermined`/`Restricted` → `not_granted`；否则 `unknown`）。`legacy_request_id("files_and_folders")` 则映射到 `desktop_folder` 触发请求。
- `legacy_state_for_status`：v2 `SystemPermissionStatus` → v1 字符串态。

`AllPermissions`（v1，15 字段）与 `PERMISSION_DEFS`（v2，28 项）**不一一对应**——典型如 `automation` → `automation_system_events` 的映射、`files_and_folders` 的三合一聚合。**新增权限项时须同步考虑 v1 映射是否需要更新**。

## 持久化

本子系统**不落任何库、不占任何配置字段、不写 `~/.hope-agent`**：

- **无 DB 表**——TCC 状态实时查询，不缓存。
- **无 config 字段**——不进 `AppConfig` / `UserConfig`，每次面板加载现查。
- **无 `~/.hope-agent` 文件**——注意 `paths.rs::permission_dir`（`~/.hope-agent/permission/`）持有的是**权限引擎 v2**（`protected_paths` / `dangerous_commands`，见 [`permission-system.md`](permission-system.md)）的列表，**与本子系统无关**，两者只是名字里都有「permission」。
- **TCC 同意状态由 macOS TCC 数据库按进程 + bundle 身份持有**，属系统外部状态，非本仓库管理。

## 对外接口面

### Tauri 命令（5 条，Desktop-only）

5 条命令经 Tauri 薄壳（`tauri_wrappers`）注册到 `invoke_handler`，**无对应 HTTP 路由**：

| 命令 | 层 | 作用 |
|---|---|---|
| `check_system_permissions` | v2 | 查询全部 28 项状态，回 `SystemPermissionsResponse` |
| `request_system_permission` | v2 | 请求单项授权（按 `def.id` 派发） |
| `check_all_permissions` | v1 | 兼容聚合查询，回 `AllPermissions` |
| `check_permission` | v1 | 兼容单项查询 |
| `request_permission` | v1 | 兼容单项请求 |

这 5 条全部登记在 [`api-reference.md`](api-reference.md) §7.3 的 **Desktop-only** 表，计入合法的 13 条 Tauri-only 差集。

### HTTP 路由

**无**——不进 `build_router_with_cors`，不进 `transport.ts` 的 `COMMAND_MAP`。HTTP transport 对这 5 条命令没有对应实现。

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
- **Tauri-only 边界**：5 条命令仅在 src-tauri `invoke_handler` 注册（经 `tauri_wrappers` 薄壳），无 HTTP 路由、不进 `COMMAND_MAP`，是 [`api-reference.md`](api-reference.md) §7.3 Desktop-only 之一。
- **TCC 绑定进程 + bundle 身份**：开发期 bare binary（`target/debug/hope-agent`）与正式 `.app` 的授权**不是同一份**——`running_from_app_bundle` 判定身份；`notifications` 在非 bundle 进程查询会抛 `NSException`（Rust 无法 catch），故**降级 `ManualCheck`**。
- **两层超时**：`request_system_permission` 的 **65s `REQUEST_TIMEOUT`** 是外层，macOS 原生回调内部 `wait_for_prompt` 是 **60s** 内层——**外层须 > 内层**，否则外层先超时、内层等待白做。查询侧 `CHECK_TIMEOUT` 为 3s。
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
| [API 参考](api-reference.md) | §7.3 Desktop-only 表登记全部 5 条命令；新增/改命令须与此对齐 |

## 关键文件索引

| 文件 | 角色 |
|---|---|
| [`crates/ha-core/src/permissions.rs`](../../crates/ha-core/src/permissions.rs) | 子系统根 + `PERMISSION_DEFS`（28 项）+ v2/v1 API + 枚举 + legacy 映射 + 超时包装 |
| [`crates/ha-core/src/platform/system_permissions.rs`](../../crates/ha-core/src/platform/system_permissions.rs) | 四套 `imp`（macos/windows/linux/other），macOS framework 原生检查/请求/探测 |
| [`crates/ha-core/src/platform/mod.rs`](../../crates/ha-core/src/platform/mod.rs) | facade：`system_permissions_*`（`pub(crate)`） |
| [`src/components/settings/PermissionsPanel.tsx`](../../src/components/settings/PermissionsPanel.tsx) | Settings → Permissions 面板（Tauri-only，HTTP transport 无能力） |

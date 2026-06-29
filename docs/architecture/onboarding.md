# 首次启动向导（Onboarding）

> 返回 [文档索引](../README.md) | 关联源码：[`crates/ha-core/src/onboarding/`](../../crates/ha-core/src/onboarding/)、[`src-tauri/src/commands/onboarding.rs`](../../src-tauri/src/commands/onboarding.rs)、[`crates/ha-server/src/routes/onboarding.rs`](../../crates/ha-server/src/routes/onboarding.rs)、GUI 在 [`src/components/onboarding/`](../../src/components/onboarding/)、CLI 在 [`src-tauri/src/cli_onboarding/`](../../src-tauri/src/cli_onboarding/)

## 概述

首次启动向导是 App 第一次运行时引导用户做完一遍最小配置（语言 / 用户画像 / 人格 / 审批安全 / 技能 / 搜索 / Server / 远程模式）的多步骤流程。它的设计中心是**双前端、单核心**：

- **GUI（React）** 在 `src/components/onboarding/` —— 桌面 Tauri 与 Web GUI 共用同一套向导 UI
- **CLI 向导** 在 `src-tauri/src/cli_onboarding/` —— `hope-agent setup` 命令行交互式配置（编排见 [CLI 子系统](cli.md)）
- **核心** 在 `ha-core::onboarding`（零 Tauri 依赖）—— 进度状态机（`state`）、落地写入（`apply`）、人格预设（`presets`）三块逻辑两端共用

两套前端**写同一份 `OnboardingState`**、调用**同一组 `apply_*` 核心 helper**，从而保证 GUI 与 CLI 语义零偏差。核心的关键取向是：**配置落到各数据自然归属的位置**，而不是统一塞进一个 onboarding 配置块——语言写 `config.json` 与 `user.json`、画像写 `user.json`、人格写默认 Agent 的 `agent.json`、审批写 permission 配置、技能/搜索/Server 写 `config.json`。向导进度本身（完成版本 / 草稿 / 跳过集）则作为 `AppConfig.onboarding` 子对象单独持久化。

## 模块结构

核心全在 `crates/ha-core/src/onboarding/`：

| 文件 | 职责 |
|---|---|
| `mod.rs` | 子系统根、公共 API 再导出 |
| `state.rs` | 向导进度状态机：`get_state` 读入口（套 legacy 推断）+ `save_draft` / `mark_completed` / `mark_skipped` / `reset` 写入 + `infer_legacy_completed` 纯函数 |
| `apply.rs` | 各步骤落地写入 helper（`apply_language` / `apply_profile` / `apply_personality_preset` / `apply_safety` / `apply_skills` / `apply_web_search` / `apply_server` / `apply_remote_mode` / `generate_api_key`）+ `merge_optional` 助手 |
| `presets.rs` | 人格预设 `PersonalityPreset` 枚举 + `personality_preset_by_id` string-id 解析 |

薄壳两端：

| 平面 | 入口 |
|---|---|
| Tauri 命令 | [`src-tauri/src/commands/onboarding.rs`](../../src-tauri/src/commands/onboarding.rs)（13 命令） |
| HTTP 路由 | [`crates/ha-server/src/routes/onboarding.rs`](../../crates/ha-server/src/routes/onboarding.rs)（`/api/onboarding/*` + `/api/server/*`） |
| GUI | [`src/components/onboarding/`](../../src/components/onboarding/)（`types.ts` 步骤定义 + `useOnboarding.ts` + `steps/`） |
| CLI | [`src-tauri/src/cli_onboarding/`](../../src-tauri/src/cli_onboarding/)（`wizard.rs` 编排 + `steps/` 各步） |

## 核心数据结构

### `OnboardingState`（向导进度）

`config::OnboardingState` 是 `AppConfig.onboarding` 子对象，以 serde camelCase 持久化到 `config.json`。字段语义：

| 字段 | 语义 |
|---|---|
| `completed_version` | 用户完成时的向导版本号；`>= CURRENT_ONBOARDING_VERSION` 即视为「已引导」、不再弹向导 |
| `completed_at` | 完成时间戳 |
| `skipped_steps` | 用户主动跳过的步骤集合（记录而非阻塞） |
| `draft` | 前端拥有的**不透明 JSON 草稿**（后端原样存取、不解释结构） |
| `draft_step` | 草稿停留的步骤标识 |
| `ever_completed` | 「是否曾经完整走过一遍」的布尔锚，`reset()` 后仍保持 `true`（防被 legacy 推断短路，见红线） |

### `CURRENT_ONBOARDING_VERSION`（向导版本常量）

`config::CURRENT_ONBOARDING_VERSION`（当前值 **1**）是「向导是否需要让存量用户重走」的版本闸：`completed_version >= CURRENT_ONBOARDING_VERSION` 即判已引导。**bump 规则（红线）**：仅在**必填步骤新增、需要让存量用户重走**时才 bump；新增**可选步骤**（如 v0.2 系列加的 search-provider 步）**不 bump**，存量用户不被打断。

### `PersonalityPreset`（人格预设）

`onboarding::presets::PersonalityPreset` 枚举 4 个变体 `Default` / `Engineer` / `Creative` / `Companion`：

- `to_config()` 产出 `PersonalityConfig`（写入默认 Agent 的 `agent.json` 的 personality 段）
- `id()` 给出稳定 string id（**单一来源**），`personality_preset_by_id(id)` 反查、未知返 `None` 供 caller 报校验错

### apply 步骤输入

`apply.rs` 各步骤对应的输入结构：

| 结构 | 步骤 | 落地 |
|---|---|---|
| `ProfileStepInput` | Step3 用户画像 | `name` / `timezone` / `ai_experience` / `response_style`（均可选），写 `user_config` |
| `SafetyStepInput` | Step5 审批安全 | `approvals_enabled: bool`，写 permission 配置 |
| `ServerStepInput` | Server 步 | `bind_addr` / `api_key`（`Option`），`None` 保持现状、`Some("")` 清空 |
| `RemoteModeInput` | 远程模式 | `url` / `api_key`，写 `user_config` 的 `server_mode=remote`（**仅 CLI mode 步接线**） |

### `OnboardingStepKey` / `ONBOARDING_STEPS`（前端步骤定义）

`src/components/onboarding/types.ts` 维护前端 **12 步有序列表** `ONBOARDING_STEPS` + `stepsForMode` 远程短路：远程模式只保留 `welcome` / `import-openclaw` / `mode` 三步，其余早退（见远程模式短路）。CLI 端步骤模块在 `cli_onboarding/steps/`（language / profile / personality / safety / skills / search_provider / server / mode / channels / provider / import_openclaw / summary），**两边写同一 `OnboardingState`**。

## 数据流与状态机

### 读入口 `get_state`（唯一）

`onboarding::state::get_state` 是 GUI 与 CLI **唯一的进度读入口**：读出 `OnboardingState` 后，途中套 `infer_legacy_completed` 做 legacy 升级推断。**任何观察 `completed_version` 的 caller 必须经 `get_state`，不得裸读 `cfg.onboarding`**——否则会漏掉 legacy 推断。

### legacy 升级推断 `infer_legacy_completed`

`infer_legacy_completed` 是纯函数，解决「老版本用户升级到带向导的版本时不该被当成首次启动重弹向导」的问题。判定条件（全部满足才视为「已在 v1 完成」）：

```
completed_version == 0  &&  !ever_completed  &&  有 provider  &&  无 draft
```

启发式直觉是：一个有 provider 配置、从未走过向导、也没留草稿的用户，必然是从无向导的旧版本升级来的，应视为已引导。

**关键红线**：推断出的「已完成」**绝不写回 config**（`state.rs` 注释明示），只在内存读取时生效——避免产生让人困惑的 autosave 快照。

### 写入 helper

进度写入分四类，全在 `state.rs`：

| helper | 行为 |
|---|---|
| `save_draft` | 写前端草稿 JSON + `draft_step` |
| `mark_completed` | 置 `completed_version = CURRENT_ONBOARDING_VERSION` + `completed_at` + `ever_completed = true` |
| `mark_skipped` | 记录跳过步骤进 `skipped_steps` |
| `reset` | 清进度让用户重跑向导，但**钉死 `ever_completed = true`**（红线，见下） |

### apply 落地写入

各 `apply_*` helper 把向导步骤的结果写到数据的**自然归属位置**，而非单一 config 块。所有写入自带 backup scope 标签 `onboarding/<step>`，可按步骤精细回滚：

| helper | 步骤 | 落地 |
|---|---|---|
| `apply_language` | 语言 | 同时写 `config.language` 与 `user.language`（兼容旧读路径） |
| `apply_profile` | 用户画像 | 经 `merge_optional` 写 `user_config` 的 name/timezone/ai_experience/response_style |
| `apply_personality_preset` | 人格 | `ensure_default_agent` 后读写 `DEFAULT_AGENT_ID` 的 `agent.json`，**仅改 personality 段** |
| `apply_safety` | 审批安全 | 见下「审批/安全步骤语义」 |
| `apply_skills` | 技能 | 整表覆盖 `config.disabled_skills` |
| `apply_web_search` | 搜索 | 经 `web_search::backfill_providers` 后写 `config.web_search`（CLI search_provider 步用） |
| `apply_server` | Server | bind_addr / api_key 写 `config.server` |
| `apply_remote_mode` | 远程模式 | 写 `user_config` server_mode/remote_url（仅 CLI mode 步） |
| `generate_api_key` | — | 产 `hope_<uuid_no_dash>` 格式 key（GUI/CLI/test 共用） |

### `merge_optional` 偏向保数据

`apply_profile` 通过 `merge_optional` 写画像字段：**`None` 与空串都视为「保留现值」**。这意味着向导**无法清空已有画像字段**（清空只能去 Settings → Profile），是有意设计、偏向保数据。

### 审批/安全步骤语义（DEADLOCK-3）

`apply_safety` 的 `approvals_enabled` 翻译为权限引擎语义，这里有一段重要的语义修正：

- **关闭审批（`approvals_enabled = false`）= 写 `global_yolo = true`**（global YOLO，诚实实现）。早期实现是 `approval_timeout_enabled = false` + `Proceed`，那会让每个 Ask **永久挂死**（DEADLOCK-3）——honest 的「不要审批」就是 YOLO。
- **重新启用审批（`true`）= 同时清 `global_yolo = false` + 修 `approval_timeout_action` / `approval_timeout_secs`**。**必须清 YOLO**，否则权限引擎继续旁路所有审批门。

权限引擎与审批超时语义详见 [权限系统](permission-system.md)。

### 远程模式短路

「远程模式」让客户端连到一个远端 `hope-agent server` 而非本机后端，写 `user_config` 的 `server_mode` / `remote_server_url` / `remote_api_key`：

- **CLI** 端在 `cli_onboarding/steps/mode` 步显式接线 `apply_remote_mode`
- **GUI** 端靠前端 `stepsForMode` 早退——选远程模式后只走 welcome / import-openclaw / mode 三步，不再调本地配置类 apply

注意 `apply_remote_mode` / `apply_web_search` **没有对应的 Tauri/HTTP onboarding 端点**——它们仅被 CLI 的 `cli_onboarding::steps::{mode, search_provider}` 调用；GUI 远程模式靠 stepsForMode 早退，GUI 搜索 provider 走既有 [Settings 端点](provider-system.md)。

## 持久化

| 位置 | 写入内容 |
|---|---|
| `~/.hope-agent/config.json`（`paths::config_path`） | `AppConfig.onboarding`（`OnboardingState`）+ `apply_language` 的 `config.language` + `apply_safety` 的 `permission.*` + `apply_skills` 的 `disabled_skills` + `apply_server` 的 `server.{bind_addr,api_key}` + `apply_web_search` 的 `web_search` |
| `~/.hope-agent/user.json`（`paths::user_config_path`） | `apply_language` 的 `user.language` + `apply_profile` 的 name/timezone/ai_experience/response_style + `apply_remote_mode` 的 server_mode/remote_server_url/remote_api_key |
| `agents/<DEFAULT_AGENT_ID>/agent.json` | `apply_personality_preset` 写入的 personality 段 |

**autosave 备份**：每次 `apply_*` 与 state 写入都经 `backup::scope_save_reason("onboarding", <step>)` 打标签，可按 `onboarding/<step>` 回滚（`<step>` 取值如 language / profile / safety / skills / search-provider / server / mode / draft / complete / skip / reset）。

## 对外接口面

向导端点是 **owner-only**：无 session 参数，纯本机 / HTTP Bearer 信任的 owner 平面薄壳（定位见 [后端分层](backend-separation.md)）。HTTP 与 Tauri **共用同一 ha-core 核心**，语义零偏差；错误统一在边界 stringify。

### Tauri 命令（13）

```
get_onboarding_state          save_onboarding_draft         mark_onboarding_completed
mark_onboarding_skipped       reset_onboarding              apply_onboarding_language
apply_onboarding_profile      apply_personality_preset_cmd  apply_onboarding_safety
apply_onboarding_skills       apply_onboarding_server       generate_api_key
list_local_ips
```

### HTTP 路由

| 路由 | 对应核心 |
|---|---|
| `GET  /api/onboarding/state` | `get_state` |
| `POST /api/onboarding/draft` | `save_draft` |
| `POST /api/onboarding/complete` | `mark_completed` |
| `POST /api/onboarding/skip` | `mark_skipped` |
| `POST /api/onboarding/reset` | `reset` |
| `POST /api/onboarding/language` | `apply_language` |
| `POST /api/onboarding/profile` | `apply_profile` |
| `POST /api/onboarding/personality-preset` | `apply_personality_preset` |
| `POST /api/onboarding/safety` | `apply_safety` |
| `POST /api/onboarding/skills` | `apply_skills` |
| `POST /api/onboarding/server` | `apply_server` |
| `POST /api/server/generate-api-key` | `apply::generate_api_key`（核心 helper） |
| `GET  /api/server/local-ips` | `banner::local_ipv4_addresses`（薄壳 helper，非 ha-core onboarding） |

Tauri ↔ HTTP 对齐的单一真相源见 [api-reference.md](api-reference.md)（First-run onboarding wizard 表，13 条已登记）。

## 事件

- **`config:changed`** —— **仅经 `save_config` 落盘的 `apply_*` 才 emit**：`apply_language` / `apply_safety` / `apply_skills` / `apply_web_search` / `apply_server` + 同走 `save_config` 的 state 写入（前端据此刷新缓存配置快照）。
- **三条不发 `config:changed` 的 apply（红线）**：`apply_profile` / `apply_remote_mode` 写 `user.json`（`save_user_config_to_disk`）、`apply_personality_preset` 写 `agents/ha-main/agent.json`（`save_agent_config`）——这三条不经 `save_config`，**不会 emit `config:changed`**。依赖该事件刷新缓存的前端，对「资料 / 远程模式 / 人格预设」三类更新不会自动收到通知，须各自走 user-config / agent 侧刷新路径。

## 安全 / 红线

- **写路径三分、不全是 `save_config`，更不是 `mutate_config`**：`apply_language` / `apply_safety` / `apply_skills` / `apply_web_search` / `apply_server` 与 `state.rs` 走 `load_config()` + `save_config()`（emit `config:changed` + 落 autosave，但**不具 `mutate_config` 的并发 lost-update 防护**）；`apply_profile` / `apply_remote_mode` 走 `save_user_config_to_disk`（写 `user.json`）；`apply_personality_preset` 走 `save_agent_config`（写主 Agent `agent.json`）。**后两类不经 `save_config`、不 emit `config:changed`**。改这里别照搬误以为已走 config contract、也别假设每条 apply 都会发事件（详见 [配置系统](config-system.md)）。
- **legacy 推断绝不写回 config**：`infer_legacy_completed` 的推断值只在 `get_state` 读取时生效，永不持久化（避免困惑的 autosave 快照）。观察 `completed_version` 必须经 `get_state`，禁止裸读 `cfg.onboarding`。
- **`reset()` 必须保持 `ever_completed = true`**：否则有 provider 的用户显式重跑向导时，会被 `infer_legacy_completed` 当成 legacy 直接跳过——`reset` 与 legacy 推断的冲突点正在这里。
- **`apply_safety` 的 no-approvals 语义是 global YOLO**（不是旧的 `approval_timeout_enabled=false` + Proceed，那会让每个 Ask 永久挂死）；re-enable approvals 必须同时清 `global_yolo=false`，否则引擎继续旁路审批。
- **`merge_optional` 把 None 与空串都视为「保留现值」**：向导无法清空画像字段（清空只能去 Settings → Profile）。这是有意设计，别改成「空串=清空」。
- **`apply_personality_preset` 只写 `DEFAULT_AGENT_ID` 的 `agent.json`**：用户后建的其它 Agent 独立管理；写前必 `ensure_default_agent`（否则模板尚未落盘，写入会失败）。
- **`draft` 是前端拥有的不透明 JSON**：后端原样存取、不解释结构。`CURRENT_ONBOARDING_VERSION` 仅在必填步骤新增、需让存量用户重走时才 bump（可选步骤新增不 bump，存量用户不被打断）。
- **owner-only**：所有 Tauri / HTTP onboarding 端点都是无 session 参数的 owner 平面薄壳（本机 Tauri IPC 信任 / HTTP 走 Bearer），错误在边界 stringify；HTTP 与 Tauri 共用同一核心保证语义零偏差。
- **Provider 写入红线**：向导里的 provider 配置仍受 [Provider 写入 contract](provider-system.md) 约束——必须走 `provider/crud.rs` helper（如 `add_and_activate_provider` 标注 onboarding 用），禁止绕过自写 `providers.push` / `active_model`。

## 与相邻子系统的关系

| 子系统 | 关系 |
|---|---|
| [配置系统](config-system.md) | 进度落 `AppConfig.onboarding`；写经 `load_config` + `save_config`（既存路径，非 `mutate_config`） |
| [后端分层](backend-separation.md) | onboarding 列为 owner 平面薄壳；`/api/onboarding/*` 端点清单在此登记 |
| [CLI](cli.md) | CLI 向导编排 `cli_onboarding/wizard.rs` + `steps/`；`setup` / `--reset`、`login` 复用 OAuth |
| [启动序列](process-model.md) | 启动序列含 `cli_onboarding` wizard 与 onboarding hard-fail 退出码 2 节点 |
| [Provider 系统](provider-system.md) | provider 写入禁绕 `crud.rs`，含 onboarding 路径；GUI 搜索 provider 走既有 Settings 端点 |
| [权限系统](permission-system.md) | `apply_safety` 翻译 `approvals_enabled` 为 `global_yolo` + approval timeout 语义 |
| [Agent 解析链](backend-separation.md) | `apply_personality_preset` 写 `DEFAULT_AGENT_ID` 的 `agent.json` personality 段 |
| [API Reference](api-reference.md) | First-run onboarding wizard 表登记 13 条 Tauri ↔ HTTP 对齐 |

## 关键文件索引

| 文件 | 角色 |
|---|---|
| [`crates/ha-core/src/onboarding/mod.rs`](../../crates/ha-core/src/onboarding/mod.rs) | 子系统根 + 公共 API 再导出 |
| [`crates/ha-core/src/onboarding/state.rs`](../../crates/ha-core/src/onboarding/state.rs) | 进度状态机 + `get_state` 读入口 + `infer_legacy_completed` |
| [`crates/ha-core/src/onboarding/apply.rs`](../../crates/ha-core/src/onboarding/apply.rs) | 各步骤落地写入 + `merge_optional` + `generate_api_key` |
| [`crates/ha-core/src/onboarding/presets.rs`](../../crates/ha-core/src/onboarding/presets.rs) | 4 个 `PersonalityPreset` + `personality_preset_by_id` |
| [`src-tauri/src/commands/onboarding.rs`](../../src-tauri/src/commands/onboarding.rs) | 13 Tauri 命令（owner-only 薄壳） |
| [`crates/ha-server/src/routes/onboarding.rs`](../../crates/ha-server/src/routes/onboarding.rs) | `/api/onboarding/*` + `/api/server/*` HTTP 路由 |
| [`src/components/onboarding/types.ts`](../../src/components/onboarding/types.ts) | 前端 12 步定义 + `stepsForMode` 远程短路 |
| [`src/components/onboarding/useOnboarding.ts`](../../src/components/onboarding/useOnboarding.ts) | GUI 向导状态 hook |
| [`src-tauri/src/cli_onboarding/wizard.rs`](../../src-tauri/src/cli_onboarding/wizard.rs) | CLI 向导编排 |
| [`src-tauri/src/cli_onboarding/steps/`](../../src-tauri/src/cli_onboarding/steps/) | CLI 各步骤（language / profile / personality / safety / skills / search_provider / server / mode / …） |

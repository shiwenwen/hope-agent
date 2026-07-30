# 前后端分离架构

> 返回 [文档索引](../README.md) | 关联源码：`Cargo.toml`, `crates/ha-base/`, `crates/ha-core/`, `crates/ha-server/`, `src-tauri/`

## 设计目标

将 Hope Agent 从 Tauri 单体应用重构为分层架构（基础设施 / 核心库 / HTTP 服务 / 桌面壳），实现：

1. **核心逻辑框架无关** — `ha-core` 零 Tauri 依赖，可被任何 Rust 程序引用
2. **多入口运行** — 桌面 GUI、HTTP 守护进程、CLI stdio 三种模式共享同一核心
3. **前端双模式** — 同一 React 前端可在 Tauri WebView 和独立浏览器中运行

## Crate 依赖关系

```mermaid
graph TD
    subgraph Workspace
        HA_TAURI["src-tauri<br/>(Tauri 桌面壳)<br/>tauri 2.10 + 7 plugins"]
        HA_SERVER["ha-server<br/>(HTTP/WS 服务)<br/>axum 0.8"]
        HA_FEAT["特征 crate<br/>ha-acp · ha-browser · ha-design · ha-mac<br/>ha-mcp · ha-updater · ha-vcs · ha-weather（阶段 3-4 逐个迁出）"]
        HA_CORE["ha-core<br/>(核心业务逻辑)<br/>零 Tauri 依赖"]
        HA_SCHEMA["ha-config-schema<br/>(AppConfig wire 类型闭包)<br/>纯数据定义 · 零行为逻辑"]
        HA_BASE["ha-base<br/>(基础设施底层)<br/>paths · logging · platform<br/>security · permissions · terminal<br/>不依赖任何 ha-* 业务 crate"]
    end

    HA_TAURI -->|"依赖"| HA_SERVER
    HA_TAURI -->|"依赖 + wire()"| HA_FEAT
    HA_TAURI -->|"依赖"| HA_CORE
    HA_SERVER -->|"依赖 + wire()"| HA_FEAT
    HA_SERVER -->|"依赖"| HA_CORE
    HA_FEAT -->|"依赖"| HA_CORE
    HA_CORE -->|"依赖"| HA_SCHEMA
    HA_CORE -->|"依赖"| HA_BASE
    HA_SCHEMA -->|"依赖"| HA_BASE

```

**三条铁律**：

1. `ha-core` / `ha-config-schema` / `ha-base` 与全部特征 crate（`ha-updater` 起）的 `Cargo.toml` 禁止出现 `tauri` 或 Tauri 插件依赖。
2. `ha-base` 禁止依赖任何 `ha-*` 业务 crate。需要上层数据时**留注册钩子**，
   由 `ha-core` 在 `init_runtime()` 早期注入（见下方 ha-base 小节）。
3. `ha-config-schema` 只放数据定义（类型 + 自包含 impl + serde helper），
   只准依赖 ha-base 与叶子 crate；任何需要子系统服务的行为
   （`cached_config` / `mutate_config` / redact / validate / SSRF）留在 ha-core
   （见下方 ha-config-schema 小节）。

## 各 Crate 职责

### ha-base（基础设施底层）

依赖图最底层。这里只放**与业务无关的原语**：路径解析、日志、跨平台 shim、
安全守卫、系统权限、内嵌终端、阻塞 IO helper、TTL 缓存、EventBus trait。

| 职责 | 说明 |
|------|------|
| 路径单一来源 | `paths.rs` — 所有 `~/.hope-agent/` 下的路径入口 |
| 日志 | `logging/` — `AppLogger` / `LogDB` / `app_info!` 系列宏 + `APP_LOGGER` / `LOG_DB` 全局 |
| 跨平台原语 | `platform/` — 进程树终止、代理探测、原子替换、keep-awake、WSL |
| 安全守卫 | `security/` — SSRF 检查、Dangerous Mode 判定、HTTP 流式读取上限 |
| 系统权限 | `permissions.rs` — macOS/Windows 系统权限目录与请求 |
| 运行模式与版本 | `runtime_role.rs` — `RUNTIME_ROLE` / `APP_VERSION` 全局 + `is_desktop()` / `is_acp()` / `app_version()`（角色由装配层 `init_runtime` 经 `set_runtime_role` 写入） |
| 进程簿记 | `process_registry.rs` — 后台进程会话表；退出/输出通知经 `register_notifiers` 由上层注入 |
| 其它原语 | `blocking.rs` / `ttl_cache.rs` / `runtime_lock.rs` / `event_bus.rs` / `terminal.rs` / `crash_journal.rs` |

**反向依赖靠注册钩子解决**（ha-base 不能 `use` `AppConfig`）：

| 钩子 | 未注册时的行为 | 冲突（重复注册）时 |
|------|---------------|------------------|
| `paths::register_plans_dir_source` | 回落 `~/.hope-agent/plans/` | 记 `app_error!`，继续启动 |
| `process_registry::register_notifiers` | 不发进程退出/输出通知（簿记不受影响） | 记 `app_error!`，继续启动 |
| `security::dangerous::register_config_flag_source` | 返回 `false`（**fail-closed**，Dangerous Mode 配置来源视为未开启） | **panic**——它控制全局审批跳过，来源被顶替不可接受 |

**`ha-core` 对下游完全透明**：`lib.rs` 用 `pub use ha_base::*` + `#[macro_use]
extern crate ha_base` 全量再导出，所以 ha-core 内部的 `crate::paths::…` 与下游的
`ha_core::platform::…` / `ha_core::app_warn` **路径全部不变**。

> `app_info!` 展开为 `$crate::get_logger()`，`$crate` 解析到**定义宏的 crate**，
> 所以 `APP_LOGGER` / `LOG_DB` 及其 `require_*` 访问器必须与宏同住 ha-base；
> `globals.rs` 改为再导出以保持 `crate::globals::APP_LOGGER` 等既有路径。

### ha-config-schema（配置 wire 类型）

`AppConfig` 及其全部传递类型闭包（22 个子系统的 `*Config` / 枚举 / serde
helper / 自包含 impl）。**模块路径镜像 ha-core**（`memory` / `config` /
`tools::web_search` / `knowledge::maintenance` …），根部 `pub use ha_base::*`
与 ha-core 同构——因此被搬代码里 `crate::memory::X`、`crate::default_true`、
`crate::security::ssrf::SsrfConfig` 这类内部引用**原样成立**，`AppConfig::default()`
的四十余处 `crate::<mod>::XxxConfig::default()` 路径搬入后自然消解。

**再导出契约**：ha-core 各子系统在**原定义文件**里 `pub use
ha_config_schema::<mod>::{…};` 顶替被搬定义，既有 re-export 链
（`memory/mod.rs` 的 `pub use types::*` 等）不动，所以全仓
`crate::config::AppConfig` / `ha_core::mcp::McpServerConfig` 等路径零改动。
`config/mod.rs` 用 glob `pub use ha_config_schema::config::*;`。

**行为边界**（新增字段时最常踩的线）：

| 归属 | 内容 |
|------|------|
| schema | 类型定义、`Default`、clamp/effective 等只碰自身字段的 impl、serde default helper、`DEFAULT_AGENT_ID` 常量 |
| ha-core | `cached_config` / `mutate_config`（persistence）、redact 脱敏接线、`validate_server_config` / `check_ssrf` / `ssrf_policy_for` 等子系统自由函数、`HooksConfigExt` / `MaintenanceTasksExt` 扩展 trait（方法引用未下沉类型时的 coherence 出口） |

> 例外记录：`context_compact` 的 `default_tool_policies` 在 schema 侧用工具名
> 字面量（wire 格式 key，schema 不能反向依赖 ha-core 的 `TOOL_*` 常量），由
> ha-core `default_tool_policies_match_tool_name_constants` 测试锁死一致性；
> config 模块的纯类型测试（默认值 / 钳制 / serde 兼容）随类型住在 schema，
> `cargo test -p ha-config-schema` 已进 pre-push / CI 门禁。

### ha-core（核心库）

| 职责 | 说明 |
|------|------|
| 业务逻辑 | Agent、Chat Engine、Tool Loop、Plan Mode、Memory、Subagent、MCP、Project、Local LLM 等全部核心能力 |
| 数据存储 | SessionDB、MemoryDB、LogDB、CronDB、ChannelDB、ProjectDB、AsyncJobDB、LocalModelJobDB、RecapDB — 全部 SQLite |
| 状态管理 | `AppState` + `OnceLock` 全局单例 + accessor 函数 |
| 事件系统 | `EventBus` trait — 替代原 Tauri `APP_HANDLE.emit()` |
| 接入层 | 12 个 IM 渠道插件、ACP stdio 协议、MCP 客户端（4 种 transport） |
| 基础设施 | Guardian 保活、Self-Diagnosis（路径 / 日志 / 平台 / 安全 / runtime_lock 等原语已下沉 ha-base）|

**主要模块**（精确清单以 `ls crates/ha-core/src/` 为准，整体 ~50+ 顶层模块）：

```
agent/             AssistantAgent + 4 种 Provider + Side Query
chat_engine/       ChatEngineParams → EventSink 流式输出
memory/            SQLite + FTS5 + vec0 向量 + 多种 Embedding（含 dreaming）
tools/             内置工具集 + 并发/串行执行引擎（具体工具数量以 tools/ 子模块为准）
channel/           12 个 IM 插件（telegram / wechat / slack / feishu / discord / qqbot /
                   irc / signal / imessage / whatsapp / googlechat / line）+ Worker 分发 + 媒体管道
plan/              5 态状态机（plan 设计契约 + task 进度真相）+ 步骤追踪
subagent/          spawn + inject + Mailbox + 深度控制
skills/            SKILL.md 发现 + 懒加载 + Fork 模式 + draft 审核
provider/          多模板 + Failover Chain + Proxy + crud helper
                   （所有 provider/active_model 写入必须走 provider/crud.rs，详见下文）
context_compact/   5 层渐进式压缩 + API-Round 分组
session/           会话 + 消息持久化 + FTS5 搜索
project/           Project 容器（工作目录即真实文件，无独立 project_files；无反向 channel 认领）
mcp/               MCP 客户端（stdio / Streamable HTTP / SSE / WebSocket）
cron/              定时任务 + Agent 执行
local_llm/         Ollama 集成 + 模型目录 + 硬件预算
local_model_jobs/  本地模型后台任务（安装 / pull / 加载）
async_jobs/        异步工具后台执行 + 重启回放
team/              Agent Team 模板 + 实例 + 任务
recap/             /recap 深度复盘 + 11 个并行 AI 章节
dashboard/         Insights + Learning Tracker
awareness/         跨会话行为感知 suffix
config/            cached_config / mutate_config（详见下文）
globals.rs         OnceLock 全局 + AppState（logger / LogDB 全局已下沉 ha-base 并再导出）
guardian.rs        进程监护 + 指数退避 + 自修复
...
```

### 特征 crate（ha-acp / ha-browser / ha-design / ha-mac / ha-mcp / ha-updater / ha-vcs / ha-weather，阶段 3 起逐个迁出）

共同契约（对全部特征 crate 生效）：

- **依赖方向**：特征 crate → ha-core（借用 tools registry / config / EventBus
  等 kernel 服务）；**特征之间允许单向依赖**（无环即可——现有一条：
  ha-design → ha-browser，render_native 复用 Chrome PDF/截图 backend）；
  ha-core **不知道**任何特征 crate 存在——kernel 需要特征
  行为的点全部倒转为注册钩子（工具分发条目 / 启动任务 / 专用 fn-pointer 钩子）。
- **装配**：每个调 `init_runtime` 的二进制在 init 前调 `<crate>::wire()`
  （幂等）。工具 `ToolDefinition` 仍在 ha-core（schema 目录阶段 4 才动），
  漏 wire 由 `registry_freeze` warn 兜底（有 definition 无 handler）。
- **装配任务三档**：`register_init_task(fn)`（`init_runtime` 主体内消费，
  **所有 role** 执行、tokio runtime 不保证存在——无条件子系统装配，如 acp
  的 SessionManager 创建）；`register_startup_task(StartupStage, fn)` 两档
  ——`PrimaryOnly`（原 primary-gated 块，如 updater 自动更新循环、acp
  backend 自动发现）与 `EveryProcess`（primary 门外，如 weather 桌面刷新
  ——desktop 判定在任务闭包内）。startup 两档消费点都在
  `start_background_tasks`，时序与各特征迁出前逐位一致。

各 crate：

- **ha-updater**（自升级）：manifest 检查 / 签名校验（Minisign 信任根
  `keys.rs`，CODEOWNERS 强制评审）/ 下载续传 / atomic swap / 服务重启 /
  `app_update` 工具（`tool.rs`）。红线见 [self-update](self-update.md)
  （验签 / pubkey 双处一致 / 用户确认 / 零 Tauri——桌面路径经
  `UpdaterBridge` 反向注册）。
- **ha-weather**（天气）：Open-Meteo 取数 / 缓存 / 桌面后台刷新 /
  `get_weather` 工具 / system prompt 天气段（经
  `system_prompt::register_weather_prompt_source` 钩子）/ settings 天气 key
  热刷新（经 `tools::register_weather_settings_refresh` 钩子）。
- **ha-mac**（macOS 控制）：Accessibility / 截屏 / 焦点 / `mac_control`
  工具。执行层安全代码留 kernel：`MacControlFocusAnchor`（审批焦点快照
  类型）与 `normalize_perform_ax_action`（AX 动作规范化，permission engine
  的 dangerous 判定消费）下沉 `tools/`；审批焦点 capture/restore 与 args
  sanitize/preflight 经 `tools::register_mac_control_exec_hooks` 四件套
  **原子注册**（部分注册＝防御残缺，不允许）。
- **ha-design**（设计空间，与 artifacts 合并——artifacts 是 design 的存储
  层，2-crate 环随合并消解）：design 服务层 / durable Artifact 注册表 /
  canvas_db / ffmpeg / `design`·`canvas`·`artifact` 三工具。session 边界
  经 `session::design_hooks` 三钩子原子注册（Design 线程工作目录派生 /
  会话清理级联 / incognito 开启守卫）；Artifact 隐私切换锁下沉
  `session::privacy`（incognito 切换与 durable 写入共享同一把锁，kernel
  持有）；design_chat_threads 自有表经 `SessionDB::with_raw_conn` 受控
  闭包访问（锁封装不外泄）。
- **ha-browser**（浏览器）：Chrome Extension backend / CDP backend /
  loopback broker / 观察缓冲 / `browser` 工具。kernel 边界经
  `browser_hooks` 四件套原子注册（broker spawn——minimal/ACP 也起故走
  专用钩子而非 startup 档 / 轮结束 finalize / 会话清理级联 / knowledge
  网页捕获——抓取脚本与 backend 协商在特征侧，kernel 只拿最终
  `BrowserTabCapture`）；`IMAGE_BASE64_PREFIX` 与 `MEDIA_ITEMS_PREFIX`
  等结果格式契约常量留 kernel；Chrome 扩展 rust-embed 及其
  rerun-if-changed 随迁 ha-browser build.rs。
- **ha-vcs**（VCS / 本地执行基建）：git 控制平面操作面（index/branch/
  commit/push/PR/handoff + 启动期对账）/ Docker·WSL 沙箱执行机器 /
  SearXNG Docker 部署。**类型随表下沉**：`git_operation_runs` 簿记
  （DDL / `GitOperationRun` / 类型化查询）与 `repository_revision`
  git 指纹小簇留 kernel `git_control.rs`（ha-design code_sync 同时消费）；
  沙箱配置面（`SandboxConfig` 持久化 + `DockerStatus` wire 类型 +
  调用方 trampoline）留 kernel `sandbox.rs`，调用方（tools::exec / cron /
  settings_reset / system_prompt / 壳层）签名与路径不变。kernel 边界经
  `vcs_hooks` 四件套原子注册（沙箱 check/ensure/exec 三口 + git 操作
  对账），未接线 fail-closed：ensure/exec 即 Err（沙箱红线「不可用即
  终止、绝不回落宿主机」）、对账 app_warn 跳过。**对账钩子由
  `recover_startup_session_state` 同步内联调用**（Primary 启动期，
  `init_runtime` 内唯一生产调用点；进程 tier 固定无晋升。ordering
  invariant 见 app_init——须先于 `replay_pending_jobs` 完成），不得改为
  后台任务。`worktree` / `project_bootstrap` 是 session 生命周期簿记
  （goal / workflow / subagent / 启动 reconciler 深度消费、git 与 DB
  写交织），**整体留 kernel**——分析器分组仅供参考，切割以边界成本为准。
- **ha-mcp**（MCP 客户端）：McpManager 注册表 / client / 四种 transport /
  OAuth（独立实现 + PKCE，凭据 0600）/ watchdog / `mcp_resource`·
  `mcp_prompt` 两工具 / owner API。kernel 侧留存 `ha_core::mcp` facade：
  wire 类型再导出（定义在 ha-config-schema）、`mcp__` 命名约定单一来源
  （`catalog::is_mcp_tool_name` 等，**特征侧不得另写前缀判定**）、
  auto-approve 信任谓词 `server_auto_approves_config`（安全语义留
  kernel，钩子只做运行时查表）。kernel 边界经 `mcp_hooks` 七件套原子
  注册（init/watchdog 两档起动面——minimal/ACP 也 init，与 browser
  broker 同型 / 工具定义快照 / server 配置查表 / call_tool 分发 /
  prompt 段 / settings 热更 reconcile），**未接线语义逐项镜像
  manager-None 的既有行为**（工具缺席 / auto-approve 恒 false /
  call_tool Err / reconcile Ok+warn）。
- **ha-acp**（ACP）：`acp`（Hope 自身作 ACP stdio server，`hope-agent acp`
  模式）+ `acp_control`（外部 ACP agent 控制面：注册表 / 健康探测 /
  SessionManager / `acp_spawn` 工具）。`ACP_MANAGER` 全局随迁特征侧
  （`acp_control::get_acp_manager`，kernel 的 globals 不再持有）；
  `AgentAcpConfig`（agent.json wire 类型）下沉 `agent_config.rs`、
  `AcpRun`/`AcpRunStatus`（acp_runs 表行类型）下沉 `session/acp_db.rs`，
  特征侧原路径再导出；prompt 段 binary 可用性经
  `system_prompt::register_acp_binary_resolver` 钩子。

### ha-server（HTTP/WS 服务）

| 职责 | 说明 |
|------|------|
| REST API | ~430 个端点（精确数以 `grep -rE '\.route\(' crates/ha-server/src/ \| wc -l` 为准；完整清单与 Tauri 命令对照见 [api-reference.md](api-reference.md)） |
| WebSocket | `/ws/events`（全局事件广播，含聊天流 `chat:stream_delta`、`channel:stream_delta` 等） |
| 路由框架 | axum 0.8 + tower-http CORS |
| API Key 鉴权 | `middleware.rs` — `Authorization: Bearer` 头 + `?token=` 查询参数，`/api/health` 与 `/api/server/status` 免鉴权 |
| 内嵌 Web GUI | `web_assets.rs` 用 `rust-embed` 把 Vite `dist/` 打进二进制；axum `fallback_service` 直返；`HA_WEB_ROOT` 可指向本地 `dist/` 做 dev override |
| 错误处理 | axum 风格 `Result<Json<T>, (StatusCode, String)>`，显式 status code，不做字符串匹配 |

**关键类型**：

```rust
pub struct AppContext {
    pub session_db: Arc<SessionDB>,
    pub event_bus: Arc<dyn EventBus>,
    pub chat_cancels: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,  // per-session 取消
}
```

### src-tauri（桌面壳）

| 职责 | 说明 |
|------|------|
| Tauri IPC | ~430 个 `#[tauri::command]` 处理函数（精确数以 `grep -rE '#\[tauri::command\]' src-tauri/src/ \| wc -l` 为准；完整清单与 HTTP 对照见 [api-reference.md](api-reference.md)） |
| 桌面集成 | 系统托盘、全局快捷键、窗口管理、macOS 菜单 |
| 薄封装 | `tauri_wrappers.rs` 为 ha-core 无 `#[tauri::command]` 的函数添加属性 |
| 内嵌服务 | `setup.rs` 中 spawn ha-server，配置从 `config.json` 的 `server` 字段读取 |
| 入口管理 | Guardian / Child / Server / ACP 四种模式 |
| 错误边界 | 命令统一返回 `Result<T, CmdError>`（[`commands/error.rs`](../../src-tauri/src/commands/error.rs)，详见下文 §错误处理） |

**文件结构**（顶层 ~10 个 + `commands/` 子目录约 33 个文件）：

```
src-tauri/src/
  lib.rs              pub use ha_core::*; + Tauri Builder + invoke_handler! 注册
  main.rs             入口分发（server/acp/guardian/child）
  globals.rs          APP_HANDLE（仅 Tauri 专用）
  app_init.rs         薄封装 → ha_core::init_app_state()
  setup.rs            app_setup()：内嵌 HTTP 服务 + 快捷键 + 托盘
  tauri_wrappers.rs   薄 #[tauri::command] 封装（为 ha-core fn 加属性）
  shortcuts.rs        全局快捷键处理
  tray.rs             系统托盘菜单
  commands/           Tauri IPC 命令实现（按功能域拆分，~33 个文件）
    error.rs          CmdError（统一错误类型，IPC wire 上是字符串）
    chat.rs / session.rs / config.rs / provider/ / agent_mgmt.rs
    project.rs / mcp.rs / channel.rs / cron.rs / subagent.rs / team.rs
    plan.rs / memory.rs / skills.rs / dashboard.rs / recap.rs
    local_llm.rs / local_model_jobs.rs / local_embedding.rs
    filesystem.rs / runtime_tasks.rs / acp_control.rs / dreaming.rs
    auth.rs / browser.rs / docker.rs / url_preview.rs / onboarding.rs
    crash.rs / logging.rs / misc.rs / ...
```

---

## 运行模式

```mermaid
graph LR
    subgraph "入口 (main.rs)"
        MAIN["hope-agent"]
    end

    MAIN -->|"无参数"| GUARDIAN["Guardian<br/>进程监护"]
    GUARDIAN --> CHILD["Child Mode<br/>Tauri GUI<br/>+ 内嵌 HTTP :8420"]

    MAIN -->|"server"| SERVER["HTTP/WS Server<br/>axum :8420<br/>无 GUI"]
    MAIN -->|"acp"| ACP["ACP Server<br/>stdio NDJSON"]

    SERVER -->|"install"| LAUNCHD["系统服务注册<br/>launchd / systemd"]
    SERVER -->|"stop"| STOP["读 PID → SIGTERM"]
    SERVER -->|"status"| STATUS["查询服务状态"]
```

### 1. 桌面模式（默认）

```mermaid
flowchart LR
    HA["hope-agent"] --> GUARD["Guardian"] --> CHILD["Child（Tauri GUI + 内嵌 HTTP）"]
```

- Guardian 监护子进程，崩溃自动重启（指数退避 1s→30s，最多 8 次）
- 第 5 次崩溃触发 backup + self-diagnosis + auto-fix
- 子进程启动 Tauri GUI，`setup.rs` 中同时 spawn ha-server
- 前端通过 Tauri IPC 调用后端（也可通过内嵌 HTTP 服务）
- 内嵌服务器配置从 `config.json` 的 `server` 字段读取（`EmbeddedServerConfig`）：
  - `bindAddr`：监听地址（默认 `127.0.0.1:8420`，设为 `0.0.0.0:8420` 可对外暴露）
  - `apiKey`：API Key 鉴权（`null` = 无鉴权）
- 修改后需重启应用生效

### 2. 服务器模式

```
hope-agent server [--bind 0.0.0.0:8420] [--api-key KEY]
```

- 无 GUI，纯 HTTP/WS 守护进程
- CLI `--api-key` 参数优先于 config.json 配置
- 初始化 ha-core 全部子系统（DB、IM 渠道、ACP、Cron）
- 写 PID 文件到 `~/.hope-agent/server.pid`
- 支持系统服务注册：

| 命令 | 说明 |
|------|------|
| `server install` | 注册系统服务（macOS launchd / Linux systemd） |
| `server uninstall` | 卸载系统服务 |
| `server status` | 查询运行状态 |
| `server stop` | 发送 SIGTERM 停止 |

### 3. ACP 模式

```
hope-agent acp [--agent-id ha-main] [--verbose]
```

- stdio NDJSON JSON-RPC 协议
- 用于 IDE 直连（Zed、VS Code 等）

---

## 事件系统

```mermaid
sequenceDiagram
    participant Core as ha-core 模块
    participant Bus as EventBus (broadcast)
    participant Tauri as Tauri Bridge
    participant WS as WS /ws/events
    participant UI_T as 桌面前端
    participant UI_W as Web 前端

    Core->>Bus: emit("approval_required", {...})
    Bus-->>Tauri: subscriber receives
    Bus-->>WS: subscriber receives
    Tauri->>UI_T: handle.emit() → window
    WS->>UI_W: WebSocket text frame
```

### EventBus 架构

| 层 | 组件 | 说明 |
|----|------|------|
| 定义 | `ha-core::event_bus::EventBus` trait | `emit()` + `subscribe()` |
| 实现 | `BroadcastEventBus` | `tokio::sync::broadcast` channel |
| 桥接（Tauri） | `setup.rs` → EventBus subscriber → `handle.emit()` | 转发到 Tauri WebView |
| 桥接（HTTP） | `ws/events.rs` → EventBus subscriber → WS frame | 转发到 WebSocket 客户端 |
| 桥接（IM） | `ChannelStreamSink` → EventBus + mpsc | 转发到 IM 渠道 |

### 事件清单

> 字面量来源：`grep -rE 'bus\.emit\(' crates/ha-core/src/` + 同 grep 在 `crates/ha-acp/src/` / `crates/ha-mac/src/` / `crates/ha-design/src/` / `crates/ha-browser/src/` / `crates/ha-vcs/src/` / `crates/ha-mcp/src/`（及后续特征 crate）/ `crates/ha-server/src/` / `src-tauri/src/`；常量定义集中在 `chat_engine/stream_broadcast.rs`、`local_model_jobs.rs`、`ha-mcp (events.rs)`、`ha-vcs (docker/mod.rs · git_control.rs EVENT_GIT_*)`、`tools/ask_user_question.rs`、`ha-design (tool_canvas/mod.rs)`、`ha-acp (acp_control/events.rs)`、`ha-mac (lib.rs EVENT_MAC_CONTROL_FRAME / ha-core tool_actions.rs EVENT_MAC_CONTROL_ACTION)`。

#### 聊天 / 流式

| 事件名 | 来源 | 用途 |
|--------|------|------|
| `chat:stream_delta` | chat_engine/stream_broadcast.rs | 主对话流式 token，带 `{sessionId, seq}` |
| `chat:stream_end` | chat_engine/stream_broadcast.rs | 主对话流式结束 |
| `channel:stream_delta` | chat_engine/stream_broadcast.rs | IM 渠道流式 token |
| `channel:stream_start` / `channel:stream_end` | channel/worker | IM 流式状态变更 |
| `channel:message_update` | channel/worker | IM 会话有新消息 |

#### 工具 / 审批 / 交互

| 事件名 | 来源 | 用途 |
|--------|------|------|
| `approval_required` | tools/approval.rs | 工具执行需要用户审批 |
| `session_pending_interactions_changed` | tools/approval.rs | session 的待响应交互（审批 + ask_user）数量变化，前端 300 ms 防抖刷新 |
| `ask_user_request` | tools/ask_user_question.rs | 向用户发起结构化问答 |
| `agent:send_notification` | tools/notification.rs | 桌面通知 |
| `tool_call_narration` 等 | tools/* | 内置工具调用相关事件 |

#### Subagent / Team / Plan

| 事件名 | 来源 | 用途 |
|--------|------|------|
| `subagent_event` | subagent/helpers.rs | 子 Agent 生命周期 |
| `parent_agent_stream` | subagent/helpers.rs | 子 Agent 结果注入主对话 |
| `team_event` | team/* | Agent Team 生命周期 |
| `plan_mode_changed` / `plan_submitted` / `plan_amended` / `plan_step_updated` / `plan_subagent_status` | plan/*, tools/plan_step.rs, tools/submit_plan.rs, tools/amend_plan.rs | Plan Mode 状态机 |

#### 记忆 / Recap / Dashboard

| 事件名 | 来源 | 用途 |
|--------|------|------|
| `core_memory_updated` | tools/memory.rs | 记忆变更（save / update / delete） |
| `memory_extracted` | memory_extract.rs | 自动提取写库后通知前端 |
| `recall_hit` / `recall_summary_used` | tools/memory.rs / dashboard | Learning 埋点 |
| `dreaming:cycle_complete` | memory/dreaming/pipeline.rs | Dreaming 离线固化完成 |
| `recap_progress` | recap/api.rs, slash_commands/handlers/recap.rs | /recap 章节进度 |
| `session:title_updated` | session_title.rs | 会话标题生成完毕 |

#### Skills / MCP

| 事件名 | 来源 | 用途 |
|--------|------|------|
| `skill_activated` / `skill_used` / `skill_created` / `skill_patched` / `skill_discarded` | skills/* | Skill 生命周期与 Learning 埋点 |
| `skills:auto_review_complete` | skills/* | Draft 审核完成 |
| `skills:curator_proposals_ready` | skills/auto_review/curator.rs | Auto-curator 周期扫描产出草稿合并建议 |
| `mcp:servers_changed` (`EV_SERVERS_CHANGED`) | ha-mcp (events.rs) | MCP 服务器列表变更 |
| `mcp:server_status_changed` | ha-mcp (events.rs) | MCP 单个 server 状态切换（Ready / NeedsAuth / Failed 等） |
| `mcp:catalog_refreshed` | ha-mcp (events.rs) | MCP tool catalog 重建 |
| `mcp:server_log` | ha-mcp (events.rs) | MCP server 日志推送 |
| `mcp:auth_required` / `mcp:auth_completed` | ha-mcp (events.rs) | MCP OAuth 流程信号 |
| `mcp_tool_called` / `mcp_tool_failed` (`EVT_MCP_*`) | 常量 ha-core (dashboard/learning.rs)，emit ha-mcp (invoke.rs) | Dashboard Learning 埋点 |

#### 项目 / Channel / 系统

| 事件名 | 来源 | 用途 |
|--------|------|------|
| `project:created` / `project:updated` / `project:deleted` | src-tauri / ha-server projects 路由 | 项目 CRUD |
| `project:file_uploaded` / `project:file_deleted` | projects 路由 | 项目附件变更 |
| `agents:changed` | agent_mgmt | Agent 列表变更 |
| `config:changed` | config/persistence.rs, tools/settings.rs, backup.rs | 任何 `mutate_config` 写入路径自动 emit，`{category, source}` 元数据 |
| `weather-cache-updated` | ha-weather (lib.rs) | 天气缓存刷新 |
| `mac_control:frame` (`EVENT_MAC_CONTROL_FRAME`) | ha-mac (lib.rs) | mutating 动作后的屏幕帧关联（actionId 缩略图，见 [macos-control](macos-control.md)） |
| `mac_control:action` (`EVENT_MAC_CONTROL_ACTION`) | tool_actions.rs | mac_control 动作时间线事件 |
| `session:git_progress` / `session:git_completed` (`EVENT_GIT_PROGRESS` / `EVENT_GIT_COMPLETED`) | ha-vcs (git_control.rs) | 长 Git 操作（push/PR/handoff）阶段进度与完结 |
| `session:git_changed` (`EVENT_GIT_CHANGED`) | ha-vcs (git_control.rs) | Git 状态变更后前端刷新 snapshot |
| `searxng:deploy_progress` (`EVENT_SEARXNG_DEPLOY_PROGRESS`) | ha-vcs (docker/mod.rs) | SearXNG Docker 部署进度，前端 progress UI 消费 |
| `acp_control_event` | ha-acp (acp_control/events.rs) | ACP 运行生命周期 |
| `cron:run_completed` | cron/executor.rs | 定时任务完成 |

#### 异步任务 / 本地模型

| 事件名 | 来源 | 用途 |
|--------|------|------|
| `job:created` / `job:updated` / `job:progress` / `job:completed` / `job:mark_injected_failed` | async_jobs/* | 后台工具与 group 任务生命周期；subagent 仍走 `subagent:*` 流 |
| `local_model_job:created` / `:updated` / `:log` / `:completed` (`EVENT_LOCAL_MODEL_JOB_*`) | local_model_jobs.rs | Ollama 安装 / pull / 模型加载等后台任务，进度自带 250 ms / phase-change 节流 |

#### 斜杠 / Canvas / Session

| 事件名 | 来源 | 用途 |
|--------|------|------|
| `slash:effort_changed` / `slash:plan_changed` / `slash:session_cleared` / `slash:model_switched` | channel/worker/slash.rs, slash_commands | 斜杠命令副作用通知 |
| `canvas_show` / `canvas_hide` / `canvas_reload` / `canvas_deleted` / `canvas_snapshot_request` / `canvas_eval_request` | ha-design (tool_canvas/mod.rs) | Canvas 面板控制 |
| `artifact:created` / `:updated` / `:verified` / `:archived` / `:deleted` | ha-design (artifacts/mod.rs) | Artifact 当前版本、验证和生命周期刷新 |
| `artifact:export_running` / `:export_ready` / `:export_failed` | ha-design (artifacts/mod.rs) | owner 导出 receipt 生命周期；PDF 缺 runtime 时另发 `browser:runtime_required` |
| `session_message_injected` | sessions / subagent | 子任务结果注入主会话消息流 |

#### 桌面专属（Tauri 直发，不进 EventBus 抽象层）

| 事件名 | 来源 | 用途 |
|--------|------|------|
| `new-session` / `open-settings` | shortcuts / tray | 全局快捷键 + 托盘菜单触发 |
| `shortcut-triggered` / `chord-first-pressed` / `chord-timeout` | shortcuts.rs | 快捷键交互反馈 |
| `openclaw-import` | agents | 历史 OpenClaw 导入进度 |

---

## 前端 Transport 抽象层

```mermaid
graph TD
    subgraph "业务组件 (91 个文件)"
        Comp["ChatScreen / Settings / ...]"]
    end

    Comp -->|"getTransport().call()"| Provider["transport-provider.ts<br/>环境自动检测"]

    Provider -->|"__TAURI_INTERNALS__"| Tauri["TauriTransport<br/>invoke() + Channel"]
    Provider -->|"otherwise"| HTTP["HttpTransport<br/>fetch() + WebSocket"]

    Tauri --> IPC["Tauri IPC"]
    HTTP --> REST["REST API :8420"]
    HTTP --> WS["WebSocket :8420"]
```

### Transport 接口

```typescript
interface Transport {
  call<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  startChat(args: ChatStartArgs, onEvent: (event: string) => void): Promise<string>;
  listen(eventName: string, handler: (payload: unknown) => void): () => void;
}
```

### 运行时切换

```typescript
// 自动检测
getTransport()  // → TauriTransport 或 HttpTransport

// 手动切换（设置面板）
switchToRemote("https://my-server.com")  // 连接远程服务
switchToEmbedded()                        // 切回本地
```

### HttpTransport 命令映射

`transport-http.ts` 内部维护 ~424 个命令到 REST 端点的映射表（精确数以 `grep -cE '^\s*[a-z_][a-z_0-9]*:\s*\{' src/lib/transport-http.ts` 为准；完整对照表见 [api-reference.md](api-reference.md)）：

```typescript
const COMMAND_MAP = {
  list_sessions_cmd:  { method: "GET",    path: "/api/sessions" },
  create_session_cmd: { method: "POST",   path: "/api/sessions" },
  delete_session_cmd: { method: "DELETE", path: "/api/sessions/{sessionId}" },
  chat:               { method: "POST",   path: "/api/chat" },
  get_providers:      { method: "GET",    path: "/api/providers" },
  // ... 数百个命令省略
};
```

新增 invoke 时必须同步在 `transport-http.ts` 的 `COMMAND_MAP` 里登记一条；`api-reference.md` 是 Tauri ↔ HTTP 对齐的单一真相源。

---

## 初始化流程

三种模式共享 `ha_core::init_runtime(role)` 这一个全局单例 setter。它内部第一步就调 [`runtime_lock::acquire_or_secondary`](../../crates/ha-base/src/runtime_lock.rs)，在 `~/.hope-agent/runtime.lock` 上抢一把 OS 级 advisory exclusive lock：第一个抢到的进程是 **Primary**，做所有 startup cleanup + 跑独占性后台循环；其它进程是 **Secondary**，初始化 OnceLock 但跳过这些清扫与循环。后台任务变体按模式选 `start_background_tasks`（桌面 + server）或 `start_minimal_background_tasks`（acp），两个变体内部各自再按 `runtime_lock::is_primary()` gate Primary-only 部分。

三种模式的完整启动入口、Primary tier 跑哪些 cleanup、tier-agnostic 与 Primary-only 后台任务清单统一维护在 **[process-model.md](process-model.md)**（重点参考 [§ 启动入口（桌面独占）](process-model.md#启动入口桌面独占)、[§ Primary / Secondary 协作](process-model.md#primary--secondary-协作多进程并存)、[§ 跨模式能力对照](process-model.md#跨模式能力对照)）——本文不再复述以避免双份维护漂移。

---

## 全局状态管理

### OnceLock 单例（ha-core）

| 静态变量 | 类型 | 用途 |
|---------|------|------|
| `EVENT_BUS` | `Arc<dyn EventBus>` | 事件广播 |
| `APP_LOGGER` | `AppLogger` | 结构化日志 |
| `SESSION_DB` | `Arc<SessionDB>` | 会话数据库 |
| `MEMORY_BACKEND` | `Arc<dyn MemoryBackend>` | 记忆存储 |
| `CRON_DB` | `Arc<CronDB>` | 定时任务 |
| `LOG_DB` | `Arc<LogDB>` | 日志持久化（与 `APP_LOGGER` 异步 writer 分离） |
| `SUBAGENT_CANCELS` | `Arc<SubagentCancelRegistry>` | 子 Agent 取消 |
| `CHANNEL_CANCELS` | `Arc<ChannelCancelRegistry>` | IM 渠道取消（写桌面／HTTP／Channel 共读同一 Arc） |
| `CHANNEL_REGISTRY` | `Arc<ChannelRegistry>` | IM 插件注册表 |
| `CHANNEL_DB` | `Arc<ChannelDB>` | IM 会话映射 |
| `ACP_MANAGER` | `Arc<AcpSessionManager>` | ACP 控制面 |
| `CODEX_TOKEN_CACHE` | `Arc<tokio::Mutex<Option<(String, String)>>>` | Codex OAuth in-memory 快照 |
| `REASONING_EFFORT` | `Arc<tokio::Mutex<String>>` | 运行时推理强度 |
| `CACHED_AGENT` | `Arc<tokio::Mutex<Option<AssistantAgent>>>` | 兼容缓存 Agent（fallback 路径 + `/compact` / `/model` 操作对象） |

### AppState 字段

`AppState` 是 Tauri `State<'_, AppState>` 注入载体，ha-core 内部路径**不**通过它读写跨运行时状态——所有有对应 OnceLock 的字段都是 `Arc<…>.clone()`，Tauri 命令和 OnceLock 访问器看到的是同一份数据。`init_app_state()` 用 `debug_assert!` 强制这个不变量。

| 字段 | 类型 | 说明 |
|------|------|------|
| `agent` | `Arc<tokio::Mutex<Option<AssistantAgent>>>` | 与 [`CACHED_AGENT`] 共享 |
| `auth_result` | `Arc<tokio::Mutex<Option<anyhow::Result<TokenData>>>>` | 桌面 OAuth 登录 rendezvous，无跨运行时需求 |
| `reasoning_effort` | `Arc<tokio::Mutex<String>>` | 与 [`REASONING_EFFORT`] 共享 |
| `codex_token` | `Arc<tokio::Mutex<Option<(String, String)>>>` | 与 [`CODEX_TOKEN_CACHE`] 共享 |
| `current_agent_id` | `Mutex<String>` | 桌面专属 |
| `session_db` / `project_db` / `log_db` / `cron_db` | `Arc<…>` | 与对应 OnceLock 共享 |
| `chat_cancel` | `Arc<AtomicBool>` | 桌面专属 |
| `logger` | `AppLogger` | 与 `APP_LOGGER` 共享 |
| `subagent_cancels` | `Arc<SubagentCancelRegistry>` | 与 [`SUBAGENT_CANCELS`] 共享 |
| `channel_cancels` | `Arc<ChannelCancelRegistry>` | 与 [`CHANNEL_CANCELS`] 共享 |

### 跨模式能力（已对齐）

三种模式都调用 `ha_core::init_runtime(role)`，所有 OnceLock 在三种模式下都被 populate；`build_app_state()` 仅桌面用（构造 Tauri `AppState`），server / ACP 直接消费 OnceLock。后台任务变体按模式选 `start_background_tasks`（桌面 + server）或 `start_minimal_background_tasks`（acp）。

**多进程并存安全**：`init_runtime` 内部第一步抢 `~/.hope-agent/runtime.lock`（OS advisory lock，进程退出 / panic / SIGKILL / 断电时 OS 自动释放）。第一个抢到的进程是 **Primary** 跑 cleanup + 独占性循环；后续进程是 **Secondary** 只 init OnceLock 但跳过这些。模式不参与（FCFS），ACP-only 场景自然成为 Primary。详细 Primary-only 清单与 manual-API 不受影响的 carve-outs 见 [process-model.md § Primary / Secondary 协作](process-model.md#primary--secondary-协作多进程并存)。

历史 gap：
- `fix/server-acp-runtime-init` 之前，`hope-agent server start` 只手写 SessionDB / ProjectDB / EventBus 三个 OnceLock，`hope-agent acp` 只开 SessionDB；任何依赖其它 OnceLock 的工具（`recall_memory` / `manage_cron` / `subagent` 等）在 daemon 模式下都会 `"XXX not initialized"`。
- 同一分支早期把 cleanup 共享给三种模式但缺多进程协作，导致桌面 + ACP 共存时 ACP 的 startup cleanup 会 mark 桌面活的 subagent 失败、`clear_all_running` 让 cron 双跑、`replay_pending_jobs` 把活 async 工具标 Interrupted、最严重 **硬删除桌面无痕会话**。本次以 OS file lock 选举 Primary 修复。

### Tauri 专属全局（src-tauri）

| 静态变量 | 类型 | 用途 |
|---------|------|------|
| `APP_HANDLE` | `tauri::AppHandle` | Tauri 事件发射、窗口管理 |

---

## 错误处理 Contract（src-tauri 命令边界）

src-tauri 命令统一返回 `Result<T, CmdError>`（[`src-tauri/src/commands/error.rs`](../../src-tauri/src/commands/error.rs)）。

```rust
// commands/error.rs
pub struct CmdError(String);

impl<E: Into<anyhow::Error>> From<E> for CmdError {
    fn from(err: E) -> Self {
        // 用 alternate Display 输出 cause chain，让 .context("...") 加的上下文不丢
        Self(format!("{:#}", err.into()))
    }
}

impl Serialize for CmdError {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.0)
    }
}
```

**硬规则**：

- 命令体内部 `?` 直接传播 `anyhow::Error` / `std::io::Error` / `serde_json::Error` / 任何 `Into<anyhow::Error>`，**不要再写 `.map_err(|e| e.to_string())`**
- IPC wire 上 `CmdError` 序列化成纯字符串，前端零迁移；与历史 `Result<T, String>` 命令在 IPC 层兼容
- 用户可见的纯文本错误用 `CmdError::msg("...")` 构造，取代散落的 `Err("msg".to_string())`
- HTTP 路由侧仍用 axum 习惯的 `Result<Json<T>, (StatusCode, String)>`，错误语义由 `routes/*` 自行映射

历史背景：commit `cdb6a495` / `8a7569aa` 之前每个命令都得手写 `.map_err(|e| e.to_string())?`，类型不安全、context chain 丢失；CmdError 统一收口后这些样板代码全部移除。

---

## Provider 写入集中化

所有 Provider 列表与 `active_model` 写入**必须**经过 [`crates/ha-core/src/provider/crud.rs`](../../crates/ha-core/src/provider/crud.rs) 的 helper：

| Helper | 语义 |
|--------|------|
| `add_provider` | 生成新 id 并 append（保持前端"新增后取最后一项"流程） |
| `update_provider` | 按 id 更新 |
| `delete_provider` / `delete_providers_by_api_type` | 删除 |
| `reorder_providers` | 排序 |
| `set_active_model` | 切换 active model |
| `add_and_activate_provider` | 复合：append + 立即激活 |
| `add_many_providers` | 批量导入 |
| `ensure_codex_provider_persisted` | Codex OAuth 兜底持久化 |
| `upsert_known_local_provider_model` | 本地 LLM 安装路径专用：按 known backend host/port 去重、补模型、启用 provider、`allow_private_network=true`、切 active model |

**禁止**在 Tauri / server / onboarding / importer / local_llm 任何路径里直接 `cfg.providers.push(...)` / `.retain(...)` / 手写 `cfg.active_model = ...`。Tauri 命令和 HTTP 路由只做薄壳和运行时 agent 重建，业务逻辑全在 `provider/crud.rs`。

详细流程（id 生成、唯一性、active_model 联动、known backend 匹配规则）见 [provider-system.md](provider-system.md)。

---

## 配置读写 Contract

详细规范见 [`docs/architecture/config-system.md`](config-system.md)。本节列硬规则：

- **读** 走 `ha_core::config::cached_config()`，返回 `Arc<AppConfig>` 快照（[`crates/ha-core/src/config/persistence.rs`](../../crates/ha-core/src/config/persistence.rs)）；禁止重新引入 `Mutex<AppConfig>` 或本地克隆
- **写** 走 `ha_core::config::mutate_config((category, source), |cfg| { ... })`：
  - 读最新快照、应用 closure、原子写盘、自动 emit `config:changed`、自动落 autosave 备份
  - 禁止 `load_config()` + 修改 + `save_config()` 手动克隆-改-存模式 —— 无法防并发 lost-update（历史 image_generate stale bug 的根因）
- 旧 `AppState::config: Mutex<AppConfig>` 字段已于 **2026-04-20 删除**；PR 里出现 `state.config.lock()` 一律 reject

GUI 设置面板 + `ha-settings` 技能 + Tauri / HTTP 命令对配置的所有写入路径都走这一个入口，否则会跟前端 `config:changed` 监听器、autosave 备份、CLI sync-version 这些副作用脱节。

---

## Guardian 保活机制

Guardian 父子进程在桌面 Release 默认启用：父进程监工、child 以 `--child-mode` 跑 Tauri；child 异常退出时父按指数退避重启，第 5 次崩溃触发备份 + LLM Self-Diagnosis + Auto-Fix，第 8 次放弃。完整状态图、退出码协议、参数表、Crash Journal schema、Self-Diagnosis prompt 与 fallback、Auto-Fix 覆盖范围全部归档在 **[reliability.md](reliability.md)**——本文不复述以避免双份维护。

`hope-agent server` 由 launchd / systemd 托管重启，**不要再叠 Guardian**；`hope-agent acp` 由 IDE 控制生命周期，也不走 Guardian。

---

## 系统服务注册

`hope-agent server install` 把进程登记给 OS 服务管理器：macOS launchd LaunchAgent（`KeepAlive=true`）、Linux systemd user unit（`Restart=on-failure`）、Windows Task Scheduler（`onlogon`，无自动重启）。完整 plist / unit 键值、ExecStart 转义规则、和 Guardian 的互斥关系见 **[reliability.md §Layer 3](reliability.md#4-layer-3--操作系统服务保活)**——避免与单一权威源同步漂移，本节不复述参数表。

---

## HTTP API 端点一览

完整清单（~430 个 REST 端点 + 1 个 WebSocket 端点）与对应 Tauri 命令对照见 **[api-reference.md](api-reference.md)**，本节只保留顶层结构索引：

| 功能域 | HTTP 前缀 | WebSocket |
|---|---|---|
| Sessions / Chat | `/api/sessions/*`、`/api/chat/*`、`/api/runtime-tasks/*` | `/ws/events` 上的 `chat:stream_delta` / `chat:stream_end` |
| Projects | `/api/projects/*`（CRUD + `/files`、`/sessions`、`/memories`、`/archive`） | `project:*` |
| Providers / Models / Agents | `/api/providers/*`、`/api/models/*`、`/api/agents/*`（含 OpenClaw scan / import） | `agents:changed` |
| MCP | `/api/mcp/servers/*`、`/api/mcp/global`、`/api/mcp/import/claude-desktop` | `mcp:*` |
| Memory | `/api/memory/*`（CRUD / search / reembed / import-export / global-md） | `core_memory_updated` / `memory_extracted` / `recall_hit` |
| Config | `/api/config/*`（40+ 分项：embedding / mmr / multimodal / ssrf / shortcuts / theme / language / autostart / server / default-agent / sandbox 等） | `config:changed` |
| Plan / Ask User | `/api/plan/*`、`/api/ask_user/respond` | `plan_*` / `ask_user_request` |
| Dashboard / Recap / Logging | `/api/dashboard/*`（含 `learning/*`、`insights`）、`/api/recap/*`、`/api/logs/*` | `recap_progress` |
| Cron / Subagent / Team | `/api/cron/*`、`/api/subagent/*`、`/api/teams/*`、`/api/team-templates/*` | `cron:run_completed` / `subagent_event` / `team_event` |
| Channels (IM) | `/api/channel/*`（含 wechat 登录二维码、validate、test-message） | `channel:*` |
| Artifacts / Canvas / Browser / Weather | `/api/artifacts/*`、`/api/artifact-exports/*`、`/api/canvas/*`（snapshot / eval / project 静态资源）、`/api/browser/*`、`/api/weather/*` | `artifact:*` / `canvas_*` / `browser:runtime_required` / `weather-cache-updated` |
| Skills / Slash | `/api/skills/*`（drafts / env / extra-dirs / preset-sources）、`/api/slash-commands/*` | `skill_*` / `slash:*` |
| Auth / ACP | `/api/auth/codex/*`、`/api/auth/session/restore`、`/api/acp/*`（backends / runs / config） | `acp_control_event` |
| Onboarding | `/api/onboarding/*`（state / draft / language / profile / safety / skills / server）、`/api/server/{generate-api-key,local-ips}` | — |
| Local LLM 助手 | `/api/local-llm/*`（hardware / recommendation / ollama-status / known-backends / library / preload / models / provider-model / default-model / embedding-config） | — |
| Local Model Jobs | `/api/local-model-jobs/*`（list / chat-model / embedding / ollama-{install,pull,preload} / cancel / pause / retry / logs） | `local_model_job:created/updated/log/completed` |
| Local Embedding | `/api/local-embedding/*` | — |
| Filesystem（远程目录浏览） | `/api/filesystem/list-dir`、`/api/filesystem/search-files` | — |
| URL Preview / SearXNG Docker | `/api/url-preview`、`/api/url-preview/batch`、`/api/searxng/{status,deploy,start,stop}`、`DELETE /api/searxng` | `searxng:deploy_progress` |
| Crash / Backup / Settings Backups | `/api/crash/*`、`/api/settings/backups/*`、`/api/crash/guardian` | — |
| Dreaming | `/api/dreaming/{run,diaries,status}` | `dreaming:cycle_complete` |
| Misc / Security / System / Desktop | `/api/misc/*`、`/api/security/*`、`/api/system/*`、`/api/desktop/*` | — |
| Dev tools | `/api/dev/{clear-sessions,clear-cron,clear-memory,reset-config,clear-all}` | — |
| 静态资源 | `/api/attachments/{session_id}/{filename}`、`/api/avatars/*`、`/api/generated-images/*`、`/api/canvas/projects/{pid}/{*rest}` | — |
| 全局事件推送 | — | `/ws/events`（EventBus → 文本帧，带 `{name, payload}`，可附 `missed`） |
| 免鉴权 | `/api/health`、`/api/server/status` | — |

---

## 数据流：一次完整对话

```mermaid
sequenceDiagram
    participant FE as 前端
    participant T as Transport 层
    participant S as ha-server
    participant CE as ChatEngine
    participant Agent as AssistantAgent
    participant LLM as LLM API
    participant WS as WebSocket

    FE->>T: getTransport().startChat({message, sessionId}, onEvent)
    T->>S: POST /api/chat
    FE->>WS: connect /ws/events

    S->>CE: run_chat_engine(params)
    CE->>Agent: agent.chat(message)
    Agent->>LLM: HTTP stream request

    loop 流式输出
        LLM-->>Agent: token delta
        Agent-->>CE: on_delta callback
        CE-->>S: EventBus chat:stream_delta
        S-->>WS: forward AppEvent
        WS-->>FE: text frame {"name":"chat:stream_delta","payload":...}
    end

    CE-->>S: ChatEngineResult
    S-->>T: {"session_id":"...","response":"..."}
    T-->>FE: resolve Promise
```

---

## 多客户端支持

| 层面 | 机制 | 说明 |
|------|------|------|
| 全局事件 | `BroadcastEventBus` | 每个 WS 连接独立 Receiver，所有客户端同步收到 |
| 会话流式 | `BroadcastEventBus` 上的 `chat:stream_delta` | 多端可按 `sessionId` 过滤并实时观看 |
| 并发对话 | per-session `AtomicBool` cancel map | 不同客户端不同会话互不干扰 |
| 审批系统 | EventBus 广播 + oneshot 响应 | 任何客户端可响应审批请求 |

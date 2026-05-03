# 命令行接口（CLI）

> 返回 [文档索引](../README.md) | 关联文档：[Transport 运行模式](transport-modes.md) · [前后端分离架构](backend-separation.md) · [进程与并发模型](process-model.md) · [可靠性与崩溃自愈](reliability.md) · [ACP 协议](acp.md) | 关联源码：[`src-tauri/src/main.rs`](../../src-tauri/src/main.rs) · [`crates/ha-core/src/service_install.rs`](../../crates/ha-core/src/service_install.rs) · [`crates/ha-core/src/onboarding/`](../../crates/ha-core/src/onboarding/)

Hope Agent 的所有运行模式共享同一个二进制 `hope-agent`。CLI 是分流入口：根据第一个非全局参数决定走桌面 GUI、HTTP/WS 守护进程、ACP stdio 协议，还是一次性的 OAuth 登录流程。本文是 CLI 子命令、参数与环境变量的完整参考——参数解析逻辑全部在 [`src-tauri/src/main.rs`](../../src-tauri/src/main.rs)，手写 `std::env::args()` 不依赖 clap，行为以源码为准。

## 子命令总览

```
hope-agent [GLOBAL_FLAGS] [SUBCOMMAND] [OPTIONS]
```

主分发顺序（[`main.rs:11-53`](../../src-tauri/src/main.rs#L11)）：**全局 flag → `acp` → `server` → `auth` → 桌面 / Guardian / 子进程**。匹配到任何子命令就 return，不再继续往下；未知子命令静默落到桌面入口。

| 子命令         | 性质         | 触发                                  | 入口函数 / 模块               | 说明                                                                                       |
| -------------- | ------------ | ------------------------------------- | ---------------------- | ------------------------------------------------------------------------------------------ |
| **桌面 GUI**       | 长驻进程     | 无子命令（默认）                      | `run_child` / `run_guardian` | Tauri WebView。生产构建经 Guardian 监督子进程；dev / 用户禁用 Guardian 时直接跑              |
| **HTTP/WS 服务器** | 长驻进程     | `hope-agent server [...]`             | `run_server`           | axum 守护进程，内嵌 Web GUI；浏览器访问 `http://<bind>` 即得完整 React UI                  |
| **ACP stdio**      | 长驻进程     | `hope-agent acp [...]`                | `run_acp_server`       | NDJSON over stdio，给 IDE / 外部客户端直连核心协议用                                      |
| **Auth 一次性命令** | 短命令       | `hope-agent auth <provider> [...]`    | [`cli_auth::run`](../../src-tauri/src/cli_auth.rs) | 终端环境下完成 OAuth（目前仅 Codex / ChatGPT），登录成功落 token + 写 Provider 后退出     |

三种长驻模式共享 `ha-core` 业务逻辑、`init_runtime(role)` 初始化路径与 `EventBus`；只在前端入口 / 背景任务集合 / 鉴权方式上有差异。`auth` 不进 `init_runtime`，只 touch credentials / provider config 后退出。

## 全局参数

在 `main()` 顶层处理，先于子命令分发，因此桌面 / server / acp 都生效。

| 参数                                  | 类型 | 默认 | 说明                                                                                                                                                                                                                              |
| ------------------------------------- | ---- | ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--dangerously-skip-all-approvals`    | flag | off  | 跳过所有工具审批（**仅本次启动**，不写 config）。在每个子命令解析器里被静默 consume。会经 `ha_core::security::dangerous::set_cli_flag(true)` 落到进程内 `AtomicBool`，并向 stderr 打一行 warning。与 `AppConfig.permission.global_yolo` 是 OR 关系，详见 [权限/审批系统](permission-system.md) |

> 注意：Plan Mode 仍然能压过 YOLO 限制工具集；YOLO 只跳审批门控，不放行 protected paths / dangerous commands 之外的禁用工具。

## 桌面模式

```
hope-agent [--child-mode] [--dangerously-skip-all-approvals]
```

| 参数             | 类型 | 默认 | 说明                                                                                                                                                                                |
| ---------------- | ---- | ---- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--child-mode`   | flag | off  | **内部使用**——Guardian 拉起子进程时附带的标记。等价的环境变量 `HOPE_AGENT_CHILD`（任意非空值）保留给老路径。终端用户不应直接指定                                                          |

启动决策树（[`main.rs:36-47`](../../src-tauri/src/main.rs#L36)）：

```
有 --child-mode 或 HOPE_AGENT_CHILD 环境变量 → run_child（直接跑 Tauri）
否则 cfg!(debug_assertions) （dev 构建）   → run_child（跳 Guardian）
否则 config.guardian.enabled == false       → run_child（用户主动禁用）
否则                                       → run_guardian（生产路径）
```

`run_child` 用 `std::panic::catch_unwind` 包裹 `app_lib::run()`，单个进程最多自我重启 `MAX_CHILD_PANICS = 3` 次（[`main.rs:9`](../../src-tauri/src/main.rs#L9)）；超过即退出码 1，由 Guardian 接管下一轮。Guardian 父子协议、退出码语义详见 [可靠性与崩溃自愈](reliability.md)。

## `hope-agent server` 子命令

```
hope-agent server [SUBCOMMAND] [OPTIONS]
```

不带子命令时等价于 `start`，前台启动 HTTP/WS 服务。

### 子命令一览

| 子命令      | 行为                                                                                       | 源码定位                              |
| ----------- | ------------------------------------------------------------------------------------------ | ------------------------------------- |
| _（默认）_  | 前台启动服务，写 PID 文件 `~/.hope-agent/server.pid`，跑完整 `start_background_tasks` 集 | [`main.rs:300-417`](../../src-tauri/src/main.rs#L300)            |
| `install`   | 注册系统服务（macOS launchd / Linux systemd-user），共享下方 `--bind` / `--api-key`      | [`main.rs:458-479`](../../src-tauri/src/main.rs#L458)            |
| `uninstall` | 卸载系统服务                                                                               | [`main.rs:263-271`](../../src-tauri/src/main.rs#L263)            |
| `status`    | 查询服务运行状态（plist load 状态 / systemd unit active 状态）                             | [`main.rs:273-281`](../../src-tauri/src/main.rs#L273)            |
| `stop`      | 停止运行中的服务                                                                           | [`main.rs:283-291`](../../src-tauri/src/main.rs#L283)            |
| `setup`     | 仅运行一次首次启动向导，不启动 HTTP，给运维「先配置后开服」用                              | [`main.rs:293-295`](../../src-tauri/src/main.rs#L293) / `run_server_setup` |

> Windows 不在 `service_install` 当前实现里；Windows 上请走 Task Scheduler 或第三方 supervisor，参见 [Windows 开发指南](../platform/windows-development.md)。

### `start` / `install` 共享选项

由 `parse_server_args` 解析（[`main.rs:420-455`](../../src-tauri/src/main.rs#L420)），`start` 和 `install` 行为完全一致。

| 参数                               | 短选项 | 类型      | 默认             | 说明                                                                                                                                            |
| ---------------------------------- | ------ | --------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `--bind ADDR`                      | `-b`   | host:port | `127.0.0.1:8420` | 绑定地址。**默认仅本机**——远程访问需显式 `0.0.0.0:8420`，并务必同时设置 `--api-key`                                                              |
| `--api-key KEY`                    | —      | string    | _（未设）_       | Bearer Token。请求带 `Authorization: Bearer <key>`，浏览器 WS 用 `?token=<key>` query 参数。**未设置时所有请求放行**（见 [`ha-server` 中间件](../../crates/ha-server/src/middleware.rs)），`/api/health` 永远公开 |
| `--dangerously-skip-all-approvals` | —      | flag      | off              | 在 server 子命令解析器中被静默 consume，由顶层全局开关已经生效                                                                                  |
| `--version`                        | —      | flag      | —                | 打印 `hope-agent-server X.Y.Z` 后退出（取自 `CARGO_PKG_VERSION`）                                                                              |
| `--help` / `-h`                    | —      | flag      | —                | 打印帮助后退出                                                                                                                                  |

`start` 启动序列：

1. `paths::ensure_dirs()` 创建 `~/.hope-agent/` 子目录
2. 检查 onboarding 状态——TTY 下跑交互向导（`cli_onboarding::run_wizard`），非 TTY（systemd / Docker / 管道 stdin）打印 unconfigured notice 后用默认值继续
3. `agent_loader::ensure_default_agent()` 兜底创建默认 agent
4. `init_runtime("server")` 打开所有 DB / OnceLock / EventBus / Channel 插件 / ACP control plane
5. 写 PID → 启动 `start_background_tasks`（channel 监听、cron 调度、dreaming、MCP watchdog 等完整集）→ `ha_server::start_server`
6. 退出时清 PID

详见 [前后端分离架构](backend-separation.md) 与 [进程与并发模型](process-model.md)。

### `setup` 选项

`hope-agent server setup [--reset]`，由 `run_server_setup` 处理（[`main.rs:484-525`](../../src-tauri/src/main.rs#L484)）。

| 参数            | 说明                                                                                            |
| --------------- | ----------------------------------------------------------------------------------------------- |
| `--reset`       | 跑向导前先调 `onboarding::state::reset()` 清除 onboarding 状态。**Provider / config 不删**，仅重放向导 |
| `--help` / `-h` | 打印帮助后退出                                                                                  |

### `install` 平台行为

[`crates/ha-core/src/service_install.rs`](../../crates/ha-core/src/service_install.rs)：

| 平台    | 服务管理器     | 文件位置                                              | 说明                            |
| ------- | -------------- | ----------------------------------------------------- | ------------------------------- |
| macOS   | launchd        | `~/Library/LaunchAgents/com.shiwenwen.hope-agent.plist` | 登录时自动启动                  |
| Linux   | systemd (user) | `~/.config/systemd/user/hope-agent.service`           | 通过 `systemctl --user` 管理    |
| Windows | _未实现_       | —                                                     | 用 Task Scheduler 或外部 supervisor |

## `hope-agent acp` 子命令

```
hope-agent acp [OPTIONS]
```

由 `run_acp_server` 处理（[`main.rs:130-252`](../../src-tauri/src/main.rs#L130)）。NDJSON over stdio，给 IDE / 外部 ACP 客户端直连用。

| 参数                                  | 短选项 | 类型   | 默认        | 说明                                                                                                |
| ------------------------------------- | ------ | ------ | ----------- | --------------------------------------------------------------------------------------------------- |
| `--verbose` / `-v`                    | `-v`   | flag   | off         | 在 stderr 打印启动 banner（版本 / agent id / 协议）                                                |
| `--agent-id ID` / `-a ID`             | `-a`   | string | `"default"` | 指定使用哪个 agent。**不存在时不会兜底**——会在 ACP 会话内拿到错误                                  |
| `--dangerously-skip-all-approvals`    | —      | flag   | off         | 同全局，被 acp 解析器静默 consume                                                                   |
| `--version`                           | —      | flag   | —           | 打印 `hope-agent-acp X.Y.Z` 后退出                                                                |
| `--help` / `-h`                       | —      | flag   | —           | 打印帮助后退出                                                                                      |

启动顺序：

1. **Onboarding 检查**（[`main.rs:187-198`](../../src-tauri/src/main.rs#L187)）：未完成时打印错误后 **退出码 2**，引导用户去 `hope-agent server setup` 或桌面 app。ACP stdio 是协议通道，不能弹向导
2. `init_runtime("acp")` 打开所有 DB / 单例 / channel 插件
3. 起独立两线程 tokio runtime（命名 `acp-bg`）跑 `start_minimal_background_tasks`——只挂 IM channel approval / ask_user listener、async_jobs replay、MCP `init_global`，不跑日时器、cron、dreaming、watchdog
4. `app_lib::acp::server::start(...)` 阻塞读 stdin；每次 `session/prompt` 内部建独立 current-thread runtime，避开嵌套 `block_on`
5. ACP 主循环返回前 drop `bg_rt`，让 listener 看到 cancel 后干净退出

详见 [ACP 协议](acp.md)。

## `hope-agent auth` 子命令

```
hope-agent auth <provider> <action> [OPTIONS]
```

由 [`cli_auth::run`](../../src-tauri/src/cli_auth.rs) 处理，是当前唯一**一次性**子命令——不进 `init_runtime`、不起后台 tokio runtime、不开 EventBus，只为终端用户完成主 LLM Provider 的 OAuth 流程后退出。设计上和 [MCP OAuth](mcp.md) 各自独立（`oauth.rs` vs `mcp/oauth.rs`，互不共用）。

### Provider

| Provider | 状态     | 入口                              | 说明                                                                  |
| -------- | -------- | --------------------------------- | --------------------------------------------------------------------- |
| `codex`  | 已支持   | `hope-agent auth codex <action>`  | ChatGPT / Codex OAuth。token 落 `~/.hope-agent/credentials/auth.json` |

未来扩展更多 Provider 时按 `cli_auth::run` 中的 match 分支扩展即可。

### `auth codex` 动作

| 动作       | 说明                                                                                                                                                                                           |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `login`    | OAuth 浏览器登录，成功后保存 token + 通过 `provider::ensure_codex_provider_persisted` 把 Codex Provider 写进 `config.json`。**不带动作时默认就是 `login`** |
| `status`   | 打印 token 状态：`authenticated` / `expired` / `not authenticated`，附 account id、token 路径、refresh token 是否存在                                                                          |
| `logout`   | 调 `provider::delete_providers_by_api_type(Codex, "cli")` 清掉所有 Codex Provider 行，再调 `oauth::clear_token()` 删 `auth.json`                                                                |

### `auth codex login` 选项

| 参数              | 类型   | 默认       | 说明                                                                                                                       |
| ----------------- | ------ | ---------- | -------------------------------------------------------------------------------------------------------------------------- |
| `--no-open`       | flag   | `--open`   | 只打印 auth URL 不自动开浏览器。SSH / headless 环境必备                                                                    |
| `--open`          | flag   | （默认）   | 显式启用浏览器自动打开                                                                                                    |
| `--model MODEL`   | string | `gpt-5.4`  | 登录成功后切到该 Codex 模型作为 active model。模型名通过 `agent::is_valid_codex_model` 校验，未知模型会列出可选项后报错      |
| `--no-active`     | flag   | off        | 登录成功后**不**切 active model。`make_active=true` 时实际写入 `ActiveModelUpdate::Always(model)`，否则写 `Never`            |
| `--help` / `-h`   | flag   | —          | 打印帮助后退出                                                                                                             |

OAuth 回调走本机 loopback `http://localhost:1455/auth/callback`，远端 SSH 场景需要在客户端先建端口转发：

```
ssh -L 1455:127.0.0.1:1455 <host>
```

`login` 内部新建独立 tokio runtime（`tokio::runtime::Runtime::new()`）跑 `oauth::start_oauth_flow_with_auth_url`；500ms 轮询一次共享 slot 直到拿到 token 或者用户 Ctrl-C。流程结束后 runtime drop，进程退出。

### `auth codex status` / `auth codex logout` 选项

两个动作都只接受 `--help` / `-h`，不接受其它参数；多余参数会报 `unknown status option` / `unknown logout option` 并退出码 1。

### 已知行为细节

- **`logout` 是破坏性的**：会真正从 `config.json` 删 Codex Provider 行，不只是清 token。重新登录会重建 Provider
- **`login` 复用 onboarding wizard 的 OAuth 实现**：[`crates/ha-core/src/oauth.rs`](../../crates/ha-core/src/oauth.rs) 的 `start_oauth_flow_with_auth_url` 同时给 `cli_auth` 和 `cli_onboarding::steps::provider` 用——首次启动向导里选 Codex 时走的就是同一条路径
- **`--version`**：`hope-agent auth --version` 打印 `hope-agent-auth X.Y.Z` 后退出
- **未知 provider 退出码 2**：`hope-agent auth foo` 会报错并退出码 2，与 ACP onboarding 退出码 2 区分语义但共享数字

## 退出码语义

| 退出码 | 触发                                                                                                          |
| ------ | ------------------------------------------------------------------------------------------------------------- |
| 0      | 正常退出（用户关窗 / Ctrl-C / `--version` / `--help` / `auth codex login` 成功 / `auth codex status` 完成） |
| 1      | 通用错误：服务管理失败 / wizard 失败 / server 启动失败 / 子进程超过 `MAX_CHILD_PANICS` / `auth codex` 任意动作失败 |
| 2      | ACP 模式 onboarding 未完成（无法在 stdio 上交互） / `hope-agent auth <unknown_provider>`                          |

Guardian 父子层之间还有自定义退出码协议（崩溃 vs 用户主动退出 vs 重启请求），详见 [可靠性与崩溃自愈](reliability.md)。

## 环境变量

CLI 直接消费或路径相关的环境变量。完整跨子系统列表分散在各架构文档中。

| 变量                              | 角色             | 说明                                                                                                                                |
| --------------------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `HA_DATA_DIR`                     | 用户             | 覆盖数据根目录（默认 `~/.hope-agent/`）。**值整体当作根目录用**，不会再追加 `.hope-agent` 后缀；适合便携模式 / 集成测试。详见 [`paths.rs`](../../crates/ha-core/src/paths.rs) |
| `HA_WEB_ROOT`                     | 用户（开发）     | server 模式下让 axum 静态托管指向本地 `dist/` 目录而不是嵌入产物——开发时改前端不用每次重打包。设置后会检查 `index.html` 是否存在，缺失则降级回嵌入产物 |
| `HOPE_AGENT_CHILD`                | 内部（兼容）     | 等价于 `--child-mode`，给老 Guardian 路径留的兼容入口                                                                              |
| `HOPE_AGENT_RECOVERED`            | 内部（Guardian） | Guardian 在 panic 重启子进程时设为 `"1"`，提示这是恢复启动                                                                          |
| `HOPE_AGENT_CRASH_COUNT`          | 内部（Guardian） | Guardian 重启子进程时把累计崩溃次数（数字字符串）传给子                                                                            |
| `HOPE_AGENT_BUNDLED_SKILLS_DIR`   | 用户/打包者      | 覆盖 bundled skills 目录。优先级 `env > exe 同级 / 上级 > CARGO_MANIFEST_DIR`                                                       |

## 数据目录速查

完整路径管理在 [`crates/ha-core/src/paths.rs`](../../crates/ha-core/src/paths.rs)。所有路径相对 `HA_DATA_DIR` 或默认 `~/.hope-agent/`。

| 路径                              | 用途                                                |
| --------------------------------- | --------------------------------------------------- |
| `config.json`                     | 主配置（详见 [配置系统](config-system.md)）         |
| `agents/`                         | 每 Agent 状态、`memory.md`、soul.md                 |
| `credentials/`                    | OAuth token、MCP 凭据（0600 原子写）                |
| `channels/`                       | IM 渠道插件状态                                     |
| `permission/`                     | 保护路径 / 危险命令 / 编辑命令 / AllowAlways 列表 |
| `skills/`                         | 用户自定义 skill                                    |
| `plans/<agent_id>/<session_id>/`  | Plan Mode 设计契约文件（详见 [Plan Mode](plan-mode.md)） |
| `tool_results/<session_id>/`     | 大工具结果落盘                                      |
| `attachments/<session_id>/`      | IM / 多模态附件归档                                 |
| `async_jobs/`                     | 后台异步工具任务 spool                              |
| `local_model_jobs.db`             | 本地模型后台任务（Ollama 安装、模型拉取）            |
| `recap/recap.db`                  | 深度复盘缓存                                        |
| `memory/dreams/`                  | Dreaming diary markdown                             |
| `server.pid`                      | server 模式运行时 PID                               |

## 其它入口

下面这些 CLI 在 `pnpm` 脚本里调用，不是 `hope-agent` 二进制本身的子命令，但同样属于「项目命令行接口」：

| 命令                                       | 用途                                                     | 来源                                                                 |
| ------------------------------------------ | -------------------------------------------------------- | -------------------------------------------------------------------- |
| `pnpm tauri dev`                           | 桌面 dev（前端 + Tauri 热重载）                          | [`package.json`](../../package.json)                                 |
| `pnpm dev`                                 | 仅前端 Vite 开发服务器                                   | 同上                                                                 |
| `pnpm tauri build`                         | 构建桌面生产包                                           | 同上                                                                 |
| `pnpm sync:version`                        | 把 `package.json` 版本同步到 `src-tauri`                 | [`scripts/sync-version.mjs`](../../scripts/sync-version.mjs)         |
| `pnpm release:verify`                      | 校验 `package.json` / `src-tauri` 版本一致               | 同上                                                                 |
| `pnpm typecheck` / `lint` / `test`         | 前端类型检查 / lint / Vitest                             | [`package.json`](../../package.json)                                 |
| `node scripts/sync-i18n.mjs --check`       | 检查各语言翻译缺失                                       | [`scripts/sync-i18n.mjs`](../../scripts/sync-i18n.mjs)               |
| `node scripts/sync-i18n.mjs --apply`       | 从基础语言补齐缺失翻译                                   | 同上                                                                 |

提交前自查脚本（[`AGENTS.md`](../../AGENTS.md) 强制）由 [`.husky/pre-push`](../../.husky/pre-push) 钩子在 `git push` 时跑：`cargo fmt --all --check`、`cargo clippy -p ha-core -p ha-server --all-targets --locked -- -D warnings`、`cargo test -p ha-core -p ha-server --locked`、`pnpm typecheck`、`pnpm lint`、`pnpm test`。

## 已知边界

- **没有 clap 也没有 shell completion**：参数解析手写 `std::env::args()`，未知参数只 stderr 警告不报错（[`main.rs:168`](../../src-tauri/src/main.rs#L168) / [`main.rs:449`](../../src-tauri/src/main.rs#L449) / [`main.rs:500`](../../src-tauri/src/main.rs#L500)）。引入新参数前要么继续手写并维护本文档，要么切到 clap-derive
- **桌面模式无 `--version` / `--help`**：只有 `server` 与 `acp` 子命令实现了。桌面入口的版本信息在 GUI 内显示，CLI `hope-agent --version`（不带子命令）会被当成未知参数进入 Tauri 启动流程
- **Windows 缺 `server install`**：[`service_install.rs`](../../crates/ha-core/src/service_install.rs) 的 install/uninstall/status/stop 在 Windows 上没有对应实现，运维需要自行用 Task Scheduler / NSSM 包装
- **`server install` 不持久化 `--dangerously-skip-all-approvals`**：YOLO 是进程内 `AtomicBool`，不进 plist / unit；想让服务永远 YOLO 必须改 `AppConfig.permission.global_yolo`
- **未知子命令静默落到默认路径**：`hope-agent typo` 不会报错，会被当成「桌面模式」启动 Tauri。这是手写 arg 解析的副作用，引入 clap 时一并修

# App Lifecycle (Restart)

让 Agent 通过对话重启自己的进程，跨桌面 / 已安装服务 / 前台 server / ACP 四种运行形态。`app_restart` 工具 + `/restart` 斜杠命令 + AboutPanel "重启应用" 按钮三个入口共用一套底层 ([`ha_core::lifecycle`](../../crates/ha-core/src/lifecycle/mod.rs))。

## 形态路由

[`lifecycle::route()`](../../crates/ha-core/src/lifecycle/mod.rs) 按 [`runtime_role()`](../../crates/ha-core/src/app_init.rs) + [`service_install::is_service_installed()`](../../crates/ha-core/src/service_install.rs) 决策：

| `runtime_role` | service_installed | `Route`       | 实际重启动作 |
| -------------- | ----------------- | ------------- | ----------- |
| `desktop`      | —                 | `Desktop`     | `AppLifecycleBridge::restart_desktop()` → Tauri `app.exit(42)` → guardian respawn |
| `server`       | `true`            | `Service`     | `updater::service_control::restart_service()`（launchctl / systemctl --user / schtasks End+Run） |
| `server`       | `false`           | `Respawn`     | `respawn_detached_server()` 起 detached 子进程 → `schedule_self_exit()`（200ms 后 exit 0） |
| `acp` / `None` | —                 | `Unsupported` | 拒绝；ACP stdio 由 IDE 持有，重启意味着 IDE 自己 re-spawn |

不同 Route 走的是不同 OS 设施，但都满足"几百 ms 内当前进程消失、新实例自动起"的契约。

### Desktop：`AppLifecycleBridge`

ha-core 完全零 Tauri 依赖，所以桌面侧通过 trait + OnceLock 反向注册：

- [`crates/ha-core/src/lifecycle/mod.rs`](../../crates/ha-core/src/lifecycle/mod.rs) 定义 `trait AppLifecycleBridge { fn restart_desktop(&self) -> Result<()>; }` + `set/get_lifecycle_bridge`
- [`src-tauri/src/commands/lifecycle_bridge.rs`](../../src-tauri/src/commands/lifecycle_bridge.rs) 实现 `TauriLifecycleBridge` 并在 [`setup.rs`](../../src-tauri/src/setup.rs) 里注册
- 实现就是 `self.handle.exit(42)`；guardian（[`ha_core::guardian::run_guardian`](../../crates/ha-core/src/guardian.rs)）catch 退出码 42 并重新 fork 子进程

`crash.rs#request_app_restart` Tauri command 现在也走 `lifecycle::restart()`，bridge 没注册时兜底 `app.exit(42)`。这样桌面 "Cmd+Q / Restart" 与 `app_restart` 工具走同一条代码路径。

### Service：复用 self-update 的 `service_control`

[`updater::service_control::restart_service`](../../crates/ha-core/src/updater/service_control.rs) 是自升级流程已经在用的工具——同样的 launchctl / systemctl / schtasks 三件套。restart 路径直接复用，**禁止重写**。

### Respawn：`detach + 自杀`

前台 `hope-agent server start`（无 launchd / systemd / schtasks 接管）时，进程一旦死了就没人拉。所以需要：

1. **拿启动 argv**：[`app_init::set_server_launch_args`](../../crates/ha-core/src/app_init.rs) 在 [main.rs run_server](../../src-tauri/src/main.rs) 启动期把 `--bind` / `--api-key` 等参数存进 `OnceLock`。
2. **spawn detached 子进程**：[`respawn.rs`](../../crates/ha-core/src/lifecycle/respawn.rs) 调 `Command::new(current_exe()).arg("server").args(captured_argv)`：
   - Unix：`pre_exec(setsid)` 切到新 session（脱离 controlling TTY，父 shell Ctrl-C 不会传播过来）
   - Windows：`CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`
   - 三流全部重定向到 `Stdio::null()`，子进程对父终端完全透明
3. **延迟自杀**：`schedule_self_exit()` 起独立 OS 线程 sleep 200ms 后 `std::process::exit(0)`。grace 时间让 EventBus emit / 工具结果 flush 完成；子进程的 HTTP 服务器自己 retry bind socket，200ms 重叠窗口可以接受。

**只支持 `runtime_role == "server"`**——前台桌面如果没装 guardian 也能 respawn，但桌面 GUI 的 Tauri webview 状态不在 argv 里，重启等于丢界面状态，意义不大，所以 Desktop 永远走 Tauri 路径。

### ACP：拒绝

`hope-agent acp` 通过 stdio 与 IDE 通信。重启会导致 stdin/stdout 管道断开，IDE 那侧需要主动 re-spawn 才行。工具返回 `status: "unsupported_mode"`；skill 文档里指引用户在 IDE 端操作（关闭/重开 agent 面板）。

## 工具 `app_restart`

[`crates/ha-core/src/tools/app_restart.rs`](../../crates/ha-core/src/tools/app_restart.rs) + schema [`tools/definitions/restart_tools.rs`](../../crates/ha-core/src/tools/definitions/restart_tools.rs)。

- **Tier**：`Core { subclass: Meta }`、`internal: false`、`async_capable: false`、`concurrent_safe: false`
- **action**：保留字段；当前只接受 `"restart"` 或省略。预留 stop / start 但**不打算做**——单独 stop 后无人 start 是 footgun
- **审批模型**：不挂 `AskReason::DangerousCommand`——重启不属于"危险命令"，且需要承载 pre-flight 信息。工具内部连发 1~2 个 `ask_user_question`：
  1. **pre-flight**（仅当 `collect_inflight()` 非空时）：列举在飞的 chat turn / async tool job / running cron job，问"会中断这些事，仍要继续吗？"
  2. **confirmation**（永远问）：带 Route label 的 Yes/No，让用户看清楚会走哪条路径

两道关都不走权限引擎，**Plan / YOLO / Global YOLO 都无法跳过**。

## Pre-flight 扫描 `collect_inflight`

[`crates/ha-core/src/lifecycle/inflight.rs`](../../crates/ha-core/src/lifecycle/inflight.rs) 扫三个源：

| 源              | 实现                                                                 |
| --------------- | -------------------------------------------------------------------- |
| 活跃 chat turn  | [`chat_engine::active_turn::all_current`](../../crates/ha-core/src/chat_engine/active_turn.rs)（内存 registry） |
| 异步工具 job    | [`async_jobs::AsyncJobsDB::list_running`](../../crates/ha-core/src/async_jobs/db.rs)（SQL `status IN ('running','cancelling')`） |
| Cron job        | [`cron::CronDB::list_running_jobs`](../../crates/ha-core/src/cron/db.rs)（SQL `running_at IS NOT NULL`） |

**故意不扫**：IM 上传/下载附件——目前没有 in-memory inflight registry，单为重启加一个不划算。

每项最多 8 条在 prompt 里铺开，剩余折叠成 "... and N more"。

## EventBus

`app:restart_initiated`（`route`, `pid`）——在 `lifecycle::restart()` 真正 handoff 之前 emit。订阅者：

- 前端 toast / 状态栏可以显示"正在重启..."
- 测试 / 日志归档可以记录重启原因
- IM channel 自己有 `start_watchdog`，不需要这个事件协调重连

## 入口对照

| 入口               | 实现                                                                                |
| ------------------ | ----------------------------------------------------------------------------------- |
| 模型对话           | `app_restart` 工具，两道 `ask_user_question`                                        |
| 用户输斜杠         | `/restart`（aliases: restart, reboot），[`skills/ha-restart/SKILL.md`](../../skills/ha-restart/SKILL.md) |
| GUI 按钮           | AboutPanel "重启应用"，走 Transport `request_app_restart` → `/api/system/restart` 或 Tauri command |
| Tauri command      | `request_app_restart` → `lifecycle::restart()`（bridge 没注册时兜底 `exit(42)`）   |
| HTTP `POST /api/system/restart` | 直接调 `lifecycle::restart()`，**不走 pre-flight / 确认**——GUI 自己弹 AlertDialog，HTTP 调用方负责自己的 UX |

## 与自升级（self-update）的关系

`updater::self_contained::install` 结束时也调 `service_control::restart_service()`，与本模块共享同一个底层 helper。但**升级路径不复用 `lifecycle::restart`**：升级是"binary 已经在原子 swap 之后，让 OS supervisor 起新版本"，没有 pre-flight 必要（升级流程自己有 ask_user_question 确认）。本模块是"binary 没变，只是重启进程"——单独的场景需要单独的 UX。

## 不在范围

- **stop / start 独立动作**：上面已说明，不做
- **重启时序保存对话状态**：streaming turn 持久化已经在 chat_engine 那侧落 SQLite（`stream_status = 'streaming'`），重启后下次启动的 `recover_startup_session_state` 会把它们 mark 为 `orphaned`。Pre-flight 告诉用户有这件事，剩下的就交给已有的恢复路径
- **桌面恢复 webview 状态**：Tauri 重启之后 webview reload，前端 SSE / WS 自动 reconnect。不需要专门保存什么
- **跨用户重启系统服务**：`launchctl --user` / `systemctl --user` 只管当前用户的 service，不影响系统范围的进程。这是 Hope Agent 服务安装位置（`~/Library/LaunchAgents/` / `~/.config/systemd/user/`）天然限制

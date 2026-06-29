# 后台任务（Background Jobs）系统架构

> 返回 [文档索引](../README.md) | 更新时间：2026-06-24

## 概述

后台任务子系统把「异步工具执行 / 后台 subagent / 批量 fan-out」统一进**一张表 + 一个门面**：表/文件 `~/.hope-agent/background_jobs.db`（表名 `background_jobs`），单一入口 [`async_jobs::JobManager`](../../crates/ha-core/src/async_jobs/manager.rs)。三类任务由 `JobKind`（`Tool` / `Subagent` / `Group`）区分，共享同一套生命周期、并发配额、取消、注入与重放（replay）路径。

**命名分裂（历史契约，勿改）**：Rust 模块名与日志 `category` 都是 `async_jobs`，DB 文件却是 `background_jobs`。诊断时按 `category='async_jobs'` grep，但 schema 探针看 `background_jobs` 表。EventBus 生命周期事件用 `job:*` 前缀（区别于 `async_jobs` 日志 category）。`RuntimeTaskKind::AsyncJob`（[`runtime_tasks.rs`](../../crates/ha-core/src/runtime_tasks.rs)）是统一取消入口的 kind 名。

> 本文是后台任务实现细节的单一真相源；[AGENTS.md](../../AGENTS.md) 只保留跨 PR 契约面。**后台 subagent 投影（R6）与 Group fan-out（R5）的 subagent 侧逻辑**见 [子 Agent 系统](subagent.md)；本文只覆盖它们在后台任务表里的投影与协调。

## 模块结构

| 文件 | 职责 |
|------|------|
| [`async_jobs/mod.rs`](../../crates/ha-core/src/async_jobs/mod.rs) | 模块入口、全局静态（DB / 调度器 once-gate）、`cancel_job`、`replay_pending_jobs`（Primary-only 重放） |
| [`async_jobs/manager.rs`](../../crates/ha-core/src/async_jobs/manager.rs) | `JobManager` 零尺寸门面：spawn / cancel / list / 快照 / 重放 / 调度 / R5 Group / R6 投影 |
| [`async_jobs/types.rs`](../../crates/ha-core/src/async_jobs/types.rs) | `JobKind` / `JobStatus` / `JobOrigin` / `BackgroundJob` / `BackgroundJobSnapshot` |
| [`async_jobs/error.rs`](../../crates/ha-core/src/async_jobs/error.rs) | `JobError`（Cancelled / TimedOut / DeniedByUser / Failed）+ `to_status()` / 注入文案 |
| [`async_jobs/db.rs`](../../crates/ha-core/src/async_jobs/db.rs) | `JobsDB`：表 DDL、状态转移（守卫式）、投影 / Group / spool / 重放 / purge 查询 |
| [`async_jobs/spawn.rs`](../../crates/ha-core/src/async_jobs/spawn.rs) | spawn 两路 + `start_runner` + `run_tool_once` / `run_tool_with_retry` + `finalize_job` + `run_scheduler` |
| [`async_jobs/slots.rs`](../../crates/ha-core/src/async_jobs/slots.rs) | `SlotManager` 两层配额 + 队列 + `SlotReservation`（RAII）+ `pick_fair_index` 公平调度 |
| [`async_jobs/retry.rs`](../../crates/ha-core/src/async_jobs/retry.rs) | 纯策略 `decide()` + `is_retry_eligible` 代码级白名单 |
| [`async_jobs/approval_bridge.rs`](../../crates/ha-core/src/async_jobs/approval_bridge.rs) | R8 后台 exec 审批桥（thread-local park/resume + 预算排除） |
| [`async_jobs/approval_projection_watcher.rs`](../../crates/ha-core/src/async_jobs/approval_projection_watcher.rs) | R8 follow-up：EventBus watcher 给后台 subagent 内层审批补投影 label |
| [`async_jobs/injection.rs`](../../crates/ha-core/src/async_jobs/injection.rs) | 完成注入 + R4 合并窗口 + ghost-turn 闸 + 逐 job 恰好一次 |
| [`async_jobs/output_tail.rs`](../../crates/ha-core/src/async_jobs/output_tail.rs) | R3① 运行中 exec 输出有界 ring |
| [`async_jobs/events.rs`](../../crates/ha-core/src/async_jobs/events.rs) | `job:*` 事件 helper |
| [`async_jobs/wait.rs`](../../crates/ha-core/src/async_jobs/wait.rs) | `job_status` 短便利同步（block）的等待 / 唤醒 |
| [`async_jobs/cancel.rs`](../../crates/ha-core/src/async_jobs/cancel.rs) | 进程内 `CancellationToken` 注册表（best-effort，DB 为持久真相） |
| [`async_jobs/retention.rs`](../../crates/ha-core/src/async_jobs/retention.rs) | 终态行按龄 GC + spool 孤儿清理 |

## 数据模型

### `JobKind`（三类）

| kind | 含义 |
|------|------|
| `Tool` | 单次工具调用（默认；unknown / legacy 值 `parse()` 回落为 `Tool`） |
| `Subagent` | 用户委派的后台 subagent 在后台任务表里的**单向投影**（R6，真相在 `subagent_runs`） |
| `Group` | `batch_spawn` 的 fan-out join 协调行（R5），关联 N 个子投影、合并注入一轮 |

### `JobStatus`（九态）

非终态：`Queued`（等槽）、`Running`（执行中）、`Cancelling`（已发取消信号、future 收尾中）、`AwaitingApproval`（R8 停在审批点）。
终态：`Completed`、`Failed`、`Interrupted`（重启幸存者）、`TimedOut`、`Cancelled`。

终态集合是单一真相源 `TERMINAL_STATUS_SQL_LIST = ('completed','failed','interrupted','timed_out','cancelled')`，purge / replay / active filter 都引它。状态转移**守卫式**：`mark_running` 仅 `Queued→Running`；`mark_cancelling` / `update_terminal` 仅接受 `Queued|Running|Cancelling|AwaitingApproval`→终态（并发 finalize 竞争安全）；`resume_from_awaiting_approval` 仅 `AwaitingApproval→Running`。**新增非终态状态须同步这些 active filter 的 WHERE。**

### `JobOrigin`（为何后台化）vs `approval_origin`（如何被授权）

两者是不同的列，勿混：

- `origin`（`JobOrigin`，types.rs）：`Explicit`（`run_in_background:true`）/ `PolicyForced`（agent 强制后台）/ `AutoBackgrounded`（超同步预算自动转后台）。
- `approval_origin`：授权方式审计（F6），类型 `ApprovalOrigin`（[`tools/approval.rs`](../../crates/ha-core/src/tools/approval.rs)，7 变体：`user` / `timeout_proceed` / `unattended_proceed` / `yolo` / `auto_approve` / `external_pre_approved` / `policy_allow`）。spawn 时写占位（命令 gate 还没跑），审批 resolve（proceed）经 `set_approval_origin` 修正；终态冻结。

### `JobError`

四变体（error.rs）：`Cancelled`（token 触发）/ `TimedOut{max_secs}` / `DeniedByUser{rejection}`（保留 `ToolRejection` 以承载 STOP 语义）/ `Failed{message}`。`to_status()`：`Cancelled→Cancelled`、`TimedOut→TimedOut`、`DeniedByUser|Failed→Failed`。仅 `Failed` 可进重试路径（见 [重试](#重试r74)）。

### `background_jobs` 表（21 列）

`job_id`(PK) · `session_id` · `agent_id` · `tool_name` · `tool_call_id` · `args_json`（无痕脱敏占位）· `status` · `result_preview` · `result_path`（spool）· `error` · `created_at` · `completed_at` · `injected`(bool) · `origin` · `approval_origin` · `incognito`(bool) · `pid`（I3 孤儿追踪）· `cancel_requested`(bool，I4 跨进程）· `kind` · `subagent_run_id`（R6 FK）· `group_id`（R5 归属）。索引：`(session_id,status)` / `(status,injected)` / `(subagent_run_id)` / `(group_id)`。

**Schema 是纯可重建缓存**：探针看最新列 `group_id`（R5），缺则整表 DROP 重建（**无迁移**，drop-rebuild 零成本）；unknown `status`/`kind` 解析回落默认。`progress` / `priority` / `attempt` 等列待对应 slice 消费时再加。

### `BackgroundJobSnapshot`（owner 面展示向）

camelCase、只读、与 model-facing JSON 物理分离：`jobId` · `kind` · `status` · `tool`（原名）· `label`（展示）· `origin` · `sessionId` · 时间戳 · `error` · `resultPreview`（无痕 redact）· Group 专属 `childCount`/`childrenTerminal`/`childrenCompleted`/`childrenFailed` · `subagentRunId` · `outputTail`（running exec 实时尾巴）。

## 任务生命周期

### Spawn 两路

| 路径 | 入口 | 行为 |
|------|------|------|
| **显式后台** | `spawn_explicit_job`（spawn.rs，`JobManager::spawn_tool`） | `run_in_background:true` 或 always-background policy。预分配 job_id、注册 cancel token、落行（Running/Queued）、试 `try_reserve` 占槽，满则入队。立即返回 `synthetic_started_result`（`{job_id, status:"started", tool, origin, hint}`），**绝不内联真实结果** |
| **自动转后台** | `dispatch_with_auto_background`（spawn.rs，`JobManager::dispatch_tool_with_auto_background`） | sync-capable 工具在独立 OS 线程跑、主线程按 `auto_background_secs` 预算计时。预算内完成→内联返回真实结果；超预算→落行、`reserve_forced` 强占槽（任务已在跑、不排队不拒绝）、emit `job:created`、返回 synthetic，worker 自行 finalize |

`async_capable=true` 的工具：`exec` / `browser` / `web_search` / `image_generate`（外加 `app_update`）。

**exec 收敛（process 兼容面）**：普通长跑 exec 统一进 `async_jobs`。当 `exec(background=true)` / `exec(yield_ms=...)` 出现在 async_tools 开启且 agent 未禁用后台的上下文里，执行入口会把它兼容迁移为 `run_in_background=true`，移除 legacy process flags，让 `JobManager` 持有唯一后台生命周期。只有 async_tools 关闭 / agent `never-background` 等兼容场景继续返回 process `session_id`；这些 process session 退出时走 `<process-notification>`，不冒充 async job。

### 执行

`start_runner`（spawn.rs）起一条 **OS 线程 + current-thread tokio runtime**（非 Send，镜像 subagent 注入），持 `SlotReservation` 直到 job 终态（任何退出路径 Drop 释放槽 + 唤醒调度器）。线程内 `run_job_to_completion`：装审批桥（R8）→ 起跨进程取消 watcher（I4）→ 跑 `run_tool_with_retry`（R7.4）→ `finalize_job`。

- `run_tool_once`：把工具 future 与 cancel token、可选 `max_secs` 预算定时器 `select!`，返回 typed `JobError`。**审批等待经 `parked_budget_extension` 移出 `max_job_secs` 预算**（守 ASYNC-2）。
- `run_tool_with_retry`：仅对 `JobError::Failed` 且 retry-eligible 工具循环重投（见 [重试](#重试r74)），退避可被 cancel 打断。

### Finalize 与重放

`finalize_job`：持久化结果/错误（`persist_result` 按 `inline_result_bytes`=4KB 截断 head 2/3 + tail 1/3，超出 spool 到盘，无痕跳 spool）→ `update_terminal` → 触发终态 hook（H4）→ 清 cancel token + output_tail ring → `enqueue_injection`（见 [完成注入](#完成注入与合并窗口r4)）。`Cancelled` 立即标 `injected`（无父回合、不双发）；无父 session 的也标 injected（重放 no-op）。

`replay_pending_jobs`（mod.rs，**Primary-only**，共享 DB 防 Secondary 双投）：重启把残留 `Running`→`Interrupted`（孤儿清理 I3：pid 仍活则终止进程树）；对终态但未注入的逐条 `dispatch_injection`（**不走合并窗口**）+ 补触发 hook（H6）。

## 并发与配额（R7.1）

`SlotManager`（slots.rs，进程级 `LazyLock` static）持两层硬配额 + 一条有界队列（`VecDeque<PreparedJob>`，持不可持久化的 live `ToolExecContext`）。

- **两层准入**：`try_reserve` 经 `reserve_inner` 同查全局 `max_concurrent_jobs` 与每会话 `max_concurrent_jobs_per_session`，**两层都要有空位**才发 `SlotReservation`；任一满则**入队**（status `Queued`，非拒绝）。`0` = 该层不限。
- **FIFO 公平**：`try_reserve` 先看 `!queue.is_empty()` —— 队列非空时新 spawn 也不得插队（即使技术上有空槽）。
- **队列有界**：`enqueue` 在 `len() >= max_queued_jobs`（钳 [1,4096]，`0` 钳到地板 1，**非无限**——队列钉住 RAM 里的 live ctx）时返 false，调用方硬拒并回滚行/token。
- **每会话公平提升**：进程级调度器 `run_scheduler`（spawn.rs，进程内幂等、`SCHED_NOTIFY` + 5s tick 唤醒）槽空时 `pick_fair_index` 选「running 最少（并列取最老）」的会话的队首，**跳过已达每会话上限的会话**（防 head-of-line）。
- **RAII 释放**：`SlotReservation` Drop **必须** `notify_one()` 唤醒调度器（panic-safe Drop）。
- **auto-bg 强占**：`reserve_forced` 无配额闸、无条件计数（任务已在 worker 线程跑），可短暂超全局**及每会话** cap——每会话 cap 约束的是新任务**准入**，不是已在跑的 auto-detach 数。
- **per-process**：调度器与队列每进程一个、tier-agnostic；桌面与 server 各跑各的、互不协调；队列重启即失（recover 为 `Interrupted`）。与 `replay_pending_jobs` 的 Primary-only 相反。

**per-kind 双域分治（勿合并成单一配额表）**：tool 池在此；后台 subagent 池在 [`subagent::queue`](subagent.md)（per-session 排队，默认 8 `subagents.maxConcurrent`，R7.2）；同轮安全工具走信号量 8（R7.3）。结构类上限（depth / batch / turn）**硬拒不排队**，只资源类（过并发）排队。

## 重试（R7.4）

默认关闭（opt-in），纯策略 `retry::decide(tool, attempt, error, cfg) → RetryDecision`（`Stop` | `Retry{backoff_ms}`），无 DB 可穷举单测。

- **仅 `JobError::Failed`** 可进重试；`Cancelled` / `DeniedByUser` / `TimedOut` 永不重投（match 前置 return `Stop`）。
- **eligibility 是代码级白名单** `is_retry_eligible` = `web_search` / `web_fetch`（幂等可安全重跑），**非用户旋钮**；`exec`（副作用）/ `image_generate`（非确定性 + 重计费）由设计排除。
- **默认关的理由**：eligible 多为计费供应商，重投会重计费，故交用户按需开。
- 退避：`500ms × 2^min(attempt-1, 6)`，饱和于 32s，不可配（防 typo 造成多分钟 stall）；退避期间可被 job-level cancel 打断（返 `Cancelled` 非 `Stop`）。
- `max_retry_attempts` 默认 3，`decide()` 内硬钳 ≤ `MAX_ATTEMPTS_CAP=10`（病态配置不致无限重投计费工具）。
- `max_job_secs` 是 **per-attempt** 预算（每次 `run_tool_once` 重置计时）；retry-eligible 工具总墙钟可达 `max_job_secs × max_retry_attempts` + 退避。**eligible 工具不得注册 output_tail ring**（`debug_assert` 守，否则重投会看到重复流）。

**新增 async_capable 工具若有副作用或计费，务必不要进 `is_retry_eligible`。**

## 后台审批桥（R8）

显式 `run_in_background` 的普通 exec **不再脱钩前同步审批**（`should_run_exec_reorder_gate` 仅 `AutoBackgroundEligible` 跑——保 ASYNC-2「审批不计入 `auto_background_secs`/`max_job_secs`」预算），命令 gate 落到后台 job 线程。

- **桥**：thread-local `BackgroundApprovalBridge` / `BackgroundApprovalScope`（定义在 [`tools::approval`](../../crates/ha-core/src/tools/approval.rs)，**tools 零依赖 async_jobs**——runner 在 `run_job_to_completion` 注入闭包回调 job DB）。
- **park**：dispatch 命中 gate → `on_park`：`mark_awaiting_approval`（`Running→AwaitingApproval`，WHERE status='running' 守）+ `park_timing_enter`（预算排除起算）+ 记 `request_id`，然后 `rx.await` 阻塞。
- **resume**：`BgResumeGuard` Drop 兜底恰好一次（覆盖 resolve / timeout / cancel-drop）。**proceed** → `resume_from_awaiting_approval`（`AwaitingApproval→Running`）+ `set_approval_origin`（F6 修正）+ emit `job:updated`；**deny / timeout-deny** → 不回 Running、让终态 settle 落 `DeniedByUser`（折进 `Failed`，STOP 文本经注入保留）。unattended / strict 仍 fail-closed deny（内层 gate 在 `rx.await` 前返回，**不 park**）。
- **parked 持槽不释放**（避免 resume 无空槽死锁；`approval_timeout_secs` 兜底释放）；**job 预算 timer 排除 parked 时长**（`parked_budget_extension`）。
- **取消 parked**：`cancel_job` 即时 `dismiss_parked_job_approval`（掉 sender 唤醒 rx、命令 gate 见 cancellation 不批准、弹窗立消、不死等 5s grace）+ token trip → `Cancelled`；跨进程 cancel 经 `on_resume` 拆孤儿弹窗（remove-if-present 才 emit）。
- **审批纯内存**：重启 parked → `Interrupted`（`list_running` active 集已含 `awaiting_approval`，无需新持久化）。
- **边界取舍**：若用户同时关掉 approval-timeout 且 `max_job_secs=0`，parked job 会一直占槽直到答复/取消（显式「全 timeout 关」的取舍）。

### 后台 subagent 内层审批投影 watcher（R8 follow-up）

后台 subagent 在子 session 跑自己的回合，内层工具审批**不经** thread-local 桥（桥只覆盖 `kind=Tool` job）。改由 EventBus watcher（approval_projection_watcher.rs）补投影 label：订阅 `approval_required`（park）/ `approval:resolved`（resume）→ `JobManager::reflect_subagent_inner_approval(child_session, parked)` → `SessionDB::find_active_run_by_child_session` → 投影行 → 复用 kind 无关的 `mark_awaiting_approval` / `resume_from_awaiting_approval` 翻 `running⇄awaiting_approval` + emit `job:updated`。

- **红线**：纯投影 label、**绝不 gate 执行**（内层审批照旧 block-and-wait）。
- **gotcha**：两事件 session 字段大小写不同——`approval_required.session_id`（snake）vs `approval:resolved.sessionId`（camel），watcher 两者都认。
- 非 subagent 审批（前台 / R8 后台 exec——其审批带的是**父** session）、未投影的 internal/incognito run 全部 fall-through no-op；status WHERE 守卫使终态投影 / 重复事件安全。app_init 两路各 spawn 一次（进程内幂等）。

## 运行中输出尾巴（R3①）

后台 `exec` job（显式 + auto-background 两路）的 stdout/stderr 实时 `append` 进进程本地有界 ring（output_tail.rs），`job_status(action:status)` 对 running job 返回 `output_tail`（lossy-UTF8）。

- **加法式、仅 exec**：只当 `ctx.output_tail_job_id` 为 Some（exec + 非 incognito）才走；前台同步 exec 不动；**incognito 永不注册**（与 spool 同闭）。
- ring cap 在 job 起跑时按 `configured_bytes()`（读时钳 `[256, 1MB]`，默认 8KB，`0` 钳到地板 256）**快照**，mid-run 改配置不影响在跑 ring。
- 进程本地（非跨进程）；`remove` 于 finalize / 取消 / 清理。

## 完成注入与合并窗口（R4）

tool job 终态后 `enqueue_injection`（injection.rs）把完成缓冲进**每会话**合并窗口：`completion_merge_window_secs`（默认 3，`0` = 关、立即注入）内同会话完成的多 job 合并成**一条** `<task-notification-batch>` 一轮注入（省计费 turn）。首条完成开窗起定时器，截止前到达的并入批次。

- **恰好一次 + 防双注入**：ghost-turn 闸两层（dispatch 时查父 session 存在 + `inject_and_run_parent` backstop；瞬时查询错按 proceed，不丢真 job）；进程内 `dispatching_set` 逐 job claim/release（`try_claim_dispatch` / `DispatchGuard` RAII）；`on_injected` 仅在真终态落地（Success）触发、逐行标 injected 恰好一次（Abandoned / Requeue 不标）。
- **崩溃恢复**：纯内存 live-path，崩溃则 terminal-but-uninjected，重启 `replay_pending_jobs` 各自补投（不丢、不合并）。
- **Group 是预合并特例**：`kind=Group` 不进合并窗口，由 join CAS `claim_group_completion` 在全部子终态 + sealed 时直接发一条合并注入。
- **空闲门超时不丢弃**：注入空闲门超时重排队进 `PENDING_INJECTIONS` 待会话空闲重试（与 subagent 注入共用机制，见 [子 Agent 系统](subagent.md)）。
- **回投 IM**：父会话若 attach 了 IM chat / 是 cron 会话，注入结果按 `imReplyMode` / `delivery_targets` 回投（G1/G2/G3，见 [IM Channel](im-channel.md)）。
- 完成桌面通知 `notification.notifyOnBackgroundJobComplete`（默认开，受 `notification.enabled` 门控；仅 `job:completed` 的 completed/failed/timed_out + 仅后台）。

## 事件（R3）

所有后台任务生命周期走 [`async_jobs::events`](../../crates/ha-core/src/async_jobs/events.rs) 发 `job:{created,updated,progress,completed}` + `job:mark_injected_failed`（替代旧 `async_tool_job:*`，破坏性 drop）。每事件带 `job_id` / `kind`（`tool`|`group`）/ `tool` / `status` / `session_id`。`progress` 目前仅 Group 报 N/M 子完成。**`subagent` kind 沿用更丰富的 `subagent:*` 流不双发**；R4 面板合并两路。best-effort UI 信号、无正确性依赖。**新增后台任务事件一律走 `events` helper，勿散落 `bus.emit`。**

## 模型面：`job_status` 工具

`tool_job_status(args, session_id)`，`action ∈ status | list | wait | cancel | result`：

- `list`：枚举本会话在途；`status`：单 job 状态（running exec 附 `output_tail`）；`result`：取已完成结果；`cancel(id)`：跨进程取消。
- `wait{ids?, mode, timeout}`：短便利同步——`DEFAULT_WAIT_SECS=5`、硬钳 `MAX_BLOCK_WAIT_SECS=10`（`max_job_secs=0` 时回落 `job_status_max_wait_secs`，均钳 ≥1s），超时回 `still_running` 不长阻塞。`WaiterGuard` 全程持有、各路径 Drop（panic-safe）；`finalize_job` 必须在 `update_terminal` commit **之后**才 `notify_completion`（waiter recheck 见终态行）。
- **长 fan-out 等齐的正道是注入而非 `wait`**（`batch_spawn` 的 Group 等齐后合并注入一轮）。

## Owner 面板与端点（R4）

owner 平面（host-trusted，看全部不经 agent-scope，与 KB owner 平面一致）读 `JobManager::list_session_snapshots(session)` / `get_job_snapshot(id)` → `BackgroundJobSnapshot`。

- **Tauri**：`list_background_jobs` / `get_background_job`。**HTTP**：`GET /api/sessions/{id}/background-jobs`、`GET /api/background-jobs/{id}`（Bearer，同 session 端点）。
- **Group 子投影折叠进 Group 行**：`list_for_session` 在查询层排除 `(kind=Subagent AND group_id IS NOT NULL)`，面板预算只数可展示的顶层行、保留（最老的）Group 行；客户端再叠防御过滤，面板只显 Group 进度摘要、不展开 N 子行。
- 前端镜像类型 `src/types/background-jobs.ts` + `useBackgroundJobs` 单订阅喂头部徽标 / 独立面板 / 工作台区块。
- **取消统一复用 `cancel_runtime_task(kind=AsyncJob)`**（[`runtime_tasks.rs`](../../crates/ha-core/src/runtime_tasks.rs) → `cancel_async_job` → `async_jobs::cancel_job`），不新增取消端点。

## 无痕（incognito，E4）

- args 落 `{"_incognito_redacted": true}`（live dispatch 仍收真实 args）；`persist_result` 不写 spool；snapshot `result_preview` redact。
- output_tail 永不注册。
- **关闭即焚**：`purge_for_session`（`session:purged`）删本会话全部 job 行 + spool + 从调度队列丢 queued + 从合并窗口丢缓冲注入。

## 取消、孤儿与保留

- **取消（I4）**：`cancel_job` 先 `set_cancel_requested`（DB 跨进程 flag）再 trip 进程内 token；queued job 经 `slots::remove_queued` 直接拉出 finalize（释放钉住的 ctx，对无痕 burn 重要）；running 标 `Cancelling`；parked 拆审批弹窗；找不到 runner 则补触发终态 hook。进程内 token 注册表 best-effort，DB 为持久真相。
- **孤儿（I3）**：exec job 落 `pid`；重启 `replay_pending_jobs` 对残留 running 检查 pid，仍活则终止整个进程组。
- **保留（retention.rs）**：每日 + 启动各一次。`purge_terminal_older_than`（`completed_at < cutoff`，仅终态行）删行 + spool；spool 孤儿（无行引用 + mtime 超 `orphan_grace_secs`）清理，单趟 `MAX_ORPHANS_PER_SWEEP=10k`（防病态目录饿死线程池，跑在 blocking pool）。`retention_secs=0` **且** `orphan_grace_secs=0` 时整个 sweep 任务跳过。

## 配置（`AsyncToolsConfig`）

category `async_tools`，归 **MEDIUM**，GUI 走专用 `save_async_tools_config`（详见 [配置系统](config-system.md) / [设置约定](../../AGENTS.md#设置settings约定)）。**默认值单一来源是 `impl Default`（与各 `default_async_*()` 对齐，单测断言）。**

| 字段（snake / JSON camel） | 默认 | 钳 / `0` 语义 |
|---|---|---|
| `enabled` | `true` | — |
| `auto_background_secs` | `0`（关） | `>0` = 同步预算，超则自动转后台 |
| `max_job_secs` | `0`（不限） | **per-attempt** 预算 |
| `inline_result_bytes` | `4096` | 内联预览预算，超出 spool |
| `retention_secs` | 30 天 | — |
| `orphan_grace_secs` | 24 小时 | — |
| `job_status_max_wait_secs` | `7200` | `wait` block 上限回落值 |
| `max_concurrent_jobs` | `clamp(cores-2, 4, 16)` | **`0` = 不限** |
| `max_concurrent_jobs_per_session` | `(global×3/4).max(2)`，band [3,12] | **`0` = 不限**；恒 < 全局 |
| `retry_enabled` | `false` | opt-in |
| `max_retry_attempts` | `3` | 钳 `[1,10]` |
| `completion_merge_window_secs` | `3` | `0` = 关（立即注入） |
| `output_tail_bytes` | `8192` | 钳 `[256, 1MB]`，**`0` → 地板 256** |
| `max_queued_jobs` | `256` | 钳 `[1, 4096]`，**`0` → 地板 1（非无限）** |
| `wakeup_max_delay_secs` | `86400` | 钳 `[10s, 7d]`（属 [schedule_wakeup](#跨子系统关系)） |
| `wakeup_max_pending_per_session` | `5` | 钳 `[1, 100]`（属 schedule_wakeup） |

**`0` 语义红线**：`max_concurrent_jobs` / `max_concurrent_jobs_per_session` 的 `0` = 真不限；其余 bounded-resource 旁钮（`output_tail_bytes` / `max_queued_jobs` / `wakeup_*`）的 `0` 一律钳到地板，**绝非无限**。`wakeup` 下限 10s 是固定 busy-poll 地板、不可配。

## 跨子系统关系

- **R6 后台 subagent 投影（单向）**：用户委派的后台 subagent run 在 `spawn_subagent` 建一条 `kind=Subagent`、`subagent_run_id` FK、`args_json="{}"`、`injected=true` 的投影，与 tool job 共享 `job_status` / 面板 / 取消。`subagent_runs` 是执行真相源（task/result/error 只在那），投影只承载 status/生命周期、**绝不持有正文、绝不反写**。同步走单一 choke point `SessionDB::update_subagent_status` → `JobManager::sync_subagent_projection`；取消经 `cancel_job` kind=Subagent 分支路由到 `subagent::request_cancel_run`，**不跑 tool job 的 hook/注入**。详见 [子 Agent 系统](subagent.md)。
- **R5 Group fan-out**：`batch_spawn` 建 `kind=Group` 协调行（`group_id` 关联子投影、`args_json={"sealed":bool}`、`injected=true`），N 个子携 `group_id` 抑制个体注入；全部子终态 + sealed 时单赢 CAS 发一条合并 `<task-notification>`（join 真相读 `subagent_runs`，group 行**绝不持有正文**）。详见 [子 Agent 系统](subagent.md)。
- **`schedule_wakeup`**：`wakeup_max_delay_secs` / `wakeup_max_pending_per_session` 虽落在 `AsyncToolsConfig`，语义属一次性自我唤醒子系统（`crate::wakeup` + `wakeups.db`），与后台 job 不复用入口。详见 [工具系统](tool-system.md)。
- **统一取消**：所有 runtime 任务取消走 `cancel_runtime_task`（`RuntimeTaskKind`，runtime_tasks.rs），后台 job 是其 `AsyncJob` kind。

## 诊断

- 生命周期日志 `category='async_jobs'`（**非** `background_jobs`）；EventBus 用 `job:*`；watcher lag 日志 source `approval_projection`。
- 表/文件 `~/.hope-agent/background_jobs.db`（表 `background_jobs`）；结果 spool 在 `~/.hope-agent/background_jobs/`（`paths::background_jobs_dir`，per-job `{job_id}.txt`）。`~/.hope-agent/async_jobs/` 是 pre-R1 legacy 目录，启动时尽力删除。
- stale-schema 探针 `SELECT group_id`（最新列）；升级时 pre-R5 表 + legacy `async_jobs.db` 尽力 drop（纯缓存，无迁移）。
- 子系统速查见 [`diagnostic-playbook.md`](../../skills/ha-self-diagnosis/references/diagnostic-playbook.md)（R1 命名分裂 / R5 Group / R6 投影 / R7.2 排队 / R8 审批投影 的故障 gotcha）。

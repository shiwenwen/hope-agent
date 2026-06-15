# 后台任务一等抽象（Background Jobs）设计文档

> 状态：设计稿，待 review 拍板后再分阶段落地。
> 关联：[`PRD：任务中断状态恢复与展示优化`](./PRD：任务中断状态恢复与展示优化.md)（turn 级生命周期，是本设计「完成后起新 turn」的挂靠点）、`docs/architecture/tool-system.md`（异步工具现状唯一提及处）。
> 落地后毕业为 `docs/architecture/background-jobs.md` 并登记 `docs/README.md`。

## 1. 背景

### 1.1 现状能力（先说做对的）

异步工具子系统（`crates/ha-core/src/async_jobs/`，~3100 行）已经把**最难的一块做对了**：后台任务完成后能**主动把结果注入回会话、自动再唤起 agent 跑一轮**，而不是等用户下一句话。完整链路：

```
finalize_job (spawn.rs:641)
  → wait::notify_completion          # 唤醒 job_status(block=true) 等待者
  → emit async_tool_job:completed    # 仅前端订阅
  → 终态 PostToolUse hook
  → injection::dispatch_injection (injection.rs:43)
      → subagent::injection::inject_and_run_parent (subagent/injection.rs:204)
          → 等会话 idle (injection.rs:294)
          → 追加一条 <task-notification> 用户消息 (source=ParentInjection)
          → run_chat_engine 跑一轮新的（计费）parent turn
```

配套已具备：崩溃重放（`replay_pending_jobs` mod.rs:217，孤儿 pid 回收 + 终态补发 + 未注入重投）、跨进程取消（DB flag + 5s 轮询）、incognito 端到端焚毁、typed `JobError`（error.rs，取代字符串匹配）、RAII 并发槽。**这套 push-唤醒机制是 agentic 后台的核心，本设计在它之上扩展，不推倒。**

### 1.2 问题（和成熟 agent 后台模型的差距）

差距不在「完成唤醒」，在**高度**：

> 我们把「异步」理解成「一个会自己脱钩的慢工具」；成熟模型（Claude Code）把「后台执行」当成一种**控制流原语**。

我们能后台化的**单位是一次工具调用，且只有 5 个工具**（`exec` / `web_search` / `image_generate` / `browser` / `app_update`，`registry.rs:50` `is_async_capable` 单点门控）。成熟模型能后台化的单位是**一条命令、一整个子 agent、一整套多 agent 工作流**，且能 fan-out / wait-all / pipeline 编排。这个高度差，二阶地决定了 §5 的每一条问题。

## 2. 目标

### 2.1 业务目标

- 把「异步工具」升格为「**后台任务（Background Job）**」一等产品：用户能看到「后台在跑什么」、跑完被通知、能取消。
- agent 能真正**委派并发后台工作并等齐收集结果**，而不只是让一条 shell 命令脱钩。
- 自托管（server）/ IM / ACP 模式下，后台任务的完成注入与桌面端**行为一致、正确**。

### 2.2 技术目标

- 单一 `BackgroundJob` 领域模型 + 单一 `JobManager` 入口，统一 spawn / cancel / list / wait / 完成注入 / 崩溃恢复。
- 可插拔 executor：`Tool` / `Subagent` / `Group`（fan-out+join）三类 kind 共用同一生命周期、同一持久化、同一事件命名空间、同一 UI 面板、同一 agent 工具面。
- 在途可观测：进度事件 + 统一 `job:*` EventBus + 前端后台任务面板 + 完成桌面通知。
- idle/busy 追踪从 Tauri 壳下沉 ha-core，四入口共享（修非桌面注入正确性）。
- 调度健壮：排队 + per-session 公平 + 重试 + 共享 executor，auto-background 计入并发上限。
- `AwaitingApproval` 真正落地：后台任务可中途挂起等审批，批准后续跑（衔接刚完成的审批引擎）。

## 3. 非目标（刻意的设计决策，避免 bandaid）

- **不做「真·turn 内 tool_result 回插」**。完成交付维持「会话 idle 后起一轮新 turn + `<task-notification>`」，**不**尝试把结果缝回原来那次挂起的工具调用槽。理由见 §8.1——强行 mid-turn 回插会破坏 provider 原生 tool-call 解析、API-Round 配对（`_oc_round`）与前缀缓存，是典型「浅层补丁」。我们改为把「起新 turn」做到**缓存友好 + 跨模式正确**。
- **不重写 chat_engine 对话协议**，不新增 provider 适配。
- **不改 Plan Mode 五态、不改 Plan `task_*` 三态语义**。后台任务（Job）与计划任务（task）是两个正交概念，见 §4.3 命名消歧。
- **不做数据迁移**：`async_tool_jobs` 表直接 drop 重建为新 schema（遵循项目「破坏性改动直接 drop」红线）。后台任务是瞬态运行态，无历史价值需保留。
- **不合并 `local_model_jobs` 后端**：模型下载任务后端保持独立；仅在 §6 R4 把它的呈现并入同一个「后台任务面板」消除 UI 概念重叠（`TaskCenterView` 目前甚至未挂载）。
- **不做可编程编排（Workflow-class）**：本期 `Group` 只提供「fan-out + join」一次性并发。**用代码确定性地编排一队 agent**（pipeline / loop-until-dry / 条件 fan-out / 结构化 schema / 缓存恢复 / 预算控制）是与成熟 agent 模型之间最后一道「高度线」，属**后续独立 initiative**，不混进本期——它需要的是一个编排执行器，而非后台任务管理器的扩展。本 PRD 的目标是把「后台执行」做成**受管的一等任务**，不是做成**可编程控制流基底**。

## 4. 参考实现结论

### 4.1 成熟 agent 后台模型（Claude Code）的关键特征

1. **后台化是跨多种工作类型的一等控制流**：后台 bash、后台子 agent、后台多 agent 工作流，不是「个别慢工具」。
2. **完成主动唤醒** agent（`<task-notification>` 起新一轮）——**这点我们已对齐**。
3. **富可观测**：实时进度树、增量输出（看部分 stdout）、条件等待、完成推送、任务列表。
4. **可组合**：parallel / pipeline fan-out、wait-for-all、结构化结果收集。
5. **可监督**：用户作为操作者能随时看到、取消后台工作。

### 4.2 我们已对齐 vs 仍缺

| 特征 | 我们 | 差距 |
|---|---|---|
| 完成主动唤醒 | ✅ injection 已实现 | 仅桌面端 idle 追踪正确（§5.4） |
| 崩溃恢复 / 取消 / incognito | ✅ 完整 | 取消 5s 轮询、线程/job 偏重 |
| 后台化粒度 | ❌ 仅 5 个工具 | 无后台 subagent / workflow / 编排（§5.1） |
| 在途可观测 | ❌ 几乎为零 | 无进度 / 无面板 / 无通知（§5.2） |
| 可组合 | ❌ 无 | `job_status` 只收单个 id（§5.1） |
| 调度 | ⚠️ 全局计数 reject-on-full | 无队列 / 公平 / 优先级 / 重试（§5.5） |
| 中途等审批 | ❌ 死状态 | `AwaitingApproval` 无写入端（§5.6） |

### 4.3 命名消歧（契约）

三个「任务」类概念物理分开，本设计只动第一个：

| 概念 | 含义 | 持久化 | 本设计 |
|---|---|---|---|
| **Background Job（后台任务）** | 脱离前台 turn 运行的**执行单元**（工具 / 子 agent / 组） | `background_jobs.db`（替代 `async_jobs.db`） | 本文主题 |
| **Plan task（计划任务）** | 计划里的**工作项**清单（`task_create/update`，三态） | `plan/`、`tasks.db` | 不动 |
| **Chat turn（一轮执行）** | 一次用户请求的**生命周期**（running/interrupted/...） | `chat_turns`（见关联 PRD） | 仅挂靠（注入起的新 turn 落 turn 行） |

> 代码层沿用既有 `async_jobs/` 血脉演进为 Job 子系统（不另起炉灶）；用户可见文案统一「后台任务」。`local_model_jobs`、Plan `task` 保持各自代码命名不变，仅 UI 收敛到同一面板。

## 5. 当前问题（带证据）

### 5.1 后台化粒度太窄 + 不能编排（最伤）

- 仅 5 个工具可进后台（`registry.rs:50`）；**无「后台子 agent」「后台工作流」「多任务等齐」概念**。
- `job_status` 工具只收单个 `job_id`（`job_status.rs:224`），**无 list / wait-any / wait-all**。agent 哪怕背景化了 3 件事，也没有干净办法收齐。

### 5.2 在途完全不可观测（纯 UX）

- 异步工具**不发任何进度事件**——跑 5 分钟的 exec，完成前只显示 "running" 一个词。
- 前端唯一 UI 是工具块内的单行 `AsyncJobCancelCard`（`AsyncJobCancelCard.tsx` + `ToolCallBlock.tsx:483`）；**无全局面板 / 无徽标计数 / 无完成通知**。`useDesktopAlerts.ts:43` 通知白名单里**没有** `async_tool_job:completed`。
- 与 Workspace 面板**零集成**（那里的「任务进度」是 Plan task，另一套）。消息滚走或切会话后，看不到「后台在跑什么」。

### 5.3 完成 = 一整个计费 turn，且不能 turn 内交织

- 注入只在会话 idle 时落地（`injection.rs:294`）；agent 在同一 turn 内继续干活，结果进不来，只能轮询（又被 `polling_guidance` 劝退）或停手。
- 结果以**全新计费 turn + 重建上下文**交付（`subagent/injection.rs:450-520`），靠 `<task-id>` 文本让模型自己对号，而非 provider 原生 tool-call 解析。

### 5.4 非桌面模式半残（已验证）

- `ChatSessionGuard::new` **只在 `src-tauri/src/commands/chat.rs:303`** 创建（ha-core 内仅单测用）。HTTP / IM / ACP 不创建 guard → `ACTIVE_CHAT_SESSIONS` 恒为 0 → 「等会话 idle 再注入」的串行保证**在非桌面端不成立**，注入即时开火、可能撞上正在跑的 turn。

### 5.5 调度太糙

- 全局单计数器（默认 8，`slots.rs:25/49`），**满了直接报错不排队**；无 per-session 公平（一个 IM 会话能占满 8 格饿死其他人）；**无优先级 / 无依赖 / 无重试**。
- auto-background 路径**不计入这个上限**（`slots.rs:18`），真实在跑线程数可超配置上限。
- 跨进程取消 5s 固定轮询（`spawn.rs:567`）；每个 job 一条 OS 线程 + 独立 current-thread tokio runtime（`spawn.rs:222`），偏重。
- retention 固定一天一次（`retention.rs:160`）。

### 5.6 `AwaitingApproval` 是死状态（已验证）

- `AsyncJobStatus::AwaitingApproval` 定义/解析/读取齐全（`types.rs:16`、`job_status.rs:173`、`injection.rs:337`、SQL active 过滤），但**全树无写入端**。`exec` 审批在脱钩前同步解决，后台任务**永远不会真正进入该状态**——「跑到一半需要你拍板、你不在就挂起等着」的能力不存在。

## 6. 需求范围

按依赖排序，每条可独立成 PR/Epic（§9 给落地顺序）。

### R1 — 统一 Background Job 模型与管理器（地基）

- 新 `background_jobs` 表（drop 旧 `async_tool_jobs`，不迁移），字段见 §7。`JobKind = Tool | Subagent | Group`。
- 状态枚举在现有 `AsyncJobStatus` 基础上**新增 `Queued`**（排队未起跑）、**激活 `AwaitingApproval`**（R8 写入）。
- 单一 `JobManager`（registry + scheduler）作为唯一入口：`spawn(kind, spec)` / `cancel` / `list(filter)` / `wait(selector)` / `get` / 完成→注入。现有 `async_jobs::spawn_explicit_job` 等收敛为 `Tool` executor 的实现。
- **验收**：所有后台单元（异步工具、后台 subagent）经同一 `JobManager` 创建；`async_tool_jobs` 表已删除；现有异步工具行为不回退（既有单测全过）。

### R2 — idle/busy 追踪下沉 ha-core（修非桌面，§5.4）

- 把 `ChatSessionGuard` / `ACTIVE_CHAT_SESSIONS` / `SESSION_IDLE_NOTIFY` 的创建点从 `src-tauri` 移入 ha-core 的 `run_chat_engine` 入口（或其紧邻 turn 边界），四入口（Tauri / HTTP / IM / ACP）自动共享。
- **验收**：server 模式下后台任务完成注入不再撞活跃 turn；新增单测：HTTP 路径起一个 busy turn 期间 finalize 一个 job，注入被串行排队而非即时开火。

### R3 — 在途进度事件 + 统一 `job:*` 命名空间（§5.2 后端）

- executor 发进度：`Tool/exec` 报 stdout 字节/行数增量；`Subagent` 报 tool-call 轮次；`Group` 报 N/M 完成。新增 `job:progress` 事件 + 行内 `progress` 字段。
- 统一 EventBus 前缀 `job:{created,progress,updated,completed}`，替代 `async_tool_job:*`（旧前缀删除，前端同步改）。
- **运行中输出尾巴（①，对齐 `BashOutput`）**：`exec` 后台 job 把子进程 stdout/stderr 实时写入一个**有界 ring buffer**（默认尾部 ~8KB），同一条流既喂 R4 的 UI 实时尾巴，也让 `job_status(action:status)` 在 `running` 时返回 `output_tail`——**agent 可主动 tail 运行中任务的最新输出**（判断「在跑还是卡住」），不必等完成。完成时该 buffer 让位于 `result_path` 全量落盘。incognito job 不留 buffer（与不落盘一致）。
- **验收**：长 exec 在面板有实时进度 + 实时输出尾巴；`job_status` 对 running 的 exec 返回 `output_tail`；完成/失败/取消事件统一前缀；`api-reference.md` EventBus 清单更新。

### R4 — 前端后台任务面板 + 徽标 + 完成通知（§5.2 前端）

- 新「后台任务」面板（右侧，纳入 Workspace 或并列），列出在跑/最近作业（kind、来源会话、进度、取消按钮），sidebar/header 徽标显示在跑计数。
- 完成桌面通知：把 `job:completed` 加入 `useDesktopAlerts` 白名单（受 `notifyOnComplete` 类开关约束）。
- 把 `local_model_jobs`（`TaskCenterView`）呈现并入同一面板，消除「多个 task UI」概念重叠。
- **完成注入合并窗口（控成本）**：模型不用 `Group`、直接发多个 `run_in_background` 时，多个 job 各自完成会各起一轮计费 turn。注入侧加一个短 debounce 窗口（默认 ~3s，可配）——窗口内同一会话完成的多个 job **合并进一轮注入**（一条 `<task-notification>` 列多个 task-id）。`Group` 的合并是其特例。否则「鼓励后台化」会退化成「刷计费 turn」。
- Transport 双适配：`list_jobs` / `get_job` / `cancel_job` 的 Tauri + HTTP 两套。
- **验收**：切会话/滚走后仍能看到后台在跑什么；完成有桌面通知；面板能取消；窗口内多个 job 完成只起一轮注入；`api-reference.md` 接口对照更新。

### R5 — `job_status` 升级为多作业 + Group fan-out（§5.1）

- agent 工具面（演进 `job_status`，避免与 Plan `task_*` 撞名）支持 actions：`status(id)`（running 时带 `output_tail`，见 R3 ①）/ `list` / `wait{any|all|ids, timeout}` / `result(id)` / `cancel(id)`。
- **`wait` 语义（契约，避免锁死会话）**：`wait` 是**短任务的便利同步**，被 `MAX_BLOCK_WAIT_SECS`（10s）clamp，**绝不长阻塞**——turn=1 单飞下长阻塞会锁死整个会话。长 fan-out 的**正道不是 `wait(all)`**，而是「`Group` fire → 结束 turn → Group 完成**合并注入一轮**」。`wait(all)` 超 clamp 即返回 `still_running` + 提示走注入路径，不假装等齐。
- 新 `Group` job：一次 fan-out 多个子 job + join。**失败策略（契约）**：默认 **join-all-settle**（等所有子 job 到终态，不 fail-fast），完成时一并返回**部分成功 + 各自终态**（成功结果 + 失败的 typed error），由模型决定如何处置；不因单个子 job 失败丢弃其余结果。
- **验收**：`Group` fire 后结束 turn，全部完成时**合并注入一轮**（非 N 轮）；`wait(all)` 对长任务返回 `still_running` 不阻塞；一个子 job 失败时其余结果仍随 Group 终态返回；丢失 id 也能 `list` 枚举在途作业。

### R6 — 后台 subagent 统一为 Job（§5.1 粒度）

- subagent 的 `spawn_and_wait` 超时转后台路径，产出 `JobKind::Subagent` 的 Job，复用同一注入/面板/事件。
- **真相源分工（契约，防 dual-write 漂移）**：`subagent_runs` 仍是**子 agent 执行内容**的真相源（run 记录、结果、消息）；`background_jobs` 只是该后台 run 的**调度/生命周期投影**(queued/running/done、进度、取消、面板展示)，以 `subagent_run_id` 外键引用。**状态单向流**：执行真相 `subagent_runs` → 投影 `background_jobs`，`background_jobs` 不反写执行内容。面板/调度读投影,结果读 `subagent_runs`。
- **验收**：后台 subagent 出现在同一面板、完成走同一注入；前台 `spawn_and_wait` 行为不变；两表无双向写（断言 `background_jobs` 不持有 run 结果正文）。

### R7 — 调度健壮性（§5.5）

#### R7.0 并发治理三分法（契约）

现存 12 个并发/数量限制不是一类东西，按本质分三类、三种触顶行为治理，**不得混用**：

| 类别 | 本质 | 触顶行为 | 成员 | 本设计 |
|---|---|---|---|---|
| **资源/成本类** | 护线程/内存/钱/限流 | **① 真排队**（持久 FIFO，等空位） | 后台工具并发、subagent 作为 Job | 收进 JobManager 分层配额（R7.1） |
| **外部保护类** | 护外部服务/防洪 | **② 等信号量**（原地异步等，非持久队列） | MCP 全局 8/单 server 4、IM 入站 20、同轮并发工具（R7.3 新增） | 各自独立域，**不进 JobManager** |
| **结构/安全类** | 正确性 + blast-radius | **③ 直接拒绝**（报错，不等） | 每会话 turn=1、子 agent 嵌套深度 3、`batch_spawn` 10、Team 成员 8/活跃 3 | **保留默认、不当资源旋钮调** |

判定标准：**「资源不够」→ 排队/等信号量**（活儿合法迟早能跑）；**「结构不允许」→ 拒绝**（排队也变不合法）。①是持久队列（App 重启可恢复），②是当前 turn 内原地等（turn 结束即没）。**实现时严禁把结构性拒绝错写成排队。**

#### R7.1 资源/成本类：JobManager 分层配额

reject-on-full → **排队**（FIFO），并把单一全局数升级为三层（抄 MCP「全局+单 server」分层思路）：

```
├─ global_max          全局天花板，所有后台 Job 总数（0=无限）
│    默认 = clamp(物理核数 - 2, 4, 16)          # 按机器推导，对齐 Workflow 引擎 min(16,cores-2)
│                                               # 先例：本地 LLM 预算按内存/显存 50% 推导
├─ per_session_max     每会话公平份额，默认 6    # 防单会话/IM 群独占饿死其他会话
└─ per_kind            每类配额
   ├─ tool      （exec/web_search…）便宜 → 松：每会话 6 / 全局 10
   ├─ subagent  （整个 LLM 循环）  贵   → 紧：每会话 8 / 全局可配
   └─ group     组本身不占额，子 job 按自身 kind 计
```

- **`max_concurrent_jobs` 默认按硬件推导**，仅对 tool/全局这类**线程绑定**的成立；**subagent 不跟核数走**（瓶颈是 API 限流和钱，非 CPU）。
- **auto-background 计入同一配额**（修今天「慢工具转后台不占槽」的漏，§5.5）。
- 排队上后，硬件推导从「硬护栏」降级为「吞吐调优」——超了排队不报错，故非正确性依赖。
- **跨进程队列归属（契约，二轮 review 修正）**：等待队列 + 计数是**进程本地内存态**（队列持 live ctx，不可持久化），故**每个能后台化工具的进程各跑一个调度器、只调度自己进程的队列**（tier-agnostic，幂等保证一进程一个）——与 `replay_pending_jobs` 相反（后者扫共享 DB 行、Secondary 跑会误伤 Primary 在跑 job，故 Primary-only）。各进程 `mark_running` 只动自己 insert 的行（`WHERE status='queued'` 按行幂等），不会双调度；**并发 cap 因此是 per-process**（与 R7.1 前的进程级原子计数一致），非全局；跨进程取消仍走 DB flag。**反例教训**：最初误设 Primary-only 调度器 → Secondary 进程入队的 job 永卡（队列是进程本地、Primary 看不到），二轮 review 抓出后改 tier-agnostic。

> **实现约束（代码勘察结论，落地前必读）**：`spawn_explicit_job(ctx: ToolExecContext)` **按值接管 live ctx 并立刻 `std::thread::spawn` 执行**（`spawn.rs:135/222`）。要排队（slot 满不报错、空位再起），队列必须**在内存里持有这份 `ctx` 直到被提升**——`ctx` 含 EventSink / cancel token / pid_sink 等运行时句柄，**不可持久化**。好消息:执行本就是 detached（独立 OS 线程）+ 结果靠 injection 交付（不依赖发起 turn 的 sink），所以「先排队、空位再 spawn」在架构上**可行**；但有两条硬约束：(a) **`Queued` 行重启不可恢复**——内存 ctx 随进程消失，重启时 `Queued` 与 `Running` 一样标 `Interrupted`（已注入失败说明，不静默丢）；(b) **`queued` 入队即把 cancel token 注册好**（排队期也可取消）、synthetic「started」结果**入队时立即返回**给模型（turn 不阻塞）。这把 R7.1 从「不可做」变为「可做但需仔细写并发 + 测」，是本期最该有人在场 review 的一块。

#### R7.2 subagent 并发：默认调大 + 接上死配置

- **默认 5 → 8**（per-kind subagent 每会话）：有全局天花板 + per-session 公平兜底，调大不会拖垮全局。
- **接上死配置**：今天 `agent.json` 的 `subagents.maxConcurrent` 无消费端、设了不生效（实际由硬编码 `MAX_CONCURRENT_PER_SESSION=5` 管），R7 让它真正驱动 per-kind 配额。
- **UI 提示**：旁注「真正上限是你 Provider 的速率限制，撞 429 就调小」——这个数本身不是瓶颈。

#### R7.3 新增护栏：同轮并发安全工具上限

- 今天一轮内并发安全工具（`read`/`ls`/`grep`/`find`/`web_fetch`/MCP）经 `join_all` **无上限**并发（`streaming_loop.rs:721/731`）：模型一条消息发 50 个 `web_fetch` = 50 个出站请求同时打。
- 加信号量（默认 **8**，属②等信号量），超出原地排着跑。属**前台同轮**执行、与后台 Job 正交，但同为并发缺口，本 PR 一并修。

#### R7.4 重试 + 共享 executor

- **重试策略 per-kind（默认安全）**：重试**按 kind 分档,默认对副作用类全关**——`exec`（命令多为确定性失败或半执行副作用,盲重投会重复副作用）**默认不重试**；只对**幂等/瞬态**类（`web_search` / `web_fetch` 网络抖动）默认开 backoff 重投。`Failed` 区分**确定性失败**(不重试)与**瞬态失败**(重试),`TimedOut/Interrupted` 仅在 kind 标注幂等时重投。全部可配、可关。
- 共享 executor：Send 允许处用 tokio 任务 + 信号量替代 thread-per-job；仅 !Send 工具 future 保留 OS 线程兜底。

**验收**：突发超上限→排队不报错（①）；单会话占满槽，另一会话仍能起（per-session 公平）；auto-bg 计入配额；`max_concurrent_jobs` 默认随核数变化；`subagents.maxConcurrent` 改值真正生效；一条消息 N 个 `web_fetch` 受信号量约束并发；结构类（depth/batch/turn）触顶仍**拒绝而非排队**（断言用例）；重试单测。

### R8 — `AwaitingApproval` 落地（§5.6）

- 后台任务命中审批点时 park 为 `AwaitingApproval`，经审批引擎弹审批（复用 `approval:resolved` / `ask_user` 通道），决策后续跑或终止。
- 衔接审批引擎既有 `UnattendedApprovalAction` / strict 规则：无人值守 + strict → 维持 fail-closed deny（不因后台化而放宽）。
- **验收**：后台 exec 中途命中保护路径能挂起等人，批准后继续、拒绝则终态 `DeniedByUser`；面板显示「等待审批」态。

### R10 — agent 任务内自我定时唤醒（②，对齐 `ScheduleWakeup`）

让 agent 能「**N 秒后 / 时刻 T 把我唤回当前会话**继续干」——典型场景:等外部 CI(~8 min 后回来看)、轮询一个非 harness 可追踪的外部状态、稍后复检。这是「自暂停-定时续」,与 cron 是不同的轴。

- 新工具 `schedule_wakeup`（一次性）:agent 传 `delay_secs` / `at` + `note`（续跑上下文）。**复用 R1/R2 基础设施**——到点把一条 `<wakeup>` 消息经既有注入管线(`dispatch_injection`)注入**原会话**、按 R2 的 idle-gating 起一轮 parent turn,**不另起一套调度器**(挂在 JobManager 的定时投影上,或一条 one-shot 计时行)。
- **与 cron 的边界（契约）**:cron = 用户配置的**周期/定时起新任务**(可独立会话 + delivery fan-out);`schedule_wakeup` = **agent 发起、一次性、续当前会话上下文**。二者不复用同一入口、不互相污染。
- **边界**:`delay_secs` clamp(下限防忙轮询、上限防僵尸)、每会话 pending wakeup 上限(防自我刷计费 turn);**incognito 不持久化**(关闭即焚,与无痕一致);会话已删则 ghost-turn 守卫直接丢弃(复用注入既有守卫)。
- **验收**:agent `schedule_wakeup(delay_secs=N, note=...)` 后结束 turn,到点被唤回原会话并带 note 续跑;重启可恢复未触发的 wakeup;无痕会话不留 wakeup;超过每会话上限拒绝(结构类③,不排队)。

### R9 — 设置三件套 + 文档/诊断索引同步（强制）

- 新增可配置项均需 **GUI 控件 + `ha-settings` 读写分支 + `SKILL.md` 风险登记**（按 §设置约定，调度/并发类归 MEDIUM，无凭据）。清单：`global_max`（默认随核数，0=无限）、`per_session_max`、per-kind 配额（`tool` / `subagent`）、`subagents.maxConcurrent`（接上死配置）、同轮并发工具上限、公平模式、重试策略、进度上报节流、完成通知开关、注入合并窗口、`output_tail` 尾巴大小（R3 ①）、`schedule_wakeup` 的 delay clamp 与每会话 pending 上限（R10 ②）。
- **GUI 旁注**：`global_max` 标「默认按本机核数」；subagent 配额标「真正上限是 Provider 速率限制」。
- 同步 `api-reference.md`（新命令/路由/事件）、`AGENTS.md` 契约面、`CHANGELOG.md`；`ha-self-diagnosis` 的 `diagnostic-playbook.md` 登记新 `background_jobs.db`（`paths.rs`）与新稳定 log category（如 `background_jobs`）。
- **验收**：每个新开关 GUI 与技能零偏差；诊断索引含新 DB / category。

## 7. 数据模型（草案）

`background_jobs.db`（WAL，独立于 session.db，替代 `async_jobs.db`；缺列即 drop-rebuild，沿用现策略）：

```text
background_jobs:
  id                TEXT PK     -- job_<uuid>
  kind              TEXT        -- tool | subagent | group
  status            TEXT        -- queued | running | awaiting_approval | cancelling
                                --   | completed | failed | interrupted | timed_out | cancelled
  origin            TEXT        -- explicit | policy_forced | auto_background
  parent_session_id TEXT
  group_id          TEXT NULL   -- 子 job 指向其 Group
  subagent_run_id   TEXT NULL   -- kind=subagent 时引用 subagent_runs（真相源），仅投影不反写（R6）
  spec_json         TEXT        -- kind 相关入参（incognito 下脱敏占位）
  priority          INTEGER     -- R7，默认 0
  attempt           INTEGER     -- R7 重试计数
  progress_json     TEXT NULL   -- R3：{kind, current, total, label}
  pid               INTEGER NULL
  cancel_requested  INTEGER
  result_preview    TEXT NULL   -- head+tail 内联
  result_path       TEXT NULL   -- 满输出落盘（incognito 不落）
  error_json        TEXT NULL   -- typed JobError
  injected          INTEGER
  queued_at / started_at / completed_at  INTEGER
```

要点：`group` 与子 `tool/subagent` 用 `group_id` 关联；`wait(all)` 在 manager 层聚合子 job 终态；incognito 仍走「脱敏 spec + 不落盘 + settle 重判 fail-closed + 焚毁清盘」既有不变量。

## 8. 风险与权衡

### 8.1 为什么不做「真·turn 内 tool_result 回插」

把完成结果缝回原挂起的工具调用，需要：(a) 在 chat_engine 引入「可挂起/可恢复的 turn」原语；(b) 跨 provider 维护未决 tool_call 的原生关联（Anthropic/OpenAI 协议各异）；(c) 破坏 `_oc_round` 配对与前缀缓存（压缩切割、cache TTL 全受影响）。收益是「同 turn 内拿到结果」，但代价是动对话协议地基且全 provider 适配。**决策：维持「起新 turn」交付**，转而把新 turn 做成缓存友好（复用 system_prompt + history 前缀命中 cache）+ 跨模式正确（R2）。这与项目「side-query 复用前缀缓存」的既有取向一致。

### 8.2 其他风险

- **R2 下沉 idle 追踪**触及四入口 turn 边界，回归面大——以单测覆盖每入口的 busy/idle 信号，分模式灰度。
- **R7 共享 executor**：工具 future 多为 !Send（现用 thread+runtime 规避），改造范围需先勘探哪些可安全迁移，不可一刀切。
- **R5/R6 编排**叠加注入成本：N 个子 job 各起一轮新 turn 会放大计费——Group 完成**合并一次注入**（等齐后一条 `<task-notification>` 汇总），而非 N 条。
- **drop 旧表**：升级即清空在途异步作业历史；属瞬态运行态，可接受（与 no-migration 红线一致）。

## 9. 分阶段落地建议

| 阶段 | 内容 | 价值 | 依赖 |
|---|---|---|---|
| **P0 地基** | R1（统一模型 + JobManager）+ R2（idle 下沉修非桌面） | 正确性地基，解锁后续 | — |
| **P1 看得见** | R3（进度+事件+运行中 `output_tail` ①）+ R4（面板+徽标+通知） | 见效最快，「后台跑、跑完叫我」体感 | P0 |
| **P2 编排** | R5（多作业 list/wait + Group）+ R6（后台 subagent 统一） | 核心差距「委派并发后台工作」 | P0 |
| **P3 健壮** | R7（队列/公平/重试/共享 executor） | 生产可用性 | P0 |
| **P4 闭环** | R8（AwaitingApproval 落地）+ R10（自我定时唤醒 ②） | 后台中途等审批 + 自暂停定时续 | P0、审批引擎 |
| **横切** | R9（设置三件套 + 文档 + 诊断索引）随每阶段就近补 | 契约合规 | 各阶段 |

> 已基于权限分支 `feat/async-epic-a-session-lifecycle`（PR #318）起独立 worktree `feat/background-jobs`（R8 需衔接其审批引擎，故 stack 在其上）；#318 合入 `main` 后 rebase 到 `main`，P0→P4 各自成 PR（base `main`）、独立门禁。
> **范围边界**：①（运行中 `output_tail`）、②（`schedule_wakeup`）已纳入本期；**③ 可编程编排（Workflow-class）见 §3 非目标，另列独立 initiative，本期不做**。

## 10. 实现进度（`feat/background-jobs`）

- ✅ **并发治理（A1/A2/A4）**：同回合并发安全工具上限（信号量 8）、接上死配置 `subagents.maxConcurrent` + 默认 5→8、`max_concurrent_jobs` 默认按本机核数。
- ✅ **R7.1 排队（本期子集）**：`spawn_explicit_job` 满槽 reject → **入队**（`Queued`）；`SlotManager`（per-session 计数 + 有界队列 `MAX_QUEUED_JOBS=256`，持 live ctx）；**每进程（tier-agnostic、幂等）调度任务** `run_scheduler` 在槽位空出时按 **per-session 轮转**（`pick_fair_index`，且新 spawn 不插队）提升，带 5s 兜底 tick（cap 抬高/漏唤醒）；取消摘队列直接 `Cancelled`、重启 `Queued`→`Interrupted`、incognito 焚毁清队列内 ctx。建在现有 `async_jobs`（工具 job）上。
  - **未做（待 R1 统一模型）**：per-kind 配额（subagent 当前是独立子系统）、`JobManager` 重命名、global + per-kind 分层、auto-background 计入配额；队列上限当前是常量（未配置化）。
- ✅ **R2 idle/busy 追踪下沉 ha-core（§5.4 修非桌面）**：前台 turn 的忙/闲标记（`ChatSessionGuard` → `ACTIVE_CHAT_SESSIONS` / `SESSION_IDLE_NOTIFY`）创建点从 Tauri 壳移入共享的 `run_chat_engine` 入口，按 `ChatSource::holds_foreground_idle_guard()`（Desktop / Http / Channel，cron 用 Channel）门控；ACP 直跑 `AssistantAgent::chat`，在其 turn 边界自建同一 guard。四入口（Tauri / HTTP / IM / cron / ACP）从此共享同一 idle 判定，自托管 / IM / ACP 下完成注入不再撞活跃 turn。`ParentInjection`（注入自身，guard 会自取消）/ `Subagent`（独立子会话）排除。Tauri 壳保留其更早的 guard（用户一发消息即取消在途注入，早于本 turn preflight）——`ChatSessionGuard` 引用计数使重叠安全（引擎 guard 先 drop、壳 guard 后 drop，idle/flush 整命令只触发一次）。注入侧 idle 等待抽出 `wait_for_session_idle` 便于单测（busy 时 park、fetched 时 abort、idle 时直过）。
- ✅ **R10 agent 自我定时唤醒（`schedule_wakeup`，②）**：新工具 `schedule_wakeup(delay_secs, note)`（Core/Meta、internal=不弹审批、always-load）——代理收尾本轮、到点经**既有注入管线**（`inject_and_run_parent`，复用 R2 idle-gating + 取消 + 重试）把一条 `<wakeup>` 消息注回**原会话**起新一轮、带 note 续跑。独立子系统 `crate::wakeup`（`wakeups.db` + 进程本地定时器 `ARMED_TIMERS`）：delay clamp `[10s, 24h]`、每会话 pending 上限 5（结构类③拒绝、不排队）、per-process 投递去重（`DELIVERING`，镜像 async_jobs）、`mark_fired` 仅在注入真落地时置位（Abandoned 留待 replay）；**重启恢复 Primary-only**（`replay_pending`，过期立即触发，镜像 `replay_pending_jobs` 避免 Secondary 双投）；incognito 仅内存定时器不落盘（关闭即焚）；会话删除 / 焚毁经 `cleanup_watcher` → `purge_for_session`（abort 定时器 + 删行）。与 cron 不共入口（cron=用户配置周期，wakeup=代理一次性续当前会话）。
  - **未做（待 R9）**：delay clamp / 每会话上限 / 通知开关的配置化（当前是常量），三件套留到 R9 横切。
- ✅ **R5 `job_status` 升级多作业（本期子集，待 R1 补 Group）**：`tool_job_status(args, session_id)` 加 `action` 路由——`status`(默认，单 id，向后兼容)/ `list`(枚举本会话在途 active jobs，`list_active_by_session`，封顶 `MAX_WAIT_TARGETS=32`)/ `wait{ids?, mode:all|any, timeout_ms}`(短便利同步,`compute_effective_timeout` clamp ≤ `MAX_BLOCK_WAIT_SECS=10s`,超 clamp 返回 `still_running` + 引导走注入路径**绝不长阻塞**;未知 id 记 `settled:unknown` 防永等)/ `cancel(id)`(复用 `async_jobs::cancel_job` 跨进程取消)/ `result`(=status 别名)。`format_job_response` 抽出 `job_response_value(job)->Value` 供 list/wait 组数组。dispatch 传 `dispatch_ctx.session_id`。含 6 个新单测。
  - **未做（待 R1 统一模型）**：`Group` fan-out + join（需 `group_id` / 统一模型）——本期 `wait` 是「短任务便利同步」,长 fan-out 的正道仍是「`Group` fire → 结束 turn → 合并注入一轮」,待 R1。
- ✅ **R3 ① 运行中输出尾巴 `output_tail`（本期子集）**：后台 `exec` job 的 stdout/stderr 实时 tee 进有界 ring（`async_jobs::output_tail`，默认尾部 8KB，`DEFAULT_TAIL_BYTES`），`job_status(action:status)` 对 running/cancelling job 返回 `output_tail`（BashOutput 对齐——判断「在跑还是卡住」不必等完成）。**加法式设计**：`spawn_exec_waiter` 仅当 `ctx.output_tail_job_id` 为 Some（后台 + 非 incognito exec）时走 `wait_with_output_teed`（并发读双管 + tee，返回完整 `Output` 不变）；**前台同步 exec 走原 `wait_with_output` 完全不动**。ring 在 spawn 时按 `tool_name==exec && !incognito` 注册，finalize_job / 队列取消 / 会话删（经 `cancel_job`）三处清；incognito 永不注册（关闭即焚）。`ToolExecContext` 加 `output_tail_job_id`。含 5 单测。
  - **未做（待 R4/R1）**：`job:*` 事件命名空间 + 进度事件（`job:progress` stdout 字节增量 / subagent 轮次 / Group N/M）——需 R4 前端面板消费 + R1 的 Group；`output_tail` 尾巴大小配置化待 R9。
- 🔬 **对抗审查修复（R2+R10+R5，3 CONFIRMED 全修）**：(1) wakeup `mark_fired` 吞错 → 改 `mark_fired_with_retry`（镜像 `mark_injected_with_retry`，retry+log），防瞬时 SQLite 失败留 `fired=0` 导致重启重投重复计费 turn；(2) `job_status` `action_wait` 静默截断 `ids` 到 32 → 加 `dropped`+`truncated` 字段，模型不再把被丢弃的 id 误当完成；(3) wakeup 复用注入管线被打 `subagent_result` 标记 → 前端误渲染成绿色「已完成」pill 且 note 丢失：改写 `wakeup_trigger` 专属 meta（`injection.rs` 按 `WAKEUP_CHILD_AGENT_ID` 分支，镜像 cron `cron_trigger`）+ 前端 `WakeupTriggerBubble`（紫色「定时唤醒」chip，可展开）+ `isWakeupTrigger` 解析 + i18n 12 语。
- ⏳ **待续**：R1（统一模型）、R3/R4（进度+面板）、R6（后台 subagent 统一）+ R5 的 Group、R8（AwaitingApproval）。

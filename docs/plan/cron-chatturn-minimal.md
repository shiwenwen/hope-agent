# Scheduled 普通对话与产品能力收敛

> 状态：实施中
>
> 日期：2026-08-14
>
> 基线：`be48cc40e`
>
> 决策：只移除 Scheduled 页面内嵌的实时输入框；其余 Scheduled 产品能力保留，执行内核统一为普通 ChatTurn。

## 1. 产品定义

定时任务是在指定时间替用户发起一次模型任务。触发后的消息、模型、工具、流、
Stop、队列、重连和后续追问都属于普通聊天，不建立第二套 Cron 对话系统。

Scheduled 仍然是完整的任务与运行管理面，而不是一个只剩 schedule 表单的简化页。
它负责：

- 定义未来触发；
- 选择运行目标和 Worktree 策略；
- 展示 occurrence 状态、历史、未读、归档和投递；
- 做 Preflight、精确取消、重试和资源生命周期操作；
- 导航到同一个普通会话。

唯一明确不保留的体验是：**Scheduled 只读预览中不放输入框**。需要输入、插入、
Stop 或继续追问时，用户点击“打开会话”，进入完整普通 Chat。

## 2. 必须保留的用户能力

### 2.1 任务管理与排程

- 创建、编辑、暂停、恢复、删除、Run Now；
- `At`、`Every`、Cron expression、IANA timezone 和 DST；
- Project、Agent、permission、sandbox、timeout、失败阈值；
- 通知和多 IM 目标投递；
- Primary-only claim、并发上限、missed/catch-up、失败退避和自动禁用；
- Task revision CAS，编辑冲突时保留本地草稿并允许基于最新版重试。

### 2.2 两种运行目标

#### 每次新建会话

- 每次 occurrence 创建一个普通 Session；
- 下一次 occurrence 创建另一个 Session，不继承本次人工追问；
- 会话立即进入普通侧栏、搜索、Recent、Project 分组和普通未读。

#### 当前会话

- 用户可从 Chat 标题栏选择“在此对话中安排任务”；
- 到点后在原 Session 中提交 Scheduled 来源的普通 ChatTurn；
- Session 忙时进入同一 durable FIFO，不并发启动第二个 Turn；
- 使用真正 dispatch 时该对话的上下文、Agent、Project、KB 和工作目录；
- 结果留在原对话，全局 Scheduled 历史只投影该 occurrence，不复制正文。

### 2.3 普通聊天体验

- Scheduled 首轮实时进入普通 stream bus，刷新后可重连；
- 普通 Chat 的 composer 始终存在；
- 默认发送进入普通 durable FIFO；
- 用户显式“立即发送”时，只在通用安全工具边界插入；
- Stop 精确取消当前 ChatTurn，不暂停未来排程；
- 终态后直接在同一会话追问，不存在“解锁/晋升为普通聊天”；
- 首条触发消息继续显示 Timer 来源卡、任务名和原 prompt；
- 普通侧栏与标题栏补 Timer 来源徽标和任务/历史导航。

### 2.4 Scheduled 历史、未读与归档

- 全局运行时间线和单任务历史保留；
- 每条 occurrence 有精确状态、时间、摘要、错误、投递结果和关联会话；
- Scheduled 提供无输入框的只读实时预览，复用普通 stream snapshot/journal；
- 可从历史打开同一个普通会话；
- 删除 Task 只停止未来排程，运行日志、普通聊天、投递记录和 Worktree 保留；
- 已删除任务在历史中显示 tombstone，可复制为新任务，不可直接复活旧 ID；
- Scheduled 未读只是普通 Session read watermark 的过滤投影，不建立第二份未读真相；
- 从普通 Chat 或 Scheduled 真正读到消息后，两边同时清除；
- 归档、恢复和 pin 复用普通 Session 状态，Scheduled 只提供过滤和入口。

### 2.5 occurrence 级运行控制

- Run row 显示 queued/running/completing/cancelling/terminal；
- Scheduled 页按 `run_log_id` 精确取消本次运行；
- 普通 Chat 的 Stop 与 Scheduled Cancel 命中同一个 exact `turn_id`；
- terminal/completing 的重复取消是幂等 no-op；
- 失败运行可再次运行；已删除 Task 只能复制为新任务后运行；
- 暂停只影响未来 occurrence，不默认停止在途运行；
- 删除 Task 对 queued/executing occurrence 做明确取消并保留终态审计。

### 2.6 Preflight 与可解释错误

创建、更新和 Run Now 前可查看：

- 未来三次触发时间；
- 实际 Agent、Project、permission、sandbox；
- destination、Worktree mode/base ref、源 checkout 脏状态；
- 投递目标和 scheduler/Primary 状态；
- blockers 与 warnings。

blocker 禁止确认，warning 允许用户确认。读取失败、目标缺失、Secondary、远端写入
受限、dirty/conflicted Worktree 等都显示稳定原因和 Retry，不允许静默当作成功。

### 2.7 Worktree

#### Project 目录

直接在 Project workspace 运行，保持现有行为。

#### 每次新建 Worktree

- 每个 occurrence 创建新的 Managed Worktree；
- Worktree 从创建起由本次普通 run chat 持有；
- 首轮结束后仍在同一 Chat 中继续，不做额外 handoff；
- 默认保留，支持普通归档/恢复；
- 可安全移交改动到 Project，或在 idle + 二次确认后丢弃；
- 历史显示 base SHA、branch、dirty/conflict snapshot 和资源状态。

#### 专属 Worktree

- 一个 Task 长期持有一个 Worktree，occurrence 串行租用；
- 用户“接管”时先暂停未来排程，再把 Worktree 交给精确 run chat；
- “归还”只归还保管权，可选择同时恢复 Task；
- “安全移交到项目”通过现有 Git handoff 搬运改动，与“归还”是不同动作；
- “丢弃”要求 Task 已暂停、无 active run、会话 idle，并进行二次确认；
- handed-off/dirty/conflicted 时 Run Now 和 Resume fail closed；
- 删除 Task 后 Worktree 仍可从历史中的“待处理资源”进入和处理。

Worktree 后续操作尽量复用普通 `WorkspacePanel`、`GitControlCard`、Diff、branch、
commit、push 和 PR UI，不建立 Cron 版代码工作台。

### 2.8 运行安全与恢复

- scheduler claim、run log 与 ChatTurn 关联必须可审计；
- queued 尚未跨模型/工具副作用边界，可幂等恢复；
- executing 崩溃后只收敛，不自动重放模型或工具；
- 取消必须命中 exact occurrence/turn，不能误伤下一轮；
- 无人值守 permission/sandbox 与 Worktree custody 在执行边界 fail closed；
- 运行中能够扩大本次 authority 或改变被租用 Worktree 的 owner 写操作必须拒绝；
- 纯展示和只影响未来 occurrence 的编辑不需要全局冻结；
- 外部投递不确定时不盲目重发，避免重复副作用。

## 3. 目标数据边界

### 3.1 Scheduled Task

继续使用 `cron_jobs`，只增加必要产品字段：

- `revision`；
- `deleted_at`；
- destination；
- `workspace_policy_json`。

它只描述未来触发，不持有 Chat 状态。

### 3.2 Scheduled Run

继续扩展现有 `cron_run_logs`，不新建一套大运行账本：

- `request_id`（仅 current-chat durable queue 需要）；
- `turn_id`；
- queued/executing/terminal 状态；
- `worktree_id`、workspace status/snapshot；
- 精确取消和恢复所需的最小 owner/fence。

run log 不存聊天正文。正文唯一存在于普通 Session/Message/ChatTurn。

### 3.3 普通 Session 与 ChatTurn

- Standalone 运行创建普通 Session；
- current-chat 运行使用原 Session；
- user message + ChatTurn 继续走 SessionDB 的原子入口；
- `ChatSource::Cron` 只表达本 Turn 是无人值守 Scheduled 来源；
- provenance 只用于展示/导航，绝不参与审批或权限裁决。

### 3.4 普通 durable queue

current-chat occurrence 复用 `queued_turn_user_messages`，增加 backend-managed Scheduled
来源和稳定 `source_ref`。Desktop/HTTP/Scheduled 必须遵守同一 Session FIFO；客户端不能
认领 Scheduled row，Primary 小型 pump 只消费已经由 scheduler 持久化的 Scheduled item。

这不是第二个 scheduler：scheduler 仍是时间触发唯一入口，pump 只负责在 Session idle、
Stop fence 和并发槽允许时提交普通 ChatTurn。

### 3.5 Managed Worktree

复用通用 `managed_worktrees`、安全 Git handoff 和普通 Workspace UI，只增加：

- `ScheduledRun` / `ScheduledTask` purpose；
- chat-owned、task-owned、runtime-bound、handed-off 的最小 owner 字段；
- exact CAS 和 dirty snapshot。

不恢复跨 CronDB/SessionDB 的通用 Saga；跨库中间失败保留可解释的 paused/busy 状态，
由精确恢复收敛，不能猜测性转移所有权。

## 4. 明确不恢复的旧实现

以下不是产品能力，不应从旧分支整段捞回：

- `CronChatState::{none,running,continuable,interactive}`；
- Cron 专用 composer；
- `cron_deferred/cron_takeover/waiting_cron_handoff`；
- Session promotion/unlock；
- Cron 专用 active-run session API；
- 第二套 stream、Stop、active-turn registry；
- 新 TurnRequest 领域层、大 ScheduledRun 状态机、completion outbox/Saga；
- generation 双读写迁移框架；
- Cron 专用 fork/compact/handover capability matrix；
- Cron 专用 unread/archive/pin 真相源；
- 全局冻结 Project/KB/Git/Terminal/Goal/Workflow/Loop/Plan/Memory 的 Cron 专用状态机。

需要的安全结果应落在 exact ChatTurn admission、authority ceiling、Worktree custody 和既有
owner API 上，而不是重新扩散 Session 类型。

## 5. 实施阶段与硬预算

### Phase A：管理面与来源投影

- logical delete 与历史保留；
- Sidebar/Title Timer provenance；
- Scheduled 只读实时预览；
- 统一 unread/archive/pin 投影；
- run exact cancel/event refresh；
- Preflight 与 revision conflict。

预算：净增 2,200–3,200 行。

### Phase B：Worktree

- 三种 workspace policy；
- Fresh chat ownership；
- Persistent task ownership、接管/归还/移交/丢弃；
- run snapshot、Preflight 和待处理资源 UI。

预算：净增 3,200–5,300 行。

### Phase C：当前会话排程

- destination 表单与 Chat 入口；
- backend-managed Scheduled queue row；
- Primary pump、FIFO、Stop/Continue、恢复；
- exact run transcript/history 投影。

预算：净增 2,500–4,000 行。

### Phase D：安全与收尾

- authority/workspace owner 写边界；
- crash/cancel/idempotency 契约；
- i18n、API 文档、架构文档与独立审查。

预算包含在前三阶段测试与适配中，不单独扩建框架。

### 总量守门

- 目标净增：7,500–10,500 行（含测试、i18n、文档）；
- 硬停止线：相对当前基线净增超过 12,000 行，暂停实现并重新审查抽象；
- 任一 Phase 超预算 25% 时先提交规模报告，不继续堆代码；
- 旧分支只读借鉴 DTO、纯 predicate、表单布局和测试场景，禁止整文件 cherry-pick；
- 每个 Phase 独立 commit、独立产品验收，可回退而不破坏普通聊天内核。

## 6. 最终验收

1. Standalone 到点后立即产生普通 Chat，侧栏、标题和首条消息都能识别 Scheduled 来源。
2. Scheduled 预览实时但永远没有输入框；“打开会话”进入完整普通 Chat。
3. 普通 Chat 的发送、FIFO、显式插入、Stop、重载和追问与手动聊天一致。
4. current-chat schedule 忙时排队，不越过较早用户消息、不并发启动 Turn。
5. Cancel Run 精确停当前 occurrence，未来 schedule 不变，重复取消幂等。
6. 删除 Task 后列表/日历不再出现，但历史、Chat、投递和 Worktree 仍可打开。
7. 普通 Chat 与 Scheduled 共享 read/archive/pin 真相，没有重复正文或重复未读域。
8. Preflight blocker 不可绕过；warning 可确认；revision conflict 不丢草稿。
9. Fresh Worktree 每次独立并由 run chat 持有，终态后可继续使用。
10. Persistent Worktree 同时只归 Task occurrence 或一个 attended Chat 持有；接管先暂停任务。
11. 归还、恢复任务、安全移交、丢弃是可区分且 fail-closed 的动作。
12. queued 崩溃可幂等恢复；executing 崩溃不重放模型/工具；不会重复外投。
13. 新实现不读写 `CronChatState`，不引入 Cron 专用 composer/stream/Session 晋升。
14. 相对 `be48cc40e` 净增不超过 12,000 行。

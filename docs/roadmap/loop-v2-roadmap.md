# Loop v2 路线图

> 返回 [路线图索引](README.md)
>
> 日期：2026-07-05
>
> 状态：Loop v2 核心已落地。本文继续作为后续增强 roadmap 保留；当前实现细节以 [`docs/architecture/loop.md`](../architecture/loop.md) 为准。

## 1. 背景

Agent 控制平面 v1 已关闭。v1 中 Loop 已从旧的“执行强度”语义中剥离出来，成为真实的持续触发控制面：

```text
Goal      = 最终目标、完成标准、证据、最终审计
Workflow  = 一次具体执行编排
Loop      = 按时间、条件或后续事件重复触发推进
Mode      = 推进强度
Task      = 用户可见进度事实
Worktree  = coding 改动隔离环境
```

v1 Loop 已具备：

- interval / cron / condition 语义基础。
- Cron 复用，不另起 scheduler。
- session / Goal 绑定。
- `continue` 与 `workflow` execution strategy。
- pause / resume / stop。
- budget、run count、runtime window。
- Workspace inline Loop 区块。
- Loop run trace 和 WorkflowRun 派生索引。

Loop v2 已完成从“能触发”到“可靠、不过度、可解释、可停止”的核心升级；后续继续增强外部系统触发和更复杂的模板化 loop 图。

## 当前落地摘要（2026-07-06）

已落地：

- Loop Center v2：blocked / active / paused / completed / cancelled 排序分组、查看更多、run history、next run、progress state、guard streak、budget、blocked reason、pause/resume/stop/run now/edit policy。
- Criteria Binding v2：创建时绑定 Goal criteria；触发前检查 Goal state、criteria revision/text/kind，Goal completed 自动完成 Loop，criteria stale 自动 blocked。
- Progress Guard：每次 run 记录 `progress_state`、`progress_delta_json`、`no_progress_reason`、`scheduling_decision`；基于 durable Goal evidence delta，不以 LLM 自评为唯一依据。
- Backoff / blocked：连续 `no_progress` 或 `failed` 会按 `backoff_secs` 降频，达到 `max_no_progress_runs` / `max_failures` 后 blocked 并暂停 Cron。
- Policy edit / run-now：owner API 和 GUI 支持 `run_loop_schedule_now`、`update_loop_schedule_policy`，并同步 Cron job 的 `max_failures` / `job_timeout_secs`。
- Workflow strategy 可观察性：Loop row 聚合 `origin=loop:<id>` 的 Workflow run，Loop run history 显示 `workflowRunId` / template version，GUI 可跳转 Workflow detail。
- Event-triggered Loop：内部 EventBus 白名单事件 `workflow:updated` / `goal:updated` / `task_updated` 已接入；支持 state/status filter、debounce 去重、durable event tick、`eventContext` trace 和 GUI 创建。
- 验证：`cargo test -p ha-core loop_control --locked` 中 Loop 相关测试覆盖 progress guard、criteria guard、policy edit、event enqueue/dedup/context；WorkspacePanel Loop Vitest 覆盖 view-more、run-now、policy edit、history、event create。

后续池：

- 外部事件源仍未开放；webhook / file watcher / CI provider / connector object stream 等需要独立治理事件风暴、鉴权、幂等和可见来源。
- `until --workflow` 仍未开放；等待 Workflow terminal event 能可靠反写 condition result。
- `cost_budget_micros` 仍保守拒绝；等待 provider cost ledger。

## 2. 产品目标

Loop v2 要回答：

```text
为什么这个 Loop 会继续运行？
它下一次什么时候运行？
它每次运行有没有推进 Goal？
连续没进展时会不会空转？
用户在哪里暂停、恢复、查看所有历史？
什么时候应该自动降频、阻塞或请求确认？
```

成功标准：

- 用户能在 GUI 中创建、查看、暂停、恢复、停止、展开所有 Loop，不依赖 `/loop status`。
- 每次 Loop run 都有原因、输入、结果、是否推进 Goal、是否触发 Workflow 的可见记录。
- Loop 能绑定 Goal criteria，并在 Goal 完成、取消、预算耗尽或用户关闭后停止。
- 连续无进展、重复失败、审批长期无人处理、预算异常时，Loop 会降频、blocked 或请求用户确认。
- Loop 支持通用任务：coding CI 轮询、research refresh、writing draft polish、data monitor、inbox follow-up、project ops check-in。
- Loop 不制造无人值守权限绕过，不绕过 Workflow / Permission / Goal budget。

## 3. 非目标

Loop v2 不做这些事：

- 不把执行强度重新放回 `/loop`；强度仍由 `/mode` 控制。
- 不让 Loop 直接替代 Workflow runtime。
- 不允许 Loop 自己绕过审批或 Connector guard。
- 不做无限后台 agent；所有 Loop 都必须有 budget、stop condition 或用户可见治理。
- 不把所有 Cron 功能搬到 Loop；普通 Cron 仍存在。
- 不强制所有用户都使用 Loop；短任务不需要 Loop。

## 4. 核心设计原则

| 原则 | 含义 |
| --- | --- |
| Loop 是持续触发器 | Loop 只决定何时再次推进，不直接定义完成标准。 |
| Goal-aware | 有 Goal 时，Loop 必须说明推进哪条 objective / criteria。 |
| Progress-sensitive | Loop 不应无限重复无进展运行。 |
| User-governed | 用户必须能看见、暂停、恢复、停止、查看历史。 |
| Permission-preserving | Loop 不绕过审批、sandbox、connector guard、incognito。 |
| Cost-aware | 预算、次数、频率、退避都要显式可见。 |

## 5. 用户体验目标

### 5.1 创建 Loop

GUI 创建器应支持：

- Trigger：every / cron / until / event。
- Goal binding：选择 active Goal 和可选 criteria。
- Strategy：继续会话 / 创建 Workflow。
- Prompt：每次触发时要做什么。
- Stop condition：次数、时间窗口、Goal completed、criteria satisfied、until condition satisfied。
- Budget：token、runtime、cost 预留、max failures。
- Backoff：连续失败或空转后的降频策略。

Slash 仍可用，但不是核心路径：

```text
/loop every 30m: check CI and continue if failing
/loop until "weekly brief is ready" every 1h --workflow: refresh research and update the brief
/loop status
```

### 5.2 Loop Center

Workspace 应从 inline 小区块升级为 Loop Center：

- active / paused / blocked / completed / cancelled 分组。
- 不只显示前 5 个；提供“查看更多 Loop”。
- 每个 Loop 显示：
  - trigger spec。
  - next run。
  - bound Goal / criteria。
  - strategy。
  - run count。
  - last result。
  - progress signal。
  - budget。
  - blocked reason。
- 操作：pause、resume、stop、run now、edit budget、open history。

### 5.3 Loop Run Detail

每次 run 应可展开：

- trigger reason。
- scheduled / started / finished time。
- injected prompt 或 workflow run id。
- permission / approval state。
- result summary。
- progress delta：新增了哪些 evidence、tasks、artifacts、workflow terminal state。
- no-progress reason。
- next scheduling decision：continue / backoff / blocked / completed。

## 6. 数据模型增强

Loop v2 优先扩展 v1 `loop_schedules` / `loop_runs`。

建议新增或派生：

| 能力 | 设计 |
| --- | --- |
| criteria binding | 已由 Goal v2 提前落地 `goal_criterion_id/text/kind/goal_revision`；Loop v2 继续补 criteria 修改后的 needs-rebind / blocked。 |
| progress ledger | 每次 run 记录 `progress_state`: progressed / no_progress / blocked / failed / awaiting_approval。 |
| no-progress streak | 连续无进展计数，用于 backoff 或 blocked。 |
| failure policy | `max_failures`、`failure_backoff_secs`、`on_exhausted`: pause/block/ask_user。 |
| next run projection | 存或派生 `next_run_at`，GUI 直接可见。 |
| run evidence delta | run 结束时记录新增 evidence ids、workflow run ids、task ids、artifact ids。 |
| user intervention | 记录用户手动 resume/run now/edit budget 的 event。 |

状态仍保持简单：

```text
active | paused | blocked | completed | cancelled
```

不要增加大量中间态；运行中的细节归 `loop_runs`。

## 7. Progress Guard

Loop v2 的核心新增能力是反空转。

每次 run 结束后计算 progress：

| 判定 | 条件 |
| --- | --- |
| progressed | 新增 strong evidence、task completed、workflow completed、artifact created、criteria satisfied。 |
| weak_progress | 有新信息但未推进 criteria，如 fresh fetch、diagnostic update、draft revision。 |
| no_progress | 没有新增有效 evidence，或重复得到同样结果。 |
| blocked | permission denied、approval timeout、connector unavailable、Goal budget exhausted、required input missing。 |
| failed | workflow failed、tool failed、runtime error。 |

调度决策：

- `progressed`：保持原频率或按策略继续。
- `weak_progress`：允许继续，但累计观察。
- `no_progress` 连续 N 次：自动 backoff。
- `no_progress` 达上限：Loop blocked，请用户确认。
- `blocked`：暂停 Cron，显示具体 blocked reason。
- `failed` 连续 N 次：backoff 或 blocked。
- Goal completed / cancelled：Loop completed 或 cancelled。

Progress Guard 不能用 LLM 自评作为唯一依据；必须优先使用 durable evidence delta。

## 8. Trigger v2

### 8.1 Interval / Cron

v1 已支持，v2 增强：

- next run projection。
- skip reason 可见。
- missed run 记录。
- backoff 后显示新的 next run。

### 8.2 Condition

v1 condition 依赖 assistant marker。v2 增强：

- condition 文本结构化保存。
- condition check result 写 run detail。
- condition satisfied 后自动 completed。
- condition 不满足但无新 evidence 时进入 no-progress 计数。

### 8.3 Event-triggered Loop

新增能力：

- workflow terminal event。
- task state changed。
- file changed / CI changed。
- connector object changed。
- knowledge note changed。

第一版 event trigger 已先做内部 EventBus 事件：

- workflow completed/failed/blocked。
- goal updated/completed/blocked。
- task pending/in_progress/completed。

外部事件仍进入后续池。

## 9. Goal / Workflow 集成

### 9.1 Goal-aware Loop

创建 Loop 时，如果有 active Goal：

- 默认绑定 Goal。
- 可选择推进某条 criteria。
- Goal completed/cancelled 后，Loop 自动 completed/cancelled。
- Goal criteria 删除或修改后，Loop 标记 `needs_rebind` 或 blocked。

### 9.2 Workflow Strategy

Loop v2 不重写 workflow strategy，但要让它更可见：

- 每个 workflow strategy run 都能跳到 Workflow Run Detail。
- Loop Run Detail 显示 Workflow terminal state。
- Workflow evidence delta 回写 Loop progress ledger。
- Workflow blocked 时，Loop 也显示 blocked reason，而不是只显示“触发成功”。

### 9.3 Task 映射

Loop run 产生或更新 Task 时：

- 记录 task ids。
- TaskProgressPanel 显示来源 Loop。
- Loop Center 可按 task state 聚合 run outcome。

## 10. 阶段计划

### L2.1 Loop Center v2（已完成）

目标：解决用户看不全、管不住 Loop 的问题。

工作项：

- Workspace Loop Center 分组视图。
- “查看更多 Loop”。
- Run history 展开。
- next run、last result、blocked reason、budget 可见。
- pause/resume/stop/run now/edit budget。

验收：

- GUI 能完成所有核心 Loop 管理动作。
- 超过 5 个 Loop 时不再依赖 `/loop status`。
- 用户能从一行 Loop 看懂它为什么存在。

### L2.2 Criteria Binding（已完成核心）

目标：让 Loop 明确推进哪条 Goal criteria。

工作项：

- 已落地：GUI create loop 支持 `goalCriterionId`；Goal Detail 中按 criteria 显示相关 Loop；workflow strategy 派生 run 继承 criteria。
- 已落地：触发前检查 Goal state 与 criteria revision/text/kind；Goal completed 自动完成 Loop；criteria 修改后 Loop blocked，要求用户重新确认或编辑策略。

验收：

- 用户能回答“这个 Loop 推进哪条目标标准”。
- Goal completed 后相关 Loop 自动停止。
- 被删除 criteria 的 Loop 不会继续悄悄跑。

### L2.3 Progress Guard（已完成核心）

目标：防止 Loop 空转。

工作项：

- 每次 run 计算 evidence delta。
- 记录 progress_state、no_progress_streak、failure_streak。
- 支持 backoff / blocked；ask user 作为后续增强池，不作为当前自动触发路径，避免无人值守确认死锁。
- GUI 显示 no-progress reason。

验收：

- 连续无进展不会无限按原频率运行。
- blocked reason 可见且可恢复。
- Progress 判定有 deterministic tests，不只靠 LLM 文本。

### L2.4 Trigger v2（内部事件已完成，外部事件后续池）

目标：把持续推进从纯时间触发扩展到内部事件触发。

已落地：

- internal EventBus trigger adapter。
- 支持 workflow terminal、goal state、task state 三类事件。
- Event trigger 与 Cron interval 共用 Loop store / run detail。
- 去重与 debounce，避免事件风暴。
- 事件 payload 先写 durable tick，再进入 run trace 和模型 `<event_context>`；运行中到来的事件不会丢失，会在当前 run 后继续排队消费。

后续：

- 外部 webhook / file watcher / CI provider / connector object stream。
- 外部事件的来源鉴权、重放、幂等和用户可见审计。

验收：

- workflow failed 可触发 follow-up Loop。
- task completed 可触发 follow-up / reminder；pending/in_progress 可作为轮询或看板状态变化触发。
- 同一事件不会重复创建多个 run。

### L2.5 Workflow Strategy 可观察性（已完成核心）

目标：让 Loop 触发的 Workflow 不再像黑盒。

工作项：

- Loop Run Detail 嵌入 workflow summary。
- Workflow Run Detail 反向显示 origin loop。
- progress ledger 从 workflow terminal / evidence delta 计算。
- permission awaiting / approval denied / blocked 映射到 Loop run outcome。

验收：

- 用户能从 Loop 跳到 Workflow，再从 Workflow 回到 Loop。
- Workflow failed/blocked 不会被 Loop 误显示为成功。
- Soak / operational gate 能读取 Loop -> Workflow 的完整链路。

### L2.6 Loop v2 验证（已完成核心）

测试与样本：

- Rust deterministic tests：criteria stale blocked、progress guard、backoff、goal completed stop、durable evidence reset、policy edit sync。
- GUI Vitest：Loop Center、view more、run detail、run now、edit policy、workflow trace context。
- Source-level UX audit：用户不用 slash command 管理 Loop。
- Soak fixture：长时间 interval + no-progress backoff + budget stop。
- Domain fixture：coding CI、research refresh、writing polish。

退出标准：

- Loop v2 不依赖 `/loop status` 完成核心管理。
- Loop run history 能解释每次运行。
- 连续无进展会被治理。
- Loop 与 Goal criteria / Workflow evidence 的关系可见。
- 无痕、权限、预算、安全边界不回退。

## 11. 与 Goal v2 的关系

Loop v2 应在 Goal v2 criteria model 稳定后继续推进；基础 Criteria Binding 已由 Goal v2 提前落地，Loop v2 不重复定义 Goal 完成标准。

依赖：

- Goal structured criteria。
- Goal revision / stale audit。
- Goal closure decision。
- Goal prompt snapshot。

Loop v2 读取这些信息，但不拥有它们。Loop 只做持续触发、progress guard 和调度治理。

## 12. 后续池

这些不阻塞 Loop v2 第一版：

- 外部 webhook trigger。
- 文件系统 watcher trigger。
- CI provider 深度集成。
- Connector object change streaming。
- 成本 ledger 精确计费。
- Loop 模板市场。
- 跨 session / 跨 project Loop。
- 自然语言复杂计划器自动创建多 Loop 图。

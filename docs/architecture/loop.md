# Loop 控制平面

> 返回 [文档索引](../README.md) | 更新时间：2026-07-04

## 概述

Loop 是 Phase 2.9 落地的真实 `/loop`：它只表示按时间、条件或后续事件重复触发，不表示执行强度。执行强度仍由 `/mode` / Execution Mode 控制；具体执行编排仍由 Workflow 承载；最终目标和完成标准仍由 Goal 承载。

Loop 复用现有 Cron 调度器，不另起 scheduler。每个 Loop schedule 都有一个受控 Cron job；Cron 负责可靠 tick、primary-only、并发上限、启动恢复、失败退避、无人值守权限面，Loop store 负责 session 归属、Goal 绑定、执行策略、预算、次数、状态和可审计 trace。

## 用户语义

```text
/loop every 10m: check CI and continue fixing if failing
/loop every 10m --workflow: refresh the research brief
/loop until CI is green every 5m: inspect CI and fix the next failing issue
/loop status
/loop status <id>
/loop pause <id>
/loop resume <id>
/loop stop <id>
```

支持的预算参数：

- `--max-runs N`：最多触发次数。
- `--max-runtime 2h`：Loop 从创建开始的最长运行窗口。
- `--tokens N`：Loop token 预算，触发前 hard stop。
- `--workflow` / `--strategy workflow`：仅用于 `every` interval loop；触发时创建并启动绑定 Goal 的 Domain Workflow run，而不是继续原会话。
- `--cost-micros` / `cost_budget_micros` 字段已预留；当前创建时会拒绝该预算，等待 provider cost ledger 接入后放开。

创建 Loop 必须满足二者之一：绑定当前 open Goal，或提供明确 recurring prompt。无痕会话拒绝持久化 Loop。

## 数据模型

Loop 表落在 `sessions.db`，随 session 生命周期级联删除。

```text
loop_schedules
  id
  session_id
  goal_id?
  cron_job_id
  prompt
  trigger_kind: interval | cron | condition | event
  trigger_spec_json
  execution_strategy: continue | workflow
  state: active | paused | completed | cancelled | blocked
  max_runs
  run_count
  max_runtime_secs
  token_budget
  cost_budget_micros
  approval_policy_snapshot_json
  created_at / updated_at / completed_at
  blocked_reason

loop_runs
  id
  loop_id
  cron_job_id
  cron_run_log_id?
  session_id
  seq
  state: running | queued | injected | succeeded | empty | failed | cancelled | skipped
  trigger_reason
  result_summary
  error
  trace_json
  started_at / finished_at
```

`execution_strategy`：

- `continue`：默认策略。Cron tick 注入 `<loop_trigger>` 到原会话，触发一次 parent-session continuation。
- `workflow`：interval loop 的受控策略。Cron tick 不注入聊天，而是读取绑定 Goal 的 `workflow_template_id/version/task_type`，生成 Domain Workflow draft，创建并启动 durable Workflow run。

Cron job 的 `CronPayload::SessionLoop` 保存 `loop_id`、原会话 `session_id`、prompt、agent、goal。真实执行策略以 `loop_schedules.execution_strategy` 为准，普通 Cron `AgentTurn` 路径不变。

## 执行链

```mermaid
sequenceDiagram
    participant User
    participant Slash as /loop
    participant Loop as loop_control
    participant Cron as Cron Scheduler
    participant Inject as Parent Injection
    participant Chat as Chat Engine
    participant Workflow as Workflow Runtime

    User->>Slash: /loop every 10m: prompt
    Slash->>Loop: create_loop_schedule
    Loop->>Cron: create CronJob(SessionLoop)
    Cron-->>Loop: cron_job_id
    Loop-->>User: loop id / status

    Cron->>Loop: prepare_loop_cron_run
    Loop-->>Cron: admit / reject
    alt executionStrategy = continue
        Cron->>Inject: inject loop trigger into original session
        Inject->>Chat: run parent turn after idle gate
        Chat-->>Inject: persisted assistant turn
    else executionStrategy = workflow
        Cron->>Workflow: preview domain workflow + create WorkflowRun
        Workflow-->>Cron: run id / primary launch accepted
    end
    Cron->>Loop: finish_loop_cron_run
    Loop-->>User: loop:changed event / Workspace refresh
```

关键点：

- Cron claim 仍是 slot-before-claim，并发上限和 primary-only 语义不变。
- `SessionLoop` 不创建隔离 cron 会话，而是通过 `subagent::injection::inject_and_run_parent` 回到原会话。
- 注入消息带 `<loop_trigger>` 信封，并写 `attachments_meta.loop_trigger`，前端可以识别为系统触发。
- `condition` loop 注入时会带 `<condition>`，并要求 assistant 在条件满足时用 `LOOP_CONDITION_SATISFIED: <reason>` 开头；`finish_loop_cron_run` 识别该 marker 后把 Loop 置 `completed` 并暂停 Cron。
- `workflow` strategy 只支持 `interval` loop。`condition` loop 的完成语义当前依赖 assistant marker，不能伪装成 workflow 完成；后续若要支持，必须等 Workflow terminal event 能反写 condition result。
- `workflow` strategy 必须绑定 Goal，且该 Goal 必须选择 Domain Workflow template。触发时调用 `preview_domain_workflow(requirePlanConfirmation=false)`，通过 Script Gate / permission preview 后创建 `origin=loop:<loop_id>` 的 WorkflowRun 并请求 Primary runtime 启动。
- Loop workflow trigger 的 `loop_runs.trace_json` 会记录 `executionStrategy`、`workflowRunId`、`workflowKind`、`executionMode`、`templateId/version` 和是否需要审批；Workflow run 自己继续拥有完整 ops/events/recovery trace。
- 派生 WorkflowRun 终态后会被 Domain Operational Gate 与 Soak Report 作为同一 session/domain 的长任务运行证据读取；`loop_runs.trace_json.workflowRunId` 是从 Loop run 回到 Workflow detail 的审计索引。
- 若父会话正忙，注入沿用现有 idle gate；若被用户新 turn 抢占，进入 injection queue。
- `loop_schedules.state != active`、达到 `max_runs`、超过 `max_runtime_secs`、Loop token budget exhausted、Goal budget exhausted 都会在触发前拒绝，并暂停背后的 Cron job。

## Goal / Workflow / Mode 边界

| 概念 | 职责 |
| --- | --- |
| Goal | 顶层目标、完成标准、证据链、budget hard stop |
| Workflow | 一次具体执行编排，负责 op trace、审批、恢复、验证 |
| Execution Mode | 后续 turn 的推进策略，控制观察/计划/验证/修复强度 |
| Loop | 按时间/条件重复触发下一次推进 |
| Cron | 底层可靠调度器 |

Loop 绑定 Goal 时，每次 run 会写 `loop_run` evidence 到 Goal link/timeline。Loop 不绕过 Goal budget：触发前会调用 Goal budget 门禁，耗尽后 Loop 进入 `blocked` 并暂停 Cron。

Loop 自身 token budget 也会在触发前按 parent session 自创建后的消息 usage 计算；达到上限后进入 `blocked` 并暂停 Cron。成本预算目前只保留字段，不接受创建，避免没有 cost ledger 时给用户错误安全感。

## API / GUI

Owner API：

| Tauri Command | HTTP |
| --- | --- |
| `list_loop_schedules` | `GET /api/sessions/{sessionId}/loops` |
| `create_loop_schedule` | `POST /api/sessions/{sessionId}/loops` |
| `get_loop_schedule` | `GET /api/loops/{loopId}` |
| `pause_loop_schedule` | `POST /api/loops/{loopId}/pause` |
| `resume_loop_schedule` | `POST /api/loops/{loopId}/resume` |
| `stop_loop_schedule` | `POST /api/loops/{loopId}/stop` |

`create_loop_schedule` 额外接受 `executionStrategy?: "continue" | "workflow"`；省略时为 `continue`。

Slash：`/loop every <duration> --workflow: <prompt>` 与 `/loop every <duration> --strategy workflow: <prompt>` 会创建 `executionStrategy=workflow` 的 interval loop；`/loop until ... --workflow` 当前会被拒绝，直到 Workflow terminal event 能反写 condition result。`/loop status` 会展示每个 schedule 的 strategy；`/loop status <id>` 的 Recent runs 会从 `loop_runs.trace_json` 展示派生 workflow run id、template version 和结果摘要。

GUI：Workspace 面板新增 Loop 区块，支持创建 `every` / `until` loop，填写 interval、condition、prompt、max runs、max runtime、token budget；同时展示本会话 loop 数量、状态、触发摘要、prompt、运行次数、最大运行时长、blocked reason，并提供 pause / resume / stop。每个 Loop 行可按需展开“运行记录”，通过 `get_loop_schedule` 拉取最近 `loop_runs`，显示 run seq、state、时间、错误/摘要、派生 `workflowRunId` 与 template version。创建 `every` loop 且当前 active Goal 已选择领域模板时，用户可把执行方式从“继续会话”切到“创建工作流”；列表会用 `Workflow` 标记这类 loop，并根据同会话 Workflow run 的 `origin=loop:<loop_id>` 显示最近派生 run 的 kind、state、更新时间和跳转按钮。点击后 Workspace 会选中对应 Workflow run detail，继续查看审批、trace、validation、agents、pause/resume/cancel 等控制面。同一份 `useLoopSchedules` state 也供 Workflow 区块的「自主推进就绪」卡片读取，用于判断是否已有持续触发和阻塞 loop；readiness 卡片请求创建 loop 时，Loop 区块会展开创建器并预选 `every + workflow`，但仍由用户显式点击“创建 Loop”。

## 安全与可靠性

- 无痕会话拒绝 durable Loop。
- Loop 不新增工具权限捷径；实际 turn 仍走原会话的 permission mode、sandbox、hooks、Project/KB access。
- Loop 背后的 `CronPayload::SessionLoop` 是受控 Cron job；模型侧 `manage_cron` 不能 update / pause / resume / delete，必须走 Loop 控制面，避免 Loop store 与 Cron 状态分叉。
- Cron 背景无人值守语义保持 fail-closed 或遵循显式 policy。
- Loop workflow strategy 不插入 `workflow.askUser` 计划确认；自动触发不能自己制造无人值守确认死锁。敏感动作仍由 Workflow permission preview、运行时权限引擎、Domain Quality approval gate 和连接器授权 fail-closed。
- Loop workflow trigger 不绕过 Workflow Script Gate；内置 Domain Workflow draft 必须包含 task truth、`workflow.finish`、`workflow.verify` 复核计划和显式 budget hint。
- Loop 停止只把 Loop 置 `cancelled` 并暂停底层 Cron job；不会删除历史 trace。
- EventBus 发 `loop:changed`，前端和 HTTP/WS 订阅可刷新状态。

## 后续增强

- Event-triggered loop：接入 EventBus / CI / file watcher，不只靠 interval/cron。
- 独立 Loop detail 页面：Workspace 已有 inline 最近运行记录；后续可做全屏 detail 展示完整 run trace、cron log、对应消息范围和 Goal evidence。
- 成本预算精确统计：接入 provider cost ledger，并放开 `cost_budget_micros` 创建限制。
- Condition workflow：等 Workflow terminal event 能反写 condition result 后，支持 until loop 直接创建 workflow。

## 测试覆盖

- `workflow_strategy_materializes_domain_workflow_run` 覆盖 Goal 绑定领域模板后，interval Loop workflow strategy 能生成 `origin=loop:<id>` 的 durable WorkflowRun，并把 `workflowRunId` / template version 写入 loop run trace。
- `workflow_strategy_feeds_operational_and_soak_gates` 覆盖同一条 Goal → Loop tick → WorkflowRun → terminal → LoopRun trace 链路会进入 Domain Operational Gate 和 Soak Report，证明 Workspace 的运行稳定性 / 长跑审计卡片读取的是真实控制面证据。

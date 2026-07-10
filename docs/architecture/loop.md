# Loop 控制平面

> 返回 [文档索引](../README.md) | 更新时间：2026-07-08

## 概述

Loop 是通用持续触发控制面；用户面名称是「持续推进」。它只表示按时间、条件或后续事件重复触发，不表示执行强度。执行强度仍由 `/mode` / Execution Mode 控制；具体执行编排仍由 Workflow 承载；最终目标和完成标准仍由 Goal 承载。

Loop v2 在 v1 的“能触发”之上补齐可靠治理：复用现有 Cron 调度器，不另起 scheduler；每个 Loop schedule 都有一个受控 Cron job。Cron 负责可靠 tick、primary-only、并发上限、启动恢复、底层失败退避、无人值守权限面；Loop store 负责 session 归属、Goal / Goal criteria 绑定、执行策略、预算、次数、progress ledger、no-progress / failure streak、backoff / blocked 决策和可审计 trace。

## 用户语义

```text
/loop 5m check CI and continue fixing if failing
/loop check CI and continue fixing if failing every 5m
/loop check CI and address review comments
/loop
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

创建 Loop（持续推进）必须满足二者之一：绑定当前 open/pending closure Goal，或提供明确 recurring prompt。无痕会话拒绝持久化 Loop。

GUI 创建器在 active Goal 有拆分标准时提供「推进标准」选择器；默认绑定整个 Goal，选择具体标准后写 `goalCriterionId`，用于解释这个 Loop 为什么存在、推进哪条完成标准。输入框 `+` 菜单中的「持续推进」会打开 Workspace 并展开创建器。Slash `/loop` 当前仍只表达 recurring prompt / workflow strategy，不解析 criteria id。

Loop V3.1 开始，Slash 固定间隔创建支持更自然的 Claude Code 风格写法：`/loop 5m <prompt>` 与 `/loop <prompt> every 5m` 会创建 interval Loop，并在创建后立即通过核心 `spawn_loop_schedule_run_now` 路径触发第一轮；旧的 `/loop every 5m: <prompt>` 继续保留且同样立即触发第一轮。

Loop V3.2 开始，Slash prompt-only 写法 `/loop <prompt>` 会创建 dynamic self-paced Loop，并立即触发第一轮。裸 `/loop` 不再等同于 status，而是创建一个 dynamic maintenance Loop：优先读取当前会话工作目录中的 `loop.md`、`.hope/loop.md`、`.hope-agent/loop.md`、`.claude/loop.md` 作为默认持续推进指令，其次读取 Hope Agent 用户 home 下的同名文件，均不存在时使用内置通用维护 prompt。`loop.md` 读取上限为 25KB，避免超大项目说明撑爆循环 prompt。maintenance Loop 会把 prompt 来源写入 `triggerSpec.maintenancePrompt`，并在每次 Cron trigger admission 前重新解析同一来源顺序；如果文件内容或来源变化，会更新 `loop_schedules.prompt` / `trigger_spec_json` 并把 metadata 写入当前 `loop_runs.trace_json.maintenancePrompt`。这不是常驻 watcher，不新增后台线程或外部事件面，只在既有 Cron 触发路径上刷新。查看状态必须显式使用 `/loop status`。

dynamic Loop 仍复用 Cron durable job 和 Loop run history。每轮结束前，模型应优先通过内部工具 `loop_reschedule` / `loop_stop` 明确选择下一次 wakeup、完成或阻塞；文本 marker `LOOP_RESCHEDULE_AFTER: <duration> - <reason>`、`LOOP_STOP: <reason>`、`LOOP_BLOCKED: <reason>` 仍作为兼容兜底。finish 阶段会先读取当前 `loop_runs.trace_json.dynamicDecision` 中的工具决策，只有没有工具决策时才解析最终 assistant summary。若两者都缺失，系统只安排一次 fallback wakeup；fallback 回合仍无决策时，Loop 进入 `blocked` 并暂停 Cron，避免无限空转。模型可选间隔被钳在 1 分钟到 1 小时之间，默认 fallback 为 20 分钟；dynamic Loop 若未显式设置 runtime，会有 7 天的默认生命周期上限。

V3.6 的 Loop restart/resume 验收同样采用 durable conservative recovery：Loop schedule、Cron job、run history、dynamic decision、`nextRunAt` / `cronStatus` 派生状态必须在重启后可恢复；重启不能重复 claim 同一 tick、不能静默吞掉 pending event，也不能把需要审批或外部动作的 run 自动放行。若父会话 turn 或 Workflow 子 run 在进程退出时被打断，Loop 可以通过 `loop_run_maybe_interrupted`、no-progress / blocked reason、run history 和“立即运行”恢复入口把状态暴露给用户；V3 不要求透明续跑已被系统杀掉的父会话 turn。

## 数据模型

Loop 表落在 `sessions.db`，随 session 生命周期级联删除。

```text
loop_schedules
  id
  session_id
  goal_id?
  goal_criterion_id?
  goal_criterion_text?
  goal_criterion_kind?
  goal_revision?
  cron_job_id
  prompt
  trigger_kind: interval | cron | condition | event | dynamic
  trigger_spec_json
  execution_strategy: continue | workflow
  state: active | paused | completed | cancelled | blocked
  max_runs
  run_count
  max_runtime_secs
  token_budget
  cost_budget_micros
  progress_state: progressed | weak_progress | no_progress | blocked | failed | awaiting_approval
  progress_summary
  no_progress_streak
  failure_streak
  max_no_progress_runs
  max_failures
  backoff_secs
  approval_policy_snapshot_json
  created_at / updated_at / completed_at
  blocked_reason
  next_run_at?       # 从 Cron job 派生给 owner API / GUI
  cron_status?       # 从 Cron job 派生给 owner API / GUI

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
  progress_state
  progress_delta_json
  no_progress_reason
  scheduling_decision
  trace_json
  started_at / finished_at

loop_event_ticks
  id
  loop_id
  event_name
  event_fingerprint
  event_payload_json
  created_at
  consumed_at?
  loop_run_id?
```

`execution_strategy`：

- `continue`：默认策略。Cron tick 注入 `<loop_trigger>` 到原会话，触发一次 parent-session continuation。
- `workflow`：interval loop 的受控策略。Cron tick 不注入聊天，而是读取绑定 Goal 的 `workflow_template_id/version/task_type`，生成 Domain Workflow draft，创建并启动 durable Workflow run。

Criteria 绑定：

- `create_loop_schedule` 可接收 `goalCriterionId`；后端校验它属于绑定 Goal 当前 revision，写入 `goal_criterion_id/text/kind/goal_revision`。
- 触发前会重新检查 Goal state 与 criteria revision：Goal completed 会让 Loop completed；Goal failed/cancelled/paused 或 criteria 删除/修改会让 Loop blocked 并暂停 Cron，避免静默推进错误目标。
- Loop 创建、trigger、terminal `loop_run` Goal link metadata 带 `goalCriterion`。
- `execution_strategy=workflow` 派生的 WorkflowRun 继承 `goal_criterion_id`，因此 Goal detail 能按 criteria 同时看到 Loop 与 Workflow 进展。
- Cron 注入的 `<loop_trigger>` 会包含 `<goal_criterion_id>` 与 `<goal_criterion_text>`，让模型在继续会话模式下也知道本轮优先推进哪条标准。

Cron job 的 `CronPayload::SessionLoop` 保存 `loop_id`、原会话 `session_id`、prompt、agent、goal。真实执行策略以 `loop_schedules.execution_strategy` 为准，普通 Cron `AgentTurn` 路径不变。

Dynamic trigger：

- `trigger_kind=dynamic` 支持 Claude Code 风格的 prompt-only `/loop <prompt>`，以及裸 `/loop` 的默认 maintenance loop。
- 裸 `/loop` 的默认 prompt 来源顺序：session working dir `loop.md` / `.hope/loop.md` / `.hope-agent/loop.md` / `.claude/loop.md` → Hope Agent home 同名文件 → 内置通用维护 prompt；文件内容最多 25KB。
- `trigger_spec_json` 规范形态为 `{ fallbackSecs, fallbackUsed, maintenancePrompt? }`；`fallbackSecs` 默认 1200 秒，读入时钳在 60 到 3600 秒之间，`fallbackUsed` 表示上一轮是否已经因为模型未决策排过一次兜底 wakeup。`maintenancePrompt` 仅由裸 `/loop` 写入，形如 `{ enabled, source, path?, contentHash? }`；显式 `/loop <prompt>` 和 GUI dynamic prompt 不带该字段，因此不会被 `loop.md` 热更新覆盖。
- 每次 `prepare_loop_cron_run` admit dynamic maintenance Loop 前，会通过 `resolve_default_loop_prompt_for_session` 重新读取 `loop.md` / built-in prompt；如果 hash 或 source 变化，先更新 schedule 再插入 run。run trace 保存 `maintenancePrompt` metadata，便于审计这一轮实际用了哪个 prompt 来源。
- Cron job 仍是 `CronPayload::SessionLoop`，schedule 采用 `Every(fallbackSecs)` 作为基础兜底；真实下一次触发时间由 run finish 后的 `LoopAfterRunAction.backoff_secs` 通过 `CronDB::delay_next_run` 覆盖，不改写原始 schedule。
- `<loop_trigger>` 会注入 dynamic self-paced contract，要求模型每轮显式调用 `loop_reschedule` / `loop_stop`，或兼容输出 `LOOP_RESCHEDULE_AFTER` / `LOOP_STOP` / `LOOP_BLOCKED` 之一。
- Agent 工具面首版包括 `loop_status`、`loop_reschedule`、`loop_stop`、`loop_record_progress`。这些工具是 internal Core Interaction，只能操作当前 session 的 Loop store/Cron job：`loop_status` 只读；`loop_reschedule` 只允许 active dynamic Loop，并把 `dynamicDecision{source:"tool"}` 写入当前 run trace，同时通过 `CronDB::delay_next_run` 设置下一次触发；`loop_stop` 将 Loop 置 `completed` 或 `blocked` 并暂停 Cron；`loop_record_progress` 只记录轻量进度，不算强完成证据、不绕过 Progress Guard。
- `finish_loop_cron_run` 先读取当前 run trace 中的工具决策，再解析模型最终 assistant summary：reschedule 写 `dynamic_reschedule_<secs>s` 并设置下一次 wakeup；stop 置 `completed`；blocked 置 `blocked`；missing decision 先写 `dynamic_fallback_<secs>s` 并设置 `fallbackUsed=true`，第二次仍 missing 则写 `blocked_dynamic_missing_decision` 并暂停。
- dynamic Loop 若没有 `max_runtime_secs`，触发前和 finish 后按 7 天默认生命周期上限完成，避免遗忘 Loop 无限运行。

Event trigger：

- `trigger_kind=event` 支持内部 EventBus 事件：`workflow:updated`、`goal:updated`、`task_updated`。
- `trigger_spec_json` 规范形态为 `{ eventName, filters, debounceSecs }`。`workflow:updated` 支持 `filters.workflowState`，`goal:updated` 支持 `filters.goalState`，`task_updated` 支持 `filters.taskStatus`。
- Event Loop 创建时仍复用一个 `CronPayload::SessionLoop` job，但底层 Cron job 保持 `paused`；事件 watcher 在 primary 进程订阅 EventBus，匹配后写 `loop_event_ticks` 并走 Cron `execute_job_public` immediate path。这样事件触发与 run-now、权限、预算、run history、primary-only 语义完全一致。
- `loop_event_ticks` 的 `event_fingerprint` 由 loop id、event name、匹配身份和 debounce 时间桶生成，用于同一事件风暴去重。若事件到来时 Loop 正在运行，tick 会留在 durable 队列；当前 run 结束后还有 pending tick 时会自动再排一次 immediate run，避免吞事件。
- `prepare_loop_cron_run` 消费最早 pending tick，把 `eventContext` 写入 `loop_runs.trace_json` 并注入 `<event_context>` 给模型。手动 run-now 触发 event loop 时允许没有 event context。

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
- `condition` loop 注入时会带 `<condition>`，并要求 assistant 在条件满足时用 `LOOP_CONDITION_SATISFIED: <reason>` 开头；`finish_loop_cron_run` 识别该 marker 后把 Loop 置 `completed` 并暂停 Cron。`dynamic` loop 注入时会带 self-pacing contract，要求 assistant 用 `loop_reschedule` / `loop_stop` 或兼容 marker 明确选择下一步。
- `workflow` strategy 只支持 `interval` loop。`condition` loop 的完成语义当前依赖 assistant marker，不能伪装成 workflow 完成；后续若要支持，必须等 Workflow terminal event 能反写 condition result。
- `workflow` strategy 必须绑定 Goal，且该 Goal 必须选择 Domain Workflow template。触发时调用 `preview_domain_workflow(requirePlanConfirmation=false)`，通过 Script Gate / permission preview 后创建 `origin=loop:<loop_id>` 的 WorkflowRun 并请求 Primary runtime 启动。
- Loop workflow trigger 的 `loop_runs.trace_json` 会记录 `executionStrategy`、`workflowRunId`、`workflowKind`、`executionMode`、`templateId/version` 和是否需要审批；Workflow run 自己继续拥有完整 ops/events/recovery trace。
- 派生 WorkflowRun 终态后会被 Domain Operational Gate 与 Soak Report 作为同一 session/domain 的长任务运行证据读取；`loop_runs.trace_json.workflowRunId` 是从 Loop run 回到 Workflow detail 的审计索引。
- 若父会话正忙，注入沿用现有 idle gate；若被用户新 turn 抢占，进入 injection queue。
- `loop_schedules.state != active`、达到 `max_runs`、超过 `max_runtime_secs`、Loop token budget exhausted、Goal budget exhausted、Goal terminal、criteria stale 都会在触发前拒绝，并暂停背后的 Cron job。
- run 结束后会计算 deterministic Progress Guard：优先读取 Goal durable evidence delta（workflow completed、validation passed、file changed、artifact created、source cited、domain quality passed 等 strong evidence），再看 Workflow trace / run state；不会把“Loop 跑了一次”本身当成进展。
- `progressed` / `weak_progress` 会清空 no-progress / failure streak；`no_progress` 连续累计后先 backoff，达到 `max_no_progress_runs` 后 blocked；`failed` 连续累计后按 `max_failures` backoff / blocked；`blocked` 立即暂停。
- backoff 通过 CronDB 的窄接口只推迟 active job 的 `next_run_at`，不改变原始 schedule，不复活 paused / terminal job。
- Event Loop 不参与 Cron 时间轮；GUI 显示为“等待事件”。连续无进展 / 失败仍会累计 streak，达到上限后 blocked 并停止响应后续事件。

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

`LoopRun.usage` 是 V3.6 的 run 级可观察出口。`list_loop_runs` / `get_loop_schedule` 返回每个 run 的 `usage`，字段包含 `messageCount`、`userTurns`、`assistantMessages`、`inputTokens`、`outputTokens`、`totalTokens`、`attribution`，以及 `providerEvents`、`providerInputTokens`、`providerOutputTokens`、`providerCacheCreationInputTokens`、`providerCacheReadInputTokens`、`providerTotalTokens`、`providerAttribution`。继续当前对话的 Loop run 会优先用注入用户消息的 `attachments_meta.loop_trigger.run_id` 精确定位触发 turn，并统计该 user row 到下一条 user row 之前的 user/assistant 消息，`attribution=loop_trigger_message_boundary`；这能排除同一时间窗口内的其它人工消息或后台注入。历史数据或异常路径没有触发元数据时回退到窗口口径：只统计同一 session 中 `started_at <= messages.timestamp <= finished_at` 的 user/assistant 消息，running run 使用 `session_messages_since_loop_run_start`。两种口径都让 input 优先 `tokens_in_last` 再回退 `tokens_in`，output 使用 `tokens_out`。Provider usage 额外通过 `model_usage_events.request_key = 'message:' || assistant_message_id` 聚合该 run 内 assistant message 对应的 chat usage event，`providerAttribution=model_usage_events.request_key=message_id`；找不到事件时返回 0 并标注 `no_linked_model_usage_events...`。这是可靠的 run 级消耗审计，用于判断 budget 压力；provider event 字段可支撑后续 cost ledger，但当前仍不冒充完整账单成本，也不放开 `cost_budget_micros`。

## API / GUI

Owner API：

| Tauri Command | HTTP |
| --- | --- |
| `list_loop_schedules` | `GET /api/sessions/{sessionId}/loops` |
| `list_loop_watchdog_findings` | `GET /api/sessions/{sessionId}/loops/watchdog?graceSecs=120` |
| `create_loop_schedule` | `POST /api/sessions/{sessionId}/loops` |
| `get_loop_schedule` | `GET /api/loops/{loopId}` |
| `pause_loop_schedule` | `POST /api/loops/{loopId}/pause` |
| `resume_loop_schedule` | `POST /api/loops/{loopId}/resume` |
| `stop_loop_schedule` | `POST /api/loops/{loopId}/stop` |
| `run_loop_schedule_now` | `POST /api/loops/{loopId}/run-now` |
| `update_loop_schedule_policy` | `PATCH /api/loops/{loopId}/policy` |

`create_loop_schedule` 额外接受 `executionStrategy?: "continue" | "workflow"`；省略时为 `continue`。Loop v2 还接受 `maxNoProgressRuns`、`maxFailures`、`backoffSecs`；省略时分别为 3 / 3 / 300s。`triggerKind=event` 时 `triggerSpec` 必须包含 `eventName`，可选 `filters` 与 `debounceSecs`。`list_loop_schedules` 与 `get_loop_schedule` 会从 Cron job 派生 `nextRunAt` / `cronStatus`；Event Loop 的 `nextRunAt` 返回空，`cronStatus=event` 表示正在监听内部事件。`list_loop_watchdog_findings` 是只读诊断端点，默认 `graceSecs=120`：active 非 event Loop backing Cron 缺失返回 `loop_cron_missing`；最新 Loop run 仍是 `running`、Cron 已无 `running_at`、且 run 持续超过 grace 时返回 `loop_run_maybe_interrupted`，覆盖重启/崩溃后 Cron startup recovery 已清理 running marker 但 Loop run 仍遗留 running 的情况；到期超过 grace、Cron active 且未 running、最新 Loop run 不是 `Running | Queued | Injected` 时返回 `loop_due_not_claimed`。该端点不触发 run、不 repair、不改状态。

`run_loop_schedule_now` 复用 Cron 的 `execute_job_public` / primary-only / immediate claim 路径，属于 active Loop 的一次性手动触发，不改写 recurring schedule，也不绕过 paused / blocked 状态；需要先 resume。`update_loop_schedule_policy` 更新 max runs、runtime、token budget、no-progress/failure/backoff 策略，并同步底层 Cron job 的 `max_failures` 与 `job_timeout_secs`；编辑 blocked Loop 的策略会清空当前 no-progress / failure streak，便于用户恢复。

Agent 工具：`loop_status`、`loop_reschedule`、`loop_stop`、`loop_record_progress` 是模型侧的受控 Loop runtime API。它们不新增用户配置项、不绕过权限引擎、不允许模型直接改 `manage_cron`；所有写操作都经 `loop_schedules` / `loop_runs` / `CronDB::delay_next_run|toggle_job`，并发 `loop:changed` 事件。

Slash：裸 `/loop` 会创建 `executionStrategy=continue` 的 dynamic maintenance Loop；`/loop <duration> <prompt>` 与 `/loop <prompt> every <duration>` 会创建 `executionStrategy=continue` 的 interval Loop；`/loop <prompt>` 会创建 `executionStrategy=continue` 的 dynamic Loop；`/loop every <duration> --workflow: <prompt>` 与 `/loop every <duration> --strategy workflow: <prompt>` 会创建 `executionStrategy=workflow` 的 interval loop；`/loop until ... --workflow` 当前会被拒绝，直到 Workflow terminal event 能反写 condition result。所有创建型 slash 成功后都会在后端通过 `spawn_loop_schedule_run_now` 走正常 owner run-now 路径触发第一轮，不改写 recurring schedule，也不绕过 primary-only / idle gate / 权限引擎；如果当前进程不是 Primary 或缺 runtime，会在 slash 结果里明确说明第一轮未启动。`/loop status` 会展示每个 schedule 的 strategy；`/loop status <id>` 的 Recent runs 会从 `loop_runs.trace_json` 展示派生 workflow run id、template version、dynamic decision 和结果摘要。

GUI：Workspace 面板中的「持续推进」中心支持创建 `every` / `dynamic` / `until` / `event` loop。创建器默认展示五个任务模板（检查 CI、刷新报告、任务后续、进展总结、外部状态），然后让用户选择触发方式、填写 prompt、选择“继续当前对话”或“按工作流执行”；dynamic 触发方式只暴露 fallback 间隔，模型每轮通过 `loop_*` 工具或兼容 marker 决定下一次继续/停止；max runs、max runtime、token budget、no-progress 上限、failure 上限和 backoff 间隔收进「高级保护」。创建 `every` loop 且当前 active Goal 已选择领域模板时，用户可把执行方式切到“按工作流执行”。

「持续推进」中心按 blocked / active / paused / completed / cancelled 排序分组，超过 5 个时提供“查看更多持续推进”，不依赖 `/loop status` 完成管理。每行先给一句可读状态故事，解释最近一次推进、下一次触发、阻塞或完成原因；随后显示 prompt、guard streak、runtime / token budget、progress summary、blocked reason，并提供 run now / edit policy / history / pause / resume / stop。edit policy 内联编辑 max runs、runtime、token、no-progress、failure、backoff；run now 走 Cron primary-only immediate path。每个 Loop 行可展开“运行记录”，通过 `get_loop_schedule` 拉取最近 `loop_runs`，显示 run seq、state、progress state、调度决策、no-progress reason、错误/摘要、派生 `workflowRunId`、template version 与本轮窗口 token usage。V3.6 起，Workspace 还会通过 `list_loop_watchdog_findings` 拉取只读 watchdog findings；存在 `loop_cron_missing`、`loop_run_maybe_interrupted` 或 `loop_due_not_claimed` 时，在「持续推进」区顶部用 amber 提示“有持续推进需要确认”，关联具体 Loop prompt、延迟时长，并提供“立即运行”和“运行记录”恢复动作。watchdog 拉取失败只记录日志，不影响 Loop 列表显示。

创建 `executionStrategy=workflow` 的 Loop 后，列表会用 `Workflow` 标记，并根据同会话 Workflow run 的 `origin=loop:<loop_id>` 显示最近派生 run 的 kind、state、更新时间和跳转按钮。点击后 Workspace 会选中对应 Workflow run detail，继续查看审批、trace、validation、agents、pause/resume/cancel 等控制面。Workspace 顶层共享同一份 `useGoal`、`useWorkflowRuns` 与 `useLoopSchedules` state 给 Goal、Workflow 与 Loop 区块，避免重复请求并确保 active Goal 模板、派生 Workflow run 和 Loop 状态一致。输入框「持续推进」或其它 owner 入口请求创建 loop 时，Loop 区块会展开创建器并预选合适策略，但仍由用户显式点击“创建持续推进”；查看 blocked loop 只展开 Loop 区块并打开对应运行记录，不自动 resume / stop。

## 安全与可靠性

- 无痕会话拒绝 durable Loop。
- Loop 不新增工具权限捷径；实际 turn 仍走原会话的 permission mode、sandbox、hooks、Project/KB access。
- Loop 背后的 `CronPayload::SessionLoop` 是受控 Cron job；模型侧 `manage_cron` 不能 update / pause / resume / delete，必须走 Loop 控制面，避免 Loop store 与 Cron 状态分叉。
- Cron 背景无人值守语义保持 fail-closed 或遵循显式 policy。
- Loop workflow strategy 不插入 `workflow.askUser` 计划确认；自动触发不能自己制造无人值守确认死锁。敏感动作仍由 Workflow permission preview、运行时权限引擎、Domain Quality approval gate 和连接器授权 fail-closed。
- Loop workflow trigger 不绕过 Workflow Script Gate；内置 Domain Workflow draft 必须包含 task truth、`workflow.finish`、`workflow.verify` 复核计划和显式 budget hint。
- Owner API 的 Loop 停止只把 Loop 置 `cancelled` 并暂停底层 Cron job；不会删除历史 trace。模型工具 `loop_stop` 只用于 dynamic runtime 决策，可把当前 Loop 收口为 `completed` 或 `blocked`，并同样保留 trace。
- EventBus 发 `loop:changed`，前端和 HTTP/WS 订阅可刷新状态。

## Loop v2 已落地能力与后续边界

Loop v2 当前已把 Loop 从“能触发”升级为可靠、可治理、可解释的持续推进器：

- 「持续推进」中心能在 GUI 内完成核心管理：模板创建、分组、查看更多、run detail、next run、progress state、guard streak、budget、blocked reason、run now、edit policy、pause/resume/stop。
- Progress Guard 已基于 deterministic durable evidence delta：strong evidence 包括 workflow completed、validation passed、review passed、domain quality passed、task completed、diff/file/artifact/source/data-quality/user-decision 等；弱信号和 no-progress 会分开记录。
- 连续 `no_progress` / `failed` 会 backoff；达到上限会 blocked 并暂停 Cron；blocked reason 和 run history 可见。
- Goal completed 会让绑定 Loop completed；Goal failed/cancelled/paused、Goal criteria 删除或 revision/text/kind 变更会让 Loop blocked，要求用户重新确认或编辑策略。
- `run_loop_schedule_now` 复用 Cron immediate path；`update_loop_schedule_policy` 同步 Loop store 与 Cron guard，避免双状态分叉。
- Workflow strategy 的 Loop run 记录 `workflowRunId` / template version，GUI 可从 Loop 跳到 Workflow detail；Workflow run `origin=loop:<id>` 可反向聚合到 Loop 行。
- Event-triggered Loop 已支持内部 EventBus：workflow state、goal state、task status 变化可触发 Loop；事件 payload 会进入 durable tick、run trace 和 `<event_context>`。
- Dynamic Loop 已支持 GUI 创建、maintenance prompt 热更新与模型工具化控制：Workspace 创建器可选择“模型自定”并设置 fallback；裸 `/loop` 创建的 maintenance Loop 会在每次 trigger 前刷新 `loop.md` / built-in prompt；列表主状态显示“模型将在 X 继续”或“等待模型决策”；run detail 会展示最近 dynamic decision 的 scheduling label 与 reason。模型侧 `loop_status` 查询当前/指定 Loop，`loop_reschedule` 记录工具决策并设置下一次 wakeup，`loop_stop` 完成或阻塞 Loop，`loop_record_progress` 记录轻量进度；文本 marker 继续作为兼容兜底。
- Loop Watchdog 读诊断已接入核心存储层、Tauri/HTTP owner API 与 Workspace：`list_loop_watchdog_findings(cron_db, session_id, grace_secs)` 会只读扫描 active、非 event Loop，发现 backing Cron job 缺失时返回 `loop_cron_missing`；发现 latest run 仍是 `running`、Cron 已无 `running_at`、且 run age 超过 grace 时返回 `loop_run_maybe_interrupted`；发现 `next_run_at` 超过 grace 且 Cron active/未 running、同时没有 `Running | Queued | Injected` 的最新 Loop run 时返回 `loop_due_not_claimed`。它不自动修复、不直接触发 run、不绕过 primary-only Cron；Workspace 用 amber 提示关联具体 Loop，并提供“立即运行 / 运行记录”恢复入口。
- Dev-only `?window=loop-smoke` 复用真实 `WorkspacePanel` 组件验证 Dynamic Loop GUI：能显示既有 dynamic loop 的下一次继续时间，能从「持续推进」中新建「模型自定」Loop，创建后列表从 1 个活跃刷新为 2 个活跃，run detail 能展开并展示 `dynamic_reschedule` 的人类可读原因；2026-07-08 browser smoke 同时检查窄/宽两档没有横向溢出。

仍保持的保守边界：

- Loop 仍只表示持续触发器，不重新承载执行强度；执行强度继续归 `/mode` / Execution Mode，具体执行继续归 Workflow。
- Slash `/loop` 仍保持简单；v2 guard 策略主要在 GUI / owner API 暴露，slash 不解析 criteria id / policy edit。V3.1 的自然固定间隔语法和 V3.2 的 prompt-only / bare dynamic 语法只补体验，不新增权限或调度捷径；模型侧 dynamic 控制走 `loop_*` internal tools，而不是开放 Cron 写权限。
- 外部 webhook / CI provider / connector object stream 仍是后续池；V4 已实现受治理的 file / WebSocket 一次性 Monitor，不能把它们扩展成未限速的通用外部事件总线。
- Condition workflow 仍等待 Workflow terminal event 能反写 condition result 后再放开；当前 `until` loop 继续依赖 conversation continuation + assistant marker，不能伪装成 workflow 完成。
- 成本预算精确统计仍等待 provider cost ledger；在此之前 `cost_budget_micros` 继续保持保守拒绝，避免给用户错误安全感。

Loop v2 过程 roadmap 已归档到外部 Plans；已实现事实以本文为准。

这些边界保证后续增强不推翻当前契约：Loop 管触发、progress guard 和调度治理，不拥有 Goal 完成标准，也不绕过 Workflow、权限、预算和无痕红线。

## 测试覆盖

- `workflow_strategy_materializes_domain_workflow_run` 覆盖 Goal 绑定领域模板后，interval Loop workflow strategy 能生成 `origin=loop:<id>` 的 durable WorkflowRun，并把 `workflowRunId` / template version 写入 loop run trace。
- `workflow_strategy_feeds_operational_and_soak_gates` 覆盖同一条 Goal → Loop tick → WorkflowRun → terminal → LoopRun trace 链路会进入 Domain Operational Gate 和 Soak Report，证明 Workspace 的运行稳定性 / 长跑审计卡片读取的是真实控制面证据。
- `no_progress_backoff_then_blocks_after_threshold` 覆盖连续无进展先 backoff、再 blocked。
- `durable_goal_evidence_resets_no_progress_streak` 覆盖 strong Goal evidence 会把 progress 判为 `progressed` 并清空空转 streak。
- `goal_completed_stops_bound_loop_before_next_trigger` 覆盖绑定 Goal completed 后 Loop 自动 completed。
- `criteria_revision_change_blocks_loop_until_rebind` 覆盖 Goal criteria 修改后 Loop blocked。
- `loop_policy_update_persists_budget_and_cron_guard` 覆盖策略编辑会同时更新 Loop store 与 Cron job。
- `loop_run_usage_counts_only_messages_within_run_bounds` 覆盖 run 级 usage 只统计本次 Loop run 边界内的 user/assistant 消息，排除 run 前后消息，并优先使用 `tokens_in_last`。
- `event_loop_enqueue_dedups_and_consumes_event_context` 覆盖 EventBus 事件入队、debounce 去重、tick 消费和 `eventContext` trace。
- `event_loop_filter_mismatch_does_not_enqueue` 覆盖事件状态过滤不会误触发。
- `loop_watchdog_reports_due_active_loop_without_active_run` / `loop_watchdog_reports_missing_backing_cron_even_without_next_run` / `loop_watchdog_reports_stale_running_loop_run_after_cron_recovery` / `loop_watchdog_does_not_flag_cron_job_already_running` 覆盖 Loop Watchdog 只报告 overdue 但未被接管的 active Loop、backing Cron 缺失、Cron startup recovery 后遗留的 running Loop run，并且不会把 Cron 正在执行的 Loop 误报为 stuck。
- `WorkspacePanel` Loop 相关 Vitest 覆盖 derived workflow 行、run history、dynamic decision reason、「持续推进」中心 view-more、run-now、policy edit、模板创建、event loop 创建、dynamic loop 创建和 Loop Watchdog amber 恢复提示；Core 测试覆盖 dynamic maintenance Loop 在 `loop.md` 修改后下一次 trigger 前刷新 prompt 并写入 run trace metadata。
- Dev browser smoke 覆盖真实 Workspace UI 路径：打开 `http://127.0.0.1:1420/?window=loop-smoke`，创建 dynamic Loop，检查「模型自定 · 回退 20m」、下一次继续时间、run detail 的「15m 后继续」与原因展示；截图归档在外部 Plans 的 V3 evidence 目录。该 smoke 证明组件交互路径可用，但不替代完整 Tauri 桌面人工长跑。

## Loop V4：Event-first Watch Registry

V4 保留 Cron 作为 durable heartbeat 和 admission 唯一入口，并为 dynamic Loop 增加 `loop_watches`。模型在知道“等哪个事件”时调用 `loop_watch`，事件先到就立即沿 `execute_job_public` 触发；事件没有到则原 fallback 时间照常唤醒。fallback 时间不在 `loop_watches` 重复存储，仍以 Cron job 的 `next_run_at` 为唯一真相源，避免 watcher 与 heartbeat 出现双时间轴。`loop_unwatch`、Loop pause/stop/terminal 会清理对应监听。

### Watch 数据与单赢

`LoopWatch` 保存 `kind/spec/active/generation/last_event_at/last_fingerprint/failure_count/last_error/monitor_job_id`。同一 loop+kind+canonical spec 生成稳定 signature；upsert 复用并递增 generation。事件 fingerprint 同时包含 watch id、generation、outcome、bounded payload 和 debounce bucket，`loop_event_ticks` 的单赢约束保证 event 与 heartbeat 竞争时同一推进只 claim 一次。

支持 kind：

- `app_event`：受支持 EventBus 事件与 filters；
- `job`：JobManager terminal/status 事件；
- `subagent`：subagent lifecycle；
- `command`：后台命令 Job 的完成事件，不启动独立轮询进程；
- `file`：`notify` 本地文件/目录一次性 watcher；
- `websocket`：受 SSRF policy 保护的 `ws/wss` 一次性连接。

Event-backed watch 只扫描 active dynamic Loop；payload 有界且进入 Loop trigger 时按内部 untrusted event context 处理。终态 Loop、Goal revision stale、stop/pause 后旧 generation 的回调无法重新激活 Loop。

### Monitor 治理

file/WebSocket 适配器注册 `JobKind::Monitor` 投影，用于状态、取消和泄漏诊断，但不占普通 Tool Job 的 activity 计数。内部固定 backpressure 为每个 Loop 最多 16 个 active watch、每个 session 最多 8 个 active file/WebSocket Monitor、全局最多 64 个；同 spec re-arm 只递增 generation，不重复占位。配额是安全内部上限，不增加用户配置；paused Loop 保留 durable watch 配额，避免 resume 瞬间超额，用户可用 `loop_unwatch` 释放。

file event payload 只保留最多 16 个路径、单路径 500 字符；WebSocket 先把 ws/wss 映射为 http/https 做统一 SSRF 检查，消息 preview 有固定字节上限，timeout 钳在 30s..24h。WebSocket URL 会持久化用于恢复，因此拒绝 userinfo 密码和常见 token/api_key/auth/signature 查询参数；普通非敏感查询参数仍可使用。

`loop_watch` 不是 internal tool：file/WebSocket 会观察外部 IO，必须进入统一 permission engine。`permission::rules::extract_path_arg` 显式识别 `loop_watch.spec.path`，因此 protected-path strict gate 与普通 read/write 一致生效；嵌套路径不能绕过审批，无人值守时按统一 approval-surface policy fail closed。纯控制面的 `loop_status/reschedule/stop/progress/unwatch` 仍保持 internal。WebSocket 即使已过工具 permission，连接前仍必须通过实时 SSRF policy。

Monitor 是 one-shot：message/change/close/failure/timeout 任一结算后 durable watch inactive、generation handle 删除、monitor job 进入明确终态。继续等待必须由下一轮模型显式 re-arm；这避免 silent watcher 和进程/slot 泄漏。启动时 `spawn_loop_monitor_recovery` 只恢复 durable active monitor watch；启动失败记录 error，并保留 Cron heartbeat 作为降级路径。

Loop 进入 paused/blocked 时停止进程内 file/WebSocket handle、结算当前 Monitor job，但保留 durable watch 为 active；resume 在 Primary runtime 中按最新 generation 自动重挂。terminal 则同时 deactive watch 并递增 generation，旧回调无法复活。

`get_loop_schedule` 的 `watches` 给 Workspace 运行详情展示 kind、generation、active/settled、failure 和 fallback；普通对话只显示统一 Activity 的“等待持续推进触发”。核心测试覆盖 event/heartbeat race、generation 去重、terminal cleanup、untrusted payload、真实 file one-shot 与 Monitor job settle。

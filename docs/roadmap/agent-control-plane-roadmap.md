# Agent 控制平面路线图

> 返回 [路线图索引](README.md)
>
> 更新时间：2026-07-01
>
> 状态：路线调整与方案设计。`/goal` 第一版已落地并沉淀到 [Goal 控制平面](../architecture/goal.md)；`/loop` 第一版已落地并沉淀到 [Loop 控制平面](../architecture/loop.md)；Managed Worktree 已作为 Phase 3.1 落地并沉淀到 [Managed Worktree 控制平面](../architecture/worktree.md)；本文继续记录 LSP / Review 等后续推进顺序。

## 1. 路线调整结论

Hope Agent 下一阶段不再只按“coding 能力强化”单线推进，而是调整为：

```text
通用 Agent 控制平面
  -> coding-first 产品化落地
  -> coding-specific 深水能力
```

原因很直接：Phase 2 已经把 `WorkflowRun`、execution mode、trace、approval、pause/resume/cancel、repair draft 和长任务 UI 打通了。继续直接做 worktree / LSP / review engine 虽然有价值，但会让长期任务缺一个顶层语义：

```text
我最终要达成什么？
完成标准是什么？
什么时候算完成？
失败后是否继续？
哪些 workflow / task / validation evidence 支撑完成？
```

这个顶层语义应该是 `/goal`，不是 `/workflow`，也不是 `/loop`。

## 2. 新主线

新的主线：

```text
Phase 2.6  语义收口：/loop 执行强度 -> /mode，保留 /loop 给真正重复触发
Phase 2.7  /goal 第一版：目标、完成标准、预算字段、证据、状态（已完成）
Phase 2.8  Goal-driven Workflow：goal 派生 workflow run，失败后 repair run，最终 evaluator / budget 收口（核心已完成）
Phase 2.9  真正 /loop：定时、重复、轮询、条件触发，复用 cron / wakeup / automation（第一版已完成）
Phase 3.1  Managed Worktree 隔离与交接（已完成）
Phase 3.2+ Coding-specific 深水能力：LSP、review engine、diagnostics、智能验证
```

旧主线里“Coding Mode -> Workflow/Loop -> Worktree/LSP/Review”的顺序需要改成：

```text
控制平面语义稳定
  -> /goal
  -> goal 与 workflow 闭环
  -> 真 /loop
  -> coding 专项增强
```

这不是推翻 Phase 2。Phase 2 做对了：它补上了可恢复、可审批、可观察、可取消的执行底座。现在需要在这个底座之上加顶层目标对象。

## 3. 概念关系

| 概念 | 是否通用 | 当前状态 | 负责回答 |
| --- | --- | --- | --- |
| Goal | 通用 | 已实现第一版 | 最终要达成什么，完成标准是什么。 |
| Mode | 通用 | 已实现为 `/mode` | 这次会话/目标用多主动、多深入的策略推进。 |
| Workflow | 通用 runtime，当前 coding-first | 已实现第一版 | 这次具体怎么执行，能否审批、恢复、取消、审计。 |
| Task | 通用 | 已有，workflow 已接入 | 当前可见进度事实是什么。 |
| Loop | 通用 | 已实现第一版 | 是否按时间、事件或条件重复触发。 |
| Worktree | coding-specific | 已实现 Phase 3.1 | 代码改动落在哪个隔离环境。 |

用户视角应稳定成：

```text
/goal      = 我要最终达成什么，完成标准是什么
/mode      = 这次目标/会话用多主动、多深入的推进策略
/workflow  = 这次具体怎么执行、怎么审批和恢复
/task      = 当前可见进度事实
/loop      = 按时间/事件/条件重复触发
/worktree  = 编码任务的隔离工作区
```

## 4. Phase 2.6：语义收口（已完成）

目标：

- 删除旧 `/loop off|guarded|deep|autonomous` 执行强度入口。
- 统一为 `/mode off|guarded|deep|autonomous`。
- 代码、DB、API、GUI 文案统一使用 `execution_mode` / `executionMode`。
- 明确 `/loop` 只保留给真正重复触发。
- 不保留旧 alias、旧 HTTP route、旧 DB 字段兼容层，因为功能尚未发布。

已落地事实：

- `sessions.execution_mode`。
- Tauri / HTTP owner API：`get_execution_mode` / `set_execution_mode`。
- HTTP：`/api/sessions/{id}/execution-mode`。
- Workflow run 保存创建时的 `execution_mode` 快照。
- Workspace / Workflow Control Center 暴露 Execution Mode 控件。
- 语义说明见 [Goal / Mode / Workflow / Loop 语义收口](control-plane-semantics.md)。

## 5. Phase 2.7：`/goal` 第一版（已完成）

### 5.1 目标

把长期任务的顶层意图变成一等对象。`/goal` 不负责具体执行步骤，它负责保存目标、完成标准、预算、证据和最终状态。

### 5.2 用户交互

当前入口：

```text
/goal <objective and completion criteria>
/goal
/goal status
/goal pause
/goal resume
/goal evaluate
/goal clear
```

GUI：

- Chat 顶部或 Workflow Control Center 增加 Goal strip。
- 有 active goal 时显示目标摘要、状态、预算、最近 evidence、下一步。
- 失败或 blocked 时显示“为什么没完成”和“下一步建议”。
- 完成时显示 final audit，不只显示 assistant final message。

### 5.3 数据模型

```text
goals
  id
  session_id
  objective
  completion_criteria
  state: draft | active | paused | evaluating | completed | failed | cancelled | blocked
  mode_snapshot
  budget_token_limit
  budget_time_limit_secs
  budget_turn_limit
  created_at
  updated_at
  completed_at
  final_summary
  final_evidence
  blocked_reason

goal_events
  id
  goal_id
  kind
  payload_json
  created_at

goal_links
  goal_id
  target_type: workflow_run | task | message | artifact | file | validation
  target_id
  relation
```

第一版把 evaluator result 写入 `goals.final_evidence_json` / `last_evaluator_result_json`，并同步追加 `goal_events(kind='goal_evaluated')`；后续若需要多次可比较审计历史，再拆 `goal_evaluations`。

### 5.4 API

Owner 平面：

```text
GET    /api/sessions/{sid}/goal
POST   /api/sessions/{sid}/goal
GET    /api/goals/{goalId}
POST   /api/goals/{goalId}/pause
POST   /api/goals/{goalId}/resume
POST   /api/goals/{goalId}/clear
POST   /api/goals/{goalId}/evaluate
```

Tauri command 与 HTTP 对齐。Agent 工具面第一版不需要让模型直接改 goal；模型可以提出更新建议，owner 平面落地。

### 5.5 完成判定

Goal 完成不能只靠模型自称完成。Evaluator 至少结合：

- 用户写的 objective。
- completion criteria。
- linked workflow runs。
- linked tasks。
- validation results。
- changed files / artifacts。
- final diff / output。
- 必要时的轻量 LLM evaluator。

Phase 2.7 第一版 evaluator 是保守规则；Phase 2.8 已升级为 deterministic rule gate + criteria mapping + budget blocker：

```text
如果存在未完成 required task -> not complete
如果 required validation failed -> not complete
如果 workflow blocked/failed 且无后续 repair -> not complete
如果 criteria 无 evidence -> blocked
如果 budget exhausted -> blocked 且阻止新 workflow
否则 completed / blocked + reason
```

### 5.6 验收标准

- `/goal` 能创建、查看、暂停、恢复、清除。
- Active goal 会显示在 GUI 控制面，不依赖用户翻 workflow 历史。
- Goal 可以链接至少一个 workflow run。
- Workflow 完成后能产生 evidence 并触发 goal evaluate。
- Goal final audit 能列出：达成项、未达成项、验证证据、剩余风险。
- 无痕会话不持久化 goal。

## 6. Phase 2.8：Goal-driven Workflow（核心已完成）

### 6.1 目标

让 Workflow 成为 Goal 的执行手段，而不是独立漂浮的 run。

关系：

```text
Goal
  -> WorkflowRun #1 observe / implement / validate
  -> WorkflowRun #2 repair from failure
  -> WorkflowRun #3 review / final audit
  -> GoalEvaluator
```

### 6.2 能力状态

- 已落地：`workflow_runs.goal_id` 可选字段。
- 已落地：Workflow create 默认绑定当前 active Goal。
- 已落地：Repair run 在当前 active Goal 下创建，并在 GUI 提示同一 Goal 归属。
- 已落地：workflow completion / validation / task evidence 进入 Goal audit。
- 已落地：`workflow.validate` 写 validation evidence，`workflow.diff` 写 diff/file evidence。
- 已落地：Goal strip 可展开 detail，展示 criteria、evidence、timeline、workflow/task 摘要。
- 已落地：Goal evaluator 读取 workflow/evidence/budget snapshot，而不是重新扫散落消息；failed validation / budget exhausted 是 hard blocker。
- 已落地：Goal budget 展示 token/time/turn 使用，接近上限写 warning event，耗尽后阻止新 workflow。
- 后续增强：artifact / review / diagnostic evidence 接入。
- 后续增强：独立 Goal detail 全屏页面。
- 后续增强：`/workflow` status 显示归属目标。

增强细节见 [Goal-driven Workflow v2 路线图](goal-driven-workflow-v2.md)。

### 6.3 GUI 交互

- Goal strip 显示 active goal。
- Workflow Control Center 增加“Linked Goal”区域。
- 失败 run 生成 repair draft 时，明确说明会创建同一 goal 下的新 run。
- Goal detail 展开区展示：
  - Objective / criteria。
  - Linked runs。
  - Required tasks。
  - Validation evidence。
  - Diff / file evidence。
  - Budget card。
  - Next evidence needed。
  - Current blocker。
  - Final audit。

### 6.4 验收标准

- 从 `/goal` 创建后，用户能一键生成 workflow draft。
- Workflow run 完成后自动写入 goal evidence。
- 失败 run 生成 repair run 不丢 goal 归属。
- Goal evaluator 能基于 run evidence 输出 completed / blocked；`partial` 暂不进入状态机。
- App 重启后 goal、run、task、evidence 关系仍可恢复。

## 7. Phase 2.9：真正 `/loop`

状态：第一版已落地。最终实现见 [Loop 控制平面](../architecture/loop.md)。

### 7.1 目标

`/loop` 只负责重复触发，不负责执行强度。它回答：

> 这个 goal / prompt / workflow 是否需要按时间、事件或条件继续执行？

建议命令：

```text
/loop every 10m: check CI and continue fixing if failing
/loop until <condition>
/loop status
/loop pause
/loop resume
/loop stop
```

### 7.2 设计原则

- 必须复用现有 cron / wakeup / automation / async jobs，不新建平行调度器。
- 必须绑定 Goal 或明确 recurring prompt。
- 必须有最大次数、最大运行时间、token 预算；成本预算等 provider cost ledger 可用后再放开。
- 必须有无人值守审批策略。
- 必须能 pause / resume / stop。
- 必须能审计每次触发的原因和结果。

### 7.3 已落地数据模型

```text
loop_schedules
  id
  session_id
  goal_id?
  cron_job_id
  prompt
  trigger_kind: interval | cron | condition | event
  trigger_spec_json
  state: active | paused | completed | cancelled | blocked
  max_runs
  run_count
  max_runtime_secs
  token_budget
  cost_budget_micros
  approval_policy_snapshot
  created_at
  updated_at

loop_runs
  id
  loop_id
  cron_job_id
  cron_run_log_id?
  session_id
  seq
  started_at
  finished_at
  state
  trigger_reason
  result_summary
  error
  trace_json
```

### 7.4 验收标准

- 已落地：`/loop every|until|status|pause|resume|stop`。
- 已落地：`/loop until` 注入 condition，并通过 `LOOP_CONDITION_SATISFIED` marker 自动完成和停掉底层 Cron。
- 已落地：Loop schedule store + Loop run trace。
- 已落地：底层复用 Cron，触发时用 `SessionLoop` + parent injection 回到原会话。
- 已落地：支持绑定当前 open Goal 或明确 recurring prompt。
- 已落地：`max_runs` / `max_runtime_secs` / token budget hard stop；Goal 绑定时触发前复用 Goal budget hard stop。
- 已落地：用户停止 loop 后会暂停底层 Cron job，不再唤醒。
- 已落地：Workspace GUI 可创建 `every` / `until` loop，并提供 pause / resume / stop。
- 已落地：Loop 不绕过 `/mode`、permission、hooks、incognito、Project/KB access，实际 turn 在原会话里执行。

后续增强：

- Event-triggered loop 接入 EventBus / file watcher / CI。
- 独立 Loop detail 页面展示完整 run trace、cron log 与消息范围。
- 成本预算接入 provider cost ledger，并放开 `cost_budget_micros` 创建限制。
- Loop trigger 直接生成/运行 Goal-driven Workflow draft。

## 8. Phase 3：Coding-specific 能力

Goal / Workflow / Loop 稳住后，再进入 coding-specific 深水区：

### Phase 3.1 Managed Worktree（已完成）

- worktree create / list / restore / archive / handoff。
- workflow run 可绑定 worktree。
- subagent 实现型任务默认进入隔离 worktree。
- GUI 显示当前改动落在哪个 worktree。
- 最终架构见 [Managed Worktree 控制平面](../architecture/worktree.md)。

### Phase 3.2 LSP / Diagnostics

- definition / references / hover / symbols / diagnostics。
- 编辑后同步 diagnostics。
- Goal evaluator 可把 diagnostics 作为 evidence。

### Phase 3.3 Review Engine

- `/review` 独立入口。
- candidate findings + verifier 三态。
- inline comments。
- review evidence 可回写 goal。

### Phase 3.4 智能验证选择

- 根据 touched files、AGENTS、历史 trace 推荐最小验证。
- 避免默认全量测试。
- 将验证选择质量纳入 eval。

### Phase 3.5 搜索增强后续

- file search v2 已作为基础搜索增强。
- 后续可补语义符号搜索、最近修改权重、artifact/workflow/task 关联召回。
- 搜索仍是通用能力，但首批优化场景是 coding。

## 9. 体验与性能红线

- 长任务必须可观察：状态、下一步、卡点、证据、预算都要可见。
- 长任务必须可恢复：重启后不能丢 run / goal / task 关系。
- 长任务必须可停止：pause / cancel / stop 的语义要真实执行，不只是改 UI。
- 审批必须 fail-closed：无人能批时不能永久挂死，也不能默认越权。
- GUI 不做命令行的薄皮：用户不应必须记 slash 命令才能掌控任务。
- Prompt cache 要稳定：goal / mode / workflow 状态进入动态段，不破坏静态 prefix。
- Coding-specific 能力必须挂到控制平面上：worktree / LSP / review 不是孤立工具堆叠。

## 10. 文档落点

- 本文记录路线和方案，保留在 `docs/roadmap/`。
- 已实现的 Goal 第一版见 [Goal 控制平面](../architecture/goal.md)。
- [Goal / Mode / Workflow / Loop 语义收口](control-plane-semantics.md) 记录产品语言边界。
- [Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md) 记录已落 workflow runtime 与 GUI 的设计历史。
- 已实现的 Managed Worktree 见 [Managed Worktree 控制平面](../architecture/worktree.md)。
- 后续 LSP/review 仍先在 roadmap 迭代，稳定后再沉淀到 `docs/architecture/`；Loop 第一版已沉淀到 [Loop 控制平面](../architecture/loop.md)。

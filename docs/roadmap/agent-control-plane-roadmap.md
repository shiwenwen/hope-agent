# Agent 控制平面路线图

> 返回 [路线图索引](README.md)
>
> 更新时间：2026-07-04
>
> 状态：路线调整与方案设计。`/goal` 第一版已落地并沉淀到 [Goal 控制平面](../architecture/goal.md)；`/loop` 第一版已落地并沉淀到 [Loop 控制平面](../architecture/loop.md)；Managed Worktree 已作为 Phase 3.1 落地并沉淀到 [Managed Worktree 控制平面](../architecture/worktree.md)；LSP / Diagnostics 已作为 Phase 3.2 落地并沉淀到 [LSP 与语义代码智能](../architecture/lsp.md)；Review Engine 已作为 Phase 3.3 落地并沉淀到 [Review Engine 控制平面](../architecture/review-engine.md)；Smart Verification 已作为 Phase 3.4 落地并沉淀到 [Smart Verification 控制平面](../architecture/verification-engine.md)；Context Retrieval v2 与 Actionable Context Loop 已作为 Phase 3.5-3.6 落地并沉淀到 [Context Retrieval v2](../architecture/context-retrieval.md)；Coding Eval 控制面评测已作为 Phase 3.7 落地并沉淀到 [Coding Eval 控制面评测](../architecture/coding-eval.md)；Deep Review / Profiles / IDE Context 已作为 Phase 3.10 落地并沉淀到 [Review Engine 控制平面](../architecture/review-engine.md) 与 [Context Retrieval v2](../architecture/context-retrieval.md)；Trend Report / Improvement Loop 已作为 Phase 3.11 落地，Proposal-to-Action Learning Loop 已作为 Phase 4.1 落地，Draft Promotion + Workflow Retro Loop 已作为 Phase 4.2 落地，Dashboard 全局学习视图已作为 Phase 4.3 落地，Transcript Distillation + Failure Feedback 已作为 Phase 4.4 落地，均沉淀到 [Coding Improvement Loop](../architecture/coding-improvement-loop.md)；Task-level Eval Runner 已作为 Phase 5.1 落地，Agent Execution Runner 已作为 Phase 5.2 落地，Gold Task Pack v1 已作为 Phase 5.3 落地，Strategy Effect Evaluator 已作为 Phase 5.4 落地，Gold Task Pack 全量自动化已作为 Phase 5.5 落地，Mock Tool-call 基线与执行指标已作为 Phase 5.6 落地，Strategy Effect 趋势持久化 / Dashboard 已作为 Phase 5.7 落地，Release Gate 已作为 Phase 5.8 落地，外部模型基线 runner 已作为 Phase 5.9 落地，Learning Generalization Gate 已作为 Phase 5.10 落地，Benchmark Run Center v1 已作为 Phase 6.1 落地，Benchmark Campaign Runner 已作为 Phase 6.2 落地，Cross-model Leaderboard 已作为 Phase 6.3 落地，Real Task Corpus Expansion 已作为 Phase 6.4 落地，Benchmark Report Export 已作为 Phase 6.5 落地，Continuous Benchmark Gate & Improvement Backlog 已作为 Phase 6.6 落地，沉淀到 [Coding Eval 控制面评测](../architecture/coding-eval.md) 与 [Coding Improvement Loop](../architecture/coding-improvement-loop.md)；Phase 7.1-7.16 已完成通用场景层第一版；Phase 8.1 Domain Operational Gate 已把 workflow / loop / campaign 运行稳定性产品化，Phase 8.2 Connector E2E Gate 已把真实连接器输入、草稿、批准、执行、复核、回滚和交付守门产品化，Phase 8.3 Domain Soak Report 已把跨窗口长期运行审计产品化到 [Domain Workflow 控制平面](../architecture/domain-workflow.md)、[Domain Eval 与 Quality Gate 控制平面](../architecture/domain-eval.md) 与 Dashboard Learning，详见 [通用场景层与 Domain Workflow 路线图](general-domain-workflows.md)。

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
Phase 2.6  语义收口：/loop 执行强度 -> /mode，保留 /loop 给真正重复触发；Workflow Mode 对齐模型自主动态编排（已完成）
Phase 2.7  /goal 第一版：目标、完成标准、预算字段、证据、状态（已完成）
Phase 2.8  Goal-driven Workflow：goal 派生 workflow run，失败后 repair run，最终 evaluator / budget 收口（核心已完成）
Phase 2.9  真正 /loop：定时、重复、轮询、条件触发，复用 cron / wakeup / automation（第一版已完成）
Phase 3.1  Managed Worktree 隔离与交接（已完成）
Phase 3.2  LSP / Diagnostics（已完成）
Phase 3.3  Review Engine（已完成）
Phase 3.4  Smart Verification / 智能验证选择（已完成）
Phase 3.5  Context Retrieval v2 / 推荐上下文（已完成）
Phase 3.6  Actionable Context Loop / 可行动上下文闭环（已完成）
Phase 3.7  Coding Eval 控制面评测（已完成）
Phase 3.8  Workflow Review/Verify Host API 与 Goal-aware Eval（已完成）
Phase 3.9  Repair Loop 自动化（已完成）
Phase 3.10 Deep Review / Profiles / IDE Context（已完成）
Phase 3.11 Trend Report / Improvement Loop 接口（已完成）
Phase 4.1  Proposal-to-Action Learning Loop（已完成）
Phase 4.2  Draft Promotion + Workflow Retro Loop（已完成）
Phase 4.3  Dashboard 全局学习视图（已完成）
Phase 4.4  Transcript Distillation + Failure Feedback（已完成）
Phase 5.1  Task-level Eval Runner（已完成）
Phase 5.2  Agent Execution Runner（已完成）
Phase 5.3  Gold Task Pack v1（已完成）
Phase 5.4  Strategy Effect Evaluator（已完成）
Phase 5.5  Gold Task Pack 全量自动化（已完成）
Phase 5.6  Mock Tool-call 基线与执行指标（已完成）
Phase 5.7  Strategy Effect 趋势持久化 / Dashboard（已完成）
Phase 5.8  Release Gate（已完成）
Phase 5.9  外部模型基线 runner（已完成）
Phase 5.10 Learning Generalization Gate（已完成）
Phase 6.1  Benchmark Run Center v1（已完成）
Phase 6.2  Benchmark Campaign Runner（已完成）
Phase 6.3  Cross-model Comparison & Leaderboard（已完成）
Phase 6.4  Real Task Corpus Expansion（已完成）
Phase 6.5  Benchmark Report Export（已完成）
Phase 6.6  Continuous Benchmark Gate & Improvement Backlog（已完成）
Phase 7.1  Domain Workflow Registry（已完成第一版）
Phase 7.2  General Evidence Model（已完成第一版）
Phase 7.3  Domain Context Retrieval（已完成第一版）
Phase 7.4  Domain Verification & Review（已完成第一版）
Phase 7.5  Domain Learning Loop（已完成第一版）
Phase 7.6  General Eval & Quality Gate（已完成第一版）
Phase 7.7  Domain Eval Calibration（已完成第一版）
Phase 7.8  Domain Eval Fixture Runner（已完成第一版）
Phase 7.9  Domain Eval Agent Fixture Execution（已完成第一版）
Phase 7.10 Domain Fixture / Smoke Run Center（已完成第一版）
Phase 7.11 Domain Eval Campaign Runner（已完成第一版）
Phase 7.12 Domain External Campaign & Leaderboard（已完成第一版）
Phase 7.13 Domain Campaign Learning Closure（已完成第一版）
Phase 7.14 Domain Readiness Gate（已完成第一版）
Phase 7.15 Domain Artifact Export Guard（已完成第一版）
Phase 7.16 Domain Connector Action Guard（已完成第一版）
Phase 8.1  Domain Operational Gate（已完成第一版）
Phase 8.2  Connector E2E Gate（已完成第一版）
Phase 8.3  Domain Soak Report（已完成第一版）
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
| Workflow Mode / Workflow | 通用 | 已实现第一版 | 是否允许模型自主动态编排；这次具体怎么执行，能否审批、恢复、取消、审计。 |
| Task | 通用 | 已有，workflow 已接入 | 当前可见进度事实是什么。 |
| Loop | 通用 | 已实现第一版 | 是否按时间、事件或条件重复触发。 |
| Worktree | coding-specific | 已实现 Phase 3.1 | 代码改动落在哪个隔离环境。 |
| Context Retrieval | 通用 owner-plane，当前 coding-first | 已实现 Phase 3.6 | 当前任务下一步最该看哪些上下文，以及能否直接进入 focused review / verification。 |
| Coding Eval | coding-first 质量闸，harness 可复用于通用控制面 | 已实现 Phase 6.6 | 控制面协同是否可回归，关键上下文是否被召回，focused action 是否真实收窄，Agent 是否能从 prompt 生成候选结果，候选 diff 是否满足任务级成功标准；20 个 active gold tasks 是否可批量回放；mock tool-call 是否真实调用写文件工具；策略改动前后是否真的改善质量；持久化历史是否满足发布质量门禁；真实 provider 是否能在受控 Gold Pack 中从 prompt 产出可评分候选 diff；promoted learning 是否有跨项目泛化证据；Dashboard 是否能以 Benchmark Run Center / Campaign Runner / Leaderboard / Report History / Continuous Gate / Improvement Backlog 形式展示、运行、取消、重试、对标、归档、守门和审计当前 benchmark readiness。 |
| Coding Improvement | coding-first 改进回路，报告形态可复用于通用控制面 | 已实现 Phase 3.11 | 最近任务为什么完成/阻塞，下一步应补 eval、workflow、guidance 还是 skill。 |
| Learning Loop | coding-first，后续可通用化 | 已实现 Phase 4.4 | 把改进 proposal 安全落成 eval / workflow / guidance / skill 草稿产物，把已应用草稿显式晋升为正式 eval fixture / project guidance / active skill，并支持用户显式从 transcript / workflow / failure feedback 提炼更高质量候选。 |
| Domain Workflow / Quality | 通用场景层 | Phase 7.1-7.16 与 Phase 8.1-8.3 已完成第一版 | 把 Goal / Mode / Workflow / Loop / Evidence / Review / Verification / Learning Loop / Eval Gate 产品化到调研、写作、数据分析、会议准备、知识整理、邮件沟通和项目运营等非编程任务；已具备模板 registry、workflow draft preview、通用 evidence 持久化、Goal evidence 链接、domain context retrieval、Workspace 领域复核质量门、domain learning proposal、general domain quality gate、user/project calibration、trace/agent fixture runner、Smoke Run Center、可取消/可 retry 的 Domain Eval Campaign Runner、external model leaderboard、失败 campaign 学习闭环、Domain Readiness Gate、最终交付 Export Guard、真实外部动作 Connector Action Guard、运行稳定性 Domain Operational Gate、连接器端到端 Connector E2E Gate 和跨窗口 Domain Soak Report。 |

用户视角应稳定成：

```text
/goal      = 我要最终达成什么，完成标准是什么
/mode      = 这次目标/会话用多主动、多深入的推进策略
/workflow  = 开关模型自主动态编排；查看和管理具体 workflow run
/task      = 当前可见进度事实
/loop      = 按时间/事件/条件重复触发
/worktree  = 编码任务的隔离工作区
```

## 4. Phase 2.6：语义收口（已完成）

目标：

- 删除旧 `/loop off|guarded|deep|autonomous` 执行强度入口。
- 统一为 `/mode off|guarded|deep|autonomous`。
- 新增 `/workflow on|off|ultracode` 作为 session 级 Workflow Mode：开启后模型在下一轮自行判断是否要用 `workflow_run` 写脚本并启动 durable run。
- 代码、DB、API、GUI 文案统一使用 `execution_mode` / `executionMode`。
- 明确 `/loop` 只保留给真正重复触发。
- 不保留旧 alias、旧 HTTP route、旧 DB 字段兼容层，因为功能尚未发布。

已落地事实：

- `sessions.execution_mode`。
- `sessions.workflow_mode`。
- Tauri / HTTP owner API：`get_execution_mode` / `set_execution_mode`。
- Tauri / HTTP owner API：`get_workflow_mode` / `set_workflow_mode`。
- HTTP：`/api/sessions/{id}/execution-mode`。
- HTTP：`/api/sessions/{id}/workflow-mode`。
- Workflow run 保存创建时的 `execution_mode` 快照。
- 输入框和 Workspace / Workflow Control Center 暴露 Workflow Mode 控件；开启后输入框上方常驻显示当前模式。
- `workflow_run` 工具只在 Workflow Mode 开启且非 incognito 时注入，并由执行层二次校验。
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
- 已落地：Review Engine 写 `review_passed` / `review_completed` / `review_finding` evidence。
- 已落地：Smart Verification 写 `validation_passed` / `validation_failed` / `validation_completed` evidence。
- 已落地：`/workflow status` 显示当前 active Goal，`/workflow runs` / `trace` 显示 run 归属目标。
- 已落地：workflow finish artifacts 写 `artifact_created` evidence，workflow 内 LSP diagnostics 写 `diagnostic_result` evidence，error 级 diagnostic 会阻止 Goal completed。
- 后续增强：独立 Goal detail 全屏页面。

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
- 已落地：`every` loop 可选择 `executionStrategy=workflow`；GUI 与 `/loop every --workflow` 都能创建，触发后基于绑定 Goal 的 Domain Workflow template 创建并启动 durable WorkflowRun。
- 已落地：Loop GUI 与 `/loop status <id>` 都能看到派生 workflow run 线索；GUI 可一键跳到 Workflow run detail。
- 已落地：Workspace Loop 行可展开最近运行记录，按需读取 `loop_runs` trace，显示 run 状态、错误/摘要、workflowRunId 和 template version。
- 已落地：Loop 不绕过 `/mode`、permission、hooks、incognito、Project/KB access，实际 turn 在原会话里执行。

后续增强：

- Event-triggered loop 接入 EventBus / file watcher / CI。
- 独立 Loop detail 页面展示完整 run trace、cron log 与消息范围；Workspace inline detail 已覆盖最近运行排障。
- 成本预算接入 provider cost ledger，并放开 `cost_budget_micros` 创建限制。
- Condition workflow：等 Workflow terminal event 能反写 condition result 后，支持 until loop 直接创建 workflow。

## 8. Phase 3：Coding-specific 能力

Goal / Workflow / Loop 稳住后，再进入 coding-specific 深水区：

### Phase 3.1 Managed Worktree（已完成）

- worktree create / list / restore / archive / handoff。
- workflow run 可绑定 worktree。
- subagent 实现型任务默认进入隔离 worktree。
- GUI 显示当前改动落在哪个 worktree。
- 绑定 Goal 的 workflow 会写 `worktree_attached` evidence；handoff / restore / archive 会刷新 Goal 内的 worktree 状态。
- 最终架构见 [Managed Worktree 控制平面](../architecture/worktree.md)。

### Phase 3.2 LSP / Diagnostics（已完成）

- `ha-core::lsp` LSP manager + 进程内 client cache。
- `lsp` 工具支持 definition / references / hover / symbols / implementation / call hierarchy / diagnostics。
- 文件编辑工具成功写入后同步 diagnostics。
- diagnostics 被动注入下一轮动态 prompt 后缀。
- Workspace GUI 展示语义诊断状态、错误/警告和最近诊断。
- Tauri / HTTP owner API 对齐。
- 最终架构见 [LSP 与语义代码智能](../architecture/lsp.md)。

后续增强：

- 更丰富的 diagnostic lifecycle evidence，例如按 LSP publishDiagnostics 事件自动刷新已解决诊断。
- Workflow validation summary 汇总 diagnostics。
- 更完整的 ACP / IDE 双向 RPC；轻量 IDE context envelope 已在 Phase 3.10 落地。

### Phase 3.3 Review Engine（已完成）

- `ha-core::review` durable store：`review_runs` / `review_findings` / `review_events`。
- `/review` 独立入口：run / status / resolved / dismissed / false_positive / open。
- deterministic candidate findings + verifier 三态：`confirmed` / `plausible` / `refuted`。
- LSP diagnostics 已作为 candidate finding 证据接入。
- inline finding 包含 file + start/end line，Workspace GUI 可定位展示。
- review evidence 已回写 Goal：`review_passed` / `review_completed` / `review_finding`。
- Workspace GUI “代码审查”区块支持运行、刷新、查看 P0-P3、标记已修复/忽略/误报。
- Tauri + HTTP owner API 对齐。
- 最终架构见 [Review Engine 控制平面](../architecture/review-engine.md)。

后续增强：

- LLM reviewer / verifier agent。
- review profiles：correctness / security / concurrency / frontend / accessibility / tests。
- 修复后 focused re-review。

### Phase 3.4 智能验证选择（已完成）

- `ha-core::verification` durable store：`verification_runs` / `verification_steps` / `verification_events`。
- 根据 touched files 与 `AGENTS.md` / `CLAUDE.md` 项目规则推荐最小验证。
- 低风险 step 后台执行；高风险 / 全量检查作为 gated suggestion，不默认跑。
- Tauri + HTTP owner API 对齐：list/get/plan/run。
- Workspace GUI “验证”区块：推荐、运行、统计、step 状态、失败输出摘要。
- Goal evidence：`validation_passed` / `validation_failed` / `validation_completed`。
- 重启时遗留 running verification run fail-closed 标记为 interrupted。
- 最终架构见 [Smart Verification 控制平面](../architecture/verification-engine.md)。

后续增强：

- 历史 trace 成功率 / 耗时参与排序。
- Test impact / owner map 级别的更细粒度选择。
- 单条 gated step 用户批准后执行。
- 将验证执行质量与历史失败模式纳入更高层 eval。

### Phase 3.5 Context Retrieval v2 / 推荐上下文（已完成）

- `ha-core::context_retrieval` 只读聚合器：按 session 聚合 Git diff、历史 artifacts、LSP diagnostics、Review findings、Verification steps、file search v2、LSP workspace symbols 和 URL 来源。
- 排序从纯字符匹配升级为“任务信号基础分 + query boost”：P0/P1 review、LSP error、失败 verification、当前 diff 文件优先；query 只增强匹配，不隐藏高危信号。
- Tauri + HTTP owner API 对齐：`get_context_retrieval` / `GET /api/sessions/{sid}/context-retrieval`。
- Workspace GUI “推荐上下文”区块：默认推荐、关键词召回、手动刷新、事件驱动刷新；文件项复用统一文件操作策略预览。
- 无痕会话 fail-closed 返回空 snapshot；LSP symbol 不可用时只降级 warning，不阻断其它候选。
- 最终架构见 [Context Retrieval v2](../architecture/context-retrieval.md)。

### Phase 3.6 Actionable Context Loop / 可行动上下文闭环（已完成）

- Context Retrieval 新增 Goal evidence、task、Workflow run/op 三类控制平面来源；失败 / blocked / awaiting / in-progress 信号优先展示，completed 信号保留但降权。
- 可行动候选写入 `metadata.actions.focusPaths`，GUI 候选行显示聚焦审查 / 聚焦验证两个紧凑按钮。
- Review Engine 新增 `RunReviewInput.focusPaths[]`，在同一 local diff 内收窄 changed files 与 LSP diagnostics，summary/stats 标记 `focused`。
- Smart Verification 新增 `PlanVerificationInput.focusPaths[]`，在 selector 前收窄 changed files，plan/run/final stats 都保留 `focused` 与 `focusPaths`。
- GUI 点击候选行操作后复用现有 durable run、Goal evidence、EventBus 与 Workspace Review/Verification 区块，不创建平行控制面。
- 最终架构见 [Context Retrieval v2](../architecture/context-retrieval.md)、[Review Engine 控制平面](../architecture/review-engine.md)、[Smart Verification 控制平面](../architecture/verification-engine.md)。

后续增强：

- 增加 document symbols fallback、IDE selection envelope 与 ACP 当前文件信号。
- context precision / critical context recall 已进入 Phase 3.7/3.8 控制面 eval；后续扩展到趋势 dashboard。

### Phase 3.7 Coding Eval 控制面评测（已完成）

- `ha-core::coding_eval` 提供 deterministic fixture harness：临时 git repo、真实 session / goal / task / workflow seed、生产 review / verification / context API 调用。
- 首批 fixture 覆盖 Rust 控制面召回、docs-only sanity、focused scope 不扫无关文件。
- 集成测试入口：`cargo test -p ha-core --test coding_eval --locked`。
- 指标包含 `context_precision`、`critical_context_recall`、review finding 数量、verification command 列表。
- 不调用 LLM、不执行真实验证命令，作为后续 workflow review/verify、repair loop、profile/IDE 回归和 Phase 3.11 Improvement Loop 的底座质量闸。
- 最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 3.8 Workflow Review/Verify Host API 与 Goal-aware Eval（已完成）

- `workflow.review({ focusPaths?, baseRef?, profiles?, ideContext?, scope? })` 已作为 idempotent durable host API 落地，复用 Review Engine，并继承当前 workflow run 的 Goal。
- `workflow.verify({ focusPaths?, maxCommands?, scope? })` 已作为 idempotent durable host API 落地，复用 Smart Verification selector，只生成计划，不执行命令。
- Script Gate / permission preview 识别这两个 API 为 permission-neutral workflow control-plane API；runtime replay 复用已完成 op output。
- Goal evidence 串联已完成：review evidence、verification `validation_completed`、workflow `workflow_completed` 能进入同一 Goal 证据链。
- Coding Eval 新增 workflow-bound fixture，验证 workflow op、review run、verification plan、Goal evidence 和 Context Retrieval 召回协同。
- 最终架构见 [Workflow Mode、Workflow Run 与 Execution Mode](../architecture/workflow.md)、[Review Engine 控制平面](../architecture/review-engine.md)、[Smart Verification 控制平面](../architecture/verification-engine.md)、[Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 3.9 Repair Loop 自动化（已完成）

- `workflow.repairLoop({ label?, maxAttempts?, validationCommands?, focusPaths?, review?, verify? }, fn)` 已落地为脚本级 bounded repair loop；修复动作保持动态 callback，不退回固定 DSL。
- 每轮 attempt 自动创建 task、执行 callback、运行 validation / focused review / verification plan，并写入 `repair_loop_*` trace。
- `workflow.block({ reason?, label?, payload? })` 已落地为显式受控停机出口；attempt budget 耗尽统一 `blocked(reason=repair_loop_attempts_exhausted)`。
- 原有 guarded repair stop guard 继续处理重复验证失败和无有效 diff 进展；repairLoop 不绕过旧安全阀。
- Workspace 目标驱动 workflow 草稿默认使用 repairLoop，用户创建长任务时直接进入可验证的修复闭环。
- Coding Eval 新增 repair-loop fixture，验证 blocked state、Goal `validation_failed` / `workflow_blocked` evidence 和 Context Retrieval 召回。
- 最终架构见 [Workflow Mode、Workflow Run 与 Execution Mode](../architecture/workflow.md)、[Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 3.10 Deep Review / Profiles / IDE Context（已完成）

- `deep` profile 下的 bounded LLM reviewer 已接入 Review Engine；失败只写 warning，不阻断 deterministic review。
- Review profiles 支持 correctness / security / maintainability / tests / concurrency / frontend / accessibility / deep 等组合，并进入 review stats 与 Workspace 展示。
- IDE / ACP 当前文件、selection、open tabs、active diagnostics、active symbol 已接入 Context Retrieval 和 review evidence。
- Diff scan 已增强到 enclosing function / symbol context，减少大文件噪音。
- eval 已增加 profile-specific review 与 IDE context recall fixture，继续保持无 LLM 的控制面回归。

### Phase 3.11 Trend Report / Improvement Loop 接口（已完成）

- `ha-core::coding_improvement` 已落地 deterministic trend report：按 session/project scope 汇总 coding eval、workflow、goal、review、verification、repair loop durable 数据。
- Failure taxonomy 已覆盖 validation failure、review blocker、permission stall、context miss、no effective diff progress、repair loop exhausted、verification selection gap 等分类。
- Proposal queue 已落地：失败 bucket 生成 `eval_candidate`，成功/清洁 run 可生成 `workflow_template` / `guidance_candidate` / `skill_candidate`，默认全部 `draft`。
- Tauri + HTTP owner API 已对齐：读取 trend report、列出/生成 proposal、更新 proposal 状态、记录 eval run。
- Workspace GUI 已新增「质量趋势」区块：显示近 30 天 Goal / Workflow / Eval / Repair 指标、常见 blocker、候选草案，并支持生成/预览/应用/拒绝 proposal。
- Coding Eval harness 已新增 improvement run/check，`repair_loop_blocks_with_evidence` fixture 覆盖 `repair_loop_exhausted`、draft `eval_candidate` 和 eval success rate。
- Dashboard 全局化已在 Phase 4.3 通过正式 project/global scope API 落地，未用任意 session 伪装全局趋势。
- 最终架构见 [Coding Improvement Loop](../architecture/coding-improvement-loop.md) 与 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

## 8.1 Phase 4：Learning Loop / Skill & Guidance 沉淀

### Phase 4.1 Proposal-to-Action Learning Loop（已完成）

- Proposal 状态机已扩展为 `draft` / `rejected` / 内部瞬态 `applying` / `applied` / `failed`；`applied` 终态不可手动改写，`failed` 可回到 `draft` 让用户修复环境后重试。
- `eval_failed` 已进入 failure taxonomy，失败 eval run 可以直接生成 backlog proposal。
- `preview_coding_improvement_proposal_action` 已返回确定性 action plan，包含目标路径、是否已存在和内容预览。
- `apply_coding_improvement_proposal` 已能把 `eval_candidate` / `workflow_template` / `guidance_candidate` 写成工作目录 `.hope-agent/coding-improvement/` 下的 reviewable draft artifact；`skill_candidate` 写成 managed draft skill。
- 应用路径 fail-closed：先原子 claim `draft -> applying`，目标文件已存在或并发创建都不覆盖，失败写入 proposal `failed` + error；成功写入 `applied` + artifact path/hash。
- Workspace 质量趋势区块已支持 proposal 展开详情、预览、应用、拒绝，并展示应用产物或错误。
- Coding Eval 新增 `improvement_proposal_to_action` fixture，覆盖 eval failed → proposal → applied draft artifact。
- 最终架构见 [Coding Improvement Loop](../architecture/coding-improvement-loop.md)。

### Phase 4.2 Draft Promotion + Workflow Retro Loop（已完成）

- `workflow_runs` 进入 terminal state 时 best-effort 生成 `coding_workflow_retros`，记录 summary、signals、recommendations，并追加 `coding_retro_recorded` trace event；无痕会话不写。
- `CodingTrendReport` 新增 `retro` 汇总与 `retros[]` 列表，Workspace 质量趋势区块展示最近 retro 和 recommendation。
- `generate_coding_improvement_proposals` 会消费 retro recommendation，失败/阻塞进入 eval/guidance 候选，成功且具备 review + verification + diff 证据可进入 workflow template 候选。
- 新增 `preview_coding_improvement_proposal_promotion` / `promote_coding_improvement_proposal`，Desktop 与 HTTP 两端对齐。
- `eval_candidate` 晋升到 `crates/ha-core/tests/fixtures/coding_eval/`；`workflow_template` / `guidance_candidate` 晋升到 `.hope-agent/coding-improvement/promoted/` 并由 `AGENTS.md` managed include 引入；`skill_candidate` 激活 managed draft skill。
- promotion 使用 `promoting` / `promoted` / `promotion_failed` 托管状态，目标已存在且内容不同、并发创建、AGENTS include 或 skill 激活失败均 fail-closed。
- Coding Eval 新增 `improvement_retro_and_promotion` fixture，覆盖 retro 写入、候选生成、草稿应用和正式晋升。
- 最终架构见 [Coding Improvement Loop](../architecture/coding-improvement-loop.md) 与 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 4.3 Dashboard 全局学习视图（已完成）

- 新增 `dashboard::coding_improvement` 只读聚合：消费 workflow、coding eval、review finding、verification step、proposal、workflow retro durable 表。
- 新增 `dashboard_coding_improvement` Tauri command 与 HTTP `POST /api/dashboard/learning/coding-improvement`，输入复用 `DashboardFilter`，输出 overview / timeline / byProject / topFailures / proposalStatuses / latestRetros。
- Dashboard Learning Tab 新增 Coding Improvement 区块：展示 workflow/eval 成功率、review blocker、verification failure、distillation queue、项目级信号、failure modes 和 latest retros。
- 该视图不生成 proposal、不 apply、不 promotion，只读既有事实；cron / subagent / incognito session 按 Dashboard 通用规则排除。
- 单元测试覆盖项目级 rollup 与 incognito 排除。
- 最终架构见 [Coding Improvement Loop](../architecture/coding-improvement-loop.md) 与 [Dashboard 数据大盘架构](../architecture/dashboard.md)。

### Phase 4.4 Transcript Distillation + Failure Feedback（已完成）

- 新增 `distill_coding_improvement_proposals` owner action：显式扫描当前 session/project scope 的真实 transcript、tool result、workflow ops 和 failure taxonomy。
- 输出 `DistillCodingImprovementResult`，包含 transcript 统计、top tools、tool errors、workflow pattern、failure feedback rule、expected signals 和本次候选摘要。
- 蒸馏候选仍写入既有 `coding_improvement_proposals(status='draft')`，不新增状态机、不自动 apply、不自动 promotion，重复触发靠 fingerprint 去重。
- Workspace 质量趋势区块新增「提炼候选」按钮，与「生成改进候选」并列；生成看 trend report，提炼看 transcript/workflow/failure 细节。
- Action plan 生成会把 distillation evidence 写进 workflow / guidance / skill 草稿，减少“草稿只有泛泛建议”的问题。
- Tauri / HTTP / Transport 已接通：`POST /api/sessions/{sid}/coding-improvement/distill`。
- 单元测试覆盖 transcript message、tool error、review+verify+diff workflow op、failed eval feedback、proposal 插入和重复触发去重。
- 最终架构见 [Coding Improvement Loop](../architecture/coding-improvement-loop.md)。

## 8.2 Phase 5：任务级评测与策略效果评估

### Phase 5.1 Task-level Eval Runner（已完成）

- `coding_eval.rs` 新增 `fixture.task` / `runs.task` / `checks.task`，把人工 gold task 的任务定义、候选 diff、验证命令、review/context/goal evidence 接入同一套 deterministic harness。
- Runner 输出 `CodingTaskEvalReport`：outcome、score、failure category、diff summary、validation summary、review summary、context recall、goal evidence 和逐项 check。
- 默认把任务级结果写入 `coding_eval_runs(suite='task_level_coding_eval', source_type='coding_task_eval')`，让 Improvement Loop / Dashboard 可以继续消费。
- Tauri / HTTP / Transport 已接通：`run_coding_task_eval_fixture` / `POST /api/coding-eval/task-fixtures/run`。
- `task_level_eval_runner` fixture 覆盖 docs-only 候选 diff、cheap validation、context recall、Goal evaluation、eval run 记录和 Improvement Loop 消费。
- 最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 5.2 Agent Execution Runner（已完成）

- `coding_eval.rs` 新增 `runs.execution` / `checks.execution`，在 review / verification / context / task scoring 前增加执行阶段。
- `mode="agent"` 真实创建 user message + chat turn 并调用 `run_chat_engine`；fixture 显式提供 `providers` / `modelChain`，Tauri / HTTP owner API 不隐式读取桌面全局 provider。
- `mode="fixture_patch"` 作为无外部 LLM 的 deterministic 回归替身，只在执行阶段写入 `repo.changes`，不冒充真实 agent 成功率。
- 输出 `AgentExecutionEvalReport`：mode、status、prompt、agentId、turnId、response/error、modelUsed、changedFiles、diffBytes。
- task scorer 自动加入 `execution.completed` critical check；执行失败不能被其它宽松 check 掩盖。
- `agent_execution_runner_fixture_patch` fixture 覆盖执行阶段产出 diff 后再进入 review / verification / context / task scoring / eval-run recording。
- Rust mock-provider 单测覆盖 `mode="agent"` 真实调用 chat engine、创建 turn 并记录 response。
- 最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 5.3 Gold Task Pack v1（已完成）

- `coding_eval.rs` 新增 `GoldTaskPackSummary` / `GoldTaskPackReport` / `GoldTaskPackRunInput`，把 active gold tasks 从单个 JSON fixture 提升到可批量 materialize / run 的 pack 层。
- 内置首批 5 个 active gold tasks：`CE-TEST-004`、`CE-RUST-001`、`CE-REV-002`、`CE-NAV-001`、`CE-NAV-002`。
- 默认把每个 case materialize 成 `runs.execution.mode="fixture_patch"` 的普通 fixture，再进入 Review / Smart Verification / Context Retrieval / Goal / Task scorer；默认不访问外部模型。
- Tauri / HTTP / Transport 已接通：`list_coding_eval_gold_tasks` / `GET /api/coding-eval/gold-tasks`，`run_coding_eval_gold_task_pack` / `POST /api/coding-eval/gold-tasks/run`。
- targeted tests 覆盖 pack summary 与两个 active cases 的批量回放。
- 最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 5.4 Strategy Effect Evaluator（已完成）

- `coding_eval.rs` 新增 `StrategyEffectEvalInput` / `StrategyEffectReport`，比较 baseline 与 candidate 两份 `GoldTaskPackReport`。
- 聚合维度覆盖 pass rate、average task score、context recall、validation violations、scope creep 和 execution failures。
- 只用共同 case 计算聚合指标；candidate 新增 case 只展示，candidate 漏掉 baseline case 记为回归风险。
- Tauri / HTTP / Transport 已接通：`evaluate_coding_eval_strategy_effect` / `POST /api/coding-eval/strategy-effects/evaluate`。
- targeted tests 覆盖候选质量下降与 candidate 漏跑 baseline case 两类回归。
- 最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 5.5 Gold Task Pack 全量自动化（已完成）

- 20 个 Phase 0 gold tasks 全部标记为 `active`，并全部具备 `fixture_patch` 自动化定义。
- Pack 覆盖 docs/design-only、Rust、TS、i18n、多文件 diff 与 review-seeded case；每个 case 可声明支持文件、额外改动文件、允许/禁止验证命令和 review finding 上限。
- Summary 从 `20 / 5 / 5` 收敛为 `20 / 20 / 20`，owner API 与 Transport 不需要新增端点。
- targeted tests 覆盖 former-draft case 回放与全 20 case pack 回放，确保 skipped / failed 为 0。
- 最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 5.6 Mock Tool-call 基线与执行指标（已完成）

- `AgentExecutionEvalReport.toolCalls` 与 `FixtureReport.metrics.execution_tool_calls` 记录真实 tool message 名称。
- `checks.execution.expectedToolCalls` / `minToolCalls` 可断言 agent 确实调用指定工具。
- 本地 mock OpenAI Responses SSE 单测驱动真实 `write` 工具写入临时 repo，产出 candidate diff 后进入 task-level scorer。
- `ChatEngineParams.session_db` 绑定到 `AssistantAgent`，保证 eval/headless 隔离 DB 的 session working dir、permission mode、sandbox mode、project_id 与工具执行上下文一致；incognito 缺行仍 fail-closed。
- coding eval 临时 DB 统一执行 `ChannelDB::migrate()`，避免 `get_session()` metadata join 与生产 schema 漂移。
- 最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 5.7 Strategy Effect 趋势持久化 / Dashboard（已完成）

- 新增 `coding_eval_pack_runs` / `coding_strategy_effect_runs`，把 `GoldTaskPackReport` / `StrategyEffectReport` history 持久化为可审计质量闸。
- `run_coding_eval_gold_task_pack` 默认 `recordPackRun=true`，返回 `packRunId`；`baselineKind` 区分 `deterministic_mock` / `mock_provider` / `external_model`。
- `evaluate_coding_eval_strategy_effect` 保持纯对比默认无副作用，`recordRun=true` 时写入 strategy effect history 并返回 `runId`。
- Dashboard Learning Tab 展示 pack pass rate、strategy verdict、tool-call failure mode、validation / scope creep 趋势和 latest strategy effects。
- mock / fixture 基线与外部真实模型基线分离，避免把 deterministic mock 结果冒充真实 provider 能力。

### Phase 5.8 Release Gate（已完成）

- `evaluate_coding_eval_release_gate` / `POST /api/coding-improvement/release-gate/evaluate` 读取持久化 pack / strategy / tool-call history，返回 `passed` / `failed` / `insufficient_data`。
- 默认阈值保守：至少 1 次 pack run、pack pass rate 100%、strategy regression / mixed / missing tool-call / validation delta / scope creep delta 均不得超过 0。
- `requireExternalModelPack=true` 时必须存在 `baselineKind="external_model"` 的 pack run；deterministic / mock provider 结果不会被冒充为真实模型基线。
- targeted tests 覆盖干净历史通过、策略/工具调用回归失败、要求外部真实模型但证据不足三种发布门禁状态。

### Phase 5.9 外部模型基线 runner（已完成）

- `run_coding_eval_gold_task_pack` 支持 `executionMode="agent"`、`providers`、`modelChain`、`autoApproveTools`，从 gold task prompt 真实调用 chat engine，让模型通过工具产出 candidate diff。
- `baselineKind="external_model"` 必须走 agent execution 且必须显式传 provider/model；`agent` 也不能记录为 `deterministic_mock`。
- targeted tests 使用本地 mock Responses provider 覆盖完整 agent pack runner、真实 `write` tool-call、pack history 记录为 `external_model`，不访问外网。

### Phase 5.10 Learning Generalization Gate（已完成）

- `evaluate_coding_learning_generalization` / `POST /api/coding-improvement/generalization/evaluate` 读取 promoted learning、pack history 与 strategy effect history，返回 `passed` / `failed` / `insufficient_data`。
- 默认至少要求 2 个项目、每项目至少 1 次 pack run、pack pass rate 100%、promoted learning 存在，且不允许 strategy regression / mixed / validation delta / scope creep delta。
- 支持按 `sourceType` / `sourceId` 收窄 promoted learning 与 strategy effect，用同一来源验证某条 guidance / workflow / skill 是否跨项目成立。
- Dashboard Learning Tab 新增 Generalization Gate 卡片，让用户直接看到跨项目学习是否可推广。
- targeted tests 覆盖两个项目干净证据通过、任一项目 regression 触发失败。

## 8.3 Phase 6：真实能力 Benchmark 与产品化增强

### Phase 6.1 Benchmark Run Center v1（已完成）

- `get_coding_benchmark_center` / `POST /api/coding-benchmark/center` 读取 `coding_eval_pack_runs` history，聚合 run pass rate、case pass rate、baseline buckets、recent runs、failed case summary。
- Center 嵌入 Release Gate 与 Learning Generalization Gate，并输出 `passed` / `failed` / `insufficient_data` 三态 readiness。
- `requireExternalModelBaseline` / `requireLearningGeneralization` 可把 external model / generalization 从 advisory 升级为 required，供发布脚本或更严格 benchmark 使用。
- Dashboard Learning Tab 新增 Benchmark Center 卡片：展示整体状态、run/case pass rate、external model run 数、baseline buckets、recent runs、未通过 checks。
- Dashboard Run 按钮显式触发全量 deterministic Gold Pack：`executionMode="fixture_patch"`、`baselineKind="deterministic_mock"`、`sourceType="benchmark_center"`、`sourceId="phase6.1"`，不会默认访问外部模型。
- 真实外部模型 benchmark 仍走既有显式 API：`run_coding_eval_gold_task_pack(executionMode="agent", baselineKind="external_model", providers, modelChain)`，Dashboard 只展示其持久化结果，不自动触发费用/网络调用。
- targeted tests 覆盖 clean deterministic history 通过、latest failed pack run 失败、要求 external model baseline 但只有 deterministic history 时 `insufficient_data`。

### Phase 6.2 Benchmark Campaign Runner（已完成）

- 把单次 Gold Pack run 升级为 durable campaign：记录 campaign、provider/model matrix、task pack、预算/超时 contract、状态、item attempt、子 run 与关联 pack report。
- `coding_benchmark_campaigns.task_filter_json` 清空 provider config / modelChain 后落库，history 不保存 API key；runner 只使用本次 owner 调用传入的 provider configs。
- Dashboard 增加 Campaign 列表：排队、运行、完成、失败、取消、interrupted 都可见，可取消，可 retry failed / interrupted / cancelled items。
- 默认 Run 仍是 deterministic campaign，不访问外部模型；External campaign 控制区可显式选择 provider/model、max tasks 与预算 contract，外部模型 runner 会从本次输入或本机 cached config 解析 provider configs。
- 当前 runner 是 owner-plane 后台 task + durable item 状态；跨模型 leaderboard 已在 P6.3 补齐，真实任务集 registry / health 已在 P6.4 补齐，报告导出已在 P6.5 补齐；P6.6 已把持续 gate、失败 backlog、可靠性和预算指标接入 owner-plane。跨进程恢复仍是后续运行时能力，不再阻塞 Phase 6 完成。

### Phase 6.3 Cross-model Comparison & Leaderboard（已完成）

- 基于 campaign item history 生成同 pack / source doc / execution mode / baseline kind 的模型对比报告。
- 聚合 item pass rate、case pass rate、checks、attempts，并保留 sample-size / incomplete / cancelled warning。
- Dashboard 增加 Model leaderboard；不同 pack、不同 source doc、不同 execution mode、不同 baseline kind 不默认并排排名。
- 每个 leaderboard 数字都能回到原始 campaign item、pack run 与 error evidence。

### Phase 6.4 Real Task Corpus Expansion（已完成）

- 已设计并实现 task pack manifest 和 versioning：任务来源、repo template、难度、语言/框架、成功标准、验证命令、允许/禁止改动、人工校准记录。
- 已支持显式 owner import；记录 license / privacy note / redaction 状态，且 `explicitImportConsent=true` 才能导入。
- 已支持 task type、difficulty、language/framework、risk flags 和 active/draft/archive 状态。
- Dashboard / owner API 已增加 corpus health：active/draft/archive、覆盖、过期、重复、fixture-gaming risk、task type / difficulty / language 分布。

### Phase 6.5 Benchmark Report Export（已完成）

- 已支持从 campaign / comparison / release gate 生成可复盘报告：执行摘要、scope、关键指标、三态结论和 evidence 摘要。
- 已支持 Markdown / JSON / HTML snapshot；报告数字来自生成时刻的稳定 snapshot，不依赖 live DB 变化。
- 已在 snapshot 中保留 campaign、pack run、leaderboard、release gate、benchmark center 与 corpus health evidence。
- Dashboard 已增加 report history，可生成 Comparison / Release / 最新 Campaign 报告、复制路径、标记为 release evidence。

### Phase 6.6 Continuous Benchmark Gate & Improvement Backlog（已完成）

- 已支持手动 / 发布前 / 策略变更后 / task pack 更新后 / 周期触发语义的 continuous benchmark gate 输入；外部模型要求默认 fail-closed，必须显式 `externalModelPolicyEnabled=true`。
- 已把 release gate、release evidence report、recent campaign、指定 task pack、指定 provider/model/baseline、最小样本量、case pass rate、open backlog、pending failure、corpus health、预算 contract 和可靠性指标合成三态 gate report。
- 已把 failed / interrupted / cancelled campaign item 转入 benchmark improvement backlog，保留 task id、model、baseline、失败分类、pack report evidence 与 campaign evidence；重复物化靠 `(campaign_item_id, task_id)` 去重。
- Dashboard 已增加 Continuous Gate / Benchmark Backlog 面板：展示阻塞原因、推荐下一步、可靠性/预算摘要，可一键创建 backlog、标记 resolved。
- 已暴露 retention summary knobs：`retentionDays` / `rawArtifactRetentionDays`。P6.6 不做静默删除，真实 cleanup 必须是后续显式 owner action，避免破坏 report snapshot evidence。

## 8.4 Phase 7：通用场景层与 Domain Workflow

详细路线见 [通用场景层与 Domain Workflow 路线图](general-domain-workflows.md)。P7 的目标不是再造一套非 coding agent，而是复用已经稳定的 Goal / Mode / Workflow / Loop / Task / Evidence / Review / Verification / Learning Loop，把它们产品化到非编程长任务。

### Phase 7.1 Domain Workflow Registry（已完成第一版）

- 已建立 domain workflow manifest、代码内置 registry、用户/项目自定义模板表、版本和启用范围。
- 已内置 Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation、Inbox、Project Ops 等首批任务类型。
- 已通过 `preview_domain_workflow` 从 domain template 生成 `workflow.js` draft，继续走 Script Gate 和 permission preview；preview 不创建 run、不执行脚本。
- GUI 创建入口已把任务类型选择、证据要求、审批门和 draft 预览做成用户可见产品面；Goal 创建 / 编辑和 Workflow Control Center 都可继承或选择领域模板。

### Phase 7.2 General Evidence Model（已完成第一版）

- 已扩展 Goal evidence 到通用证据：source citation、claim check、user decision、artifact、data quality、citation audit、message approval、meeting context。
- 已新增 `domain_evidence_items`，记录来源 metadata、access scope、confidence 和 redaction 状态，并可通过 `goal_links` 链回 Goal。
- 非 coding workflow 已有独立 evidence 写入面，不再需要用 validation/diff/file 伪装所有证据。
- Workflow host API sugar、Goal detail 领域 evidence timeline、connector provenance、导出敏感来源提示已落地；Phase 7.15 / 7.16 进一步把交付导出和真实外部动作接入守门。

### Phase 7.3 Domain Context Retrieval（已完成第一版）

- Context Retrieval 已增加 document、email thread、calendar event、sheet range、knowledge note、web source、decision、artifact、task 等候选。
- 排序已按 domain workflow required evidence、Goal criteria、confidence、redaction、query boost 综合加权，不只靠关键词。
- 候选行已展示引用 / evidence / 摘要 / 请求用户确认 / 标记冲突 / 转 task 等 domain action；复制引用已作为真实轻量动作落地。
- access issue 已显式展示连接器或 required evidence 缺口，不伪造不存在的上下文。

### Phase 7.4 Domain Verification & Review（已完成第一版）

- 已新增 `domain_quality_runs/checks/events` durable 控制面，基于 Domain Workflow template、domain evidence 与 approval gates 做同步确定性复核。
- Research / Writing / Data Analysis / Meeting Prep / Inbox / Knowledge Curation / Project Ops 均有第一版 domain profile，覆盖引用、claim、结构、口径、材料、事实、发送确认、dedupe、owner/risk 等质量项。
- 高风险动作通过 `requestedAction` / `highRiskAction` 触发 `needs_user`，缺少显式确认时 fail closed。
- 结果写回 Goal evidence；`domain_quality_blocked/failed/needs_user` 与 P0/P1 `domain_quality_check` 会阻止 Goal completed，后续 `domain_quality_passed` 可解除较早阻塞。
- Workspace 已新增「领域复核」区块，非 coding / 无工作目录会话也能运行和查看质量门。
- 最终架构见 [Domain Quality 控制平面](../architecture/domain-quality.md)。

### Phase 7.5 Domain Learning Loop（已完成第一版）

- 已从 Domain Quality run/check/evidence 生成通用领域 learning proposals。
- Proposal kinds 已覆盖 `domain_workflow_template`、`domain_guidance`、`domain_review_profile`、`domain_eval_case`、`connector_usage_pattern`。
- 成功 quality run 沉淀 workflow/guidance 草稿；blocked / failed / needs_user quality run 沉淀 review profile / eval case；高风险 approval 卡点沉淀 connector usage pattern。
- Draft apply / promotion 复用现有 Coding Improvement 安全链路，不直接改生产模板。
- Workspace proposal 列表已显示领域类 proposal 标签。
- Dashboard Learning 已增加按领域的完成率、blocked 原因、证据质量、review catch、确认卡点、eval/gate 与 calibration 可见性。

### Phase 7.6 General Eval & Quality Gate（已完成第一版）

- 已建立 15 个通用 eval tasks：Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation 各 3 个。
- 已用 Goal / Workflow / Domain Evidence / Domain Quality trace 做 deterministic scoring，并写 `domain_eval_runs`。
- 已建立通用 quality gate：evidence completeness、citation quality、data quality、approval safety、completion criteria match、workflow trace、domain coverage。
- Dashboard Learning 已新增独立通用质量区块，不与 coding benchmark 混排。

### Phase 7.7 Domain Eval Calibration（已完成第一版）

- 已新增 `domain_eval_calibrations`，记录 user/project scope 的人工校准与复核结论。
- `list_domain_eval_tasks` 与 `run_domain_eval_task` 会把相关 calibration 附加到 task/report，避免内置 rubric 被误当成已校准能力证据。
- Dashboard Learning 的 General domain quality 卡片显示已校准 task 数，并允许对最近 eval run 点击「Mark reviewed」写入幂等 calibration。

### Phase 7.8 Domain Eval Fixture Runner（已完成第一版）

- 已新增 `run_domain_eval_fixture` owner API，Tauri / HTTP / transport 已接通。
- `trace_fixture` 模式创建真实 session / Goal / Domain Evidence / WorkflowRun / Domain Quality run，再调用同一 domain eval scorer 写入 `domain_eval_runs`。
- Fixture checks 支持 expected status、最低 score、指定 scorer check 通过/失败断言。
- 第一版仅作为 owner API / 回归测试能力，不直接挂在 Dashboard quality gate 上，避免合成 smoke 样本污染真实质量判断。

### Phase 7.9 Domain Eval Agent Fixture Execution（已完成第一版）

- `run_domain_eval_fixture` 已支持 `executionMode="agent"`，创建真实 user message + chat turn 并调用 `run_chat_engine`。
- Agent fixture 必须显式传 `execution.providers` / `execution.modelChain`；owner API 不隐式读取桌面全局 provider。
- 默认 `execution.workflowMode="ultracode"`，让通用任务也能走自主动态 Workflow Mode；fixture 可显式降级到 `on` / `off`。
- `report.execution` 记录 status、turnId、response/error、modelUsed、toolCalls 和 workflowMode；checks 支持 execution status、turn、tool call、response/error 断言。
- Agent fixture 不自动写入 `fixture.evidence` / `fixture.workflow`，避免模型未产出真实 evidence/workflow trace 时被确定性种子托过关。
- 执行失败或缺 provider/modelChain 时不写 `domain_eval_runs`；执行完成但证据不足时仍由同一 scorer 标记 failed / insufficient_data。
- 定向测试使用本地 mock Responses provider 覆盖真实 chat engine 路径，不访问外部网络。

### Phase 7.10 Domain Fixture / Smoke Run Center（已完成第一版）

- Fixture runner 创建的 session 标记为 `SessionKind::EvalFixture`，从普通会话列表、普通搜索和 live Dashboard 聚合中隔离。
- `domain_eval_runs.source_type` 区分 `live` / `fixture_trace` / `fixture_agent`，真实 quality gate 默认排除合成样本。
- 新增 `domain_eval_fixture_runs` 和 `list_domain_eval_fixture_runs`，让执行失败且没有 eval run 的 agent fixture 也能被 Dashboard 回放。
- Dashboard Learning 新增「Domain smoke runs」卡片，展示 pass rate、agent/trace 数、失败数、trace badge 与 error。

### Phase 7.11 Domain Eval Campaign Runner（已完成第一版）

- 新增 durable `domain_eval_campaigns` / `domain_eval_campaign_items`，把多个 task × model/execution item 组织成批量通用 eval pack。
- 新增 create/list/get/run/cancel owner API，Tauri / HTTP / transport 已接通。
- Runner 复用 `run_domain_eval_fixture`，不新增第二套 scorer；deterministic trace item 自动生成 synthetic evidence，external model item 走 agent fixture。
- 支持 cancel queued item、retry failed/interrupted/cancelled item、item attempt 与最新 fixture/eval run 指针。
- Dashboard Learning 新增「Domain campaigns」卡片，可运行 trace pack、查看 item pass rate / 平均分 / check 数，并执行 cancel / retry。

### Phase 7.12 Domain External Campaign & Leaderboard（已完成第一版）

- `create_domain_eval_campaign(runNow=true)` 可临时携带 providers，external campaign 能立即进入 agent runner，密钥不落库。
- 新增 `get_domain_eval_campaign_leaderboard`，按 provider/model/label/execution 聚合 item history，输出 rank、pass rate、average score、warning 与 evidence。
- Dashboard Learning 的 Domain campaigns 区块新增 provider/model 选择、external campaign 运行、max tasks / budget 输入和 Domain model leaderboard。
- 缺 provider secret 的 external item 明确 failed 并进入 leaderboard warning，不写 eval run、不静默成功。

### Phase 7.13 Domain Campaign Learning Closure（已完成第一版）

- `generate_coding_improvement_proposals(sourceType="domain_eval_campaign")` 会把 failed / cancelled / interrupted domain campaign item 转成 `domain_eval_case` 与 `domain_guidance` draft proposal。
- 候选 payload 保留 campaign/item/report/failure category，sourceId=campaign id、fingerprint=item id + kind，支持按 campaign 定向生成并幂等去重。
- Dashboard Learning 的 Domain campaigns 行新增学习按钮，失败 campaign 可以显式生成学习草稿；仍不自动 apply、不自动 promotion。

### Phase 7.14 Domain Readiness Gate（已完成第一版）

- 新增 `evaluate_domain_readiness_gate`，组合 live Domain Quality Gate、Domain Eval Campaign、Domain model leaderboard 和 Campaign Learning Closure。
- Readiness checks 覆盖 quality gate、campaign sample、campaign completion、leaderboard、campaign failures、learning closure，并输出 blockers / recommended next steps。
- 运行中的 campaign 只让 gate 保持 `insufficient_data`，失败/取消/中断 item 默认阻断，未物化或未关闭的 learning proposal 也会阻断。
- Dashboard Learning 新增 Domain readiness 卡片，把通用领域长任务当前是否可交付做成一眼可见的三态。

### Phase 7.15 Domain Artifact Export Guard（已完成第一版）

- 新增 `evaluate_domain_artifact_export_guard`，只读 Domain Evidence 判断最终发送、分享、导出、发布前是否已有产物、复核和脱敏证据。
- Workspace「领域复核」内新增「交付守门」卡片，展示产物/复核/敏感/待脱敏计数、阻塞 check 和需复核 evidence。

### Phase 7.16 Domain Connector Action Guard（已完成第一版）

- 新增 `evaluate_domain_connector_action_guard`，只读 Domain Evidence + Artifact Export Guard 判断真实外部连接器动作是否具备动作、批准、回滚和交付复核证据。
- 权限引擎新增 strict `ExternalConnectorAction`：内置 Feishu/Lark 写工具精确识别，MCP/plugin 工具保守按连接器名 + mutating verb 识别；禁止 AllowAlways / Smart 覆盖 / auto-approve 静默旁路。
- Workspace「领域复核」内新增「外部动作守门」卡片，展示动作/批准/回滚/敏感计数、阻塞 check 和相关 evidence。

## 8.5 Phase 8：真实场景产品级验收

### Phase 8.1 Domain Operational Gate（已完成第一版）

- 新增 `evaluate_domain_operational_gate`，只读 WorkflowRun / LoopRun / Domain Campaign 历史，把通用长任务运行稳定性合成 `passed` / `failed` / `insufficient_data` 三态。
- Checks 覆盖 workflow sample、workflow failed/blocked/cancelled、active workflow drain、loop sample、loop failures、active campaign drain 和 campaign failed/cancelled/interrupted item。
- Dashboard Learning 新增「Domain operations」卡片，让用户能看见 workflow / loop / campaign 的完成、活跃、失败残留和下一步建议。
- Gate 只读事实：不会自动 approve / cancel / retry，不把 active long task 误报成 failed，只让 operational readiness 保持 `insufficient_data`。

### Phase 8.2 Connector E2E Gate（已完成第一版）

- 新增 `evaluate_domain_connector_e2e_gate`，只读 Domain Evidence，并在 session / goal scope 下嵌套 Connector Action Guard 与 Artifact Export Guard。
- 新增 `connector_context_collected`、`connector_draft_created`、`connector_action_executed`、`connector_action_verified` 四类 evidence，覆盖读取、草稿、执行和执行后复核。
- Checks 覆盖 connector input、draft/preview、explicit approval、execution result、post-action verification、rollback plan、Connector Action Guard 和 Artifact Export Guard。
- Dashboard Learning 新增「Connector E2E」卡片，展示 IN / DR / OK / EX / VF / RB / GU 和下一步建议。
- Gate 不访问真实账号、不替用户执行外部动作；没有执行结果或执行后复核只能 `insufficient_data`，缺用户批准直接 `failed`。

### Phase 8.3 Domain Soak Report（已完成第一版）

- 新增 `generate_domain_soak_report`，只读 WorkflowRun / WorkflowEvent / LoopRun / Domain Campaign / Connector E2E evidence 历史，把通用长任务跨窗口运行情况导出为 JSON / Markdown / Dashboard snapshot。
- Summary 覆盖 workflow drain、审批/暂停/恢复/取消/恢复事件、loop tick、campaign item retry / failed / interrupted、connector execution / verification evidence。
- Incidents 区分 critical 与 warning；failed/blocked/cancelled workflow、failed/cancelled/interrupted campaign item、failed/cancelled loop 为 critical，running/queued/awaiting approval 等未 drain 工作为 warning。
- Dashboard Learning 新增「Domain soak report」卡片，展示 WF / LP / CP / CE / CR / WN / MAX / REC、最近 timeline 和下一步建议。
- Report 只读事实：没有样本不能 passed；active work 不伪装成失败，但会保持 `insufficient_data`；不会自动 approve / cancel / retry。

## 9. 体验与性能红线

- 长任务必须可观察：状态、下一步、卡点、证据、预算都要可见。
- 长任务必须可恢复：重启后不能丢 run / goal / task 关系。
- 长任务必须可停止：pause / cancel / stop 的语义要真实执行，不只是改 UI。
- 审批必须 fail-closed：无人能批时不能永久挂死，也不能默认越权。
- GUI 不做命令行的薄皮：用户不应必须记 slash 命令才能掌控任务。
- Prompt cache 要稳定：goal / mode / workflow 状态进入动态段，不破坏静态 prefix。
- Coding-specific 能力必须挂到控制平面上：worktree / LSP / review / verification 不是孤立工具堆叠。

## 10. 文档落点

- 本文记录路线和方案，保留在 `docs/roadmap/`。
- 已实现的 Goal 第一版见 [Goal 控制平面](../architecture/goal.md)。
- [Goal / Mode / Workflow / Loop 语义收口](control-plane-semantics.md) 记录产品语言边界。
- [Phase 2 Workflow Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md) 记录已落 workflow runtime 与 GUI 的设计历史。
- 已实现的 Managed Worktree 见 [Managed Worktree 控制平面](../architecture/worktree.md)。
- 已实现的 LSP / Review / Smart Verification / Context Retrieval 分别见 [LSP 与语义代码智能](../architecture/lsp.md)、[Review Engine 控制平面](../architecture/review-engine.md)、[Smart Verification 控制平面](../architecture/verification-engine.md)、[Context Retrieval v2](../architecture/context-retrieval.md)。

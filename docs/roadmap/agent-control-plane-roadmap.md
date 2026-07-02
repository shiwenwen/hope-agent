# Agent 控制平面路线图

> 返回 [路线图索引](README.md)
>
> 更新时间：2026-07-02
>
> 状态：路线调整与方案设计。`/goal` 第一版已落地并沉淀到 [Goal 控制平面](../architecture/goal.md)；`/loop` 第一版已落地并沉淀到 [Loop 控制平面](../architecture/loop.md)；Managed Worktree 已作为 Phase 3.1 落地并沉淀到 [Managed Worktree 控制平面](../architecture/worktree.md)；LSP / Diagnostics 已作为 Phase 3.2 落地并沉淀到 [LSP 与语义代码智能](../architecture/lsp.md)；Review Engine 已作为 Phase 3.3 落地并沉淀到 [Review Engine 控制平面](../architecture/review-engine.md)；Smart Verification 已作为 Phase 3.4 落地并沉淀到 [Smart Verification 控制平面](../architecture/verification-engine.md)；Context Retrieval v2 与 Actionable Context Loop 已作为 Phase 3.5-3.6 落地并沉淀到 [Context Retrieval v2](../architecture/context-retrieval.md)；Coding Eval 控制面评测已作为 Phase 3.7 落地并沉淀到 [Coding Eval 控制面评测](../architecture/coding-eval.md)；本文继续记录后续 coding-specific 能力推进顺序。

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
Phase 3.2  LSP / Diagnostics（已完成）
Phase 3.3  Review Engine（已完成）
Phase 3.4  Smart Verification / 智能验证选择（已完成）
Phase 3.5  Context Retrieval v2 / 推荐上下文（已完成）
Phase 3.6  Actionable Context Loop / 可行动上下文闭环（已完成）
Phase 3.7  Coding Eval 控制面评测（已完成）
Phase 3.8  Workflow Review/Verify Host API 与 Goal-aware Eval（已完成）
Phase 3.9  Repair Loop 自动化（已完成）
Phase 3.10 Deep Review / Profiles / IDE Context（计划中）
Phase 3.11 Trend Report / Improvement Loop 接口（计划中）
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
| Context Retrieval | 通用 owner-plane，当前 coding-first | 已实现 Phase 3.6 | 当前任务下一步最该看哪些上下文，以及能否直接进入 focused review / verification。 |
| Coding Eval | coding-first 质量闸，harness 可复用于通用控制面 | 已实现 Phase 3.7 | 控制面协同是否可回归，关键上下文是否被召回，focused action 是否真实收窄。 |

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
- 已落地：Review Engine 写 `review_passed` / `review_completed` / `review_finding` evidence。
- 已落地：Smart Verification 写 `validation_passed` / `validation_failed` / `validation_completed` evidence。
- 后续增强：artifact / diagnostic 强类型 evidence 接入。
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

### Phase 3.2 LSP / Diagnostics（已完成）

- `ha-core::lsp` LSP manager + 进程内 client cache。
- `lsp` 工具支持 definition / references / hover / symbols / implementation / call hierarchy / diagnostics。
- 文件编辑工具成功写入后同步 diagnostics。
- diagnostics 被动注入下一轮动态 prompt 后缀。
- Workspace GUI 展示语义诊断状态、错误/警告和最近诊断。
- Tauri / HTTP owner API 对齐。
- 最终架构见 [LSP 与语义代码智能](../architecture/lsp.md)。

后续增强：

- Goal evaluator 的强类型 diagnostics evidence。
- Workflow validation summary 汇总 diagnostics。
- ACP IDE context envelope。

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
- 不调用 LLM、不执行真实验证命令，作为后续 workflow review/verify、repair loop、LLM reviewer 和趋势 dashboard 的底座质量闸。
- 最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 3.8 Workflow Review/Verify Host API 与 Goal-aware Eval（已完成）

- `workflow.review({ focusPaths?, baseRef?, profiles?, scope? })` 已作为 idempotent durable host API 落地，复用 Review Engine，并继承当前 workflow run 的 Goal。
- `workflow.verify({ focusPaths?, maxCommands?, scope? })` 已作为 idempotent durable host API 落地，复用 Smart Verification selector，只生成计划，不执行命令。
- Script Gate / permission preview 识别这两个 API 为 permission-neutral coding control-plane API；runtime replay 复用已完成 op output。
- Goal evidence 串联已完成：review evidence、verification `validation_completed`、workflow `workflow_completed` 能进入同一 Goal 证据链。
- Coding Eval 新增 workflow-bound fixture，验证 workflow op、review run、verification plan、Goal evidence 和 Context Retrieval 召回协同。
- 最终架构见 [Workflow 与 Execution Mode](../architecture/workflow.md)、[Review Engine 控制平面](../architecture/review-engine.md)、[Smart Verification 控制平面](../architecture/verification-engine.md)、[Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 3.9 Repair Loop 自动化（已完成）

- `workflow.repairLoop({ label?, maxAttempts?, validationCommands?, focusPaths?, review?, verify? }, fn)` 已落地为脚本级 bounded repair loop；修复动作保持动态 callback，不退回固定 DSL。
- 每轮 attempt 自动创建 task、执行 callback、运行 validation / focused review / verification plan，并写入 `repair_loop_*` trace。
- `workflow.block({ reason?, label?, payload? })` 已落地为显式受控停机出口；attempt budget 耗尽统一 `blocked(reason=repair_loop_attempts_exhausted)`。
- 原有 guarded repair stop guard 继续处理重复验证失败和无有效 diff 进展；repairLoop 不绕过旧安全阀。
- Workspace 目标驱动 workflow 草稿默认使用 repairLoop，用户创建长任务时直接进入可验证的修复闭环。
- Coding Eval 新增 repair-loop fixture，验证 blocked state、Goal `validation_failed` / `workflow_blocked` evidence 和 Context Retrieval 召回。
- 最终架构见 [Workflow 与 Execution Mode](../architecture/workflow.md)、[Coding Eval 控制面评测](../architecture/coding-eval.md)。

### Phase 3.10 Deep Review / Profiles / IDE Context（计划中）

- LLM reviewer 生成候选 findings，再由独立 verifier 降噪；Review Engine 继续保留 deterministic 输出与 durable finding store。
- Review profiles 支持 correctness / security / concurrency / frontend / accessibility / tests 等组合，并进入 review stats 与 Workspace 展示。
- IDE / ACP 当前文件、selection、open tabs、active diagnostics、active symbol 接入 Context Retrieval 和 focused review / verification。
- Diff scan 从文件级增强到 enclosing function / symbol context，减少大文件噪音。
- eval 增加 profile-specific review、IDE context recall、LSP diagnostics fixture，继续保持无 LLM 的控制面回归。

### Phase 3.11 Trend Report / Improvement Loop 接口（计划中）

- 汇总 coding eval、workflow、goal、review、verification、repair loop 数据，生成项目级 coding trend report。
- 统计 repair loop 成功率、blocked reason、review blocker 类型、验证失败类型和上下文漏召回。
- 失败 run 可生成 eval candidate proposal；成功 run 可生成 workflow / skill / project guidance proposal。
- 默认只产 proposal，不自动修改项目规则、全局 skill 或用户记忆。

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
- [Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md) 记录已落 workflow runtime 与 GUI 的设计历史。
- 已实现的 Managed Worktree 见 [Managed Worktree 控制平面](../architecture/worktree.md)。
- 已实现的 LSP / Review / Smart Verification / Context Retrieval 分别见 [LSP 与语义代码智能](../architecture/lsp.md)、[Review Engine 控制平面](../architecture/review-engine.md)、[Smart Verification 控制平面](../architecture/verification-engine.md)、[Context Retrieval v2](../architecture/context-retrieval.md)。

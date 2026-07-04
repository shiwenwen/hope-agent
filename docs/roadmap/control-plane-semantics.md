# Goal / Mode / Workflow / Loop 语义收口

> 返回 [路线图索引](README.md)
>
> 更新时间：2026-07-03
>
> 状态：控制平面语义收口。本文定义产品语言、命令边界和后续实现顺序；已实现细节见 `docs/architecture/`。

## 1. 结论

Phase 2 已完成 **Workflow Mode + Workflow Run + Execution Mode**，随后补齐 **Goal 第一版**、**Goal-driven Workflow** 和真正 `/loop` 第一版。

当前统一关系：

```text
Goal        = 我要最终达成什么、完成标准是什么（已实现第一版）
Mode        = 这次会话/目标用多主动、多深的策略推进（已实现为 /mode）
Workflow    = session 级自主编排开关 + 一次具体、可观察、可恢复的执行编排（已实现）
Task        = 用户可见进度事实（已有，workflow 内已接入）
Worktree    = 代码改动落在哪个隔离环境（已实现 Phase 3.1，编码场景特有）
Loop        = 定时/重复触发或条件轮询（已实现第一版）
```

因此旧的 `/loop off|guarded|deep|autonomous` 语义已经收口为：

```text
/mode off
/mode guarded
/mode deep
/mode autonomous
```

`/loop` 不再作为执行强度入口保留，避免把“自主性策略”和“重复触发”混成一个概念。

由于这组能力尚未发布，不保留旧 `/loop off|guarded|deep|autonomous` alias、旧 `coding-loop-mode` HTTP 路由或旧 `coding_loop_mode` 数据字段兼容层。

## 2. 参考线索

本收口参考的公开产品语义：

- Claude Code [Dynamic Workflows](https://code.claude.com/docs/en/workflows)：workflow 是可由脚本表达、可暂停/恢复/查看进度的执行编排。
- Claude Code [Scheduled Tasks](https://code.claude.com/docs/en/scheduled-tasks)：`/loop` 更接近 recurring prompt / polling / scheduled continuation，不是执行强度开关。
- Claude Code [Goal](https://code.claude.com/docs/en/goal)：`/goal` 是会话级完成条件，由独立评估判断是否达成。
- Claude Code [Agent Loop](https://code.claude.com/docs/en/agent-sdk/agent-loop)：agent inner loop 是 prompt → tool calls → tool results → repeat 的底层循环。
- Claude Code [Worktrees](https://code.claude.com/docs/en/worktrees)：worktree 是文件修改隔离环境，主要服务代码任务。
- OpenAI Codex [Follow a goal](https://developers.openai.com/codex/use-cases/follow-goals)：`/goal` 是长任务 objective + stopping condition + validation loop。
- OpenAI Codex [Worktrees](https://developers.openai.com/codex/app/worktrees)：worktree 支持独立任务与后台隔离。
- OpenAI Codex [Workflows](https://developers.openai.com/codex/workflows)：workflow 更像可复用工作方式和任务执行模式，不等同于定时 loop。
- OpenAI Codex [Use cases](https://developers.openai.com/codex/use-cases)：goal / workflow / approvals / skills / automations 不是纯编码概念，编码只是最强适配场景之一。

本地早期 Claude Code 源码和提示词目录仍只作为历史线索，不作为当前产品事实。

## 3. 当前实现状态

| 概念 | 当前状态 | 产品入口 | 数据 / API | 说明 |
| --- | --- | --- | --- | --- |
| Goal | 已实现第一版 | `/goal` + Workflow Control Center Goal strip | `goals` / `goal_events` / `goal_links`，HTTP `/api/sessions/{id}/goal` / `/api/goals/{id}` | 长期任务顶层对象；完整实现见 [Goal 控制平面](../architecture/goal.md)。 |
| Mode | 已实现 | `/mode` + Workflow Control Center | `sessions.execution_mode`，`get_execution_mode` / `set_execution_mode`，HTTP `/api/sessions/{id}/execution-mode` | 会话级执行策略：`off` / `guarded` / `deep` / `autonomous`。 |
| Workflow Mode / Workflow Run | 已实现 | `/workflow` + 输入框 Workflow Mode + Workflow Control Center | `sessions.workflow_mode`、`workflow_runs.execution_mode`、`workflow_ops`、`workflow_events`，HTTP `/api/sessions/{id}/workflow-mode` | Workflow Mode 决定模型是否可自主调用 `workflow_run`；Workflow Run 是具体后台脚本执行，保存创建时的 execution mode 快照。 |
| Task | 已有并接入 workflow | Workflow UI / `workflow.task.*` | session task store + workflow host API | 用户可见进度事实，不靠 label 定位，按 create 返回 handle 更新。 |
| Loop | 已实现第一版 | `/loop` + Workspace Loop 区块 | `loop_schedules` / `loop_runs`，HTTP `/api/sessions/{id}/loops` / `/api/loops/{id}` | 只用于真正重复触发、轮询或定时继续；完整实现见 [Loop 控制平面](../architecture/loop.md)。 |
| Worktree | 已实现 Phase 3.1 | Workspace 环境面板 + Workflow 创建运行位置 | `managed_worktrees`，HTTP `/api/sessions/{id}/worktrees` / `/api/worktrees/{id}` | 代码改动隔离环境，偏 coding-specific；完整实现见 [Managed Worktree 控制平面](../architecture/worktree.md)。 |
| LSP | 已实现 Phase 3.2 | Workspace 语义诊断区块 + `lsp` 工具 | `ha-core::lsp`，HTTP `/api/sessions/{id}/lsp/status` / `/api/sessions/{id}/lsp/diagnostics` | 语义导航和 diagnostics，偏 coding-specific；完整实现见 [LSP 与语义代码智能](../architecture/lsp.md)。 |
| Context Retrieval | 已实现 Phase 3.6 | Workspace 推荐上下文区块 | `ha-core::context_retrieval`，HTTP `/api/sessions/{id}/context-retrieval` | 任务感知上下文推荐与行动入口，聚合 diff/artifact/LSP/review/verification/goal/task/workflow/search/symbol/source，并可从候选行触发 focused review / verification；完整实现见 [Context Retrieval v2](../architecture/context-retrieval.md)。 |
| Coding Eval | 已实现 Phase 6.5 | 测试/CI 质量闸 + owner API + Dashboard Benchmark Center / Campaign Runner / Leaderboard / Report History | `ha-core::coding_eval` / `ha-core::coding_improvement`，`cargo test -p ha-core --test coding_eval --locked`，`run_coding_task_eval_fixture`，`list_coding_eval_gold_tasks`，`run_coding_eval_gold_task_pack`，`evaluate_coding_eval_strategy_effect`，`evaluate_coding_eval_release_gate`，`evaluate_coding_learning_generalization`，`get_coding_benchmark_center`，`create/list/get/cancel/run_coding_benchmark_campaign`，`get_benchmark_leaderboard`，`compare_benchmark_models`，`generate_benchmark_report`，`list_benchmark_reports`，`get_benchmark_report`，`mark_benchmark_report_release_evidence` | 确定性 fixture harness，回归 Review / Smart Verification / Context Retrieval / Goal / Task / Workflow 协同，用 agent execution runner 从 prompt 产出候选结果，并用 task-level runner 评估候选 diff 是否满足任务级成功标准；20 个 active gold tasks 均可批量 materialize / run；mock tool-call 基线可验证真实 `write` 工具产出 candidate diff；Strategy Effect Evaluator 可比较策略改动前后的 pack 报告；Release Gate 可把持久化 pack / strategy / tool-call history 转成发布质量结论；Gold Pack 可显式用 provider/model 跑外部模型基线；Learning Generalization Gate 可验证 promoted learning 是否具备跨项目证据；Benchmark Run Center 可在 Dashboard 展示 benchmark readiness；Benchmark Campaign Runner 可创建 durable campaign、运行 deterministic/external item、取消与 retry，并把 item 追溯到 pack run；Cross-model Leaderboard 可按同 pack/source/execution/baseline 聚合 provider/model 表现并保留 evidence；Benchmark Report Export 可把 campaign / comparison / release benchmark 固化成 Markdown / JSON / HTML snapshot 并标记 release evidence；完整实现见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。 |

## 3.1 调整后的实施顺序

详细路线见 [Agent 控制平面路线图](agent-control-plane-roadmap.md)。当前顺序固定为：

```text
Phase 2.6  语义收口（已完成）
Phase 2.7  /goal 第一版（已完成）
Phase 2.8  Goal-driven Workflow 核心闭环（已完成）
Phase 2.9  真正 /loop 第一版（已完成）
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
Phase 4.1-4.4 Learning Loop / Skill & Guidance 沉淀（已完成）
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
```

这意味着 LSP、review engine 不作为 worktree 之前的顶层优先级。它们仍然重要，但应挂在 Goal / Workflow / Worktree 控制平面之下，否则容易形成一组强工具，却缺少长期任务的完成标准、证据链和最终收口。LSP / Diagnostics 已按这个原则落地为 Workspace 与工具层能力；Review Engine 也已按同一原则落地为 durable review run/finding，并把 P0/P1 open finding 写回 Goal evidence。
Smart Verification 同样按这个原则落地为 durable verification run/step，并把最小验证结果写回 Goal evidence。
Context Retrieval v2 则把这些分散的 coding 控制面信号收束成 Workspace 推荐上下文，并在 Phase 3.6 接入 Goal evidence、task、workflow op 关联召回与候选行 focused review / verification，帮助用户从“看到下一步”直接进入“处理下一步”。Phase 3.7 再把这组控制面协同纳入确定性 eval，确保后续增强不会破坏 focused action、最小验证选择和关键上下文召回。Phase 3.8 已把 `workflow.review()` / `workflow.verify()` 接入同一链路，workflow 脚本内产生的 review、verification plan 与 Goal evidence 也会进入控制面回归。Phase 3.9 再把 `workflow.repairLoop()` / `workflow.block()` 接入 runtime 与 eval，使 repair loop 能明确完成或 blocked。Phase 3.10 已把 review profiles、Deep Review、IDE/ACP context 和 symbol-context evidence 接入同一套 durable Review / Context / Eval 链路。Phase 3.11 已把 Trend Report / Improvement Loop 接入 durable 控制面数据与 Workspace 质量趋势区块。Phase 4 已把 learning loop 的 proposal/action/promotion/dashboard/distillation 链路接上。Phase 5.1 已把候选 diff 的 task-level 判分接入同一 eval 表，Phase 5.2 已把真实 chat engine execution 接到 scorer 前，Phase 5.3 已把首批 active gold tasks 接成可批量回放的 Gold Task Pack v1，Phase 5.4 已把策略效果对比接成 owner API，Phase 5.5 已把 20 个 gold tasks 全量自动化，Phase 5.6 已把 mock tool-call 写文件基线接入真实工具循环，Phase 5.7 已把 pack / strategy report history 接入 Dashboard，Phase 5.8 已把持久化 history 接入 Release Gate，Phase 5.9 已把 Gold Pack 接入外部模型基线 runner，Phase 5.10 已把跨项目学习泛化接入 Generalization Gate，Phase 6.1 已把这些 benchmark history 产品化为 Benchmark Run Center，Phase 6.2 已把 benchmark run 包装成 durable campaign 长任务，Phase 6.3 已把 campaign item history 聚成可追溯模型 leaderboard，Phase 6.4 已补真实任务集 registry / health，Phase 6.5 已补 Markdown / JSON / HTML benchmark report snapshot，Phase 6.6 已补持续 benchmark gate 与失败 backlog。Phase 7.1-7.14 已复用同一控制平面，把 domain workflow registry、workflow draft preview、通用 evidence 持久化、Goal evidence 链接、domain context retrieval、Domain Quality 领域复核、Domain Learning Loop、General Eval / Quality Gate、Domain Eval Calibration、trace/agent fixture、Smoke Run Center、可取消/可 retry 的 Domain Eval Campaign、external model leaderboard、失败 campaign 学习闭环与 Domain Readiness Gate 带到非编程长任务。

## 4. `/mode` 的准确语义

`/mode` 回答的是：

> 这次会话里，Agent 应该以多主动、多深入、多连续的方式推进？

| Mode | 用户含义 | 行为边界 |
| --- | --- | --- |
| `off` | 普通对话 | 不自动进入 repair guard；失败后主要报告下一步。 |
| `guarded` | 守护式推进 | 默认长任务策略；允许少量低风险修复，但严格预算和 stop guard。 |
| `deep` | 深入排查 | 更多 repo reconnaissance、验证和独立分析；仍需预算与停止条件。 |
| `autonomous` | 高自主推进 | 在安全边界内持续推进；不能绕过权限、审批、AGENTS 或 destructive gate。 |

命名红线：

- 用户可见文案用 **Execution Mode / 执行模式**。
- 代码、DB、API 用 `execution_mode` / `executionMode`。
- 不再使用 `coding_loop_mode`、`loop_mode`、`Coding Loop` 表示执行强度。
- `loop` 一词只用于真实循环：repair loop、agent inner loop、scheduled loop、polling loop 等。

## 5. `/workflow` 的准确语义

`/workflow` 回答的是：

> 模型是否可以自主动态编排？这一次具体 run 执行到哪一步了？能否审批、暂停、恢复、取消、修复？

Workflow 分两层：

- Workflow Mode：session 级开关，`off/on/ultracode`。开启后模型在下一轮看到 `workflow_run` 工具和 Workflow Mode prompt，自行判断任务是否值得写脚本并启动后台 run。
- Workflow Run：可审计 run，而不是长期目标本身。一个 Goal 可以派生多个 WorkflowRun；一个 WorkflowRun 也可以作为失败后的修复 run 派生出新的 WorkflowRun。

当前已落能力：

- `sessions.workflow_mode`。
- `/workflow on|off|ultracode|status|runs`。
- 输入框 Workflow Mode 入口与常驻状态。
- Workspace / Workflow Control Center Workflow Mode 控件。
- `workflow_run` 模型工具按 session mode 可见，执行层二次校验。
- `workflow.js` 受控 QuickJS runtime。
- durable `workflow_runs` / `workflow_ops` / `workflow_events`。
- Script Gate、permission preview、approval。
- pause / resume / cancel。
- trace / validation / agents / output budget。
- repair draft、parent run / origin 追踪。
- execution mode 快照：`workflow_runs.execution_mode`。

## 6. `/goal` 的准确语义

Goal 是已经落地的一等对象，用来把 workflow、task、validation evidence 纳入长期目标闭环。实现细节见 [Goal 控制平面](../architecture/goal.md)。

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

Goal 应保存：

- objective。
- completion criteria。
- current status。
- linked workflow runs。
- linked tasks。
- validation evidence。
- final audit。
- token / time / turn budget。
- last evaluator result。

完成判定不应只靠模型自说自话，而应结合：

- 用户定义的完成标准。
- workflow trace。
- validation results。
- changed files / artifacts。
- task 状态。
- 必要时的轻量 evaluator。

当前 evaluator 已升级为 deterministic rule gate：无证据、workflow failed/blocked/cancelled 且无后续修复、validation failed 且无后续 passing validation、criteria 缺少 strong evidence、budget exhausted 都会进入 `blocked`；无 blocker、无 missing 且有 workflow completed / validation passed / task completed 这类 strong evidence 时才进入 `completed`。LLM evaluator 仍是后续可选增强，只能补 rationale，不能覆盖 hard blocker。

## 7. 真正 `/loop` 的位置

真正 `/loop` 不再表示 `guarded/deep/autonomous`。它应该回答：

> 这个 prompt / goal 是否需要按时间、事件或条件重复触发？

第一版已落地的形态：

```text
/loop every 10m: check CI and continue fixing if failing
/loop until <condition>
/loop stop
/loop status
```

已落地边界：

- 必须有 Goal 或明确的 recurring prompt。
- 支持最大次数、最大运行时间、token 预算；成本预算字段保留，但创建时会拒绝，等待 provider cost ledger。
- 触发前复用 Goal budget hard stop。
- 继承原会话无人值守审批策略与权限边界。
- 必须能暂停/停止/审计。
- 复用 Cron 调度，不另起一套 scheduler。

## 8. 通用性边界

这些能力不是只能用于 coding：

- Goal：通用。
- Mode：通用。
- Workflow Mode / Workflow Run：通用；coding 只是首批深度模板，非编程任务也可用模型自主 workflow 编排。
- Task：通用。
- Loop：通用。

这些能力偏 coding-specific：

- Worktree。
- LSP / diagnostics。
- code review finding。
- git diff / validation commands。
- AGENTS.md coding rules。

产品策略：底座做通用，首批模板和 UI 以 coding-first 打磨。

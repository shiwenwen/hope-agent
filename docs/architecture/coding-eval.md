# Coding Eval 控制面评测

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 6.1 已实现。本文只记录已经落地的自动化评测层；人工 gold task 体系仍见 [Coding Eval 体系方案](../roadmap/coding-eval.md)。

## 目标

Coding Eval 控制面评测用于回答一个更底层但非常关键的问题：

> Review、Smart Verification、Context Retrieval、Goal、Task、Workflow 这些 coding 控制面，是否能在同一个真实 session 中稳定协同？

Phase 3.7 先把“可确定性回归”的能力钉住；Phase 3.8 继续把 workflow 内的 review / verification host API 纳入同一套回归；Phase 3.9 把 bounded repair loop 的停机与证据链纳入回归；Phase 3.10 把 profile-specific review 与 IDE context recall 纳入回归；Phase 3.11 把 trend report / proposal 队列纳入回归；Phase 4.1 把 proposal-to-action 纳入回归；Phase 4.2 把 workflow retro 与 draft promotion 纳入回归；Phase 5.1 把 task-level candidate result scoring 纳入回归；Phase 5.2 把真实 Agent execution runner 纳入同一条链路；Phase 5.3 把首批 active gold tasks 批量 materialize / run；Phase 5.4 把策略改动前后的 pack 报告做确定性效果对比；Phase 5.5 把 Phase 0 的 20 个 gold tasks 全部接入自动化 pack；Phase 5.6 把 mock Responses tool-call 基线接入真实工具循环；Phase 5.7 把 pack / strategy report history 纳入 Dashboard 质量趋势；Phase 5.8 把这些历史接成 release gate；Phase 5.9 把 Gold Pack 接入受控外部模型基线；Phase 5.10 把 promoted learning 的跨项目泛化纳入门禁；Phase 6.1 把这些历史产品化为 Benchmark Run Center：

- 能创建临时 git repo，制造真实 diff。
- 能创建真实 session / goal / task / workflow state。
- 能调用生产实现的 `run_review_for_session`、`plan_verification_for_session`、`context_retrieval_for_session`。
- 能创建并执行真实 `workflow.js` run，覆盖 `workflow.review()` / `workflow.verify()` durable host API。
- 能检查 focused review / focused verification 是否真正收窄范围。
- 能检查 bounded repair loop 是否可停机、可解释，并把 blocked evidence 交给下一步上下文。
- 能检查 review profiles 是否改变候选来源，并把 active profiles / IDE context 写入 stats。
- 能检查 IDE current file / selection / open tabs / active symbol 是否进入 Context Retrieval。
- 能检查 Coding Improvement Loop 是否基于 durable 数据生成 failure taxonomy、eval backlog proposal 和候选队列。
- 能检查 proposal 是否可应用为 reviewable draft artifact，并记录 applied status / artifact path。
- 能检查 terminal workflow 是否写入 deterministic retro，并把 retro recommendation 送入 proposal queue。
- 能检查已应用 proposal 是否可显式晋升为正式 eval fixture / project guidance / active skill，并记录 promoted status / artifact path。
- 能按 `fixture.task` 对候选 diff 做任务级判分，检查改动文件、diff 片段、验证命令、review/context/goal 证据和约束违规。
- 能把 task-level eval report 记录到 `coding_eval_runs`，让 Improvement Loop / Dashboard 继续消费。
- 能在 `runs.execution.mode="agent"` 时真实调用 `run_chat_engine`，创建用户消息和 chat turn，让模型在临时 repo 的 session working dir 内执行任务；执行结果再进入 Review / Verification / Context / Task scorer。
- 能用本地 mock OpenAI Responses SSE 驱动真实 function-call / tool-result loop，覆盖 `write` 工具真实修改临时 repo、记录 `toolCalls`、产生 candidate diff，再由 task scorer 判定通过。
- 能在 `runs.execution.mode="fixture_patch"` 时用 fixture diff 做无模型回归，明确标记为 deterministic 替身，不冒充真实 agent。
- 能列出内置 Gold Task Pack，并把 20 个 active gold tasks 自动 materialize 成 fixture，批量运行后返回 pack-level pass/fail、skipped/error、总 checks 和逐 case report。
- 能在 Gold Pack 层显式选择 `executionMode="agent"` 并传入 `providers` / `modelChain`，让真实 provider 从 task prompt 生成候选 diff，再由同一套 Review / Verification / Context / Task scorer 判分。
- 能对两份 `GoldTaskPackReport` 做策略效果评估，比较共同 case 的通过率、任务分、关键上下文召回、验证违规、scope creep 和执行失败；候选报告漏掉 baseline case 会被标记为回归风险，候选新增 case 只展示、不用于抬高对比指标。
- 能把 `GoldTaskPackReport` 按 `baselineKind` 持久化到 `coding_eval_pack_runs`，返回 `packRunId`，区分 deterministic mock / fixture patch 与外部真实模型基线。
- 能在 `recordRun=true` 时把 `StrategyEffectReport` 持久化到 `coding_strategy_effect_runs`，返回 `runId`，供 Dashboard 展示 strategy verdict、validation / scope creep delta 和最近策略效果。
- 能通过 release gate 消费 pack history、strategy effect history 与 agent tool-call 指标，给出 `passed` / `failed` / `insufficient_data` 三态发布质量结论。
- 能通过 learning generalization gate 消费 promoted learning、pack history 与 strategy effect history，给出 `passed` / `failed` / `insufficient_data` 三态跨项目泛化结论。
- 能通过 Benchmark Run Center 消费 pack history、baseline kind、latest run、Release Gate 与 Generalization Gate，给出 `passed` / `failed` / `insufficient_data` 三态 benchmark readiness，并在 Dashboard 显示 recent runs / baseline buckets / failed case summary。
- 能计算 `context_precision`、`critical_context_recall`、review finding 数量和 verification command。
- 默认不执行项目验证命令；`agent` execution 会按传入 provider/model 访问模型服务，`fixture_patch` 不访问网络；只有 fixture 显式使用 `workflow.validate()` 时才执行受控验证命令。

## 代码入口

| 位置 | 说明 |
| --- | --- |
| `crates/ha-core/src/coding_eval.rs` | 确定性 fixture harness，供测试和后续报告复用。 |
| `crates/ha-core/tests/coding_eval.rs` | 集成测试入口，加载全部 fixture 并聚合失败信息。 |
| `crates/ha-core/tests/fixtures/coding_eval/*.json` | Phase 3.7-5.2 控制面、执行与任务级 JSON fixture；Phase 5.5+ Gold Task Pack 与 mock tool-call baseline 由 `coding_eval.rs` typed registry / unit fixture 生成。 |
| `run_coding_task_eval_fixture` | Owner-plane Tauri command；输入完整 fixture JSON，返回 `FixtureReport`。 |
| `POST /api/coding-eval/task-fixtures/run` | HTTP owner API；body 为 `{ "fixture": ... }`，返回同一 `FixtureReport`。 |
| `list_coding_eval_gold_tasks` | Owner-plane Tauri command；返回内置 Gold Task Pack 的 case summary。 |
| `run_coding_eval_gold_task_pack` | Owner-plane Tauri command；按筛选批量运行自动化 gold tasks，返回 `GoldTaskPackReport`。 |
| `GET /api/coding-eval/gold-tasks` | HTTP owner API；返回同一 `GoldTaskPackSummary`。 |
| `POST /api/coding-eval/gold-tasks/run` | HTTP owner API；body 为 `{ "input": ... }`，返回同一 `GoldTaskPackReport`。 |
| `evaluate_coding_eval_strategy_effect` | Owner-plane Tauri command；输入 baseline / candidate 两份 pack report，返回 `StrategyEffectReport`。 |
| `POST /api/coding-eval/strategy-effects/evaluate` | HTTP owner API；body 为 `{ "input": ... }`，返回同一 `StrategyEffectReport`。 |
| `evaluate_coding_eval_release_gate` | Owner-plane Tauri command；输入 release gate 阈值，返回 `CodingEvalReleaseGateReport`。 |
| `POST /api/coding-improvement/release-gate/evaluate` | HTTP owner API；body 为 `{ "input": ... }`，返回同一 release gate report。 |
| `get_coding_benchmark_center` | Owner-plane Tauri command；输入 benchmark center scope/window/requirements，返回 `CodingBenchmarkCenterReport`。 |
| `POST /api/coding-benchmark/center` | HTTP owner API；body 为 `{ "input": ... }`，返回同一 benchmark center report。 |

运行方式：

```bash
cargo test -p ha-core --test coding_eval --locked
```

## Fixture 模型

每个 fixture 是一份 JSON，包含四部分：

| 字段 | 说明 |
| --- | --- |
| `repo.files` | baseline 文件，先写入临时 git repo 并提交。 |
| `repo.changes` | baseline 后的工作区改动，形成 local diff。 |
| `task` | Phase 5.1 任务级 eval spec：任务 id、类型、提示词、期望/禁止行为、预期产物、允许验证和成功标准。 |
| `setup` | 可选 goal、task、workflow op，用来模拟长任务控制面状态。 |
| `runs` | 要运行的 agent execution、review、verification plan、workflow、context retrieval、task eval、improvement report 以及 focus paths。 |
| `checks` | 对 execution、review、verification、workflow、context、task、improvement 的确定性断言。 |

首批 fixture：

| Fixture | 覆盖目标 |
| --- | --- |
| `rust_control_plane_context` | Rust diff 触发 review finding、包级 `cargo check` 计划，并在 context 中召回 file / review / verification / goal evidence / task / workflow op。 |
| `docs_sanity_context` | docs-only diff 不应制造 review 噪音，只选择 `git diff --check`。 |
| `focused_scope_excludes_unfocused_files` | 同时存在 Rust + TS diff 时，focused review / verification 只处理指定 Rust 文件，不扫无关前端文件。 |
| `workflow_review_verify_host_apis` | workflow 内调用 `workflow.review()` / `workflow.verify()`，持久化 op、review run、verification plan，并把 Goal evidence 召回到 context。 |
| `repair_loop_blocks_with_evidence` | workflow 内调用 `workflow.repairLoop()`，验证失败且 attempt budget 耗尽后必须 blocked，并把 validation / workflow blocked evidence 召回到 context；同时验证 3.11 trend report 能识别 `repair_loop_exhausted` 并生成 draft `eval_candidate`。 |
| `profiles_ide_context_recall` | `accessibility` / `frontend` profiles 触发定向 finding，并验证 IDE context 候选、review finding 和文件上下文被召回。 |
| `improvement_proposal_to_action` | 失败 eval run 生成 `eval_candidate` proposal，并应用成 `.hope-agent/coding-improvement/eval-candidates/` 下的 reviewable draft artifact。 |
| `improvement_retro_and_promotion` | workflow terminal retro 写入 report，retro recommendation 进入 proposal queue，`eval_candidate` 草稿晋升到正式 coding eval fixture 路径。 |
| `task_level_eval_runner` | 对候选 diff 做任务级判分，覆盖 changed files、required / forbidden diff、验证命令、review/context/goal 证据、eval run 记录和 improvement 消费。 |
| `agent_execution_runner_fixture_patch` | Phase 5.2 execution runner 回归：执行阶段先产出候选 diff，再进入 review / verification / context / task scoring / eval-run recording。 |

## 执行流程

```text
JSON fixture
  -> temp git repo
  -> baseline commit
  -> SessionDB session + working_dir
  -> optional goal/task/workflow seed
  -> optional agent execution or deterministic fixture patch execution
  -> changed working tree
  -> optional production workflow run
  -> production review run
  -> production verification plan
  -> production context retrieval
  -> optional task-level candidate scoring + eval-run recording
  -> optional coding improvement report / proposal generation
  -> deterministic checks + metrics
```

关键约束：

- fixture repo 一律是临时目录，测试结束后销毁。
- `git commit` 只用于制造 baseline；不读取或修改真实工作区。
- verification 只调用 `plan_verification_for_session`，不调用 `run_verification_for_session`，因此不会执行 `cargo`、`pnpm` 或其它项目命令。
- workflow fixture 允许执行 `workflow.js` runtime，但 `workflow.verify()` 仍只生成计划；命令执行只会在 fixture 显式使用 `workflow.validate()` 时发生。
- review 使用生产 diff scanner 和 LSP diagnostic 聚合路径，但 fixture 不启动真实 LSP。
- context retrieval 使用生产聚合器，候选来自真实 DB state 和真实 local diff。
- 没有 `runs.execution` 时，task-level runner 仍按 Phase 5.1 语义评估 fixture 提供的 candidate result，也就是 `repo.changes` 形成的 diff。
- 有 `runs.execution` 时，`prepare_repo` 只写 baseline commit；candidate diff 必须由 execution stage 产生，避免把“已给好答案再判分”误当成真实执行。
- `runs.task.recordEvalRun` 默认 `true`，会写入 `coding_eval_runs(suite='task_level_coding_eval', source_type='coding_task_eval')`；`runs.task.evaluateGoal` 默认 `true`，会先刷新非 terminal goal 的 evaluator 状态。

## Agent Execution Runner

Phase 5.2 新增 `runs.execution`，把“从任务 prompt 到候选结果”的执行阶段接进同一套 eval harness。它有两种模式：

| mode | 说明 |
| --- | --- |
| `agent` | 真实执行模式。Runner 创建 user message + chat turn，调用 `run_chat_engine`，使用 fixture 中传入的 `providers` / `modelChain`，在临时 repo 作为 session working dir 的环境内运行。模型可以通过正常工具链读写文件、触发审批逻辑和产生 transcript。 |
| `fixture_patch` | 确定性回归替身。Runner 在执行阶段写入 `repo.changes`，产出同样的 execution report 和 diff，再进入 review / verification / context / task scorer。它只用于无外部 LLM 的 fixture，不代表真实 agent 成功率。 |

`runs.execution` 输入：

| 字段 | 说明 |
| --- | --- |
| `mode` | `agent` 或 `fixture_patch`，默认 `agent`。 |
| `prompt` | 可选；默认使用 `fixture.task.prompt`。 |
| `agentId` | 可选；默认 `ha-main`。 |
| `providers` / `modelChain` | `agent` 模式必需。HTTP / Tauri owner API 都从 fixture 读取，不隐式读取桌面全局 provider。 |
| `reasoningEffort` / `compactConfig` / `extraSystemContext` | 传入 chat engine 的执行参数；默认 reasoning 为 `none`，post-turn side effects 关闭。 |
| `autoApproveTools` / `deniedTools` | 传入 chat engine 的工具执行约束；危险命令、保护路径、strict approval 等底层红线仍由权限系统兜底。 |

输出 `AgentExecutionEvalReport`：

| 字段 | 说明 |
| --- | --- |
| `mode` / `status` | 执行模式与 `completed` / `failed` 状态。 |
| `prompt` / `agentId` | 本次执行使用的任务提示和 agent。 |
| `turnId` | `agent` 模式创建的 chat turn；`fixture_patch` 为 `null`。 |
| `response` / `error` | chat engine response 或失败原因。执行失败不会让 API 直接 400，而是作为 eval report 进入判分链路。 |
| `modelUsed` | 成功的模型引用。 |
| `toolCalls` | 本次执行实际落库的 tool message 名称列表，用于断言模型确实调用了预期工具，而不是只描述改动。 |
| `changedFiles` / `diffBytes` | 执行结束后的 git diff 摘要。 |

`checks.execution` 可断言 mode、status、是否必须有 turn、必须/禁止改动文件、最少 tool call 数、必需 tool call 名称、response / error 片段。`FixtureReport.metrics` 同步暴露 `execution_status`、`execution_mode`、`execution_changed_files`、`execution_tool_calls`。

Phase 5.6 的稳定 mock baseline 使用本地 `wiremock` OpenAI Responses SSE：第一轮返回 `function_call(write, { path, content })`，真实 tool loop 写入临时 repo；第二轮返回最终文本。该测试不访问外部模型服务，但覆盖了真实 chat engine、tool dispatch、session working dir、messages.tool_name 记录、diff snapshot 和 task-level scorer。为保证隔离 DB 与生产 DB 语义一致，`ChatEngineParams.session_db` 会绑定到 `AssistantAgent`，agent-side session meta lookup 优先使用本轮 DB；绑定 DB 缺失 session 行时仍按 incognito fail-closed 处理，不 fallback 到全局 DB。

## Gold Task Pack Runner

Phase 5.3 新增 Gold Task Pack v1，把 Phase 0 文档里的 active gold tasks 变成可批量运行的结构化 registry；Phase 5.5 已把首批 20 个任务全部接进可回放链路。Pack 覆盖 bugfix、test_gap、frontend_ts、rust_logic、review、repo_navigation 六类任务，既有 docs/design-only case，也有 Rust / TS / i18n 多文件 fixture_patch case：

| 范围 | 类型 | 主题 |
| --- | --- | --- |
| `CE-BUG-001..005` | `bugfix` | tool_search parsing、Plan execution guidance、preview-by-path 鉴权、async zero 语义、Knowledge owner/agent 平面。 |
| `CE-TEST-001..004` | `test_gap` | Plan 状态机非法转移、ToolDefinition visibility、incognito preview、workflow repair-loop 停机。 |
| `CE-FE-001..004` | `frontend_ts` | Workspace copy、loop/mode entry、FileKind fallback、PlanPanel i18n read-only copy。 |
| `CE-RUST-001..003` | `rust_logic` | ToolDefinition safety metadata、WorkflowRun trace 边界、validation selector。 |
| `CE-REV-001..002` | `review` | seeded diff review、review verifier tri-state。 |
| `CE-NAV-001..002` | `repo_navigation` | workflow module boundaries、LSP/ACP context boundaries。 |

`list_coding_eval_gold_tasks` / `GET /api/coding-eval/gold-tasks` 返回 `GoldTaskPackSummary`：

- `packId` / `sourceDoc`
- `totalCases` / `activeCases` / `automatedCases`（Phase 5.5 为 20 / 20 / 20）
- 每个 case 的 `id`、`taskType`、`status`、`automationStatus`、`fixtureName`、`expectedArtifacts`、`likelyFiles`、`allowedValidation`、`successCriteria`

`run_coding_eval_gold_task_pack` / `POST /api/coding-eval/gold-tasks/run` 输入 `GoldTaskPackRunInput`：

| 字段 | 说明 |
| --- | --- |
| `ids` / `statuses` / `taskTypes` | 可选筛选；默认运行所有自动化 active cases。 |
| `includeUnautomated` | 是否把未自动化 case 作为 `skipped` 返回；显式指定 `ids` 时也会返回 skipped，避免静默吞掉任务。 |
| `maxTasks` | 可选上限，用于本地 smoke 或分批运行。 |
| `executionMode` | `fixture_patch` 或 `agent`。默认 `fixture_patch`；如果传入 provider/model 或 `baselineKind="external_model"` / `mock_provider`，默认提升为 `agent`。 |
| `providers` / `modelChain` | `executionMode="agent"` 必需。owner API 不隐式读取桌面全局 provider，调用方必须显式传入受控 provider 配置。 |
| `compactConfig` / `reasoningEffort` / `extraSystemContext` / `deniedTools` | 透传给 agent execution runner 的可选执行配置。 |
| `autoApproveTools` | 是否在 eval runner 中自动批准工具调用；外部基线 smoke 通常需要显式打开，避免审批挂起。 |
| `recordEvalRuns` | 是否写入 `coding_eval_runs`，默认 `true`。 |
| `recordPackRun` | 是否写入 `coding_eval_pack_runs`，默认 `true`。 |
| `label` | 可选展示标签，例如 `baseline`、`candidate`、`external smoke`。 |
| `baselineKind` | 基线类型。`fixture_patch` 默认归一为 `deterministic_mock`；`agent` 默认记录为 `external_model`。`external_model` / `mock_provider` 必须走 `executionMode="agent"`；`agent` 不能记录为 `deterministic_mock`。 |
| `sessionId` / `projectId` | 可选归属 scope；无 session 时仍可记录全局 / 项目级 pack run。 |
| `sourceType` / `sourceId` | 可选审计来源，默认 `gold_task_pack` / `packId`。 |
| `evaluateGoal` | 是否在 task scoring 前刷新 Goal evaluator，默认 `true`。 |

Runner 会把每个自动化 case materialize 成一份普通 `CodingEvalFixture`：

```text
gold task case
  -> generated fixture baseline file
  -> runs.execution.mode="fixture_patch" | "agent"
  -> review / verification / context / task scoring
  -> GoldTaskPackReport.case.report
```

默认执行模式是 `fixture_patch`，因此不会访问外部模型；它验证的是 task schema、候选 diff、Review / Verification / Context / Goal / Task scorer 的端到端胶水。Phase 5.9 起，调用方可显式传 `executionMode="agent"`、`providers`、`modelChain` 和可选 `autoApproveTools` 跑外部真实模型基线：runner 会从每个 gold task 的 prompt 创建真实 chat turn，模型必须通过工具产生 diff，随后进入同一 scorer。Phase 5.7 会把每次 pack run 作为可审计历史保存；`baselineKind` 必须标清 deterministic / mock / external，Dashboard 与 release gate 不把确定性替身数字冒充成真实模型能力。`baselineKind="external_model"` 如果没有 agent execution 配置会 fail-fast，不能只改标签伪装外部基线。

## Strategy Effect Evaluator

Phase 5.4 新增策略效果评估器，用来回答：

> 这次 workflow policy、skill/guidance、tool contract 或 prompt 策略改动，是真的提升了任务质量，还是只改变了表面指标？

输入 `StrategyEffectEvalInput`：

| 字段 | 说明 |
| --- | --- |
| `strategyType` | 可选策略类型标签，例如 `workflow_policy`、`skill_guidance`、`tool_contract`。 |
| `baselineLabel` / `candidateLabel` | 可选展示标签，默认 `baseline` / `candidate`。 |
| `recordRun` | 是否把本次 report 写入 `coding_strategy_effect_runs`，默认 `false`；纯对比仍可无副作用运行。 |
| `baselinePackRunId` / `candidatePackRunId` | 可选关联 pack history；未显式传入时会读取报告上的 `packRunId`。 |
| `sessionId` / `projectId` | 可选归属 scope；无 session 时仍可记录全局 / 项目级 strategy effect。 |
| `sourceType` / `sourceId` | 可选审计来源，默认 `strategy_effect`。 |
| `baseline` | 改动前的一份 `GoldTaskPackReport`。 |
| `candidate` | 改动后的一份 `GoldTaskPackReport`。 |

输出 `StrategyEffectReport`：

| 字段 | 说明 |
| --- | --- |
| `runId` | `recordRun=true` 时返回的持久化 run id；纯函数评估时为空。 |
| `verdict` | `improved` / `regressed` / `mixed` / `unchanged` / `inconclusive`。 |
| `comparedCases` | 两份报告中共同 case 的数量。所有聚合指标只基于共同 case。 |
| `baselineOnlyCases` | baseline 有、candidate 缺失的 case；这类缺失会进入 `regressions`，避免候选报告通过漏跑任务抬高指标。 |
| `candidateOnlyCases` | candidate 新增的 case；只展示，不参与共同 case 聚合。 |
| `summary` | pass rate、average task score、context recall、validation violations、scope creep、execution failures 及 delta。 |
| `dimensions` | 每个维度的方向、baseline/candidate 数值、delta 与 verdict；`passRate` / `averageTaskScore` / `contextRecall` 越高越好，`validationViolations` / `scopeCreep` / `executionFailures` 越低越好。 |
| `cases` | 每个共同 case 的逐项对比，包含 status、outcome、score、context recall、违规数、scope creep、执行失败和 notes。 |
| `regressions` / `improvements` | 人可读的回归 / 改进摘要，用于 review 或 Dashboard 展示。 |

判定规则：

- 只比较共同 case，防止 candidate 通过增加简单任务稀释失败。
- candidate 漏掉 baseline case 是回归风险；即使没有共同 case，也会给出 `regressed`。
- case-level `mixed` 会同时进入 regressions 和 improvements，要求人工看具体 notes。
- `evaluate_strategy_effect()` 保持纯函数：不读写 DB、不跑模型、不执行项目命令。
- Tauri / HTTP owner API 走 `evaluate_strategy_effect_with_recording()`：默认仍无副作用；只有 `recordRun=true` 时写入 `coding_strategy_effect_runs`，并把 `runId` 返回给调用方。

## Release Gate

Phase 5.8 新增 release gate，用来回答：

> 最近一段时间的 gold pack / strategy effect / agent tool-call 历史，是否足以支持发布或推广策略改动？

输入 `CodingEvalReleaseGateInput`：

| 字段 | 说明 |
| --- | --- |
| `sessionId` / `projectId` | 可选 scope。传 session 且 session 属于项目时自动按项目聚合；无 scope 时按全局聚合。无痕 session 直接拒绝。 |
| `windowDays` | 时间窗口，默认 30 天，范围 1-180 天。 |
| `minPackRuns` | 至少需要多少次 pack run，默认 1。 |
| `minStrategyEffectRuns` | 至少需要多少次 strategy effect run，默认 0；发布策略可显式要求。 |
| `minPackPassRate` | pack-level 成功率阈值，默认 1.0。 |
| `requireExternalModelPack` | 是否要求窗口内至少有一次 `baselineKind="external_model"` 的 pack run，默认 `false`。 |
| `maxRegressedStrategyEffects` / `maxMixedStrategyEffects` | 允许的回归 / mixed strategy effect 数，默认均为 0。 |
| `maxMissingToolCallRuns` | 允许 agent 模式 task eval 出现 `toolCalls=[]` 的次数，默认 0。 |
| `maxValidationViolationDelta` / `maxScopeCreepDelta` | 允许 strategy effect 聚合后的 validation / scope creep 增量，默认均为 0。 |

输出 `CodingEvalReleaseGateReport`：

| 字段 | 说明 |
| --- | --- |
| `status` | `passed` / `failed` / `insufficient_data`。有明确质量回归时为 `failed`；缺少要求的样本或外部基线时为 `insufficient_data`；全部通过才是 `passed`。 |
| `thresholds` | 归一化后的阈值，便于 CI / UI 记录当时的发布标准。 |
| `summary` | pack run、baseline kind、case/check 汇总、strategy verdict/delta、missing tool-call run 计数。 |
| `checks` | 每条门禁的 `name`、`status`、`severity`、`expected`、`actual`、`detail`。 |

Release gate 是 owner-plane 只读能力：不跑模型、不执行项目命令、不生成 proposal、不修改历史记录。它只消费 `coding_eval_pack_runs`、`coding_strategy_effect_runs` 和 `coding_eval_runs(source_type='coding_task_eval')`，并沿用 Dashboard 的 durable 数据过滤原则：无痕、cron、subagent session 不参与发布质量判断；sessionless eval 只在全局 / 项目 scope 中按显式归属字段计入。

## Task-level Eval Runner

Phase 5.1 新增任务级 runner，用来把人工 gold task 的 schema 与确定性控制面 harness 接起来。Phase 5.2 之后，它既可以评估 fixture 已给出的候选结果，也可以评估 `runs.execution` 真实 agent / fixture patch 产生的候选结果。

输入：

| 字段 | 说明 |
| --- | --- |
| `fixture.task` | 任务定义：`id`、`taskType`、`title`、`prompt`、`expectedBehavior`、`forbiddenBehavior`、`expectedArtifacts`、`allowedValidation`、`successCriteria`。 |
| `runs.task.recordEvalRun` | 是否把任务报告写入 `coding_eval_runs`，默认 `true`。 |
| `runs.task.evaluateGoal` | 是否在判分前刷新 Goal evaluator，默认 `true`。 |
| `checks.task` | 判分断言：期望 outcome / 最低分、必须/禁止改动文件、必须/禁止 diff 片段、必须/禁止验证命令、最大改动文件数、是否要求 review / verification / context / goal evaluation、必召回上下文。 |

输出 `CodingTaskEvalReport`：

| 字段 | 说明 |
| --- | --- |
| `outcome` | `pass` / `partial` / `fail` / `blocked`。critical check 失败直接 `fail`；无 check 为 `blocked`。 |
| `score` | 通过 check 数 / 总 check 数，保留三位小数。 |
| `failureCategory` | 第一条失败 check 的 category，例如 `implementation_bug`、`validation_gap`、`scope_creep`、`context_miss`。 |
| `diff` | changed files、insertions、deletions、diff bytes。 |
| `validation` | Smart Verification 计划出的命令、命令数、allowed/disallowed 命令。 |
| `review` | 是否请求 review、finding 数、blocking finding 数。 |
| `context` | 是否请求 Context Retrieval、候选数、required context recall。 |
| `goal` | Goal 是否由 task runner 触发 evaluation、Goal state 与 evidence relation 快照。 |
| `checks` | 每条任务级 check 的 name、passed、detail、category、severity。 |

如果 `runs.execution` 存在，task report 会自动加入 `execution.completed` critical check；执行失败会让 task outcome 失败，不会被其它宽松 check 掩盖。

task-level report 会同步进入 `FixtureReport.task` 和 `FixtureReport.metrics`：

- `task_outcome`
- `task_score`
- `task_failure_category`
- `task_changed_files`
- `task_constraint_violations`

写入 `coding_eval_runs` 时，status 映射为：

| Task outcome | Eval status |
| --- | --- |
| `pass` | `passed` |
| `blocked` | `blocked` |
| `partial` / `fail` | `failed` |

## 指标

Harness 输出 `FixtureReport`：

| 指标 | 说明 |
| --- | --- |
| `context_precision` | critical candidate 命中数 / 返回候选数，用来发现推荐列表是否过散。 |
| `critical_context_recall` | critical candidate 命中数 / fixture 要求的 critical 数，用来发现关键控制面信号是否丢失。 |
| `review_findings` | review run 产生的 finding 数量。 |
| `review` checks | expected profiles、IDE context stats、finding title/category/file 断言。 |
| `verification_commands` | verification plan 选择的命令列表。 |
| `workflow` checks | workflow run 状态、op 类型、输出、Goal evidence relation。 |
| `execution` checks | execution mode/status、turn、response/error、tool calls、执行后 changed files。 |
| `task` checks | task outcome、score、changed files、diff fragment、validation commands、review/context/goal 要求、scope / policy 违规数量。 |
| `improvement` checks | trend scope、failure category、proposal kind/status、eval success rate、repair loop blocked、retro/recommendation 数、proposal apply/promote status、artifact target 断言。 |

测试失败时会输出 fixture 名、失败 check、候选或命令摘要，方便定位是 diff scanner、review、verification selector、goal evidence 还是 context ranking 出问题。

## 与人工 Coding Eval 的关系

Phase 0 的 `docs/roadmap/coding-eval*.md` 仍然负责真实任务质量：

- 任务是否真实。
- Agent 是否理解需求。
- 是否做出正确代码改动。
- 是否如实报告验证结果。
- 是否遵守项目规则。

Phase 3.7/3.8/3.9/3.10/3.11/4.x 自动化层负责控制面健康：

- focused action 是否收窄。
- 最小验证选择是否稳定。
- review finding 是否能进入 goal/context。
- goal/task/workflow evidence 是否能被下一步推荐系统看见。
- trend report 是否能解释失败模式并只生成 proposal 草案。
- terminal workflow retro 是否能稳定写入 report，并只作为 proposal 候选来源。
- draft promotion 是否需要显式触发、可回归、且目标冲突 fail-closed。
- 新功能是否破坏已有 coding control-plane glue。
- workflow 内的 review / verification 是否和 owner API、Goal evidence、Context Retrieval 保持同一语义。
- workflow repair loop 是否在预算耗尽时 blocked，而不是 failed 或伪 completed，并且 evidence 是否能被下一步召回。
- review profiles 是否真的改变 review surface，而不是只停留在 UI 文案。
- IDE / ACP 当前上下文是否能进入推荐上下文和 review stats，且没有 IDE 信号时仍可降级。

Phase 5.1 在两者之间补了一层：它把“某个候选结果是否满足任务级成功标准”变成可回归的 deterministic report。Phase 5.2 再补上真实 agent execution runner，让 owner API 可以从 task prompt 开始跑一轮 agent，再把产物交给同一个 scorer。Phase 5.3 开始把 active gold tasks 结构化成可批量回放的 pack。Phase 5.4 再把两份 pack report 的策略效果对比做成纯函数 owner API。Phase 5.5 把 20 个任务全量自动化，Phase 5.6 把 mock tool-call 写文件基线接到真实工具循环，Phase 5.7 把 pack / strategy history 接入 Dashboard，Phase 5.8 把持久化 history 接入 release gate，Phase 5.9 把 Gold Pack 接入受控外部模型基线，Phase 5.10 把跨项目学习泛化接入 owner-plane gate，Phase 6.1 把这些 durable history 接成 Benchmark Run Center。当前仍不等同于完整大规模 benchmark：它证明控制面与学习闭环可审计、可观察、可运行，真实大规模任务质量仍应由更高层 benchmark 持续跟踪。

## Improvement Loop 覆盖

Fixture 可声明：

```json
{
  "runs": {
    "improvement": {
      "generateProposals": true,
      "seedEvalRuns": [
        {
          "suite": "coding_control_plane",
          "name": "repair_loop_blocks_with_evidence",
          "status": "failed",
          "metrics": { "criticalContextRecall": 1.0 }
        }
      ]
    }
  },
  "checks": {
    "improvement": {
      "expectedScope": "session",
      "expectedFailureCategories": ["repair_loop_exhausted"],
      "expectedProposalKinds": ["eval_candidate"],
      "expectDraftOnly": true
    }
  }
}
```

这层不会把 proposal 自动写进项目规则或 skill；它只验证 `coding_improvement` 聚合器是否能稳定消费 durable control-plane 数据。Phase 4.2 允许 fixture 显式声明 `promoteAppliedProposal`，用于验证 promotion 路径本身，但仍然是 owner-plane 的确定性动作，不会由 proposal generation 或 apply 隐式触发。Phase 5.1-6.1 的 task-level report / execution metrics / pack history / strategy effect history / release gate / external model baseline / learning generalization gate / benchmark center 会进入 Improvement Loop 与 Dashboard，因此可以把任务级失败、执行失败、tool-call 缺失、scope creep、策略回归或单项目过拟合变成可审计趋势与质量判断。

两者互补：人工 eval 衡量完整 coding 能力，确定性 eval 保护控制面底座。

## 后续扩展

后续增强应优先保持 fixture 可解释、运行快、无模型依赖：

- 增加 LSP diagnostics seeded fixture。
- 增加 Goal final audit / blocked repair fixture。
- 增加 context ranking 回归样本，记录 precision / recall 趋势。
- 增加可选 HTML/JSON 报告，但不要把报告生成变成测试必需条件。
- 增强跨项目学习泛化报告的项目对比维度，例如按 artifact、proposal kind、provider baseline 和 failure mode 分层展示。

LLM reviewer 的真实模型质量、真实命令执行和完整任务通过率应进入更高层 eval，不应污染确定性控制面 fixture。当前 harness 固定 `deep` 以外的 deterministic profiles、IDE context 数据流、task scorer，以及可选的 agent execution owner path。

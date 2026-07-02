# Coding Eval 控制面评测

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 5.1 已实现。本文只记录已经落地的自动化评测层；人工 gold task 体系仍见 [Coding Eval 体系方案](../roadmap/coding-eval.md)。

## 目标

Coding Eval 控制面评测用于回答一个更底层但非常关键的问题：

> Review、Smart Verification、Context Retrieval、Goal、Task、Workflow 这些 coding 控制面，是否能在同一个真实 session 中稳定协同？

它不是完整的端到端 agent benchmark，也不调用 LLM。Phase 3.7 先把“可确定性回归”的能力钉住；Phase 3.8 继续把 workflow 内的 review / verification host API 纳入同一套回归；Phase 3.9 把 bounded repair loop 的停机与证据链纳入回归；Phase 3.10 把 profile-specific review 与 IDE context recall 纳入回归；Phase 3.11 把 trend report / proposal 队列纳入回归；Phase 4.1 把 proposal-to-action 纳入回归；Phase 4.2 把 workflow retro 与 draft promotion 纳入回归；Phase 5.1 把 task-level candidate result scoring 纳入回归：

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
- 能计算 `context_precision`、`critical_context_recall`、review finding 数量和 verification command。
- 默认不执行项目验证命令，不访问网络，不依赖外部模型；只有 fixture 显式使用 `workflow.validate()` 时才执行受控验证命令。

## 代码入口

| 位置 | 说明 |
| --- | --- |
| `crates/ha-core/src/coding_eval.rs` | 确定性 fixture harness，供测试和后续报告复用。 |
| `crates/ha-core/tests/coding_eval.rs` | 集成测试入口，加载全部 fixture 并聚合失败信息。 |
| `crates/ha-core/tests/fixtures/coding_eval/*.json` | Phase 3.7-5.1 控制面与任务级 fixture。 |
| `run_coding_task_eval_fixture` | Owner-plane Tauri command；输入完整 fixture JSON，返回 `FixtureReport`。 |
| `POST /api/coding-eval/task-fixtures/run` | HTTP owner API；body 为 `{ "fixture": ... }`，返回同一 `FixtureReport`。 |

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
| `runs` | 要运行的 review、verification plan、workflow、context retrieval、task eval、improvement report 以及 focus paths。 |
| `checks` | 对 review、verification、workflow、context、task、improvement 的确定性断言。 |

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

## 执行流程

```text
JSON fixture
  -> temp git repo
  -> baseline commit
  -> changed working tree
  -> SessionDB session + working_dir
  -> optional goal/task/workflow seed
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
- task-level runner 评估的是 fixture 提供的 candidate result，也就是 `repo.changes` 形成的 diff；它不会让真实 Agent 自动执行 prompt。
- `runs.task.recordEvalRun` 默认 `true`，会写入 `coding_eval_runs(suite='task_level_coding_eval', source_type='coding_task_eval')`；`runs.task.evaluateGoal` 默认 `true`，会先刷新非 terminal goal 的 evaluator 状态。

## Task-level Eval Runner

Phase 5.1 新增任务级 runner，用来把人工 gold task 的 schema 与确定性控制面 harness 接起来。它的边界是“评估一个已经产生的候选结果”，不是“驱动模型完成任务”。

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

Phase 5.1 在两者之间补了一层：它把“某个候选结果是否满足任务级成功标准”变成可回归的 deterministic report。它仍然不让模型端到端做题，所以不能替代人工/真实模型 eval；但它已经能验证 diff 质量、约束遵守、验证选择、review/context/goal 证据是否足以支撑一次 task-level 通过。

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

这层仍然不调用 LLM，也不会把 proposal 自动写进项目规则或 skill；它只验证 `coding_improvement` 聚合器是否能稳定消费 durable control-plane 数据。Phase 4.2 允许 fixture 显式声明 `promoteAppliedProposal`，用于验证 promotion 路径本身，但仍然是 owner-plane 的确定性动作，不会由 proposal generation 或 apply 隐式触发。Phase 5.1 的 task-level report 也写入同一 eval run 表，因此 Improvement Loop 可以把任务级失败按既有 failure taxonomy 继续生成候选。

两者互补：人工 eval 衡量完整 coding 能力，确定性 eval 保护控制面底座。

## 后续扩展

后续增强应优先保持 fixture 可解释、运行快、无模型依赖：

- 增加 LSP diagnostics seeded fixture。
- 增加 Goal final audit / blocked repair fixture。
- 增加 context ranking 回归样本，记录 precision / recall 趋势。
- 增加可选 HTML/JSON 报告，但不要把报告生成变成测试必需条件。
- 增加真实 Agent execution runner：在隔离 worktree 内让 Agent 从 prompt 开始执行，再把产物交给 Phase 5.1 task-level scorer。
- 扩展 gold task pack，把 Phase 0 的 20 个任务逐步转成可自动准备、可判分、可回放的 fixture。

LLM reviewer 的真实模型质量、真实命令执行和完整任务通过率应进入更高层 eval，不应污染这个确定性控制面 harness。当前 harness 只固定 `deep` 以外的 deterministic profiles，以及 IDE context 数据流。

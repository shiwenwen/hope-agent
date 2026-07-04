# Domain Eval 与 Quality Gate 控制平面

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 7.8 已实现。本文记录 `ha-core::domain_eval` 的最终技术事实：通用领域 eval task registry、promoted domain eval case 导入、user/project calibration 与人工复核记录、deterministic trace scoring、trace fixture runner、`domain_eval_runs` history、Domain Quality Gate、owner API 与 Dashboard 通用质量区块。

## 目标

Domain Eval 把非 coding 场景的质量判断从“感觉不错”变成可审计 run：

- Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation 各有 3 个内置 eval task，总计 15 个。
- 每个 task 都定义输入 prompt、允许工具、required evidence、success criteria、禁止行为和 calibration 记录。
- 评分读取 Goal、Workflow、Domain Evidence、Domain Quality trace，不读取或混入 coding benchmark 表。
- Quality Gate 聚合 domain eval run、domain quality run/check 和 evidence coverage，输出 `passed` / `failed` / `insufficient_data`。

这套控制面只证明通用领域任务质量，不代表 coding 能力；coding benchmark 仍由 [Coding Eval 控制面评测](coding-eval.md) 和 [Coding Improvement Loop](coding-improvement-loop.md) 承载。

## 数据模型

`SessionDB::open()` 调用 `domain_eval::ensure_tables()` 创建：

| 表 | 说明 |
| --- | --- |
| `domain_eval_runs` | 一次通用领域 eval 评分结果，字段包括 session/project、task id/version、domain、label、status、score、report JSON、source quality run、created_at。 |
| `domain_eval_tasks` | 从已晋升 `domain_eval_case` 学习产物导入的自定义 eval task。字段包括 task id/version、project、source proposal、source path、task JSON、imported_at、updated_at。 |
| `domain_eval_calibrations` | user/project scope 的人工校准与复核记录。字段包括 task id/version、domain、project、scope、reviewer、verdict、note、source eval run、created_at。 |

索引：

- `idx_domain_eval_runs_scope(project_id, session_id, domain, created_at DESC)`
- `idx_domain_eval_runs_task(task_id, created_at DESC)`
- `idx_domain_eval_runs_status(status, created_at DESC)`
- `idx_domain_eval_tasks_domain_status(status, json_extract(task_json, '$.domain'))`
- `idx_domain_eval_tasks_source(source_type, source_id)`
- `idx_domain_eval_calibrations_task(task_id, task_version, project_id, created_at DESC)`
- `idx_domain_eval_calibrations_domain(domain, project_id, created_at DESC)`
- `idx_domain_eval_calibrations_source_run(source_run_id)`

## Task Registry

内置 15 个 task：

| Domain | Task |
| --- | --- |
| `research` | `research-source-backed-brief`、`research-technical-decision`、`research-conflict-comparison` |
| `writing` | `writing-decision-memo`、`writing-prd-brief`、`writing-executive-summary` |
| `data_analysis` | `data-kpi-readout`、`data-metric-diagnostic`、`data-dashboard-qa` |
| `meeting_prep` | `meeting-prep-brief`、`meeting-agenda-risk-review`、`meeting-follow-up-plan` |
| `knowledge_curation` | `knowledge-topic-index`、`knowledge-source-synthesis`、`knowledge-vault-cleanup` |

Task schema：

| 字段 | 说明 |
| --- | --- |
| `id` / `version` | task 稳定身份；内置版本为 `1.0.0`。 |
| `domain` / `taskType` | 领域与任务类型。 |
| `input.prompt` | 半确定性 trace fixture 的任务输入。 |
| `allowedTools` | 允许工具提示；不自动授权工具。 |
| `requiredEvidence` | evidence type、最小数量、metadata key 要求。 |
| `successCriteria` | 评分者可读成功标准。 |
| `prohibitedActions` | 未经批准不得执行的 send/share/publish/external update/delete 等动作。 |
| `calibration` | built-in / proposal / user / project calibration 记录，包含 reviewer、verdict、scope、note 与可选 source run。 |

此外，`import_domain_eval_case(input)` 可以把已晋升的 `coding_improvement_proposals(kind='domain_eval_case', status='promoted')` 导入 `domain_eval_tasks`：

- 只接受 promotion record 中 `promoted=true` 且存在 JSON artifact 的 proposal。
- JSON artifact 会被规范化为 `DomainEvalTask`：读取 domain、name/title、input prompt、allowed tools、required evidence、success criteria、prohibited actions 和 calibration notes。
- 生成的 task id 采用 `learned-{domain}-{name}`，version 默认 `1.0.0`。
- 重复导入默认幂等返回 `imported=false`；`overwrite=true` 才更新既有 task JSON 和 source metadata。
- `list_domain_eval_tasks` 会合并内置 task 与 active imported task；`run_domain_eval_task` 先查内置 task，再查 imported task。
- 这是 owner-plane 显式动作，不由模型自动执行；GUI 在 Coding Trend proposal 列表中仅对已晋升的领域评测候选显示「导入评测」。

## Calibration 与人工复核

`record_domain_eval_calibration(input)` 用于显式记录某个 eval task 的人工校准结论：

- `verdict` 白名单：`approved`、`needs_calibration`、`needs_revision`、`rejected`、`stale`。
- 不传 `projectId` 时为 user scope；传 `projectId` 时为 project scope。
- 可选 `sourceRunId` 会绑定一次具体 `domain_eval_runs`，并校验 run 的 task id 与输入 task 一致。
- 同一个 `sourceRunId + reviewer + scope + projectId` 重复记录幂等返回已有 calibration，避免 Dashboard 重复点击刷出多条记录。
- `list_domain_eval_tasks(projectId?)` 会把 user scope 与对应 project scope calibration 追加到 task manifest。
- `run_domain_eval_task` 会根据 session project 把相关 calibration 写入 `DomainEvalReport.task.calibration`，让历史 run 能说明它执行时看到的校准上下文。
- Calibration 是人工证据，不会自动修改 scorer 权重，不会把 failed run 改成 passed，也不会绕过 Quality Gate 的 eval / quality 样本要求。

## Run Scoring

`run_domain_eval_task(input)` 执行同步确定性评分：

1. 读取 session，incognito 直接拒绝。
2. 读取内置或导入的 task manifest，并附加 user/project calibration。
3. 读取同 session/domain 的 Domain Evidence。
4. 读取显式 `sourceQualityRunId` 或最近同 domain 的 Domain Quality snapshot。
5. 读取 active/latest Goal snapshot 与其 workflow trace。
6. 生成 `DomainEvalReport` 并写入 `domain_eval_runs`。

评分检查：

| Check | 失败模式 |
| --- | --- |
| `evidence_completeness` | required evidence 缺失或 metadata key 不足。 |
| `citation_quality` | Research / Knowledge 没有来源，或 source 缺少 retrieved/published/date metadata。 |
| `data_quality` | Data Analysis 缺少 dataset / metric / denominator / sampleSize 等质量证据。 |
| `approval_safety` | Domain Quality 已判定高风险动作 `needs_user`，或 task 明确要求 `user_decision` / `message_draft_approved` 但缺少证据。 |
| `completion_criteria_match` | Goal 缺少完成标准，或最新 Domain Quality 未通过。 |
| `workflow_trace` | Goal 没有关联 workflow run 时标记 `insufficient_data`。 |

Run status：

- `failed`：任一检查失败。
- `insufficient_data`：无 failed，但存在缺少 trace/evidence 的不充分项。
- `passed`：无 failed/insufficient，且加权 score 达到默认阈值。

## Fixture Runner

`run_domain_eval_fixture(input)` 是 Phase 7.8 的第一版半确定性 runner，用于把一份 fixture materialize 成真实控制面 trace，再交给同一个 scorer 判分：

1. 创建真实 session。
2. 创建 Goal，objective / completion criteria 默认来自 task。
3. 写入 fixture evidence，进入 `domain_evidence`。
4. 默认创建一个 `origin='domain_eval_fixture'` 的 WorkflowRun。
5. 默认运行 Domain Quality，得到 `domain_quality_runs/checks` snapshot。
6. 调用 `run_domain_eval_task`，把 scorer 输出写入 `domain_eval_runs`。
7. 按 fixture `checks` 输出 runner 自身通过/失败状态。

当前只支持 `executionMode='trace_fixture'`。如果 fixture 请求 `agent` 或其它模式，runner 会创建 session 后返回 failed report，并写明“agent-backed domain fixture execution 尚未接 provider/model execution”。这条 fail-fast 是红线：不能把 deterministic trace fixture 伪装成真实模型执行能力。

Trace fixture runner 当前是 owner API / 回归测试能力，不挂到 Dashboard quality gate 的普通按钮上。原因是 fixture 会写入真实 `domain_eval_runs` 与 `domain_quality_runs`，如果把合成 smoke 样本当成日常质量历史展示，会污染用户对真实通用任务质量的判断。后续若要产品化展示，应单独做 fixture/smoke run 面板和来源过滤。

## Quality Gate

`evaluate_domain_quality_gate(input)` 只读历史，不调用 LLM、不运行工具、不生成 proposal。

Scope：

- `sessionId`：只看当前 session；incognito 拒绝。
- `projectId`：看项目内非 incognito session。
- 未传 scope：全局非 incognito。
- `domain` 可进一步过滤。

默认阈值：

| Threshold | 默认 |
| --- | --- |
| `minEvalRuns` | 1 |
| `minPassRate` | 1.0 |
| `minAverageScore` | 0.8 |
| `minQualityRuns` | 1 |
| `maxBlockedQualityRuns` | 0 |
| `minDomainCoverage` | 1 |
| `requireApprovalSafety` | false；Dashboard 调用设为 true |

Gate checks：

| Check | 说明 |
| --- | --- |
| `domain_eval_runs` | domain eval 样本数是否足够。 |
| `domain_eval_pass_rate` | passed run 比例是否达标。 |
| `domain_eval_average_score` | 平均 score 是否达标。 |
| `domain_quality_runs` | 是否有 Domain Quality run/check history。 |
| `blocked_domain_quality` | blocked / failed / needs_user quality run 是否超限。 |
| `domain_coverage` | 覆盖领域数是否达标。 |
| `approval_safety` | 可选；approval blocker 必须为 0。 |

Gate status：

- 任一 check `failed` -> `failed`
- 无 failed 但有 `insufficient_data` -> `insufficient_data`
- 全部 passed -> `passed`

## Owner API

Tauri / HTTP / transport 均已注册：

| Tauri Command | HTTP | 说明 |
| --- | --- | --- |
| `list_domain_eval_tasks` | `POST /api/domain-eval/tasks` | 列出内置通用 eval tasks，可按 domain 过滤。 |
| `run_domain_eval_task` | `POST /api/domain-eval/runs/run` | 对一个 session 运行确定性 domain eval 并持久化。 |
| `run_domain_eval_fixture` | `POST /api/domain-eval/fixtures/run` | 运行 trace fixture，创建 session/goal/evidence/workflow/quality trace 后用同一 scorer 判分。 |
| `import_domain_eval_case` | `POST /api/domain-eval/cases/import` | 把已晋升的 `domain_eval_case` proposal 导入 active task registry。 |
| `record_domain_eval_calibration` | `POST /api/domain-eval/calibrations/record` | 记录 task 的 user/project 人工校准或一次 eval run 的复核结论。 |
| `list_domain_eval_calibrations` | `POST /api/domain-eval/calibrations` | 查询 calibration history，可按 task/domain/project 过滤。 |
| `list_domain_eval_runs` | `POST /api/domain-eval/runs` | 列出 domain eval run history。 |
| `evaluate_domain_quality_gate` | `POST /api/domain-quality-gate/evaluate` | 计算通用领域 quality gate。 |

## Dashboard 交互

Dashboard Learning Tab 新增「General domain quality」区块：

- 展示 gate 三态。
- 展示 eval pass rate、average score、quality blockers、domain coverage。
- 展示 attention checks。
- 展示最近 domain eval run。
- 展示已校准 task 数；最近 eval run 支持点击「Mark reviewed」记录人工复核 calibration。
- 与 Release Gate / Continuous Benchmark Gate 分开展示，不生成综合分。

## 红线

- 不混排 coding benchmark：`domain_eval_runs` 与 `coding_eval_runs` 物理分表。
- 不伪造通用能力：没有 domain eval run 或 quality run 时 gate 必须 `insufficient_data`。
- 不越权运行工具：eval 只读既有 trace/evidence，不调用连接器，不发送、不发布、不改外部系统。
- 不隐式学习上线：`domain_eval_case` 必须先走 proposal preview / apply draft / explicit promotion，再由用户显式导入 task registry。
- 不让模型自校准：calibration 只暴露 owner API / GUI，不提供 agent tool 面。
- 不伪造 agent 能力：`run_domain_eval_fixture` 当前只支持 `trace_fixture`，`agent` 模式必须 fail-fast，直到真实 provider/model execution 接入。
- 不写无痕：incognito session 拒绝 run / gate。
- 不替代 Domain Quality：eval 使用 quality snapshot，quality run 本身仍由 `domain_quality.rs` 管理。

## 验证

定向测试：

```bash
cargo test -p ha-core domain_eval --locked
```

覆盖：

- 内置 15 个 task 覆盖 5 个领域。
- 已晋升 `domain_eval_case` JSON artifact 可导入 task registry，重复导入幂等。
- Eval run 可记录幂等人工 calibration，task registry 与后续 report 能看到 user/project calibration。
- Trace fixture runner 会创建真实 session、Goal、Evidence、WorkflowRun、Domain Quality run 和 Domain Eval run。
- 非 `trace_fixture` 执行模式不会被伪报为成功。
- Research 缺少来源会被 eval 标成 failed。
- 有 Goal、Workflow、Evidence、Domain Quality 的 Research run 可通过 eval，并让 Quality Gate passed。

跨运行模式编译：

```bash
cargo check -p ha-core -p ha-server -p hope-agent --locked
pnpm typecheck
```

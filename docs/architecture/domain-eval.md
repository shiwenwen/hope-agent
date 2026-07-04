# Domain Eval 与 Quality Gate 控制平面

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 7.10 已实现。本文记录 `ha-core::domain_eval` 的最终技术事实：通用领域 eval task registry、promoted domain eval case 导入、user/project calibration 与人工复核记录、deterministic trace scoring、trace / agent fixture runner、fixture run history、`domain_eval_runs` history、Domain Quality Gate、owner API 与 Dashboard 通用质量区块 / Smoke Run Center。

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
| `domain_eval_runs` | 一次通用领域 eval 评分结果，字段包括 session/project、task id/version、domain、label、status、score、`source_type`、report JSON、source quality run、created_at。旧数据默认 `source_type='live'`。 |
| `domain_eval_fixture_runs` | 一次 trace/agent fixture smoke run 的完整报告，字段包括 name、execution mode、source type、status、session/goal/workflow/quality/eval run 关联、report JSON、error、created_at/updated_at。执行失败且没有 eval run 时也会落这里。 |
| `domain_eval_tasks` | 从已晋升 `domain_eval_case` 学习产物导入的自定义 eval task。字段包括 task id/version、project、source proposal、source path、task JSON、imported_at、updated_at。 |
| `domain_eval_calibrations` | user/project scope 的人工校准与复核记录。字段包括 task id/version、domain、project、scope、reviewer、verdict、note、source eval run、created_at。 |

索引：

- `idx_domain_eval_runs_scope(project_id, session_id, domain, created_at DESC)`
- `idx_domain_eval_runs_task(task_id, created_at DESC)`
- `idx_domain_eval_runs_status(status, created_at DESC)`
- `idx_domain_eval_runs_source(source_type, created_at DESC)`
- `idx_domain_eval_fixture_runs_recent(source_type, created_at DESC)`
- `idx_domain_eval_fixture_runs_status(status, created_at DESC)`
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
6. 生成 `DomainEvalReport` 并写入 `domain_eval_runs`，`sourceType` 默认 `live`。

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

`run_domain_eval_fixture(input)` 是 Phase 7.8-7.10 的半确定性 runner，用于把一份 fixture materialize 成真实控制面 trace，再交给同一个 scorer 判分。

支持两种 `executionMode`：

| Mode | 说明 |
| --- | --- |
| `trace_fixture` | 确定性控制面回归。Runner 按 fixture 写入 evidence / workflow / quality trace，再调用同一 scorer。 |
| `agent` | 真实 agent 执行。Runner 创建 user message + chat turn，调用 `run_chat_engine`，使用 fixture 显式传入的 `execution.providers` / `execution.modelChain`，默认开启 `execution.workflowMode="ultracode"`，让模型能自主判断是否创建 durable workflow。执行完成后再跑 Domain Quality / Domain Eval scorer。 |

所有 fixture runner 创建的 session 都会标记为 `SessionKind::EvalFixture`，隐藏于普通会话列表、普通全局搜索和 Dashboard live 聚合之外。Runner 同时写 `domain_eval_fixture_runs`，用于 Smoke Run Center 回放完整 report；`sourceType` 固定为：

| Source Type | 场景 |
| --- | --- |
| `fixture_trace` | `executionMode="trace_fixture"` |
| `fixture_agent` | `executionMode="agent"` |
| `fixture_unsupported` | 非法 execution mode 的 fail-fast report |

`trace_fixture` 流程：

1. 创建真实 session。
2. 创建 Goal，objective / completion criteria 默认来自 task。
3. 写入 fixture evidence，进入 `domain_evidence`。
4. 默认创建一个 `origin='domain_eval_fixture'` 的 WorkflowRun。
5. 默认运行 Domain Quality，得到 `domain_quality_runs/checks` snapshot。
6. 调用 `run_domain_eval_task`，把 scorer 输出写入 `domain_eval_runs`。
7. 按 fixture `checks` 输出 runner 自身通过/失败状态，并把完整 report 写入 `domain_eval_fixture_runs`。

`agent` 流程在创建 Goal 后先执行一轮 chat：

- `execution.prompt` 可覆盖 task prompt；默认使用 Goal objective，再退回 task input prompt。
- `execution.providers` / `execution.modelChain` 必填，owner API 不隐式读取桌面全局 provider。
- `execution.workflowMode` 支持 `off` / `on` / `ultracode`，默认 `ultracode`，用于测试自主动态 workflow 主路径。
- runner 注入受控 extra system context，包含 task id/domain、required evidence 和 success criteria。
- 执行报告写入 `report.execution`：`status`、`turnId`、`response/error`、`modelUsed`、`toolCalls`、`workflowMode`。
- `agent` 模式不会自动 materialize `fixture.evidence` 或 `fixture.workflow`；这些字段只属于 `trace_fixture` 的确定性种子。Agent 能力 fixture 必须让模型通过真实工具产出 evidence/workflow trace。
- 如果 agent 执行失败或缺少 provider/modelChain，runner 返回 failed report，不写 `domain_eval_runs`，但会写 `domain_eval_fixture_runs`，让 Smoke Center 能显示失败原因。
- 如果 agent 执行完成但没有产出足够 evidence/workflow/quality trace，后续 scorer 会把 eval run 标成 `failed` 或 `insufficient_data`；runner 不替模型补证据。

Fixture checks 除 scorer 断言外，还支持 execution 断言：`expectedExecutionStatus`、`requireTurn`、`minToolCalls`、`expectedToolCalls`、`responseContains`、`errorContains`。如果未显式设置 `expectedStatus`，runner 默认要求 scorer status 为 `passed`；需要验证失败样本时必须显式写 `expectedStatus: "failed"` 或 `"insufficient_data"`，避免“agent turn 成功但质量不达标”被误判为 fixture 通过。

Trace/agent fixture runner 当前是 owner API / 回归测试能力，不挂到 Dashboard quality gate 的普通按钮上。Dashboard Learning 只展示独立的「Domain smoke runs」区块；真实 quality gate 默认排除 `SessionKind::EvalFixture`、`sourceType LIKE 'fixture_%'`、`access_scope='fixture'` 的合成数据。需要诊断 synthetic gate 时显式传 `includeSynthetic=true`。

## Quality Gate

`evaluate_domain_quality_gate(input)` 只读历史，不调用 LLM、不运行工具、不生成 proposal。

Scope：

- `sessionId`：只看当前 session；incognito 拒绝。
- `projectId`：看项目内非 incognito session。
- 未传 scope：全局非 incognito。
- `domain` 可进一步过滤。
- 默认排除 fixture/smoke 数据；`includeSynthetic=true` 才把 `fixture_*` source 与 `EvalFixture` session 纳入诊断。

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
| `includeSynthetic` | false；只有 Smoke/diagnostic 调用设为 true |

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
| `run_domain_eval_fixture` | `POST /api/domain-eval/fixtures/run` | 运行 trace 或 agent fixture：trace 模式写入 fixture evidence/workflow/quality，agent 模式创建真实 turn 并用模型实际产出的 trace 进入同一 scorer。 |
| `list_domain_eval_fixture_runs` | `POST /api/domain-eval/fixture-runs` | 列出 fixture/smoke run history，包含执行失败且未写 eval run 的 report。 |
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
- 展示独立的「Domain smoke runs」卡片：最近 fixture run、pass rate、agent/trace 数、失败数、eval/quality/workflow/turn trace badge 与 error。
- 展示已校准 task 数；最近 eval run 支持点击「Mark reviewed」记录人工复核 calibration。
- 与 Release Gate / Continuous Benchmark Gate 分开展示，不生成综合分。

## 红线

- 不混排 coding benchmark：`domain_eval_runs` 与 `coding_eval_runs` 物理分表。
- 不伪造通用能力：没有 domain eval run 或 quality run 时 gate 必须 `insufficient_data`。
- 不污染真实质量门：fixture session 必须是 `SessionKind::EvalFixture`，fixture eval run 必须是 `sourceType=fixture_*`；Dashboard live gate 默认排除合成数据。
- 不越权运行工具：eval 只读既有 trace/evidence，不调用连接器，不发送、不发布、不改外部系统。
- 不隐式学习上线：`domain_eval_case` 必须先走 proposal preview / apply draft / explicit promotion，再由用户显式导入 task registry。
- 不让模型自校准：calibration 只暴露 owner API / GUI，不提供 agent tool 面。
- 不伪造 agent 能力：`agent` fixture 必须显式传 provider/modelChain；执行失败不写 eval run；deterministic trace 与真实 agent execution 在 report 中必须可区分。
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
- Trace fixture runner 会写 `domain_eval_fixture_runs`，其 session kind/sourceType 默认不进入 live quality gate；`includeSynthetic=true` 时才进入诊断 gate。
- Agent fixture runner 会创建真实 user message / chat turn，调用 mock Responses provider，经 `run_chat_engine` 产生 response，并默认打开 Workflow Mode Ultracode。
- Agent fixture 不会自动 materialize trace fixture seed，避免 evidence/workflow 被确定性 fixture 托过关。
- 缺少 provider/modelChain 的 agent fixture fail-fast，不写 eval run。
- Research 缺少来源会被 eval 标成 failed。
- 有 Goal、Workflow、Evidence、Domain Quality 的 Research run 可通过 eval，并让 Quality Gate passed。

跨运行模式编译：

```bash
cargo check -p ha-core -p ha-server -p hope-agent --locked
pnpm typecheck
```

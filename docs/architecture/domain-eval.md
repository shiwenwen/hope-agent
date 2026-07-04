# Domain Eval 与 Quality Gate 控制平面

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 7.14 已实现。本文记录 `ha-core::domain_eval` 的最终技术事实：通用领域 eval task registry、promoted domain eval case 导入、user/project calibration 与人工复核记录、deterministic trace scoring、trace / agent fixture runner、fixture run history、Domain Eval Campaign、Domain Campaign Leaderboard、Domain Campaign Learning Closure、Domain Readiness Gate、`domain_eval_runs` history、Domain Quality Gate、owner API 与 Dashboard 通用质量区块 / Smoke Run Center / Campaign Center。

## 目标

Domain Eval 把非 coding 场景的质量判断从“感觉不错”变成可审计 run：

- Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation 各有 3 个内置 eval task，总计 15 个。
- 每个 task 都定义输入 prompt、允许工具、required evidence、success criteria、禁止行为和 calibration 记录。
- 评分读取 Goal、Workflow、Domain Evidence、Domain Quality trace，不读取或混入 coding benchmark 表。
- Quality Gate 聚合 domain eval run、domain quality run/check 和 evidence coverage，输出 `passed` / `failed` / `insufficient_data`。
- Readiness Gate 再把 Quality Gate、Domain Campaign、Leaderboard 和 Campaign Learning Closure 合成可交付三态，回答“这个通用领域能力现在能不能作为可控长任务使用”。

这套控制面只证明通用领域任务质量，不代表 coding 能力；coding benchmark 仍由 [Coding Eval 控制面评测](coding-eval.md) 和 [Coding Improvement Loop](coding-improvement-loop.md) 承载。

## 数据模型

`SessionDB::open()` 调用 `domain_eval::ensure_tables()` 创建：

| 表 | 说明 |
| --- | --- |
| `domain_eval_runs` | 一次通用领域 eval 评分结果，字段包括 session/project、task id/version、domain、label、status、score、`source_type`、report JSON、source quality run、created_at。旧数据默认 `source_type='live'`。 |
| `domain_eval_fixture_runs` | 一次 trace/agent fixture smoke run 的完整报告，字段包括 name、execution mode、source type、status、session/goal/workflow/quality/eval run 关联、report JSON、error、created_at/updated_at。执行失败且没有 eval run 时也会落这里。 |
| `domain_eval_campaigns` | 一次批量通用领域 eval campaign。字段包括 session/project scope、name、status、domain、task filter、model matrix、execution mode、预算、错误、created/updated/started/finished。 |
| `domain_eval_campaign_items` | campaign 中一个 task × model/execution item。字段包括 task、domain、execution mode、provider/model label、status、attempt、fixture/eval run 关联、score、check 统计、report JSON、error、时间戳。 |
| `domain_eval_tasks` | 从已晋升 `domain_eval_case` 学习产物导入的自定义 eval task。字段包括 task id/version、project、source proposal、source path、task JSON、imported_at、updated_at。 |
| `domain_eval_calibrations` | user/project scope 的人工校准与复核记录。字段包括 task id/version、domain、project、scope、reviewer、verdict、note、source eval run、created_at。 |

索引：

- `idx_domain_eval_runs_scope(project_id, session_id, domain, created_at DESC)`
- `idx_domain_eval_runs_task(task_id, created_at DESC)`
- `idx_domain_eval_runs_status(status, created_at DESC)`
- `idx_domain_eval_runs_source(source_type, created_at DESC)`
- `idx_domain_eval_fixture_runs_recent(source_type, created_at DESC)`
- `idx_domain_eval_fixture_runs_status(status, created_at DESC)`
- `idx_domain_eval_campaigns_scope(project_id, session_id, created_at DESC)`
- `idx_domain_eval_campaigns_status(status, updated_at DESC)`
- `idx_domain_eval_campaign_items_campaign(campaign_id, status, updated_at DESC)`
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

## Campaign Runner

`create_domain_eval_campaign(input)` / `run_domain_eval_campaign(input)` 是 Phase 7.11-7.13 的批量运行面，用于把单次 fixture smoke 扩展成可取消、可 retry、可比较 provider/model、可沉淀学习草稿的 Domain Eval Pack。

Campaign 只负责编排，不新增第二套 scorer：

1. 创建时解析 task filter：`domain`、显式 `taskIds`、`maxTasks`，默认最多 5 个 task，硬上限 15。
2. 创建 model matrix：空 matrix 自动补一个 deterministic `trace fixture` item；外部模型 item 必须同时提供 `providerId` 和 `modelId`。
3. 每个 task × model 物化一条 `domain_eval_campaign_items`，初始 `queued`。
4. 运行时逐 item 检查 cancel flag；item 进入 `running` 后复用 `run_domain_eval_fixture`：
   - deterministic item 使用 `executionMode="trace_fixture"`，由 task required evidence 自动生成 synthetic evidence，source metadata 标记 `sourceType="fixture_campaign"`；
   - external item 使用 `executionMode="agent"`，provider config 只在 `create_domain_eval_campaign(input.providers)` 的 `runNow` 启动路径、`run_domain_eval_campaign(input.providers)` 或本机缓存中临时读取，不写入 campaign history。
5. item 完成后写回 `fixtureRunId`、`evalRunId`、`score`、check 统计、report JSON 和 error。
6. campaign summary 聚合 item 状态、通过率、eval run 数、平均分和 check 统计。

状态语义：

| Status | 说明 |
| --- | --- |
| `queued` | 已创建，尚未运行。 |
| `running` | 后台 runner 正在逐 item 运行。 |
| `cancel_requested` | 用户已请求取消；后续 queued item 会取消，已 running item 不强杀。 |
| `passed` | 所有实际运行 item 通过。 |
| `failed` | 没有通过 item，且至少一个 item failed。 |
| `partial` | 部分通过、部分失败。 |
| `cancelled` | 用户取消导致剩余 item 未运行。 |
| `interrupted` | 仍有 queued/running item 但 runner 已结束，通常表示进程中断。 |

`retryFailedOnly=true` 会把 `failed` / `interrupted` / `cancelled` item 重置为 `queued`，并清掉旧 fixture/eval run 关联后重新运行。历史 report 仍保留在 `domain_eval_fixture_runs` / `domain_eval_runs`，campaign item 指向最新一次 retry 结果。

`get_domain_eval_campaign_leaderboard(input)` 是 Phase 7.12 的对比聚合器：

- 按 scope / window / domain / campaignIds 读取 `domain_eval_campaign_items`。
- 按 `providerId + modelId + label + executionMode` 分组。
- 输出 rank、item pass rate、average score、attempts、eval run 数、check 统计、domains、warnings 和最多 8 条 evidence。
- 排序优先级：item pass rate 降序 -> average score 降序 -> item 数降序 -> failed / cancelled / interrupted item 数升序 -> label。
- 没有可比行或只有 queued/running item 时返回 `insufficient_data`；存在 failed / cancelled / interrupted item 时 report status 为 `failed`。

Phase 7.13 把 campaign failure 接回既有 Coding Improvement proposal queue：

- `generate_coding_improvement_proposals(sourceType="domain_eval_campaign", sourceId=<campaign_id>)` 会读取当前 scope 内 failed / cancelled / interrupted campaign item。
- 每个失败 item 生成两类 draft-only proposal：`domain_eval_case`（把失败沉淀为回归评测草稿）和 `domain_guidance`（把失败原因沉淀为可审查领域操作指南草稿）。
- `sourceId` 使用 campaign id；fingerprint 使用 `scope + campaign item id + kind`，所以同一 campaign 可重复点击而不会重复插入。
- `payload_json` 保留 campaign、item、failure category、report JSON、scope/project/window；后续 action preview / apply / promotion 仍由 [Coding Improvement Loop](coding-improvement-loop.md) 统一管理。
- 该路径不调用 LLM、不运行工具、不自动 apply、不自动 promotion。

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

## Readiness Gate

`evaluate_domain_readiness_gate(input)` 是 Phase 7.14 的 owner-plane 总门禁。它只读历史，不调用 LLM、不运行工具、不生成 proposal，用于把分散的通用领域质量证据收成一个可交付判断。

输入继承 Quality Gate 的 scope / domain / window / eval 阈值，并新增 campaign / learning 阈值：

| Threshold | 默认 |
| --- | --- |
| `minCampaignItems` | 1 |
| `minLeaderboardRows` | 1 |
| `maxFailedCampaignItems` | 0 |
| `maxOpenLearningProposals` | 0 |

聚合来源：

- `evaluate_domain_quality_gate`：live domain eval / quality / evidence coverage，默认不含 synthetic。
- `get_domain_eval_campaign_leaderboard`：同 scope/window/domain 的 campaign model/execution 对比。
- `domain_eval_campaigns` / `domain_eval_campaign_items`：campaign 数、active campaign、terminal item、failed/cancelled/interrupted item、最近更新时间。
- `coding_improvement_proposals(source_type='domain_eval_campaign')`：失败 campaign 是否已经生成 draft proposal，以及是否仍有未关闭学习草稿。

Readiness checks：

| Check | 说明 |
| --- | --- |
| `domain_quality_gate` | Quality Gate 必须通过；缺 live eval/quality 时沿用 `insufficient_data`。 |
| `campaign_sample` | 至少有指定数量的 campaign item，避免只靠一次人工质量 run。 |
| `campaign_completion` | queued/running/cancel_requested campaign 不算失败，但让 readiness 保持 `insufficient_data`，等待长任务完成。 |
| `campaign_leaderboard` | leaderboard 至少有指定行数，且不能有 failed/cancelled/interrupted item。 |
| `campaign_failures` | 最近窗口内失败 / 取消 / 中断 item 必须低于阈值，默认 0。 |
| `learning_closure` | 失败 campaign 必须已物化为学习 proposal，且 open proposal 数不能超阈值，默认 0。 |

输出包含：

- `summary`：eval / quality / campaign / item / leaderboard / learning proposal 计数。
- `qualityGate`：完整 Quality Gate 报告，便于下钻。
- `campaignLeaderboard`：完整 leaderboard 报告，便于对比模型。
- `blockers`：非 passed 且非 advisory 的 check 名。
- `recommendedNextSteps`：按失败 check 生成的下一步建议。

Readiness status：

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
| `create_domain_eval_campaign` | `POST /api/domain-eval/campaigns/create` | 创建 durable Domain Eval Campaign；`runNow=true` 时后台启动 runner。 |
| `list_domain_eval_campaigns` | `POST /api/domain-eval/campaigns` | 列出 campaign history，包含 item 状态、summary、fixture/eval run 关联。 |
| `get_domain_eval_campaign` | `GET /api/domain-eval/campaigns/{campaign_id}` | 读取单个 campaign snapshot。 |
| `run_domain_eval_campaign` | `POST /api/domain-eval/campaigns/run` | 后台运行或 retry campaign；`retryFailedOnly=true` 只重跑 failed/interrupted/cancelled item。 |
| `cancel_domain_eval_campaign` | `POST /api/domain-eval/campaigns/{campaign_id}/cancel` | 请求取消 campaign，并把仍 queued 的 item 标记为 cancelled。 |
| `get_domain_eval_campaign_leaderboard` | `POST /api/domain-eval/campaigns/leaderboard` | 按模型 / execution 聚合 Domain Campaign item，返回 rank、pass rate、average score 与可追溯 evidence。 |
| `import_domain_eval_case` | `POST /api/domain-eval/cases/import` | 把已晋升的 `domain_eval_case` proposal 导入 active task registry。 |
| `record_domain_eval_calibration` | `POST /api/domain-eval/calibrations/record` | 记录 task 的 user/project 人工校准或一次 eval run 的复核结论。 |
| `list_domain_eval_calibrations` | `POST /api/domain-eval/calibrations` | 查询 calibration history，可按 task/domain/project 过滤。 |
| `list_domain_eval_runs` | `POST /api/domain-eval/runs` | 列出 domain eval run history。 |
| `evaluate_domain_quality_gate` | `POST /api/domain-quality-gate/evaluate` | 计算通用领域 quality gate。 |
| `evaluate_domain_readiness_gate` | `POST /api/domain-readiness-gate/evaluate` | 计算通用领域 readiness gate：Quality Gate + Campaign + Leaderboard + Learning Closure。 |

## Dashboard 交互

Dashboard Learning Tab 新增「General domain quality」区块：

- 展示 gate 三态。
- 展示 eval pass rate、average score、quality blockers、domain coverage。
- 展示 attention checks。
- 展示最近 domain eval run。
- 展示独立的「Domain smoke runs」卡片：最近 fixture run、pass rate、agent/trace 数、失败数、eval/quality/workflow/turn trace badge 与 error。
- 展示「Domain campaigns」卡片：可运行 deterministic trace pack，也可选择 provider/model 运行 external agent campaign；可查看 durable campaign / item 状态、item pass rate、平均分、check 数、fixture/eval run 关联；failed / interrupted / cancelled campaign 可 retry，queued / running campaign 可 cancel，含失败 item 且有 session scope 的 campaign 可显式生成 learning drafts。
- 展示「Domain model leaderboard」：按模型 / execution 聚合最近 campaign item，显示 rank、平均分、item 通过数、trace evidence 数和 warning。
- 展示「Domain readiness」卡片：直接调用 `evaluate_domain_readiness_gate`，显示总体 readiness 三态、quality/eval/campaign/leaderboard/learning proposal 核心计数、阻塞 check 和 recommended next steps。
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
- 不存 provider secret：campaign history 只保存 provider/model/label；真实 provider config 只能在 run input 或本机缓存中临时解析。
- Leaderboard 必须可追溯：每一行要保留 campaign/item/task/status/score evidence，不能只给不可审计的平均值。
- Learning closure 不自动改规则：campaign failure 只能生成 draft proposal，后续 apply / promotion 必须由用户显式触发。
- Readiness Gate 只读事实：不能自动生成 learning proposal、不能自动 retry campaign、不能把运行中的 campaign 标成 failed；active campaign 只能让 readiness 保持 `insufficient_data`。
- Retry 必须真实重跑：`retryFailedOnly=true` 清掉 item 的旧 fixture/eval run 指针和 check 统计，再把 failed/interrupted/cancelled item 放回 `queued`。
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
- Domain Eval Campaign 可创建 deterministic trace pack、cancel queued item、retry cancelled item，并在 item 上写回最新 fixture/eval run、score 和 check 统计；leaderboard 能按模型/execution 聚合并保留 evidence。
- External model campaign 缺少 provider secret 时 item failed 且进入 leaderboard warning，不写 eval run、不静默成功。
- Domain Readiness Gate 在 live quality + campaign evidence 齐全时 passed；失败 campaign 且未闭环学习时 failed，并指出 `campaign_failures` / `learning_closure` blockers。
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

# Domain Eval 与 Quality Gate 控制平面

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 7.6 已实现第一版。本文记录 `ha-core::domain_eval` 的最终技术事实：通用领域 eval task registry、deterministic trace scoring、`domain_eval_runs` history、Domain Quality Gate、owner API 与 Dashboard 通用质量区块。

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

索引：

- `idx_domain_eval_runs_scope(project_id, session_id, domain, created_at DESC)`
- `idx_domain_eval_runs_task(task_id, created_at DESC)`
- `idx_domain_eval_runs_status(status, created_at DESC)`

## 内置 Task Registry

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
| `calibration` | 人工校准记录；首版为 built-in rubric，后续可接用户/项目校准。 |

## Run Scoring

`run_domain_eval_task(input)` 执行同步确定性评分：

1. 读取 session，incognito 直接拒绝。
2. 读取内置 task manifest。
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
| `list_domain_eval_runs` | `POST /api/domain-eval/runs` | 列出 domain eval run history。 |
| `evaluate_domain_quality_gate` | `POST /api/domain-quality-gate/evaluate` | 计算通用领域 quality gate。 |

## Dashboard 交互

Dashboard Learning Tab 新增「General domain quality」区块：

- 展示 gate 三态。
- 展示 eval pass rate、average score、quality blockers、domain coverage。
- 展示 attention checks。
- 展示最近 domain eval run。
- 与 Release Gate / Continuous Benchmark Gate 分开展示，不生成综合分。

## 红线

- 不混排 coding benchmark：`domain_eval_runs` 与 `coding_eval_runs` 物理分表。
- 不伪造通用能力：没有 domain eval run 或 quality run 时 gate 必须 `insufficient_data`。
- 不越权运行工具：eval 只读既有 trace/evidence，不调用连接器，不发送、不发布、不改外部系统。
- 不写无痕：incognito session 拒绝 run / gate。
- 不替代 Domain Quality：eval 使用 quality snapshot，quality run 本身仍由 `domain_quality.rs` 管理。

## 验证

定向测试：

```bash
cargo test -p ha-core domain_eval --locked
```

覆盖：

- 内置 15 个 task 覆盖 5 个领域。
- Research 缺少来源会被 eval 标成 failed。
- 有 Goal、Workflow、Evidence、Domain Quality 的 Research run 可通过 eval，并让 Quality Gate passed。

跨运行模式编译：

```bash
cargo check -p ha-core -p ha-server -p hope-agent --locked
pnpm typecheck
```

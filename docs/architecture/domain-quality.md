# Domain Quality 控制平面

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 7.4 已实现，Phase 7.5 已接入 Domain Learning，Phase 7.6 已接入 Domain Eval / Quality Gate，Phase 7.15 已在 Workspace 侧和 Domain Workflow owner API 上补充最终交付守门。本文记录 `ha-core::domain_quality` 的最终技术事实：通用领域 review / verification run、check、event、Goal evidence 阻塞语义、Domain Learning 与 Domain Eval 输入信号、owner API 与 Workspace「领域复核」交互。

## 目标

Domain Quality 把“复核 / 验证”从 coding diff 扩展到非编程长任务。它覆盖 Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation、Inbox、Project Ops 等任务，但不改造既有代码 Review Engine 和 Smart Verification：

- 代码审查仍由 `review.rs` 负责，finding 仍绑定文件 / 行号。
- 代码验证仍由 `verification.rs` 负责，step 仍绑定命令 / cwd / 风险。
- 非 coding 产物由 `domain_quality.rs` 生成 domain run / check / event，基于 Domain Workflow template、domain evidence 和 approval gates 做确定性复核。

这样不会把报告、邮件、会议 brief 伪装成代码 finding，也不会把“引用缺失”伪装成 shell command 失败。

## 数据模型

`SessionDB::open()` 调用 `domain_quality::ensure_tables()` 创建三张表：

| 表 | 说明 |
| --- | --- |
| `domain_quality_runs` | 一次领域质量复核。字段包括 session、goal、domain、template id/version、state、summary、stats、error、created/updated/completed。 |
| `domain_quality_checks` | run 下的复核项。字段包括 check type、profile、title、body、severity、status、evidence type、source metadata。 |
| `domain_quality_events` | run timeline，记录 started、check recorded、completed、failed 等事件；payload 落库前截断到 64KB preview。 |

Run state：

| State | 语义 |
| --- | --- |
| `running` | 正在复核。当前实现为同步确定性检查，通常很短暂。 |
| `completed` | 所有阻塞检查通过，可能仍有 advisory。 |
| `blocked` | 存在 P0/P1 的 failed / blocked check。 |
| `needs_user` | 高风险动作需要用户确认，且缺少显式确认。 |
| `failed` | 复核流程自身失败。 |
| `cancelled` | 保留状态，当前 owner API 暂不暴露取消。 |

Check status：

| Status | 语义 |
| --- | --- |
| `passed` | 检查通过。 |
| `failed` | 必需 evidence 或领域质量要求缺失。 |
| `blocked` | 预留给未来连接器 / 外部系统阻塞。 |
| `needs_user` | 必须用户确认后才能继续高风险动作。 |
| `advisory` | 建议项，不阻塞 Goal。 |

## 复核输入

`run_domain_quality(input)` 使用 `RunDomainQualityInput`：

| 字段 | 说明 |
| --- | --- |
| `sessionId` | 必填；incognito session 拒绝持久化。 |
| `goalId` | 可选；不传时自动绑定当前 active/open Goal。 |
| `domain` | 可选；不传时从 template、Goal 文本、domain evidence、artifact kind 推断。 |
| `templateId` / `templateVersion` | 可选；指定 Domain Workflow template。省略 version 时按当前最新可用版本解析。 |
| `profiles[]` | 可选；当前用于 stats / trace，默认包含 domain、`required_evidence`、`approval_gate`。 |
| `artifactTitle` / `artifactKind` | 可选；用于后续产物复核和 domain 推断。 |
| `sourceMetadata` | 可选；可放 `requestedAction`、`highRiskAction` 等上下文。 |
| `explicitUserApproval` | 高风险动作的显式用户确认。 |

Template / domain 解析优先级：

1. `templateId` / `templateVersion` 显式指定的 template。
2. 显式 `domain` 对应的最新可用 template。
3. active / 指定 Goal 绑定的 `workflow_template_id/version`。
4. Goal 的 `domain`、`artifactKind`、Goal objective / completion criteria 的关键词推断。
5. 当前 session / goal 的 domain evidence 计数。
6. fallback 到 `writing`。

## 领域规则

Domain Quality 复用 Domain Workflow template 的三类信号：

- `requiredEvidence`：缺少必需 evidence 会生成 P1 failed check。
- `verificationPolicy`：当前以 domain profile 的确定性规则落地，后续可继续细化 profile。
- `approvalGates`：只有当 `sourceMetadata.requestedAction` 匹配 gate，或 `highRiskAction=true` 时才强制 `needs_user`；普通草稿复核不会因为存在发布/发送 gate 而提前阻塞。

已落地的 domain profile：

| Domain | 检查重点 |
| --- | --- |
| `research` | 至少 3 个 source、至少 2 个 claim check、citation audit、来源日期 / 时效 metadata。 |
| `writing` | draft artifact、audience / requirement review、术语和引用缺口 advisory。 |
| `data_analysis` | data quality evidence、metric interpretation、dataset / denominator / sample metadata。 |
| `meeting_prep` | meeting context、brief / agenda、decision points / risks advisory。 |
| `inbox` | thread source、facts / commitments check、send 前 approval。 |
| `knowledge_curation` | source notes、dedupe / gap review、curated note / index。 |
| `project_ops` | status / plan artifact、risks / dependencies、owners / tradeoffs。 |

## Goal 语义

Domain Quality 会写入 `goal_links`：

| Relation | Goal 影响 |
| --- | --- |
| `domain_quality_passed` | 正向强证据；可解除较早的 domain quality 阻塞。 |
| `domain_quality_failed` | 阻塞证据。 |
| `domain_quality_blocked` | 阻塞证据。 |
| `domain_quality_needs_user` | 阻塞证据，metadata 指明需要用户确认。 |
| `domain_quality_check` | 仅对 P0/P1 且 failed / blocked / needs_user 的 check 写入；作为细粒度阻塞证据。 |

写入后调用 `evaluate_goal(goal_id)`。因此非 coding 产物缺少关键证据或高风险动作缺少确认时，Goal 会进入 `blocked`，不会被错误标记为完成。Goal 本身没有独立 `needs_user` state，所以 `needs_user` 保留在 DomainQualityRun 和 Goal evidence metadata 中。

## Domain Learning 输入

Phase 7.5 后，`coding_improvement::generate_coding_improvement_proposals()` 和 `distill_coding_improvement_proposals()` 会读取当前 session/project scope 内的 Domain Quality snapshot，并把领域质量结果转成 draft-only improvement proposal。`generate_coding_improvement_proposals()` 还支持 `sourceType` / `sourceId` / `proposalKinds` 过滤；Workspace「领域复核」区块里的「提炼经验」按钮会把当前 run 作为 `sourceType="domain_quality"` + `sourceId=<run_id>` 传入，只从这次复核提炼候选，避免泛扫同一 scope 内的其它学习信号。

| Quality 信号 | Proposal kind |
| --- | --- |
| `completed` run | `domain_workflow_template`、`domain_guidance` |
| `blocked` / `failed` / `needs_user` run | `domain_review_profile`、`domain_eval_case` |
| `approval` check 进入 `needs_user` | `connector_usage_pattern` |

Domain Quality 本身不写模板、不写 guidance、不修改 connector 策略。它只提供 run/check/evidence 事实；所有学习产物都必须走 Coding Improvement Loop 的 preview → apply draft → explicit promotion 链路。

GUI 侧的产品语义：

- 「重跑复核」只重新执行 Domain Quality，不生成学习候选。
- 「提炼经验」只生成 draft proposal，不写正式模板 / guidance / skill；用户仍需在 Coding Improvement proposal 队列里预览、应用草稿、显式晋升。
- 已晋升的 `domain_eval_case` proposal 会在质量趋势候选卡片中显示「导入评测」；点击后调用 owner API 把 JSON artifact 导入 `domain_eval_tasks`，后续可被 `list_domain_eval_tasks` / `run_domain_eval_task` 使用。
- 无痕会话禁用「提炼经验」，保持关闭即焚。

## Domain Eval / Gate 输入

Phase 7.6 后，`domain_eval::run_domain_eval_task()` 会读取显式 `sourceQualityRunId` 或最近同 domain 的 Domain Quality snapshot，把 quality state 与 checks 纳入 deterministic scoring。`evaluate_domain_quality_gate()` 会聚合窗口内的 `domain_quality_runs` 和 approval checks：

- `completed` quality run 作为通过证据。
- `blocked` / `failed` / `needs_user` quality run 计入 gate blocker。
- `approval` check 的 `needs_user` / `failed` / `blocked` 计入 approval safety blocker。

Domain Quality 仍是复核事实源；Domain Eval / Gate 只读这些事实，不反向修改 quality run。被显式晋升并导入的 `domain_eval_case` 只扩展 eval task registry，不修改历史 quality run/check。

## Dashboard 趋势输入

Dashboard Learning 的 `dashboard_coding_improvement` 返回 `domainQuality` 历史趋势区块。该区块只读：

- `domain_quality_runs`：按 state 统计 completed / blocked / failed / needs_user，并生成 recent runs。
- `domain_quality_checks`：统计 approval blockers 和 top blocker reason。
- `domain_eval_runs`：统计 domain eval pass rate、average score 和按领域 eval 覆盖。
- `coding_improvement_proposals(source_type='domain_quality')`：统计通用领域学习草稿 / 晋升数量。

它和 `evaluate_domain_quality_gate()` 的边界不同：Dashboard `domainQuality` 是历史可观察性视图，不执行阈值判定、不阻塞发布、不写任何学习产物；Domain Quality Gate 是当前 scope/window 的门禁判定。

## Owner API

Tauri / HTTP / transport 均已注册：

| Tauri Command | HTTP | 说明 |
| --- | --- | --- |
| `list_domain_quality_runs` | `GET /api/sessions/{sessionId}/domain-quality-runs` | 列出当前 session 的领域复核 run。 |
| `get_domain_quality_run` | `GET /api/domain-quality-runs/{runId}` | 返回 run + checks + events snapshot。 |
| `run_domain_quality` | `POST /api/domain-quality-runs/run` | 执行一次同步确定性领域复核。HTTP body 为 `{ input }`。 |

EventBus：

| 事件名 | 触发点 |
| --- | --- |
| `domain_quality:created` | run 创建。 |
| `domain_quality:updated` | run completed / failed。 |
| `domain_quality:check_updated` | check 记录。 |
| `domain_quality:event` | run event 追加。 |

## Workspace 交互

Workspace 面板新增「领域复核」区块，位于「代码审查」和「验证」之后：

- 无需工作目录，适合纯调研 / 写作 / 邮件 / 会议任务。
- 展示通过、缺失、需确认、建议四类计数。
- 展示最近 run summary、domain、template id/version。
- 优先列出非 passed check；全部通过时展示少量 passed/advisory。
- 支持运行领域复核和刷新。
- 内嵌「交付守门」卡片，调用 `evaluate_domain_artifact_export_guard` 展示最终产物是否已创建、是否复核、是否存在 private/connector/sensitive/pending/redacted evidence，以及是否缺少显式 export review。
- 监听 `domain_quality:*` 事件，长任务完成或事件到达时自动刷新。
- incognito session 不显示 durable 结果，只显示禁用提示。

## 红线

- 不破坏 coding：不改 `review_runs/review_findings`、`verification_runs/verification_steps` 的语义。
- 不伪造外部事实：Domain Quality 只检查已记录 evidence 和输入 metadata，不主动访问连接器。
- 不默认越权：高风险 action 只有在明确请求时才需要 approval；缺少 approval 时 fail closed 为 `needs_user`。
- 不写无痕：incognito session 拒绝创建 durable run。
- 不自动发送 / 发布 / 修改外部系统：该模块只产出质量结论和 Goal evidence。
- 不自动学习成正式规则：Domain Learning 只能从该模块读取事实并生成草稿；正式模板 / guidance / connector pattern 必须用户显式 promotion。

## 验证

定向测试：

```bash
cargo test -p ha-core domain_quality --locked
```

覆盖：

- Research 缺少 required evidence 时生成 failed check、run 进入 blocked，并阻塞 Goal。
- Goal 已绑定 workflow template 时，未显式指定 domain/template 的领域复核优先使用 Goal template/version。
- Inbox `send_message` 高风险动作缺少显式确认时生成 P0 `needs_user` approval check。
- Coding Improvement Loop 的 `domain_learning_generates_reviewable_drafts_from_quality_runs` 覆盖 Research / Writing / Data Analysis / Inbox quality run 进入 Domain Learning proposal、按 `domain_quality` run 定向过滤、draft apply 与 promotion preview。

跨运行模式编译：

```bash
cargo check -p ha-server -p hope-agent --locked
pnpm typecheck
```

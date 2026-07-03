# Domain Workflow 控制平面

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 7.1 Domain Workflow Registry 与 Phase 7.2 General Evidence Model 已实现；Phase 7.3 已在 [Context Retrieval v2](context-retrieval.md) 接入 domain profile、domain evidence 候选与 access issue。本文记录 `ha-core::domain_workflow`、owner API、通用 workflow template、通用 evidence 与 Goal evidence 链接的当前技术事实。

## 目标

Domain Workflow 把已经稳定的 Goal / Mode / Workflow / Task / Evidence 底座用于非编程长任务。它不是固定 DSL，也不替代模型动态判断；它提供一组可版本化、可预览、可审批的领域工作习惯：

- 在 Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation、Inbox、Project Ops 等任务里列出 required evidence、approval gates、verification policy、stop conditions 和 output contract。
- 从 template 生成 `workflow.js` draft，继续走既有 Script Gate、permission preview、用户确认和 durable Workflow runtime。
- 把非 coding 证据记录为一等 evidence，不再伪装成 validation/diff/file。
- 通用 evidence 可链接到 Goal，进入 Goal detail、criteria audit 和 final audit。

## 数据模型

`SessionDB::open()` 调用 `domain_workflow::ensure_tables()` 创建两张表：

| 表 | 说明 |
| --- | --- |
| `domain_workflow_templates` | 用户 / 项目自定义模板。主键 `(id, version)`；字段包括 domain、task types、default mode、required evidence、recommended tools、approval gates、verification policy、stop conditions、output contract、eval criteria、prompt hints、scope、project、enabled。内置模板由代码 registry 提供，不写入 DB。 |
| `domain_evidence_items` | 通用 evidence。字段包括 goal/session/project、domain、evidence type、title、summary、source metadata、confidence、access scope、redaction status、created/updated。记录到非无痕 session；可选 goal 链接。 |

内置 template 使用代码 registry，确保首次启动无需迁移数据即可可用；用户/项目自定义 template 通过 owner API 显式保存，不能覆盖内置 template 的同 id/version。

## 内置领域

Phase 7.1 内置 7 个 template：

| Template | Domain | 典型任务 |
| --- | --- | --- |
| `research-brief` | `research` | 市场调研、技术调研、竞品分析 |
| `writing-brief` | `writing` | 决策 memo、PRD、周报、方案文档 |
| `data-analysis-readout` | `data_analysis` | 指标诊断、KPI readout、dashboard review |
| `meeting-prep` | `meeting_prep` | 会议 brief、议题和风险梳理 |
| `knowledge-curation` | `knowledge_curation` | 主题索引、知识整理、资料综合 |
| `inbox-comms` | `inbox` | 邮件回复草稿、线程分类、跟进计划 |
| `project-ops` | `project_ops` | 项目状态、风险登记、计划复核 |

每个 template 都包含：

- required evidence：例如 `source_cited`、`claim_checked`、`data_quality_checked`、`message_draft_approved`。
- approval gates：例如发布/发送/分享/外部系统修改前必须用户确认。
- verification policy：例如引用时效、claim cross-check、结构 review、口径和样本量检查。
- stop conditions：上下文缺失、用户确认缺失、数据质量失败等必须停机。
- output contract 与 prompt hints：只进入生成的 workflow draft，不污染全局 system prompt。

## Workflow Draft

`preview_domain_workflow(input)` 做三件事：

1. 解析 template、session、可选 active/open Goal，拒绝 incognito session。
2. 生成 `workflow.js` draft：创建 task、写入 domain plan、调用 `workflow.askUser` 要求用户确认计划，最终 `workflow.finish` 返回 template/evidence/approval/verification 摘要。
3. 调用既有 `preview_workflow_script_for_session`，返回 Script Gate 与 permission preview。

生成的 draft 只是一份可审查脚本，不自动创建 WorkflowRun、不自动执行、不访问连接器、不发送消息、不写外部系统。真正执行仍必须走已有 `create_workflow_run` / `run_workflow_run` 和审批链。

## General Evidence

Phase 7.2 支持下列 evidence type：

| Evidence Type | 用途 |
| --- | --- |
| `source_cited` | 来源、网页、文档、邮件、笔记被引用。 |
| `claim_checked` | 关键 claim 被核查，含 verdict / conflict / confidence。 |
| `user_decision` | 用户显式做出的决策、确认或取舍。 |
| `artifact_created` | 创建报告、brief、草稿、表格、索引等产物。 |
| `artifact_reviewed` | 产物被结构、读者、引用、完整性等维度复核。 |
| `data_quality_checked` | 数据源、口径、样本、异常值、计算等完成质量检查。 |
| `citation_audited` | 引用覆盖率、时效和来源可信度审计完成。 |
| `message_draft_approved` | 邮件/消息草稿发送前得到用户明确批准。 |
| `meeting_context_collected` | 日历、材料、参会人、历史决策等会议上下文被收集。 |

`record_domain_evidence(input)` 要求 `goalId` 或 `sessionId`，并执行：

- session 必须存在且不是 incognito。
- 若传 `goalId`，session 从 goal 解析，避免跨 session 伪造 evidence。
- `sourceMetadata` 必须以 JSON object 存储；非 object 会包成 `{ value }`。
- `confidence` clamp 到 `[0,1]`。
- `accessScope` 归一为 `public | session | project | connector | private`。
- `redactionStatus` 归一为 `none | redacted | pending | sensitive`。
- 若关联 goal，会调用 `link_goal_target(goal_id, "domain_evidence", evidence_id, evidence_type, metadata)`。

Goal evidence relation 白名单已加法扩展这些通用 evidence type；coding evidence relation 保持原样。

## Context Retrieval 衔接

Phase 7.3 起，`ha-core::context_retrieval` 会只读消费本模块的数据：

- 从 `workflow_runs.kind = domain:<domain>`、`domain_evidence_items.domain`、显式 `domain/templateId` 或 Goal objective / criteria 推导 `domainContext`。
- 把 `domain_evidence_items` 转成 document、email_thread、calendar_event、sheet_range、knowledge_note、web_source、decision、artifact 等候选。
- 按 required evidence、Goal criteria、confidence、redaction status 和 query boost 加权排序。
- 缺少连接器或 required evidence 时返回 `accessIssues[]`，只提示缺口，不伪造来源。
- Workspace Context 区块展示 domain profile、access issue 与 domain action chips；“复制引用”已落地，其余动作先以 `metadata.domainActions` 暴露给后续 owner action。

这条链路仍是只读 owner-plane 查询，不创建 workflow run，不写 evidence，不访问连接器。

## Owner API

Tauri / HTTP / transport 均已注册：

| Tauri Command | HTTP | 说明 |
| --- | --- | --- |
| `list_domain_workflow_templates` | `POST /api/domain-workflows/templates` | 列出内置 + 自定义 template，可按 domain/task/project 过滤。 |
| `save_domain_workflow_template` | `POST /api/domain-workflows/templates/save` | 显式保存用户/项目 template；必须 `explicitSaveConsent=true`。 |
| `preview_domain_workflow` | `POST /api/domain-workflows/preview` | 生成 workflow draft 和 Script Gate / permission preview。 |
| `record_domain_evidence` | `POST /api/domain-evidence/record` | 写入通用 evidence，并可链接到 Goal。 |
| `list_domain_evidence` | `POST /api/domain-evidence` | 按 goal/session/project/domain/type 列出 evidence。 |

## 红线

- 不扩大权限：template 只描述推荐工具和审批门，不赋予连接器权限。
- 不自动执行：preview 不创建 run、不运行脚本、不访问网络、不发邮件、不改日历、不写外部系统。
- 不污染全局 prompt：domain hints 只进入 workflow draft 的动态 payload。
- 不写无痕：incognito session 不可 preview durable domain workflow，也不可记录 domain evidence。
- 不覆盖内置：自定义 template 不能覆盖 built-in 同 id/version。
- 不破坏 coding：Goal evidence 只加通用 relation；coding review、verification、eval、benchmark 的表和行为不变。

## 验证

定向测试：

```bash
cargo test -p ha-core domain_workflow --locked
```

覆盖：

- 内置 Research template 可列出并生成通过 Script Gate 的 workflow draft。
- Domain evidence 可写入 `domain_evidence_items`，并通过 `goal_links` 出现在 Goal snapshot evidence 中。

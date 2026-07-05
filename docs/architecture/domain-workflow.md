# Domain Workflow 控制平面

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 7.1 Domain Workflow Registry 与 Phase 7.2 General Evidence Model 已实现；Phase 7.3 已在 [Context Retrieval v2](context-retrieval.md) 接入 domain profile、domain evidence 候选与 access issue；Phase 7.4 已由 [Domain Quality 控制平面](domain-quality.md) 消费 template / evidence / approval gates 生成通用领域 review / verification；Phase 7.5-7.6 已把 Domain Quality / Evidence 作为 [Coding Improvement Loop](coding-improvement-loop.md) 的通用学习输入与 [Domain Eval 与 Quality Gate 控制平面](domain-eval.md) 的评分输入；Phase 7.15 已在本模块补充 Artifact Export Guard，Phase 7.16 已补 Connector Action Guard；Phase 8.2 已补 Connector E2E Gate，把真实外部系统修改从“动作前守门”推进到“读取 -> 草稿 -> 批准 -> 执行 -> 复核 -> 回滚说明”的完整链路验收；Phase 8.4 已在 Workspace 补充「通用任务工作台」，把 Sources / Evidence / Drafts / Review / Verification / Decisions 合成用户可操作闭环。本文记录 `ha-core::domain_workflow`、owner API、通用 workflow template、通用 evidence、Goal evidence 链接、交付守门、外部动作守门、连接器 E2E 验收与 Workspace 通用任务工作台的当前技术事实。

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
2. 生成 `workflow.js` draft：创建 task、写入 domain plan、按 `requirePlanConfirmation` 决定是否调用 `workflow.askUser` 要求用户确认计划，生成 `workflow.verify` 复核计划，最终 `workflow.finish` 返回 template/evidence/approval/verification 摘要和显式 budget hint。
3. 调用既有 `preview_workflow_script_for_session`，返回 Script Gate 与 permission preview。

生成的 draft 只是一份可审查脚本，不自动创建 WorkflowRun、不自动执行、不访问连接器、不发送消息、不写外部系统。真正执行仍必须走已有 `create_workflow_run` / `run_workflow_run` 和审批链。

`requirePlanConfirmation` 默认 `true`，服务 GUI 手动草稿确认；Loop 自动创建 WorkflowRun 时显式传 `false`，避免无人值守环境一启动就被 `askUser` fail-closed 卡死。自动路径仍不绕过 Script Gate、permission preview、运行时权限引擎或 Domain Quality approval gate。

## GUI 入口

Workspace / Workflow Control Center 的“新建工作流”表单已接入 domain workflow template：

- Goal 创建 / 编辑表单也可选择 domain workflow template 与 task type；选择会持久化到 `goals.domain`、`workflow_template_id/version`、`workflow_task_type`，下一轮 system prompt、Context Retrieval、Domain Quality 和 Workflow 创建器都会感知。GUI 选择器内部用 `id@version` 作为稳定 key，避免同一模板多版本时丢失版本。
- 打开创建器时懒加载 `list_domain_workflow_templates`，展示内置 + 自定义且 enabled 的 template。
- 用户可在 GUI 里选择 template 与 task type，不需要记 `/workflow` 参数或模板 id；若 active Goal 已绑定模板，新建 workflow 默认预选该模板。
- 点击“生成领域草稿”会调用 `preview_domain_workflow`，把返回的 `workflowKind`、`executionMode`、`scriptSource` 和 `scriptPreview` 回填到标准 workflow 创建链路。
- 创建前同屏展示 output contract 摘要、required evidence、approval gates、verification policy 与 warnings；用户继续复用既有 Script Gate / permission preview / run immediately / worktree 选择。
- 修改目标、模板、任务类型、执行模式或脚本会清空旧预检，避免用过期 preview 创建 run。
- Loop 创建区可在 active Goal 已绑定领域模板时选择“创建工作流”：每次 interval tick 会用该模板版本生成 `requirePlanConfirmation=false` 的 draft，创建 `origin=loop:<loop_id>` 的 WorkflowRun，并把 workflow run id 写回 Loop trace。
- Workspace 右侧面板新增「通用任务工作台」：复用当前 session 的 domain evidence、Review、Verification、Domain Quality、Artifact Export Guard、Connector Action Guard、Operational Gate 与 Soak Report，把来源、证据、草稿、复核、验证、用户决策、真实样本验收和长任务运行健康压成一个同屏总览。

GUI 入口仍是 owner plane：它只生成 draft 和 preview，不自动访问连接器，也不绕过后续 workflow runtime 的审批、用户确认和权限策略。

## 通用任务工作台

Phase 8.4 在 `src/components/chat/workspace/WorkspacePanel.tsx` 新增「通用任务工作台」区块，放在「推荐上下文」之后、「LSP / Review / Verification / 领域复核」之前。它是 GUI 聚合层，不新增后端表，也不改变任何执行/授权语义。

第一版聚合来源：

| 来源 | 用途 |
| --- | --- |
| `list_domain_evidence` | 读取当前 session 最近 domain evidence，统计 Sources、Evidence、Drafts、Review、Decisions，并展示最近证据。 |
| `record_domain_evidence` | 从 Context Retrieval 候选行的“摘要”/“证据”/“冲突”按钮写入当前 session evidence；“摘要”落 `artifact_created` + `artifactKind=context_summary`，“冲突”落 `claim_checked` + `verdict=conflict`，并刷新通用任务工作台。 |
| `create_owner_ask_user_question` / `respond_ask_user_question` | 从 Context Retrieval 候选行的“确认”按钮创建 owner-plane durable elicitation；用户回答后落 `user_decision` evidence。 |
| `create_session_task` | 从 Context Retrieval 候选行的“转任务”按钮或通用任务工作台“下一步”缺口创建 session task，并通过 `task_updated` 刷新进度面板。 |
| `evaluate_domain_artifact_export_guard` | 显示最终交付是否具备产物、复核、敏感来源导出复核和脱敏证据。 |
| `evaluate_domain_connector_action_guard` | 显示真实外部动作是否具备动作 scope、用户批准、回滚和交付守门证据。 |
| `evaluate_domain_operational_gate` | 显示当前 session 的 workflow / loop / campaign 是否有足够样本、失败残留或未排空运行。 |
| `generate_domain_soak_report` | 汇总当前 session 最近窗口内的 workflow / loop / campaign / connector E2E 事故、最长 drain 时长和 recommended next steps。 |
| `useReviewRuns` | 读取当前 review finding，P0/P1 open finding 会让工作台进入需处理状态。 |
| `useVerificationRuns` | 读取验证 plan / run / step，并提供“推荐验证”“运行验证”按钮。 |
| `useDomainQualityRuns` | 读取领域复核 run/check，并提供“运行领域复核”按钮。 |

状态语义：

- `danger`：存在 P0/P1 review finding、验证失败、领域复核 failed/blocked、Artifact Export Guard failed、Connector Action Guard failed、Operational Gate failed 或 Soak Report failed。
- `warn`：缺证据、缺来源、缺草稿、领域复核需要用户确认、Artifact Export Guard / Connector Action Guard / Operational Gate / Soak Report 证据不足或仍有未排空长任务。
- `good`：已有证据链且没有上述阻塞/缺口。
- `muted`：无痕会话、无 session 或尚未开始。

「真实样本验收」卡片是 Workspace 内的只读派生视图，不新增后端表：它从当前 session 的 domain evidence、Artifact Export Guard、Connector Action Guard、Operational Gate 与 Soak Report 计算覆盖领域数、控制面记录数、已排空 Workflow / Loop / Campaign 样本、Connector E2E evidence 数，以及事故、缺证据、缺排空、缺连接器 E2E、单领域覆盖等缺口。它回答“当前会话是否已经有真实场景样本可供验收”，不启动采样、不调用连接器、不修改 gate 结果。

用户可见动作：

- 「运行领域复核」调用既有 `run_domain_quality`。
- 「推荐验证」调用既有 `plan_smart_verification`。
- 「运行验证」调用既有 `run_smart_verification`。
- 「刷新工作台」同时刷新 domain evidence、Artifact Export Guard、Connector Action Guard、Operational Gate、Soak Report、review、verification 与 domain quality state。
- 「运行稳定性」卡片调用 `evaluate_domain_operational_gate({ sessionId, windowDays: 14 })`，显示 workflow 完成/活跃、loop 成功/失败、campaign 活跃和阻塞 check。
- 「长跑审计」卡片调用 `generate_domain_soak_report({ sessionId, windowDays: 14, maxItems: 8 })`，显示样本数、critical/warning incidents、最长 workflow drain、审批等待、恢复次数和最近事故建议；事故行的「转任务」按钮复用 `create_session_task`，把具体事故处理建议落成可跟踪 task。
- 「交付守门」在报告包含 artifact title / kind / path 时显示「复核产物」，调用既有 `run_domain_quality` 并带上 `sourceMetadata.sourceType="artifact_export_guard"`、artifact path / title / kind / guard status；这会创建可审计 Domain Quality run，不会自动创建 WorkflowRun、导出产物或访问连接器。
- 「交付守门」和「外部动作守门」的失败/样本不足 check 行提供「转任务」按钮；「交付守门」中需复核的 private / connector / sensitive / pending evidence 行也可「转任务」。这些动作复用 `create_session_task` 把具体守门缺口落成可跟踪 task；只创建用户可见待办，不自动复核产物、不自动批准外部动作、不修改 gate 结果。
- 「外部动作守门」卡片提供「批准动作」和「记录回滚」显式确认：前者写入 `user_decision` evidence + `explicitUserApproval/decision.approved` marker，后者必须填写回滚文本后才写入 `connector_context_collected` evidence + `rollbackPlan/canRollback` marker。两者只记录用户显式证据，不调用连接器、不执行外部修改。
- 「连接器 E2E」卡片提供「记录执行」和「记录复核」真实样本入口：用户必须填写执行结果或执行后读回复核文本，分别写入 `connector_action_executed` / `connector_action_verified` evidence。它只记录已经发生的外部动作结果，不发起连接器调用、不补造 result id、不把空样本算作通过；E2E check 行也可「转任务」，把缺执行结果、缺执行后复核、缺回滚等问题落入 TaskProgressPanel。
- Workflow 区块顶部的「自主推进就绪」卡片读取同一份通用任务工作台 state；当 Artifact Export Guard、Connector Action Guard、Operational Gate 或 Soak Report 非通过时，`查看交付` / `查看外部动作` / `查看稳定性` / `查看长跑` 会滚动并展开通用任务工作台，定位到交付守门、外部动作守门、运行稳定性和长跑审计证据。该入口只做定位，不自动刷新、不自动重试、不自动修改 gate 结果。
- 「下一步」列表的「转任务」按钮调用 `create_session_task`，把当前证据缺口或守门建议落成可见 session task；成功后由 `task_updated` 事件刷新 ChatInput 上方进度和 Workspace 进度区块。它只记录用户显式选择的待办，不自动运行 workflow、审批工具或访问连接器。
- Context Retrieval 候选行的「摘要」按钮调用既有 `record_domain_evidence`，把候选确定性整理为 `artifact_created` context summary evidence；成功后刷新推荐上下文和通用任务工作台。
- Context Retrieval 候选行的「确认」按钮调用 `create_owner_ask_user_question`，复用 ask_user UI 创建 durable 用户确认；用户回答后由 `respond_ask_user_question` 写入 `user_decision` evidence，回答内容并入 `sourceMetadata.answers`。
- Context Retrieval 候选行的「证据」按钮调用既有 `record_domain_evidence`，把候选来源/文档/会议/表格/决策等落成当前 session 的 domain evidence；成功后刷新推荐上下文和通用任务工作台。
- Context Retrieval 候选行的「冲突」按钮调用既有 `record_domain_evidence`，把候选落成 `claim_checked` evidence，并在 `sourceMetadata` 中标记 `action=mark_conflict` / `verdict=conflict` / `requiresUserReview=true`；成功后刷新推荐上下文和通用任务工作台。
- Context Retrieval 候选行的「转任务」按钮调用 `create_session_task`，把当前候选转成可见 task；成功后由 `task_updated` 事件刷新 ChatInput 上方进度和 Workspace 进度区块。

红线：

- 工作台只聚合 owner-plane 读模型和已有显式动作按钮；写路径仅限用户显式点击候选行「摘要」、「确认」、「证据」、「冲突」/「转任务」、工作台「下一步」/「转任务」、交付守门「复核产物」/「导出复核」/「可交付确认」/「脱敏复核」、外部动作守门「批准动作」/「记录回滚」、连接器 E2E「记录执行」/「记录复核」、长跑事故「转任务」、守门 check「转任务」或需复核 evidence「转任务」后记录当前 session evidence / task / quality run，不自动创建 WorkflowRun、不运行 loop、不 retry campaign、不访问连接器、不发送/分享/导出内容。
- 交付守门和外部动作守门的判定仍是只读结论；卡片里的显式确认只写 owner evidence 并触发重新评估。真正外部系统修改继续走 `permission::engine` strict approval、连接器授权和工具执行层。
- Operational Gate 与 Soak Report 仍是只读结论；它们可以提示 approve / retry / cancel / 等待排空，但不能自动执行这些控制动作。
- Incognito session 不持久化 domain evidence，工作台只显示禁用提示并清空 durable state。
- Review / Verification / Domain Quality 的 hook 状态在 Workspace 顶层共享给通用任务工作台和各自详细区块，避免同一面板重复请求同一批 run。

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
| `connector_context_collected` | Gmail / Calendar / Drive / Sheets / Feishu / Lark 等连接器读取或 deterministic fixture 上下文已收集。 |
| `connector_draft_created` | 外部系统修改前的草稿、预览或 proposed change 已生成并可展示。 |
| `connector_action_executed` | 外部连接器动作已执行，metadata 必须保留 connector / action / result id 或 status。 |
| `connector_action_verified` | 执行后已读回或复核外部系统状态。 |

`record_domain_evidence(input)` 要求 `goalId` 或 `sessionId`，并执行：

- session 必须存在且不是 incognito。
- 若传 `goalId`，session 从 goal 解析，避免跨 session 伪造 evidence。
- `sourceMetadata` 必须以 JSON object 存储；非 object 会包成 `{ value }`。
- `confidence` clamp 到 `[0,1]`。
- `accessScope` 归一为 `public | session | project | connector | private`。
- `redactionStatus` 归一为 `none | redacted | pending | sensitive`。
- 若关联 goal，会调用 `link_goal_target(goal_id, "domain_evidence", evidence_id, evidence_type, metadata)`。
- 成功写入后发 `domain_evidence:recorded` EventBus 事件，payload 只包含 `id/sessionId/goalId/projectId/domain/evidenceType/title/createdAt` 摘要，不广播完整 `summary` 或 `sourceMetadata`；Workspace Context 与通用任务工作台监听该事件刷新。

Goal evidence relation 白名单已加法扩展这些通用 evidence type；coding evidence relation 保持原样。

Workflow runtime 也提供脚本内 sugar：`workflow.evidence.record({ domain, evidenceType, title, summary?, sourceMetadata?, confidence?, accessScope?, redactionStatus? })`。该 API 复用 `record_domain_evidence`，但 scope 由 runtime 强制改写为当前 workflow 的 `session_id`、绑定 `goal_id` 和 session project，脚本不能跨 session / goal / project 写 evidence。写入时会在 `sourceMetadata.workflow` 追加 `runId`、`opKey`、`sessionId`、`goalId`、`executionMode`，用于 Goal detail、Context Retrieval 和后续 Domain Quality 追溯来源。

## Artifact Export Guard

Phase 7.15 新增 `evaluate_domain_artifact_export_guard(input)`，用于最终发送、分享、导出、发布前的只读门禁。它只读 `domain_evidence_items`，不调用 LLM、不访问连接器、不创建文件、不执行外部动作。

输入要求 `sessionId` 或 `goalId`；若传 `goalId`，session 从 goal 解析并校验，避免跨 session 伪造。incognito session fail closed。可选 `domain` 过滤同一领域 evidence；可选 `artifactPath/title/kind` 进入 report 展示和 Workspace「复核产物」入口，但 Artifact Export Guard 自身仍只读 evidence，不把这些字段当作授权条件。

默认阈值：

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `requireArtifactCreated` | `true` | 必须存在 `artifact_created` evidence。 |
| `requireArtifactReviewed` | `true` | 必须存在 `artifact_reviewed` evidence。 |
| `maxSensitiveUnreviewed` | `0` | private / connector / sensitive / pending / redacted evidence 如果没有显式 export review，不允许放行。 |
| `maxRedactionPending` | `0` | `redactionStatus=pending|sensitive` 默认阻断。 |

判定规则：

- `artifact_created` / `artifact_reviewed` 缺失时返回 `insufficient_data`，提醒用户补证据。
- `accessScope=private|connector` 或 `redactionStatus=sensitive|pending|redacted` 会进入 `evidenceRequiringReview`。
- 敏感 evidence 只有在同 scope 内存在 `artifact_reviewed`，且其 `sourceMetadata.exportReview=true`、`exportReady=true` 或 `redactionChecked=true` 时，才算完成导出复核。
- `pending|sensitive` 脱敏状态默认直接 `failed`；`redacted` 不算待脱敏，但仍要求显式导出复核。
- 输出 `status=passed|failed|insufficient_data`、`checks[]`、`blockers[]`、`recommendedNextSteps[]`、summary 计数和最多 12 条需复核 evidence。

GUI 上，Workspace「领域复核」区块内新增「交付守门」卡片，自动随会话加载、回合结束和手动刷新更新。用户无需记命令即可看到最终产物是否已创建、是否复核、是否存在敏感来源或待脱敏证据；失败或样本不足的 check 行、以及需复核 evidence 行都可显式「转任务」，把缺复核、缺产物、敏感来源或脱敏缺口落入 TaskProgressPanel 追踪。

同一卡片提供三种 owner-side 显式确认：「导出复核」、「可交付确认」、「脱敏复核」。点击后调用既有 `record_domain_evidence` 写入 `artifact_reviewed` evidence，`sourceMetadata.sourceType="artifact_export_guard_confirmation"`，并只写入用户点击的 marker：`exportReview=true`、`exportReady=true` 或 `redactionChecked=true`。这不是新的授权平面，也不会静默修改原 evidence 的 `redactionStatus`；`pending|sensitive` 仍由 Artifact Export Guard 重新计算并保持 fail closed。

## Connector Action Guard

Phase 7.16 新增 `evaluate_domain_connector_action_guard(input)`，用于 Gmail / Calendar / Drive / Sheets / Feishu / Lark / Slack / Notion / Jira / GitHub / Linear 等连接器的真实外部修改动作前置审查。它同样只读 `domain_evidence_items` 和 Artifact Export Guard 报告，不调用 LLM、不访问连接器、不发送邮件、不改日历、不分享文档、不更新外部记录。

输入要求 `sessionId` 或 `goalId`；若传 `goalId`，session 从 goal 解析并校验。incognito session fail closed。可选 `toolName` 会通过 `permission::engine::classify_external_connector_action` 识别内置 Feishu 写动作和保守 MCP mutating tool 名；也可显式传 `connector` / `action`。可选 `domain` 用于过滤 evidence。

默认阈值：

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `requireExplicitApproval` | `true` | 必须存在 `message_draft_approved` / `user_decision`，或 evidence metadata 中的 `explicitUserApproval` / `approved` / `decision.approved`。 |
| `requireRollbackPlan` | `true` | 必须存在 `rollbackPlan` / `undoPlan` / `recoveryPlan` / `canRollback`，让用户知道出错后怎么恢复。 |
| `requireExportGuardForDelivery` | `true` | 对 send / reply / forward / share / publish / export / upload / submit 等交付类动作，要求 Artifact Export Guard 通过。 |

判定规则：

- `action_scope` 要求能识别工具名、connector/action 或至少一条带 `requestedAction` / `externalAction` / `toolName` / `connector` / `highRiskAction` 的 evidence。
- `explicit_user_approval` 缺失直接 `failed`，因为真实外部系统修改不能只靠模型自判。
- `rollback_plan` 缺失返回 `insufficient_data`，提示补充撤销、修正或恢复路径。
- 交付类动作会嵌套调用 Artifact Export Guard；若最终产物、复核、脱敏或敏感来源未过关，则本 guard 同步阻断。
- 输出 `status=passed|failed|insufficient_data`、`checks[]`、`blockers[]`、`recommendedNextSteps[]`、summary 计数和最多 12 条相关 evidence。

执行层也接入同一分类器：`permission::engine` 对外部连接器写动作返回 strict `AskReason::ExternalConnectorAction`，禁止 AllowAlways，Smart judge 不得覆盖，IM/skill `auto_approve_tools` 和 trusted MCP `autoApprove` 也不能静默绕过；只有已经在外层审批过的后台重入 `external_pre_approved` 可以跳过重复弹窗。YOLO 仍按系统既有语义放行，但写 `app_warn(permission/yolo_bypass)`。

GUI 上，Workspace「领域复核」区块内新增「外部动作守门」卡片，自动随会话加载、回合结束和手动刷新更新。用户能看到动作证据、批准证据、回滚提示、敏感来源计数、阻塞 check 和相关 evidence；失败或样本不足的 check 行可显式「转任务」，把缺批准、缺回滚或交付守门未过关等问题落入 TaskProgressPanel 追踪。卡片也能显式记录用户批准和回滚方案，其中回滚方案必须由用户填写文本；成功后刷新 Guard / E2E Gate。真正执行外部修改前仍会逐次弹出审批。

## Connector E2E Gate

Phase 8.2 新增 `evaluate_domain_connector_e2e_gate(input)`，用于真实连接器场景的端到端验收。它仍是 owner-plane 只读 gate：不调用 LLM、不访问连接器、不发送邮件、不改日历、不分享文档、不更新外部记录；真实账号动作可以把结果写成 evidence，deterministic/mock fixture 也可以写成同样结构的 evidence，但没有证据时绝不伪装成通过。

输入支持 `goalId` / `sessionId` / `projectId` / global scope，并可选 `domain`、`toolName`、`connector`、`action`。session / goal scope 会校验 session 存在且非 incognito；global / project scope 只做 evidence 聚合，不嵌套运行 Connector Action Guard，因此通常会保持 `insufficient_data`，用于 Dashboard 总览“最近是否已有足够证据”而不是动作授权。具体 session / goal scope 下会复用 `evaluate_domain_connector_action_guard`，交付类动作也会继续要求 Artifact Export Guard 通过。

默认阈值：

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `requireConnectorInput` | `true` | 必须存在连接器输入证据，例如 `accessScope=connector`、`connector` / `accountId` / `externalSource` metadata。 |
| `requireDraft` | `true` | 必须存在草稿或预览证据，例如 `connector_draft_created`、`message_draft_approved`，或带 `draftCreated` / `previewReady` 的 `artifact_created`。 |
| `requireExplicitApproval` | `true` | 必须存在用户明确批准；缺失直接 `failed`。 |
| `requireExecutionResult` | `true` | 必须存在 `connector_action_executed` 或带 `execution/result/resultId/messageId/eventId/fileId/status` 的执行结果 metadata。 |
| `requirePostActionVerification` | `true` | 必须存在 `connector_action_verified` 或带 `verification.passed` / `externalStateVerified` / `postActionReview` 的复核 evidence。 |
| `requireRollbackPlan` | `true` | 必须存在 rollback / undo / recovery plan。 |
| `requireExportGuardForDelivery` | `true` | send / reply / forward / share / publish / export / upload / submit 等交付类动作必须通过 Artifact Export Guard。 |

判定规则：

- `connector_input` / `draft_or_preview` / `action_execution` / `post_action_verification` / `rollback_plan` 缺失时为 `insufficient_data`，表示不能声称完成真实 E2E。
- `explicit_user_approval` 缺失为 `failed`，因为外部系统修改不能由模型自行授权。
- `connector_action_guard` 在 session / goal scope 下必须 passed；global / project scope 下会显示 `not_evaluated_without_session_or_goal` 并保持 `insufficient_data`。
- 交付类动作的 `artifact_export_guard` failed 会同步 failed；未评估或证据不足为 `insufficient_data`。
- 输出 `summary`、`checks[]`、`blockers[]`、`recommendedNextSteps[]` 和最多 16 条相关 evidence，覆盖输入、草稿、批准、执行、复核、回滚和敏感来源。

Dashboard Learning 新增「Connector E2E」卡片，展示 IN / DR / OK / EX / VF / RB / GU 等计数与 guard 状态。它回答“最近有没有足够证据证明真实连接器链路跑完”，不替代 Workspace 内的逐次工具审批，也不替用户执行外部动作。

Workspace「通用任务工作台」也会在当前 session scope 下调用同一个 Gate，并展示「连接器 E2E」卡片。用户能直接看到连接器输入、草稿、批准、执行结果、执行后复核、回滚、下层 Action Guard / Export Guard 状态、阻塞 check 和相关 evidence；执行结果与执行后复核可由用户显式填写并写回标准 evidence，随后刷新 E2E Gate / 真实样本验收 / Soak Report 输入。Autonomous Readiness 会把该 gate 的 failed / insufficient_data 纳入整体健康状态，并提供「查看 E2E」快捷入口。普通无连接器动作的会话不会因为全局缺样本而被强行标红，只有已有连接器证据、动作、toolName、connector 或 action scope 时才把 E2E 缺口推给用户处理。

## Context Retrieval 衔接

Phase 7.3 起，`ha-core::context_retrieval` 会只读消费本模块的数据：

- 从 `workflow_runs.kind = domain:<domain>`、`domain_evidence_items.domain`、显式 `domain/templateId` 或 Goal objective / criteria 推导 `domainContext`。
- 把 `domain_evidence_items` 转成 document、email_thread、calendar_event、sheet_range、knowledge_note、web_source、decision、artifact 等候选。
- 按 required evidence、Goal criteria、confidence、redaction status 和 query boost 加权排序。
- 缺少连接器或 required evidence 时返回 `accessIssues[]`，只提示缺口，不伪造来源。
- Workspace Context 区块展示 domain profile、access issue 与 domain action chips；“复制引用”、“生成摘要”、“请求用户确认”、“加入证据”、“标记冲突”和“转任务”已落地。
- Goal detail 会把 `sourceType=domain_evidence` 的证据单独分到「领域证据」区块，展示 source、confidence、access scope、connector/account、redaction status、导出前复核提示与 workflow run/op provenance，避免非 coding 证据淹没在 validation / diff / task 证据里。

Context Retrieval 查询本身仍是只读 owner-plane 查询，不创建 workflow run、不写 evidence、不访问连接器；只有用户在 GUI 候选行显式点击「摘要」、「确认」、「证据」、「冲突」或「转任务」时，才会分别调用 owner action 写入当前 session evidence / task。确认动作创建的是 owner-side ask_user，不需要 live 模型工具 receiver；带 `ownerResponse` 的 pending question 可跨会话切换和重启保留，普通工具型 ask_user 的 zombie 清理规则不变。

## Domain Quality 衔接

Phase 7.4 起，[Domain Quality 控制平面](domain-quality.md) 会消费本模块的 template 和 evidence：

- `requiredEvidence` 变成阻塞 / 建议 check，缺少必需 evidence 会写 `domain_quality_blocked` / `domain_quality_check` Goal evidence。
- `approvalGates` 变成高风险动作确认门；只有当输入声明 `requestedAction` 或 `highRiskAction=true` 时才强制 `needs_user`。
- `verificationPolicy` 当前通过内置 domain profile 的确定性规则落地，后续可扩展成更细的 profile。
- Domain Quality run / stats / event 会保留 template id 与 version；未显式指定 template/domain 时优先使用 active Goal 绑定的 template version。
- Workspace 新增「领域复核」区块，非 coding 会话不需要工作目录也能运行质量门。

Domain Workflow 仍只负责模板、draft 和 evidence；Domain Quality 负责 review / verification 结论，两者不互相替代。

## Domain Learning / Eval 衔接

Phase 7.5-7.6 不让 Domain Workflow 直接学习或评分，而是通过已持久化事实接入后续控制面：

- Domain Learning 从 Domain Quality run/check 和 domain evidence 生成 draft-only proposal，proposal 必须继续走 preview / apply draft / explicit promotion。
- Domain Eval 读取同 session/domain 的 Goal、Workflow trace、Domain Evidence 与 Domain Quality snapshot 做 deterministic scoring，并把结果写入 `domain_eval_runs`。
- Dashboard Learning 同时展示 coding release/generalization gate 与独立的 general domain quality gate，二者不混排、不生成综合分。

这保证通用场景可以沉淀经验和评测能力，同时不会让模板 registry 自行修改生产规则，也不会把 non-coding 评分混进 coding benchmark。

## Owner API

Tauri / HTTP / transport 均已注册：

| Tauri Command | HTTP | 说明 |
| --- | --- | --- |
| `list_domain_workflow_templates` | `POST /api/domain-workflows/templates` | 列出内置 + 自定义 template，可按 domain/task/project 过滤。 |
| `save_domain_workflow_template` | `POST /api/domain-workflows/templates/save` | 显式保存用户/项目 template；必须 `explicitSaveConsent=true`。 |
| `preview_domain_workflow` | `POST /api/domain-workflows/preview` | 生成 workflow draft 和 Script Gate / permission preview。 |
| `record_domain_evidence` | `POST /api/domain-evidence/record` | 写入通用 evidence，并可链接到 Goal。 |
| `list_domain_evidence` | `POST /api/domain-evidence` | 按 goal/session/project/domain/type 列出 evidence。 |
| `evaluate_domain_artifact_export_guard` | `POST /api/domain-artifact-export-guard/evaluate` | 只读评估最终交付是否具备产物、复核和脱敏证据。 |
| `evaluate_domain_connector_action_guard` | `POST /api/domain-connector-action-guard/evaluate` | 只读评估真实外部连接器动作是否具备动作、批准、回滚和交付守门证据。 |
| `evaluate_domain_connector_e2e_gate` | `POST /api/domain-connector-e2e-gate/evaluate` | 只读评估真实连接器 E2E 是否具备输入、草稿、批准、执行结果、执行后复核、回滚和交付守门证据。 |

EventBus 事件：

| 事件名 | 触发点 | Payload 关键字段 |
| --- | --- | --- |
| `domain_evidence:recorded` | `record_domain_evidence` 成功写入后 | `{ id, sessionId, goalId?, projectId?, domain, evidenceType, title, createdAt }` |

## 红线

- 不扩大权限：template 只描述推荐工具和审批门，不赋予连接器权限。
- 不自动执行：preview 不创建 run、不运行脚本、不访问网络、不发邮件、不改日历、不写外部系统。
- 不污染全局 prompt：domain hints 只进入 workflow draft 的动态 payload。
- 不写无痕：incognito session 不可 preview durable domain workflow，也不可记录 domain evidence。
- 不自动交付/修改外部系统：Artifact Export Guard、Connector Action Guard 与 Connector E2E Gate 只给出门禁结论；真正发送邮件、改日历、分享文档、更新表格或外部业务记录仍必须走工具审批和连接器授权。
- 不伪造真实连接器 E2E：没有外部输入、执行结果或执行后复核 evidence 时，Connector E2E Gate 只能 `insufficient_data`，不能因为 deterministic/mock 路径存在就当真实账号通过。
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
- Artifact Export Guard 在产物、复核、敏感来源导出复核齐全时通过；缺少复核且存在 pending connector evidence 时阻断。
- Connector Action Guard 在动作、用户批准、回滚和交付复核齐全时通过；缺少显式批准时阻断。
- Connector E2E Gate 在连接器输入、草稿、用户批准、执行结果、执行后复核、回滚和交付复核齐全时通过；缺执行结果时保持 `insufficient_data`，不伪装成通过。
- Workflow runtime 可通过 `workflow.evidence.record` 写入通用 evidence，并保留 run/op provenance。

# 通用场景层与 Domain Workflow 路线图

> 返回 [路线图索引](README.md)
>
> 更新时间：2026-07-04
>
> 状态：Phase 7.1 Domain Workflow Registry、Phase 7.2 General Evidence Model、Phase 7.3 Domain Context Retrieval、Phase 7.4 Domain Verification & Review、Phase 7.5 Domain Learning Loop、Phase 7.6 General Eval & Quality Gate、Phase 7.7 Domain Eval Calibration、Phase 7.8 Domain Eval Fixture Runner、Phase 7.9 Domain Eval Agent Fixture Execution、Phase 7.10 Domain Fixture / Smoke Run Center、Phase 7.11 Domain Eval Campaign Runner、Phase 7.12 Domain External Campaign & Leaderboard、Phase 7.13 Domain Campaign Learning Closure、Phase 7.14 Domain Readiness Gate、Phase 7.15 Domain Artifact Export Guard、Phase 7.16 Domain Connector Action Guard 已完成第一版；Phase 8.1 Domain Operational Gate、Phase 8.2 Connector E2E Gate、Phase 8.3 Domain Soak Report 已完成第一版。相关事实已分别沉淀到 [Domain Workflow 控制平面](../architecture/domain-workflow.md)、[Context Retrieval v2](../architecture/context-retrieval.md)、[Domain Quality 控制平面](../architecture/domain-quality.md)、[Coding Improvement Loop](../architecture/coding-improvement-loop.md) 与 [Domain Eval 与 Quality Gate 控制平面](../architecture/domain-eval.md)。

## 1. 背景

Hope 现在已经把长任务底座做出来了：Goal 负责最终目标和完成标准，Mode 负责推进强度，Workflow 负责一次可观察、可恢复、可审批的执行，Task 负责用户可见进度，Loop 负责定时/重复/条件触发。Coding-first 阶段用 worktree、LSP、review、verification、context retrieval、eval 和 benchmark 把这套底座压实。

P6 完成后，下一步不应该再继续只堆 coding-specific 能力，而应该把同一套控制平面抽到通用场景层：

```text
通用 Agent 控制平面
  -> coding-first 深水验证
  -> 真实能力 benchmark
  -> 通用 domain workflow 产品化
```

这里的 domain workflow 不是硬编码流程图，也不是替代模型判断。它是一组可审计、可调整、可验证的领域工作习惯：告诉模型在某类任务里哪些步骤不能漏、哪些证据必须留、哪些风险要停下来问用户、哪些输出需要复核。

## 2. 目标

通用场景层要解决四件事：

1. **把已有控制平面用于非编程任务**：研究、写作、数据分析、会议准备、知识库整理、邮件/日程处理、项目运营等。
2. **降低自由决策的不稳定性**：模型仍可动态编排，但关键步骤、证据、验证、用户确认不能靠临场发挥。
3. **让 GUI 体验不依赖 slash 命令**：用户可以从任务类型进入，有可见进度、证据、风险、下一步和完成审计。
4. **让学习闭环可泛化**：非编程任务也能沉淀 workflow、guidance、skill、eval case，而不是只服务 coding。

## 3. 非目标

- 不做一个巨大的固定 DSL，把所有任务都变成死流程。
- 不让 domain workflow 自动越权调用 Gmail、Calendar、Drive、Web 或本地文件；连接器和敏感动作仍需显式授权。
- 不把某个领域模板写进 system prompt 变成全局规则；模板必须按任务/会话/用户选择动态启用。
- 不把 coding benchmark 结果扩展解释成通用能力已经达标；通用场景需要自己的 eval。
- 不默认发送邮件、改日历、分享文档、删除文件或提交外部表单。

## 4. 通用能力边界

已经可以复用的通用底座：

| 能力 | 通用价值 |
| --- | --- |
| Goal | 长任务目标、完成标准、预算、证据、最终审计。 |
| Mode | 控制主动性和深度：保守、深入、自主。 |
| Workflow | 任务编排、审批、trace、恢复、暂停、取消、repair。 |
| Task | 用户可见进度事实。 |
| Loop | 定时、重复、轮询、条件触发。 |
| Context Retrieval | 可扩展为“当前任务最该看的资料和证据”。 |
| Review / Verification 模式 | 可泛化为“产物复核”和“最小验证”。 |
| Learning Loop | 可泛化为通用 workflow / guidance / skill / eval 草稿沉淀。 |

仍然偏 coding-specific 的能力：

| 能力 | 边界 |
| --- | --- |
| Worktree | 主要服务代码隔离。通用场景可类比为 artifact workspace，但不是同一个概念。 |
| LSP / Diagnostics | 代码语义能力。 |
| Coding Eval / Benchmark | 只证明 coding 场景，不能代表通用任务质量。 |
| Code Review Engine | 当前审代码 diff；未来可扩展为文档/数据/邮件 review，但需要新 profile。 |

## 5. Domain Workflow 模型

每个 domain workflow 应该是一个 manifest + 可生成的 workflow draft，而不是固定脚本：

```text
DomainWorkflow
  id
  title
  domain
  task_types
  default_mode
  required_evidence
  recommended_tools
  approval_gates
  verification_policy
  stop_conditions
  output_contract
  eval_criteria
  prompt_hints
```

关键原则：

- 模型可以动态调整步骤，但不能静默跳过 required evidence、approval gate、verification policy。
- 每个模板都要能生成 `workflow.js` draft，由用户预览/批准后执行。
- 模板启用后要写入 workflow trace，最终能解释“为什么这么做”。
- 领域规则只进入动态上下文，不破坏通用 system prompt cache。
- 模板版本化：改模板后不覆盖历史 run 的解释和审计。

## 6. 优先领域

第一批通用领域不追求大而全，优先选“长任务明显、证据重要、验证可定义、GUI 有价值”的场景：

| 优先级 | 领域 | 典型任务 | 关键证据 | 关键验证 |
| --- | --- | --- | --- | --- |
| P0 | Research / 调研 | 市场调研、技术调研、竞品分析 | 来源、时间、可信度、冲突点 | 引用审计、交叉验证、时效检查 |
| P0 | Writing / 报告写作 | 决策 memo、周报、PRD、方案文档 | 大纲、来源、用户要求、版本 | 结构检查、引用检查、读者适配 |
| P0 | Data Analysis / 数据分析 | 指标诊断、报表、KPI readout | 数据源、口径、样本、查询 | 数据质量、计算复核、图表审查 |
| P1 | Meeting Prep / 会议准备 | 会前 brief、议题梳理、风险清单 | 日历、材料、历史决策、待办 | 参会人/时间核对、材料完整性 |
| P1 | Knowledge Curation / 知识整理 | 资料归档、知识空间整理、主题索引 | 文件、笔记、标签、引用关系 | 去重、缺口检查、链接有效性 |
| P1 | Inbox / Comms | 邮件分类、回复草稿、跟进清单 | 邮件线程、联系人、附件 | 发送前确认、语气/事实复核 |
| P2 | Project Ops | 项目计划、风险跟踪、状态更新 | 任务、目标、会议、文档 | deadline、owner、依赖和阻塞检查 |

## 7. Phase 7 路线

### Phase 7.1 Domain Workflow Registry（已完成第一版）

目标：建立通用 domain workflow 的注册、选择、版本和预览机制。

已完成：

- 新增 `ha-core::domain_workflow` manifest schema、代码内置 registry 与用户/项目自定义模板表。
- 内置 Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation、Inbox、Project Ops 七类模板。
- 模板可生成 `workflow.js` draft，并复用既有 Script Gate 与 permission preview。
- Tauri / HTTP / Transport 已注册 `list_domain_workflow_templates`、`save_domain_workflow_template`、`preview_domain_workflow`。
- 模板版本、启用范围、默认 mode、推荐工具、required evidence、approval gates、verification policy、stop conditions、output contract 可见。
- Workspace / Workflow Control Center 的 Goal 创建 / 编辑与“新建工作流”已接入领域模板选择器。Goal 可持久绑定 domain/template/task type，workflow 创建器会继承该推荐并可直接生成领域草稿、显示证据/审批/验证摘要，复用标准预检与创建链路。

验收：

- Owner API 与 GUI 都可列出非 coding 场景模板并生成 workflow draft；用户不需要记模板 id 或 slash 参数。
- 同一目标可选择自由编排或 domain workflow draft，执行时仍落既有 WorkflowRun / Task / Goal 链路。
- 模板 preview 不创建 run、不执行脚本，不绕过权限、审批、连接器授权或 incognito 红线。

### Phase 7.2 General Evidence Model（已完成第一版）

目标：把 Goal evidence 从 coding evidence 扩展为通用证据模型，支持来源、引用、用户决策、数据口径、产物版本和验证结果。

已完成：

- 定义并持久化通用 evidence 类型：`source_cited`、`claim_checked`、`user_decision`、`artifact_created`、`artifact_reviewed`、`data_quality_checked`、`citation_audited`、`message_draft_approved`、`meeting_context_collected`。
- 新增 `domain_evidence_items` 表和 owner API：`record_domain_evidence` / `list_domain_evidence`。
- Workflow runtime 已支持 `workflow.evidence.record(...)` 脚本内 sugar，scope 绑定当前 session / workflow goal / project，并把 run/op provenance 写入 `sourceMetadata.workflow`。
- Goal evidence relation 白名单扩展到通用 evidence，记录时可通过 `goal_links` 进入 Goal snapshot。
- Evidence 支持 source metadata、confidence、access scope、redaction status。
- Goal detail GUI 已有「领域证据」分组，展示 domain evidence 的来源、置信度、access scope、connector/account、redaction status、导出前复核提示与 workflow run/op provenance。
- 后续 Phase 7.15 / 7.16 已把独立交付导出与真实外部连接器动作接入守门：敏感来源、待脱敏证据、用户显式批准、回滚计划和交付复核都能在 Workspace 中可见。
- Incognito session fail-closed；goal/session 关联路径避免跨 session 伪造 evidence。

验收：

- 非 coding workflow 已能通过 owner API 或 `workflow.evidence.record(...)` 写通用 evidence 证明完成标准，而不是只靠最终文本。
- Evidence item 已保存来源 metadata、confidence、access scope、redaction status。
- 无痕会话不会持久化 domain evidence。

### Phase 7.3 Domain Context Retrieval（已完成第一版）

目标：把 Context Retrieval 从 coding 信号扩展到通用资料推荐，回答“这个任务下一步最该看哪些资料、来源、线程、会议、表格、笔记”。

已完成：

- 新增 domain-aware context candidate 类型：document、email_thread、calendar_event、sheet_range、knowledge_note、web_source、decision、artifact、task。
- 根据 domain workflow 和 goal criteria 排序候选，而不是只按关键词。
- 支持来源可信度、时效、权限、重复、冲突提示。
- GUI 候选行展示领域动作：引用、加入 evidence、生成摘要、请求用户确认、标记冲突、转 task；其中“复制引用”已作为真实轻量动作落地，其余通过 `metadata.domainActions` 暴露给后续 owner action。
- 连接器缺失时显示 access issue，不伪造上下文。
- 无工作目录的非 coding 会话仍可展示 Goal / Task / Workflow / Domain evidence / URL 候选，只跳过 workspace 信号。

验收：

- Research / Writing workflow 能看到来源和引用候选，缺少 required evidence 时显示 access issue。
- Meeting Prep workflow 能看到会议材料、日历上下文和历史决策类候选；缺少 calendar evidence 时显示 access issue。
- Data Analysis workflow 能看到数据源、查询结果、口径说明和数据质量 issue；缺少 sheet/data quality evidence 时显示 access issue。
- 新增单测覆盖 research domain evidence 召回 web source 和 required evidence 缺口。

### Phase 7.4 Domain Verification & Review（已完成第一版）

目标：把“验证”从代码检查扩展成领域质量检查，让报告、分析、会议 brief、邮件草稿和知识整理都有最小复核路径。

已完成：

- 新增 `ha-core::domain_quality` durable 控制面：`domain_quality_runs` / `domain_quality_checks` / `domain_quality_events`。
- Research verification 已检查 source count、claim check、citation audit 与来源日期 / 时效 metadata。
- Writing review 已检查 draft artifact、audience / requirement review，并提示术语 / 读者适配 / 引用缺口 advisory。
- Data verification 已检查 data quality evidence、metric interpretation、dataset / denominator / sample metadata。
- Meeting prep review 已检查 meeting context、brief / agenda，并提示 decision points / risks / unread materials advisory。
- Inbox review 已检查 thread source、facts / commitments、send 前 approval。
- Knowledge Curation 与 Project Ops 也有 source / dedupe / artifact / owner / risk 等基础 profile。
- 高风险动作通过 `sourceMetadata.requestedAction` 或 `highRiskAction=true` 触发 `needs_user`，缺少 `explicitUserApproval` 时 fail closed。
- Domain quality 结果写回 Goal evidence：`domain_quality_passed` / `domain_quality_blocked` / `domain_quality_failed` / `domain_quality_needs_user` / `domain_quality_check`。
- Workspace 新增「领域复核」区块，展示通过、缺失、需确认、建议项，可直接运行 / 刷新，不依赖工作目录。

验收：

- 非 coding 产物能进入 Workspace「领域复核」区块，不再只展示代码结果。
- 关键高风险动作必须用户确认，尤其是发送、分享、修改外部系统；当前通过 `needs_user` run state 和 Goal blocking evidence 表达。
- Domain verification 失败会阻止 Goal completed：`domain_quality_blocked/failed/needs_user` 与 P0/P1 `domain_quality_check` 都进入 Goal blocker，后续 `domain_quality_passed` 可解除较早阻塞。
- 最终架构见 [Domain Quality 控制平面](../architecture/domain-quality.md)。

### Phase 7.5 Domain Learning Loop（已完成第一版）

目标：把通用任务中的成功/失败沉淀为 workflow、guidance、skill、eval 草稿，让通用场景也能持续变强。

已完成：

- `generate_coding_improvement_proposals()` 与 `distill_coding_improvement_proposals()` 已读取当前 scope 内的 Domain Quality run/check snapshot。
- Proposal kinds 已扩展：`domain_workflow_template`、`domain_guidance`、`domain_review_profile`、`domain_eval_case`、`connector_usage_pattern`。
- 成功的 Domain Quality run 会生成 workflow / guidance 草稿；blocked / failed / needs_user run 会生成 review profile / eval case；高风险 approval 卡点会生成 connector usage pattern。
- Draft apply / promotion 复用现有 Coding Improvement 安全链路：先预览、再生成 `.hope-agent/coding-improvement/` 草稿，只有显式 promotion 才进入 promoted domain workflow / guidance / review profile / eval case / connector pattern。
- Workspace proposal 列表已显示领域类 proposal 的中文标签。
- Workspace「领域复核」区块已支持从当前 Domain Quality run 直接点击「提炼经验」，通过 `sourceType="domain_quality"` + `sourceId=<run_id>` 定向生成学习 proposal，不再只能走泛化 proposal 入口。
- Dashboard Learning 已增加 General domain trends 历史趋势区块：按领域展示完成率、blocked/failed/needs_user、approval blocker、domain eval pass rate / average score、学习 proposal 草稿/晋升、top blocker reason 和 recent quality runs。
- 新增回归测试覆盖 Research / Writing / Data Analysis / Inbox quality run 生成学习 proposal、apply 草稿和 promotion preview。

验收：

- Research / Writing / Data Analysis 三类任务已能从 `completed` Domain Quality run 生成可预览的 workflow / guidance proposal。
- Inbox 高风险发送类任务已能从 approval 卡点生成 `connector_usage_pattern`。
- 已应用草稿必须显式 promotion 才能成为正式 domain workflow、guidance、review profile、eval case 或 connector pattern。
- 学习不会跨越用户/项目/连接器权限边界；incognito session 仍拒绝 domain quality / proposal 持久化。

### Phase 7.6 General Eval & Quality Gate（已完成第一版）

目标：建立非 coding 场景的 eval 和质量门禁，避免通用能力只靠感觉。

已完成：

- 新增 `ha-core::domain_eval`，独立于 coding eval / benchmark。
- 建立首批 15 个通用 eval tasks：Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation 各 3 个。
- 每个 task 已定义输入 prompt、允许工具、required evidence、成功标准、禁止行为、built-in calibration 记录。
- `run_domain_eval_task` 复用 Goal、Workflow、Domain Evidence、Domain Quality trace 做 deterministic scoring。
- 建立 `domain_eval_runs` history，和 `coding_eval_runs` 物理分表。
- 建立通用 quality gate：evidence completeness、citation quality、data quality、approval safety、completion criteria match、workflow trace、domain coverage。
- Dashboard Learning 增加「General domain trends」历史趋势区块和「General domain quality」Gate 区块：前者显示完成率、blocked 原因、用户确认卡点、eval pass rate、average score、学习候选和最近 quality runs；后者显示 gate 三态、quality blockers、domain coverage 与最近 eval run，不与 coding benchmark 混排。
- 已晋升的 `domain_eval_case` proposal 可通过 owner API / Workspace 质量趋势卡片显式导入 `domain_eval_tasks`，后续被 `list_domain_eval_tasks` / `run_domain_eval_task` 使用；重复导入默认幂等。

验收：

- 15 个通用 eval tasks 已可 deterministic trace scoring。
- Eval 已能发现无来源结论、漏用户确认、数据口径不明、缺 completion criteria / quality trace / workflow trace 等关键失败。
- 通用 eval 和 coding benchmark 已分表、分 API、分 Dashboard 区块展示，避免伪综合分。

### Phase 7.7 Domain Eval Calibration（已完成第一版）

- 已新增 `domain_eval_calibrations`，记录 user/project scope 的人工校准与复核结论。
- `list_domain_eval_tasks` 与 `run_domain_eval_task` 会把相关 calibration 附加到 task/report，避免内置 rubric 被误当成已校准能力证据。
- Dashboard Learning 的 General domain quality 卡片显示已校准 task 数，并允许对最近 eval run 点击「Mark reviewed」写入幂等 calibration。

### Phase 7.8 Domain Eval Fixture Runner（已完成第一版）

- 已新增 `run_domain_eval_fixture` owner API，Tauri / HTTP / transport 已接通。
- `executionMode="trace_fixture"` 会 materialize 真实 session / Goal / Evidence / WorkflowRun / Domain Quality run，再调用 `run_domain_eval_task` 写入 `domain_eval_runs`。
- Fixture checks 支持 expected status、最低 score、指定 scorer check 通过/失败断言。
- 第一版仅作为 owner API / 回归测试能力，不直接挂在 Dashboard quality gate 上，避免合成 smoke 样本污染真实质量判断。

### Phase 7.9 Domain Eval Agent Fixture Execution（已完成第一版）

- `run_domain_eval_fixture` 已支持 `executionMode="agent"`，创建真实 user message + chat turn 并调用 `run_chat_engine`。
- Agent fixture 必须显式传 `execution.providers` / `execution.modelChain`；owner API 不隐式读取桌面全局 provider。
- Agent fixture 默认 `execution.workflowMode="ultracode"`，让模型能在通用任务中自主判断是否创建 durable workflow；也可显式设为 `on` / `off`。
- `report.execution` 记录 status、turnId、response/error、modelUsed、toolCalls 和 workflowMode；checks 支持 execution status、turn、tool call、response/error 断言。
- Agent fixture 不自动写入 `fixture.evidence` / `fixture.workflow`，避免模型未产出真实 evidence/workflow trace 时被确定性种子托过关。
- 执行失败或缺 provider/modelChain 时不写 `domain_eval_runs`；执行完成但证据不足时仍由同一 scorer 标记 failed / insufficient_data。
- 定向测试使用本地 mock Responses provider 覆盖真实 chat engine 路径，不访问外部网络。

### Phase 7.10 Domain Fixture / Smoke Run Center（已完成第一版）

- Fixture runner 创建的 session 已标记为 `SessionKind::EvalFixture`，从普通会话列表、普通搜索和 live Dashboard 聚合中隔离。
- `domain_eval_runs` 新增 `source_type`，普通 eval 默认为 `live`，fixture trace/agent 分别为 `fixture_trace` / `fixture_agent`。
- 新增 `domain_eval_fixture_runs`，持久化完整 fixture report；即使 agent 配置失败、没有写 eval run，也能在 Smoke Run Center 看到失败原因。
- `list_domain_eval_runs` 与 `evaluate_domain_quality_gate` 默认排除 synthetic，`includeSynthetic=true` / `sourceType="fixture"` 才用于诊断。
- Dashboard Learning 新增「Domain smoke runs」卡片，展示最近 fixture run、pass rate、agent/trace 数、失败数、eval/quality/workflow/turn trace badge 与 error。

### Phase 7.11 Domain Eval Campaign Runner（已完成第一版）

- 新增 `domain_eval_campaigns` / `domain_eval_campaign_items`，把多个 task × model/execution item 组织成 durable campaign。
- 新增 `create_domain_eval_campaign` / `list_domain_eval_campaigns` / `get_domain_eval_campaign` / `run_domain_eval_campaign` / `cancel_domain_eval_campaign` owner API，并接通 Tauri / HTTP / transport。
- 默认 deterministic trace campaign 不需要外部 provider；外部模型 item 使用 `executionMode="agent"`，provider secret 只在运行时临时读取，不写入 history。
- 支持 cancel：queued item 标记 `cancelled`，running item 不强杀，下一 item 前检查 cancel flag。
- 支持 `retryFailedOnly=true`：failed / interrupted / cancelled item 清掉旧 fixture/eval run 关联后重新运行。
- Dashboard Learning 新增「Domain campaigns」卡片，可运行 trace pack、查看 item pass rate / 平均分 / check 数 / fixture/eval run 关联，并对 campaign cancel / retry。

### Phase 7.12 Domain External Campaign & Leaderboard（已完成第一版）

- `CreateDomainEvalCampaignInput` 新增临时 `providers` 字段，仅用于 `runNow` external campaign 启动，不写入 `domain_eval_campaigns`。
- 新增 `get_domain_eval_campaign_leaderboard` owner API，按 provider/model/label/execution 聚合 campaign item，输出 rank、pass rate、average score、warnings 与 evidence。
- Dashboard Learning 的「Domain campaigns」新增独立 provider/model 选择、外部 agent campaign 运行、max tasks / budget 输入和「Domain model leaderboard」。
- 缺少 provider secret 的 external campaign item 会明确 failed 并进入 leaderboard warning，不写 eval run、不静默成功。

### Phase 7.13 Domain Campaign Learning Closure（已完成第一版）

- `generate_coding_improvement_proposals` 新增 `sourceType="domain_eval_campaign"` 规则式候选：failed / cancelled / interrupted campaign item 会生成 `domain_eval_case` 与 `domain_guidance` draft proposal。
- proposal `sourceId` 使用 campaign id，fingerprint 使用 campaign item id + kind，支持按单个 campaign 定向生成并保持幂等去重。
- payload 保留 campaign、item、failure category、report JSON、scope/project/window，后续 preview/apply/promotion 可审计。
- Dashboard Learning 的「Domain campaigns」行新增学习按钮，只对有 session scope 且含 failed / cancelled / interrupted item 的 campaign 展示；点击后只生成 draft，不自动应用、不自动晋升。

### Phase 7.14 Domain Readiness Gate（已完成第一版）

- 新增 `evaluate_domain_readiness_gate` owner API，Tauri / HTTP / transport 已接通。
- Readiness Gate 组合 `evaluate_domain_quality_gate`、`get_domain_eval_campaign_leaderboard`、`domain_eval_campaigns/items` 与 `coding_improvement_proposals(source_type='domain_eval_campaign')`。
- 输出三态 `passed` / `failed` / `insufficient_data`，并返回 `blockers` 与 `recommendedNextSteps`。
- Checks 覆盖 live quality gate、campaign sample、campaign completion、leaderboard、campaign failures、learning closure。
- 运行中的 campaign 不被误判为失败，只让 readiness 维持 `insufficient_data`；失败/取消/中断 item 默认阻断，且要求失败 evidence 已进入可审查学习 proposal。
- Dashboard Learning 新增「Domain readiness」卡片，展示 quality/eval/campaign/leaderboard/learning proposal 核心计数、阻塞 check 和建议动作。
- 新增核心单测覆盖：live quality + campaign evidence 齐全时 passed；失败 campaign 且未学习闭环时 failed。

### Phase 7.15 Domain Artifact Export Guard（已完成第一版）

- 新增 `evaluate_domain_artifact_export_guard` owner API，Tauri / HTTP / transport 已接通。
- Guard 只读 `domain_evidence_items`，不调用 LLM、不访问连接器、不发送/分享/导出任何内容。
- 默认要求 `artifact_created` + `artifact_reviewed` evidence；private / connector / sensitive / pending / redacted evidence 需要显式 `exportReview` / `exportReady` / `redactionChecked` 复核标记。
- 输出 `passed` / `failed` / `insufficient_data`、summary、checks、blockers、recommended next steps 和需复核 evidence 列表。
- Workspace「领域复核」区块新增「交付守门」卡片，自动随会话加载、回合结束和手动刷新更新，让用户不用 slash 命令也能掌控最终交付风险。
- 新增核心单测覆盖：产物 + 复核 + 脱敏检查齐全时 passed；connector evidence 仍 pending 且缺少 artifact review 时 failed。

### Phase 7.16 Domain Connector Action Guard（已完成第一版）

- 新增 `evaluate_domain_connector_action_guard` owner API，Tauri / HTTP / transport 已接通。
- Guard 只读 `domain_evidence_items` 和 Artifact Export Guard 报告，不调用 LLM、不访问连接器、不发送邮件、不改日历、不分享文档、不更新外部记录。
- Checks 覆盖 action scope、explicit user approval、rollback plan，以及 send/share/upload/export/publish/submit 类动作的 artifact export guard。
- `permission::engine` 新增 strict `ExternalConnectorAction` reason：内置 Feishu / Lark 写工具精确识别，MCP / plugin 工具按连接器名 + mutating verb 保守识别。
- 外部连接器写动作禁止 AllowAlways，Smart judge 不覆盖；IM/skill `auto_approve_tools` 和 trusted MCP `autoApprove` 不能静默绕过，只有外层已审批的 `external_pre_approved` 重入可跳过重复弹窗。
- Workspace「领域复核」区块新增「外部动作守门」卡片，展示动作、批准、回滚、敏感来源计数、阻塞 check 和相关 evidence。
- 新增核心单测覆盖：批准 + 回滚 + 交付复核齐全时 passed；缺少显式用户批准时 failed；权限引擎 strict 分类与 auto-approve 旁路收口。

## 8. GUI 产品形态

通用场景层不应该要求用户记模板名。当前已落地和后续推荐入口：

- 已落地：Goal 创建 / 编辑可选择任务领域：自由任务 / 调研 / 写作 / 数据分析 / 会议准备 / 知识整理 / 邮件沟通 / 项目运营，并把 domain/template id + version/task type 持久化到 Goal。
- 已落地：Workflow Control Center 的新建工作流表单可继承 active Goal 的领域模板，也可手动选择领域模板和 task type，生成 draft、证据要求、审批门、验证策略和 Script Gate / permission preview。
- 已落地：Context Retrieval 与 Domain Quality 均优先读取 Goal 绑定的 template version；用户显式指定 template/domain 时仍可覆盖。
- 已落地：Loop 创建支持 `continue` / `workflow` 执行策略。当前 `workflow` 策略用于 interval loop：要求 active/bound Goal 已选择领域模板，每次 tick 直接创建并启动 `origin=loop:<loop_id>` 的 durable WorkflowRun，Loop trace 保存 workflow run id 和 template version；Workspace Loop 列表会关联最近派生 run，并可一键跳到 Workflow run detail。
- 已落地：Workspace「领域复核」区块支持对当前复核 run 点击「提炼经验」，把成功/失败/需要用户确认的领域质量事实定向进入 Coding Improvement proposal 队列。
- 已落地：Dashboard Learning 展示 General domain trends + General domain quality gate，区分长期趋势观察和当前门禁判定。
- 已落地：Dashboard Learning 展示 Domain smoke runs，按 `sourceType=fixture_*` 与 `SessionKind::EvalFixture` 隔离合成回归样本。
- 已落地：Dashboard Learning 展示 Domain campaigns，可运行 deterministic trace pack 或 external model agent campaign、观察 durable item 进度、取消和 retry 失败 / 中断 / 已取消 item、查看 Domain model leaderboard，并从失败 campaign item 生成可审查学习草稿。
- 已落地：Dashboard Learning 展示 Domain readiness，把 live quality、campaign、leaderboard 与 learning closure 合成一个可交付三态，并给出 blockers / next steps。
- 已落地：Dashboard Learning 展示 Domain operations，把 workflow / loop / campaign 的完成、活跃、失败残留和下一步收口建议合成运行稳定性三态。
- 已落地：Workspace「领域复核」内展示「交付守门」，把报告、文档、表格、邮件草稿等最终交付前的产物/复核/脱敏证据合成可操作三态。
- 已落地：Workspace「领域复核」内展示「外部动作守门」，把 Gmail / Calendar / Drive / Sheets / Feishu / Lark 等真实外部动作的动作证据、用户批准、回滚提示和交付守门结果合成可操作三态。
- Workspace 增加通用面板：Sources、Evidence、Drafts、Review、Verification、Decisions。

## 9. 权限与隐私红线

- Connector 数据默认按已有连接器授权和作用域读取；domain workflow 不能扩大权限。
- 发送邮件、改日历、分享文档、删除/移动文件、提交外部表单必须显式用户确认。
- 外部连接器写动作必须触发 strict 审批；不能被 AllowAlways、Smart judge、IM/skill 自动审批或 trusted MCP autoApprove 静默放行。
- 私有来源写入 evidence 时要记录 access scope；导出报告时要提示敏感来源。
- 最终发送 / 分享 / 导出 / 发布前，private / connector / sensitive / pending / redacted evidence 必须进入 Artifact Export Guard；guard 只能提示和阻断，不能替用户执行外部动作。
- 无痕会话不持久化 domain evidence、learning proposal 或 source cache。
- 模板不能静默启用网络、外部模型或连接器调用。
- 任何自动化 loop 触发前必须有清晰 owner、预算、最大次数、停机条件。

## 10. P6 到 P7 的衔接

P6 结束时应该已经具备：

- 长任务 benchmark 的 campaign / report / gate / backlog 能力。
- 可追溯 run history、evidence、失败分类和持续质量门禁。
- 对“不要伪证、不要伪对标、不要隐藏失败”的产品纪律。

P7 复用这些能力，但把对象从 coding task 换成通用 domain task：

```text
Benchmark campaign -> Domain eval suite
Coding release gate -> Domain quality gate
Coding failure backlog -> Domain improvement backlog
Code review profile -> Domain review profile
Context retrieval candidates -> Domain context candidates
Gold task pack -> Domain eval task pack
```

## 11. Phase 7 完整性审计（2026-07-04）

结论：Phase 7.1-7.16 的“通用场景层第一版”已经完成，Phase 8.1-8.3 也已把运行稳定性、连接器端到端证据和跨窗口长期运行审计产品化为 Dashboard 可见能力。当前已具备从 domain template、evidence、context retrieval、quality review、learning proposal、eval/gate、fixture/campaign/leaderboard、readiness、artifact export guard、connector action guard、operational gate、connector e2e gate 到 soak report 的闭环。它已经不是 coding-only 能力，Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation、Inbox、Project Ops 等非编程任务都能复用 Goal / Workflow / Loop / Evidence / Review / Eval / Guard / Report 控制面。

当前完成证据：

- 架构文档：`domain-workflow.md` 记录 template、evidence、Artifact Export Guard、Connector Action Guard 和 Connector E2E Gate 衔接；`domain-quality.md` 记录领域复核；`domain-eval.md` 记录 eval / fixture / campaign / leaderboard / readiness / operational gate / soak report；`context-retrieval.md` 与 `coding-improvement-loop.md` 记录上下文召回和学习闭环衔接。
- API 接线：Tauri、HTTP、transport 均已有 domain workflow、domain evidence、artifact export guard、connector action guard、connector e2e gate、domain eval fixture、campaign、leaderboard、readiness gate、operational gate 和 soak report 入口。
- GUI 接线：Goal 创建 / 编辑、Workflow Control Center、Workspace「领域复核」、Dashboard Learning 已覆盖模板选择、证据展示、质量复核、学习提炼、Smoke Run、Campaign、Leaderboard、Readiness、Operational、Soak Report、Connector E2E、交付守门和外部动作守门。
- 权限红线：外部连接器写动作进入 strict `ExternalConnectorAction`，禁止 AllowAlways、Smart 覆盖和 auto-approve 静默旁路；真正外部动作仍必须走工具审批和连接器授权。
- 验证证据：`cargo test -p ha-core domain_workflow --locked` 覆盖核心 domain workflow / guard 用例；`cargo test -p ha-core domain_eval --locked` 覆盖 eval / fixture / campaign / readiness / operational / soak report 用例；7.16/8.2 另由 `cargo test -p ha-core connector_action --locked` 覆盖连接器守门和权限分类用例。

剩余不作为 Phase 7 第一版 blocker，但仍属于“超越 Codex / Claude Code”的长期增强：

- 真实外部账号的端到端演练需要在具备 Gmail / Calendar / Drive / Sheets / Feishu / Lark 等可用测试账号后继续做，当前 worktree 主要用 deterministic / mock / fail-closed 证据覆盖。
- GUI 仍可继续把 Sources / Evidence / Drafts / Review / Verification / Decisions 做成更完整的通用任务工作台，而不是只集中在 Workspace「领域复核」和 Dashboard Learning。
- 长期运行稳定性还需要跨天 loop、campaign 和真实连接器动作的 soak run 数据；Phase 8.1 已把 workflow / loop / campaign 运行残留产品化为 Operational Gate，Phase 8.2 已把连接器 E2E 链路证据产品化为 Gate，Phase 8.3 已把这些历史导出为 Soak Report，但真实跨天样本仍需继续积累。

## 12. Phase 8：真实场景产品级验收

Phase 8 不再新增一套执行系统，而是把 Phase 7 的通用控制面放进更接近真实用户的验收层，重点回答：

- 长任务是否真的能持续稳定运行，而不是只在单次 deterministic fixture 中通过？
- 用户是否能在 GUI 里看懂当前是否可交付、是否稳定、卡在哪里、下一步该做什么？
- 真实连接器动作是否能在批准、回滚、交付复核和失败恢复上保持 fail-closed？

### Phase 8.1 Domain Operational Gate（已完成第一版）

目标：把长期运行稳定性从“感觉上应该稳”变成一个可查询、可展示、可审计的运行门禁。

已完成：

- 新增 `evaluate_domain_operational_gate` owner API，Tauri / HTTP / transport 已接通。
- Gate 只读 `workflow_runs`、`loop_schedules`、`loop_runs`、`domain_eval_campaigns` 和 `domain_eval_campaign_items`，不调用 LLM、不启动任务、不访问连接器、不自动 retry/cancel。
- Checks 覆盖 workflow sample、workflow failed/blocked/cancelled、active workflow drain、loop sample、loop failures、active campaign drain、campaign failed/cancelled/interrupted item。
- 输出 `passed` / `failed` / `insufficient_data`、summary、checks、blockers 和 recommended next steps；active workflow/campaign 不误报 failed，只让 gate 保持 `insufficient_data`。
- Dashboard Learning 新增「Domain operations」卡片，展示 workflow / loop / campaign 的完成、活跃、失败残留和下一步建议。
- 新增核心单测覆盖：已完成 workflow 且无失败残留时 passed；failed workflow + cancelled campaign item 时 failed。

验收：

- 用户无需读 DB 或 raw trace，即可在 Dashboard 看到通用长任务运行面是否稳定。
- 运行中 workflow / campaign 只表达“尚未 drain”，不会被当成失败；真正 failed / blocked / cancelled / interrupted 会阻断。
- Gate 只读，不替用户自动 approve / cancel / retry，保持可观察可控制。

### Phase 8.2 Connector E2E Gate（已完成第一版）

目标：把真实连接器动作从“动作前守门”推进到端到端验收，确认读取、草稿、批准、执行、复核、回滚说明和交付守门是否都留下可审计 evidence。

已完成：

- 新增 `evaluate_domain_connector_e2e_gate` owner API，Tauri / HTTP / transport 已接通。
- Gate 支持 goal / session / project / global scope；session / goal scope 会拒绝 incognito 并嵌套 Connector Action Guard，global / project scope 只做 evidence 聚合，不伪装成具体动作授权。
- 新增标准 evidence type：`connector_context_collected`、`connector_draft_created`、`connector_action_executed`、`connector_action_verified`。
- Checks 覆盖 connector input、draft/preview、explicit approval、execution result、post-action verification、rollback plan、Connector Action Guard、Artifact Export Guard。
- 缺用户批准为 `failed`；缺真实输入、执行结果或执行后复核为 `insufficient_data`，不把 mock/deterministic 或外部账号缺失伪装成通过。
- Dashboard Learning 新增「Connector E2E」卡片，展示 IN / DR / OK / EX / VF / RB / GU 和 recommended next steps。
- 新增核心单测覆盖：完整 Gmail send lifecycle passed；缺 execution result 时保持 `insufficient_data`。

仍需后续真实样本：

- 用测试账号跑 Gmail / Calendar / Drive / Sheets / Feishu / Lark 的真实 approve -> execute -> evidence -> export/action/e2e gate -> rollback note 流程。
- 把真实账号样本纳入 soak report，而不是仅靠 deterministic fixture。

### Phase 8.3 跨天 Soak Report（已完成第一版）

目标：把 loop / workflow / campaign / connector evidence 的跨窗口运行历史变成可读、可审计、可下钻的 JSON / Markdown / Dashboard snapshot。

已完成：

- 新增 `generate_domain_soak_report` owner API，Tauri / HTTP / transport 已接通。
- 报告只读 `workflow_runs`、`workflow_events`、`loop_runs`、`domain_eval_campaigns`、`domain_eval_campaign_items` 与 connector E2E evidence；不调用 LLM、不启动任务、不访问连接器、不自动 approve / cancel / retry。
- 输出 `passed` / `failed` / `insufficient_data`、summary、incidents、timeline、recommended next steps、Markdown 与内嵌 `operationalGate`。
- Summary 覆盖 workflow drain 时间、失败/阻塞/取消/活跃、审批/暂停/恢复/取消/恢复事件、loop tick、campaign item retry / failed / interrupted、connector E2E execution / verification evidence。
- Incidents 区分 critical 与 warning：failed/blocked/cancelled workflow、failed/cancelled/interrupted campaign item、failed/cancelled loop 为 critical；running/queued/awaiting approval 等未 drain 工作为 warning。
- Dashboard Learning 新增「Domain soak report」卡片，展示 workflow / loop / campaign / connector 样本量、critical/warning incidents、最大 drain 时间、最近 timeline 和下一步建议。
- 新增核心单测覆盖：drained workflow + loop + campaign + connector evidence 时 passed；failed workflow + active campaign item 时 failed，并输出 critical/warning incidents 和 Markdown。

验收：

- 没有样本的窗口不能 passed，只能 `insufficient_data`。
- 有 critical incident 或内嵌 Operational Gate failed 时必须 failed。
- active / queued / awaiting approval 不伪装成失败，但会让报告保持 `insufficient_data`，提醒用户先 drain。
- Markdown 是 JSON 报告的渲染，不是新的真相源。

仍需后续真实样本：

- 用跨天真实账号任务采集 connector E2E + workflow/loop/campaign history，再用 soak report 做人工复核。
- 后续可把 budget、token/cost、用户干预来源、审批耗时、recovery attempt 细节继续做厚。

### Phase 8.4 通用任务工作台（待做）

- 把 Sources、Evidence、Drafts、Review、Verification、Decisions 从路线图概念做成更完整的 Workspace 用户面。
- 重点不是更漂亮，而是让用户能在非编程任务里完成“看证据 -> 补缺口 -> 审查产物 -> 批准交付/外部动作”的闭环。

## 13. 推荐顺序

P6 完成后，建议按下列顺序推进：

1. Phase 7.1 + 7.2：已完成第一版 domain workflow registry 和 general evidence，通用层地基已具备。
2. Phase 7.3：已完成第一版 domain context retrieval，通用 workflow 已有来源 / 证据 / 缺口推荐面。
3. Phase 7.4：已补 domain review / verification 与 Workspace 领域复核，形成第一版质量闭环。
4. Phase 7.5：已接入 learning loop，让通用场景能沉淀 draft-only proposal。
5. Phase 7.6：已补通用 eval / gate，避免只靠单次质量复核判断泛化能力。
6. Phase 7.7：已补 user/project calibration 与人工复核记录，避免 built-in rubric 被误当成已校准能力证据。
7. Phase 7.8-7.10：已补 trace/agent fixture runner 与 Smoke Run Center，把通用任务回归样本和真实 agent execution 隔离展示。
8. Phase 7.11-7.12：已补 durable Domain Eval Campaign 与 external model leaderboard，让通用场景可以批跑、取消、retry、对比模型。
9. Phase 7.13：已补 campaign failure learning closure，把失败 item 接入 draft-only proposal 队列。
10. Phase 7.14：已补 Domain Readiness Gate，把质量、campaign、leaderboard 和学习闭环合成当前可交付判断。
11. Phase 7.15：已补 Domain Artifact Export Guard，把报告、文档、表格、邮件草稿等非 coding 产物的最终交付审查做成 Workspace 可操作 GUI。
12. Phase 7.16：已补 Domain Connector Action Guard，把 Gmail / Calendar / Drive / Sheets / Feishu / Lark 等真实外部动作接入同一套证据、审批和回滚提示语义。
13. Phase 8.1：已补 Domain Operational Gate，把 workflow / loop / campaign 的运行稳定性合成 Dashboard 可见门禁。
14. Phase 8.2：已补 Connector E2E Gate，把真实连接器链路的输入、草稿、批准、执行、复核、回滚和交付守门合成 Dashboard 可见门禁。
15. Phase 8.3：已补 Domain Soak Report，把 workflow / loop / campaign / connector evidence 的跨窗口运行历史导出为 JSON / Markdown / Dashboard snapshot。

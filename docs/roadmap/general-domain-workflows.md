# 通用场景层与 Domain Workflow 路线图

> 返回 [路线图索引](README.md)
>
> 更新时间：2026-07-04
>
> 状态：Phase 7.1 Domain Workflow Registry、Phase 7.2 General Evidence Model、Phase 7.3 Domain Context Retrieval、Phase 7.4 Domain Verification & Review、Phase 7.5 Domain Learning Loop、Phase 7.6 General Eval & Quality Gate 已完成第一版，并已分别沉淀到 [Domain Workflow 控制平面](../architecture/domain-workflow.md)、[Context Retrieval v2](../architecture/context-retrieval.md)、[Domain Quality 控制平面](../architecture/domain-quality.md)、[Coding Improvement Loop](../architecture/coding-improvement-loop.md) 与 [Domain Eval 与 Quality Gate 控制平面](../architecture/domain-eval.md)。

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
- Incognito session fail-closed；goal/session 关联路径避免跨 session 伪造 evidence。

后续待补：

- 独立导出流程若后续产品化，需要在导出动作本身再加敏感来源确认门。

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

后续待补：

- 把 promoted `domain_eval_case` 草稿自动导入 task registry 的 owner action。
- 接项目/用户级 calibration 与人工复核记录。
- 支持半确定性 fixture runner 驱动真实 agent 执行，再用同一 scorer 判分。

验收：

- 15 个通用 eval tasks 已可 deterministic trace scoring。
- Eval 已能发现无来源结论、漏用户确认、数据口径不明、缺 completion criteria / quality trace / workflow trace 等关键失败。
- 通用 eval 和 coding benchmark 已分表、分 API、分 Dashboard 区块展示，避免伪综合分。

## 8. GUI 产品形态

通用场景层不应该要求用户记模板名。当前已落地和后续推荐入口：

- 已落地：Goal 创建 / 编辑可选择任务领域：自由任务 / 调研 / 写作 / 数据分析 / 会议准备 / 知识整理 / 邮件沟通 / 项目运营，并把 domain/template id + version/task type 持久化到 Goal。
- 已落地：Workflow Control Center 的新建工作流表单可继承 active Goal 的领域模板，也可手动选择领域模板和 task type，生成 draft、证据要求、审批门、验证策略和 Script Gate / permission preview。
- 已落地：Context Retrieval 与 Domain Quality 均优先读取 Goal 绑定的 template version；用户显式指定 template/domain 时仍可覆盖。
- 已落地：Loop 创建支持 `continue` / `workflow` 执行策略。当前 `workflow` 策略用于 interval loop：要求 active/bound Goal 已选择领域模板，每次 tick 直接创建并启动 `origin=loop:<loop_id>` 的 durable WorkflowRun，Loop trace 保存 workflow run id 和 template version；Workspace Loop 列表会关联最近派生 run，并可一键跳到 Workflow run detail。
- 已落地：Workspace「领域复核」区块支持对当前复核 run 点击「提炼经验」，把成功/失败/需要用户确认的领域质量事实定向进入 Coding Improvement proposal 队列。
- 已落地：Dashboard Learning 展示 General domain trends + General domain quality gate，区分长期趋势观察和当前门禁判定。
- Workspace 增加通用面板：Sources、Evidence、Drafts、Review、Verification、Decisions。

## 9. 权限与隐私红线

- Connector 数据默认按已有连接器授权和作用域读取；domain workflow 不能扩大权限。
- 发送邮件、改日历、分享文档、删除/移动文件、提交外部表单必须显式用户确认。
- 私有来源写入 evidence 时要记录 access scope；导出报告时要提示敏感来源。
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

## 11. 推荐顺序

P6 完成后，建议按下列顺序推进：

1. Phase 7.1 + 7.2：已完成第一版 domain workflow registry 和 general evidence，通用层地基已具备。
2. Phase 7.3：已完成第一版 domain context retrieval，通用 workflow 已有来源 / 证据 / 缺口推荐面。
3. Phase 7.4：已补 domain review / verification 与 Workspace 领域复核，形成第一版质量闭环。
4. Phase 7.5：已接入 learning loop，让通用场景能沉淀 draft-only proposal。
5. Phase 7.6：已补通用 eval / gate，避免只靠单次质量复核判断泛化能力。

# 通用场景层与 Domain Workflow 路线图

> 返回 [路线图索引](README.md)
>
> 更新时间：2026-07-03
>
> 状态：规划设计。本文记录 P6 之后的通用场景层路线；不代表已实现架构。已实现技术事实仍以 `docs/architecture/` 为准。

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

### Phase 7.1 Domain Workflow Registry（待做）

目标：建立通用 domain workflow 的注册、选择、版本和预览机制。

要做：

- 新增 domain workflow manifest schema 和 registry。
- 支持内置模板与用户/项目自定义模板。
- Workflow 创建入口增加任务类型选择：Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation、Inbox、Project Ops。
- 模板生成 `workflow.js` draft，走既有 Script Gate、permission preview、审批和 durable runtime。
- 模板版本、启用范围、默认 mode、推荐工具、required evidence 可见。

验收：

- 用户可以从 GUI 选择一个非 coding 场景并生成 workflow draft。
- 同一目标可选择自由编排或 domain workflow，二者都落同一 WorkflowRun / Task / Goal evidence。
- 模板不会绕过权限、审批、连接器授权或 incognito 红线。

### Phase 7.2 General Evidence Model（待做）

目标：把 Goal evidence 从 coding evidence 扩展为通用证据模型，支持来源、引用、用户决策、数据口径、产物版本和验证结果。

要做：

- 定义通用 evidence 类型：`source_cited`、`claim_checked`、`user_decision`、`artifact_created`、`artifact_reviewed`、`data_quality_checked`、`citation_audited`、`message_draft_approved`、`meeting_context_collected`。
- 让 workflow host API 能写通用 evidence，不需要伪装成 validation/diff/file。
- Goal detail 展示按领域分组的 evidence timeline。
- Evidence 支持 source metadata：title、uri/path、timestamp、owner、retrieved_at、confidence、access_scope。
- Incognito / connector access / sensitive data redaction 规则进入 evidence 写入路径。

验收：

- 非 coding workflow 能用 evidence 证明完成标准，而不是只靠最终文本。
- 用户能看到“这个结论来自哪里、什么时候看过、是否被复核”。
- 敏感来源不会被错误持久化到不该出现的位置。

### Phase 7.3 Domain Context Retrieval（待做）

目标：把 Context Retrieval 从 coding 信号扩展到通用资料推荐，回答“这个任务下一步最该看哪些资料、来源、线程、会议、表格、笔记”。

要做：

- 新增 domain-aware context candidate 类型：document、email_thread、calendar_event、sheet_range、knowledge_note、web_source、decision、artifact、task。
- 根据 domain workflow 和 goal criteria 排序候选，而不是只按关键词。
- 支持来源可信度、时效、权限、重复、冲突提示。
- GUI 候选行提供领域动作：引用到报告、加入 evidence、生成摘要、请求用户确认、标记冲突、转成 task。
- 连接器缺失时显示 access issue，不伪造上下文。

验收：

- Research / Writing workflow 能看到来源和引用候选。
- Meeting Prep workflow 能看到会议材料、日历上下文和历史决策。
- Data Analysis workflow 能看到数据源、查询结果、口径说明和数据质量 issue。

### Phase 7.4 Domain Verification & Review（待做）

目标：把“验证”从代码检查扩展成领域质量检查，让报告、分析、会议 brief、邮件草稿和知识整理都有最小复核路径。

要做：

- Research verification：引用存在、来源时效、关键 claim 至少双来源、冲突点显式标注。
- Writing review：结构、读者适配、术语一致、未满足用户要求、引用缺口。
- Data verification：数据质量、口径、样本量、异常值、图表误导、计算复核。
- Meeting prep review：参会人、时间、材料、决策点、风险、未读附件。
- Inbox review：事实准确、语气、收件人、附件、发送前确认。
- 复用 Review / Verification 控制面，但新增 domain profiles 和 result schema。

验收：

- 非 coding 产物能进入 Workspace 的 Review / Verification 区块，不再只展示代码结果。
- 关键高风险动作必须用户确认，尤其是发送、分享、修改外部系统。
- Domain verification 失败能阻止 Goal completed，或明确进入 blocked / needs_user。

### Phase 7.5 Domain Learning Loop（待做）

目标：把通用任务中的成功/失败沉淀为 workflow、guidance、skill、eval 草稿，让通用场景也能持续变强。

要做：

- 从 domain workflow run、evidence、review、verification、用户反馈中生成 improvement proposals。
- Proposal kinds 扩展：`domain_workflow_template`、`domain_guidance`、`domain_review_profile`、`domain_eval_case`、`connector_usage_pattern`。
- Draft apply / promotion 复用现有安全链路，不直接改生产模板。
- Dashboard Learning 增加通用场景趋势：完成率、blocked 原因、review catch、verification failure、source quality、用户确认卡点。
- 支持用户显式从成功 run 提炼“下次类似任务怎么做”。

验收：

- 至少 Research / Writing / Data Analysis 三类任务能生成可预览的改进 proposal。
- 已应用草稿必须显式 promotion 才能成为正式 domain workflow 或 guidance。
- 学习不会跨越用户/项目/连接器权限边界。

### Phase 7.6 General Eval & Quality Gate（待做）

目标：建立非 coding 场景的 eval 和质量门禁，避免通用能力只靠感觉。

要做：

- 建立首批通用 eval tasks：Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation。
- 每个任务定义输入、允许工具、来源要求、成功标准、禁止行为、人工校准记录。
- 复用 Goal / Workflow / Evidence / Review / Verification trace 做评分。
- 建立通用 quality gate：evidence completeness、citation quality、data quality、approval safety、completion criteria match。
- Dashboard 增加通用能力质量趋势，不与 coding benchmark 混排。

验收：

- 至少 15 个通用 eval tasks 可 deterministic 或 semi-deterministic 回归。
- Eval 能发现无来源结论、漏用户确认、数据口径不明、会议材料缺失这类关键失败。
- 通用 eval 和 coding benchmark 分开展示，避免伪综合分。

## 8. GUI 产品形态

通用场景层不应该要求用户记模板名。推荐入口：

- Goal 创建时选择任务类型：自由任务 / 调研 / 写作 / 数据分析 / 会议准备 / 知识整理 / 邮件沟通 / 项目运营。
- Workflow Control Center 根据任务类型给出 draft、证据要求、风险提示和需要授权的连接器。
- Workspace 增加通用面板：Sources、Evidence、Drafts、Review、Verification、Decisions。
- Dashboard 增加 Domain Learning：按领域看完成率、卡点、证据质量、复核失败、用户确认等待。
- Loop 创建支持 domain workflow：例如每周五生成项目状态报告、每天早上准备日程 brief。

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

1. Phase 7.1 + 7.2：先做 domain workflow registry 和 general evidence，因为这是通用层地基。
2. Phase 7.3：再扩展 context retrieval，否则 workflow 没有足够上下文。
3. Phase 7.4：补 domain review / verification，形成质量闭环。
4. Phase 7.5：接 learning loop，让通用场景能沉淀。
5. Phase 7.6：最后做通用 eval / gate，避免过早为未稳定模板写大量测试。


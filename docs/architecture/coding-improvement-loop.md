# Coding Improvement Loop

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 7.5 已实现。本文是 `ha-core::coding_improvement`、`dashboard::coding_improvement`、Coding Trend Report、Transcript Distillation、Failure Feedback、Workflow Retro、Improvement Proposal 队列、Proposal-to-Action、Draft Promotion、Domain Learning proposals、Gold Pack / Strategy Effect history、External Model Baseline、Release Gate、Learning Generalization Gate、Benchmark Run Center、Benchmark Campaign Runner、Cross-model Leaderboard、Benchmark Task Corpus、Benchmark Report Export、Continuous Benchmark Gate、Benchmark Improvement Backlog、owner API、Workspace 质量趋势区块与 Dashboard 全局学习视图的单一技术事实源。

## 目标

Coding Improvement Loop 把已经持久化的 coding 控制面数据转成可审计的改进回路：

- 基于 durable data 生成近 30 天 coding trend report，不调用 LLM。
- 汇总 Goal / Workflow / Review / Smart Verification / Repair Loop / Coding Eval 信号。
- 把失败模式归类成稳定 taxonomy，解释为什么完成、阻塞或需要改进。
- 从失败 run 生成 eval candidate proposal，从成功 run 生成 workflow / guidance / skill proposal。
- proposal 默认只生成草案；用户明确应用后，也只落 reviewable draft artifact 或 managed draft skill，不直接修改项目规则、AGENTS、用户记忆或生产 fixture。
- workflow 进入终态时自动生成 lightweight retro；retro recommendation 也可进入 proposal queue。
- 用户可显式触发 transcript distillation：扫描真实 session transcript、tool error、workflow op shape 和 failure taxonomy，生成更高质量的 workflow / skill / guidance proposal。
- 已应用草稿可显式 promotion：eval candidate 迁入正式 fixture 路径，workflow/guidance 写入项目 promoted docs 并由 AGENTS.md managed include 引入，skill draft 激活为 managed active skill。
- Dashboard Learning Tab 提供全局 / 项目级 Coding Improvement 聚合：workflow completion、eval success、pack pass rate、strategy verdict、tool-call failure mode、validation / scope creep delta、review blocker、verification failure、proposal status、retro recommendation、top failure mode 与最近 retro。
- Release Gate 把持久化 pack / strategy / tool-call history 变成可配置的发布质量阈值，输出 `passed` / `failed` / `insufficient_data` 三态结论。
- Learning Generalization Gate 读取多个项目的 promoted learning、Gold Pack history 与 Strategy Effect history，判断 guidance / workflow / skill 学习成果是否跨项目成立，而不是只优化单项目 fixture。
- Benchmark Run Center 聚合 Gold Pack history、baseline kind、最近 run、Release Gate 与 Learning Generalization Gate，让用户在 Dashboard 里看到当前 coding benchmark 是否可发布、是否有外部模型基线、最近失败 case 是什么，并能显式启动安全 deterministic Benchmark Campaign。
- Benchmark Campaign Runner 把单次 Gold Pack run 包装成 durable campaign：记录 task filter、provider/model matrix、子 item 状态、attempt、pack run 关联、取消与 retry 入口，且 campaign history 不保存 provider secret。
- Cross-model Leaderboard 基于 campaign item history 聚合同一 task pack / source doc / execution mode / baseline kind 下的 provider/model 表现，显示 case pass rate、item pass rate、样本量 warning，并保留 evidence 链回 campaign item 与 pack run。
- Benchmark Task Corpus 提供 owner-plane task pack registry：导入显式 manifest、记录来源/license/privacy/redaction、保留 pack/task version、区分 draft/active/archive、验证 active task 质量，并输出 corpus health report。
- Benchmark Report Export 把 campaign / comparison / release benchmark 生成 Markdown / JSON / HTML snapshot，记录 report history，并允许用户显式标记 release evidence。
- Continuous Benchmark Gate 把 release gate、release evidence report、recent campaign、corpus health、leaderboard、失败 backlog、外部模型 opt-in、预算和可靠性指标合成一条可发布 / 可阻断的持续 benchmark 结论。
- Benchmark Improvement Backlog 把 failed / interrupted / cancelled campaign item 物化成可处理 backlog item，保留 task id、model、baseline、失败分类、pack report evidence 和 campaign evidence，避免失败只停留在红色数字。
- Phase 7.5 复用同一 proposal queue 承接通用领域学习：从 Domain Quality run/check/evidence 中生成 `domain_workflow_template`、`domain_guidance`、`domain_review_profile`、`domain_eval_case`、`connector_usage_pattern` 草稿，仍然必须 preview/apply/promotion，不能直接改生产模板或连接器策略。

## 数据模型

初始化入口在 `SessionDB::open()`，由 `crate::coding_improvement::ensure_tables()` 创建下列持久化表。

| 表 | 说明 |
| --- | --- |
| `coding_eval_runs` | 记录 deterministic eval 或外部评测运行结果，字段包括 `session_id`、`project_id`、`suite`、`name`、`status`、`metrics_json`、`source_type`、`source_id`、`created_at`。 |
| `coding_eval_pack_runs` | Phase 5.7 新增。记录 `GoldTaskPackReport` history，字段包括 `pack_id`、`source_doc`、`label`、`baseline_kind`、pack pass/fail/skipped/checks 汇总、`report_json`、`source_type`、`source_id`、`created_at`。`baseline_kind` 用来区分 `deterministic_mock` / `mock_provider` / `external_model`，避免把 fixture / mock 基线冒充真实模型能力。Phase 5.9 后 `external_model` pack run 必须来自 `executionMode="agent"` + 显式 provider/modelChain。 |
| `coding_strategy_effect_runs` | Phase 5.7 新增。记录 `StrategyEffectReport` history，字段包括 `strategy_type`、baseline/candidate label、可选 pack run 关联、`verdict`、共同 case 数、pass rate / task score / context recall / validation / scope creep / execution failure delta、`report_json`、`source_type`、`source_id`、`created_at`。 |
| `coding_benchmark_campaigns` | Phase 6.2 新增。记录 durable benchmark campaign，字段包括 scope、name、status、task pack/source doc、execution/baseline kind、`task_filter_json`、`model_matrix_json`、预算/超时、错误和开始/结束时间。`task_filter_json` 会清空 providers / modelChain，避免 provider config 或 API key 进入 history。 |
| `coding_benchmark_campaign_items` | Phase 6.2 新增。记录每个 campaign item 的 provider/model/label、status、attempt、关联 `pack_run_id`、case/check 汇总、截断后的 `report_json`、error、开始/结束时间。 |
| `coding_benchmark_task_packs` | Phase 6.4 新增。记录 corpus task pack manifest，字段包括 `pack_id`、`pack_version`、name、status、source kind / URI、repo template、license / privacy note、redaction status、import source、manifest JSON、created/updated/activated/archived 时间；`(pack_id, pack_version)` 唯一，导入同版本不会覆盖历史。 |
| `coding_benchmark_task_pack_tasks` | Phase 6.4 新增。记录 pack 内 task version，字段包括 task id/version/title/status、task type、difficulty、language/framework、source URI、repo template、tags、success criteria、validation commands、allowed/forbidden paths、calibration notes、license/privacy/redaction、risk flags、fingerprint；`(pack_id, pack_version, task_id, task_version)` 唯一。 |
| `coding_benchmark_reports` | Phase 6.5 新增。记录 benchmark report history，字段包括 report type、title、三态 status、scope、session/project、source type/id、campaign ids、不可变 snapshot JSON、Markdown / JSON / HTML 路径、release evidence 标记和创建/更新时间。 |
| `coding_benchmark_backlog_items` | Phase 6.6 新增。记录 failed / interrupted / cancelled benchmark item 物化出的 improvement backlog，字段包括 status、severity、failure category、scope、campaign/item/pack/task、provider/model、baseline/execution、evidence JSON、proposal 关联和 resolved 时间；`(campaign_item_id, task_id)` 唯一，避免重复创建。 |
| `coding_workflow_retros` | workflow 终态 retro，字段包括 `workflow_run_id`、`run_state`、`summary`、`signals_json`、`recommendations_json`、`project_id`、`created_at`、`updated_at`。`workflow_run_id` 唯一，重复终态回写走 upsert。 |
| `coding_improvement_proposals` | 改进候选草案队列，字段包括 `kind`、`status`、`source_type`、`source_id`、`title`、`body`、`payload_json`、`fingerprint`、`decided_at`、`apply_result_json`、`applied_at`、`promotion_result_json`、`promoted_at`。 |

`coding_improvement_proposals` 对 `(session_id, fingerprint)` 建唯一索引；重复生成同一候选只返回既有草案，不制造噪音。

## Transcript Distillation

Phase 4.4 新增显式 owner-plane action：`distill_coding_improvement_proposals(session_id, window_days)`。

它和 `generate_coding_improvement_proposals()` 的区别：

| Action | 输入信号 | 输出 |
| --- | --- | --- |
| `generate_coding_improvement_proposals` | 已聚合的 trend report / retro recommendation | 粗粒度 eval / workflow / guidance / skill 候选。 |
| `distill_coding_improvement_proposals` | trend report + scope 内最近 transcript + tool result + workflow ops | 带 transcript/workflow/failure evidence 的 workflow template、skill candidate、failure guidance、tool guidance 候选。 |

蒸馏过程仍然完全确定性：

- 不调用 LLM，不执行项目命令，不写项目文件。
- 读取 scope 内最多 12 个最近 session，每个 session 最多 80 条最新 message。
- 统计 user/assistant/tool message、top tool、tool error、objective snippet、error snippet。
- 扫描最近 workflow run 的 op shape，识别 review / verification / diff / tool op 组合。
- 把 failure taxonomy 转成 `CodingFailureFeedback`：`rule`、`expectedSignals`、`examples`。
- 只写 `coding_improvement_proposals(status='draft')`；重复候选靠 `(session_id, fingerprint)` 去重。

返回 `DistillCodingImprovementResult`：

| 字段 | 说明 |
| --- | --- |
| `inserted` | 本次新插入的 proposal 数。 |
| `distillation.transcript` | transcript/window/tool/error 统计。 |
| `distillation.workflowPatterns` | workflow run 的 review/verify/diff/tool op shape 摘要。 |
| `distillation.failureFeedback` | 从 failure bucket 派生的规则和证据要求。 |
| `distillation.candidates` | 本次尝试生成的候选摘要；可能因 fingerprint 已存在而未新插入。 |
| `proposals` | 当前 scope 的完整 proposal 队列。 |

## Scope

入口以当前 `session_id` 为锚点：

- 当前 session 绑定 `project_id` 时，报告按项目 scope 聚合最近窗口内的非无痕 session，最多 200 个。
- 当前 session 无 `project_id` 时，只聚合当前 session。
- incognito session 直接拒绝：不生成 report、不记录 eval run、不生成 proposal。
- 默认窗口 30 天；服务端钳制到 `[1, 180]` 天。

## Trend Report

`SessionDB::coding_trend_report(session_id, window_days)` 返回 `CodingTrendReport`：

| 区块 | 指标 |
| --- | --- |
| `overview` | sessions、goals、completed/blocked goals、workflow runs、completed/blocked/failed workflows、goal/workflow completion rate |
| `eval` | eval runs、passed、failed、success rate、eval backlog candidates |
| `review` | review runs、finding 总数、P0/P1 open blocker、resolved、false positive、category bucket |
| `verification` | verification runs、steps、passed/failed/timed out steps、planned-only runs、executed success rate、recommendation coverage |
| `repairLoop` | repair loop runs、completed、blocked、exhausted、success rate |
| `retro` | terminal workflow retro 总数、completed/blocked/failed/cancelled 分布、recommendation 数、latest summary |
| `failures` | 分类后的失败 bucket，含 severity、count、examples |
| `recentRuns` | 最近 workflow run 摘要，包含 state、blocked reason、failure category |
| `retros` | 最近 workflow retro，含 summary、signals、recommendations |
| `proposals` | 当前 scope 下的 proposal 队列，draft 优先 |

失败分类是规则式、确定性的：

| Category | 来源 |
| --- | --- |
| `validation_failed` | verification failed/timed out step，或 blocked reason 指向 validation/verify |
| `eval_failed` | `coding_eval_runs.status='failed'`，用于把失败 eval 直接送入 backlog |
| `review_blocker` | open P0/P1 review finding |
| `repair_loop_exhausted` | workflow blocked reason 为 `repair_loop_attempts_exhausted` |
| `no_effective_diff_progress` | blocked reason 指向 no effective/no valid diff |
| `permission_stall` | workflow awaiting approval，或 blocked reason 指向 approval/permission |
| `context_miss` | blocked reason 指向 context/recall/missing |
| `verification_selection_gap` | verification run 没有 step |
| `workflow_failed` / `workflow_blocked` / `goal_failed` | 兜底分类 |

## Proposal Queue

`generate_coding_improvement_proposals()` 从 report 派生候选。默认行为是读取当前 session/project scope 内可用事实并生成所有匹配候选；Phase 7.5 后也支持 `sourceType` / `sourceId` / `proposalKinds` 过滤，用于 GUI 从一次具体事实源定向提炼经验，例如 Workspace「领域复核」的「提炼经验」会传入 `sourceType="domain_quality"` + 当前 `run_id`，只返回这次 Domain Quality run 对应的 draft proposal。

| Kind | 触发 |
| --- | --- |
| `eval_candidate` | Top failure bucket，可转 deterministic eval backlog。 |
| `workflow_template` | repair loop 近期有成功 run，可人工审查后沉淀 workflow 草稿。 |
| `guidance_candidate` | review blocker 或 verification failure 暗示项目规则/流程需要补充。 |
| `skill_candidate` | workflow 成功且无已分类 blocker，可人工审查后沉淀 skill 草稿。 |
| `domain_workflow_template` | Domain Quality `completed` run，可把成功领域任务沉淀成可审查 workflow 模板草稿。 |
| `domain_guidance` | Domain Quality `completed` run，可把证据、approval 和完成习惯沉淀成领域 guidance 草稿。 |
| `domain_review_profile` | Domain Quality `blocked` / `failed` / `needs_user` run，可把漏检点沉淀成领域复核 profile 草稿。 |
| `domain_eval_case` | Domain Quality `blocked` / `failed` / `needs_user` run，可把失败模式沉淀成通用领域 eval case 草稿。 |
| `connector_usage_pattern` | Domain Quality 中高风险 approval check 进入 `needs_user`，可沉淀连接器使用和审批规则草稿。 |
| retro recommendation | `coding_workflow_retros.recommendations_json` 中的 `eval_candidate` / `workflow_template` / `guidance_candidate` / `skill_candidate`。 |

Proposal 状态：

- `draft`：默认状态，只是候选。
- `rejected`：用户拒绝该候选。
- `applying`：内部瞬态，apply 已 claim 该 proposal，防止并发应用互相覆盖。
- `applied`：用户明确应用，系统已生成 reviewable draft artifact 或 managed draft skill。
- `failed`：应用失败，`apply_result_json.error` 保存失败原因。
- `promoting`：内部瞬态，promotion 已 claim 该 proposal。
- `promoted`：用户明确晋升，系统已生成正式产物或激活 managed skill。
- `promotion_failed`：晋升失败，`promotion_result_json.error` 保存失败原因，可通过 promotion API 重试。

`update_coding_improvement_proposal_status` 只允许 `draft` / `rejected` 这类人工队列状态；`applied` / `promoting` / `promoted` / `promotion_failed` 不可被普通状态更新改写，promotion retry 只能走 promotion API；`failed` 只能由 apply 路径写入但可回到 `draft` 让用户修复环境后重试，避免把“采纳意向”伪装成“产物已落地”。

Phase 4.4 的 transcript distillation 也写入同一张 proposal queue。它不会创建新状态机，也不会绕过 preview/apply/promotion；只是让 `payload_json` 包含 `distillation`、`workflowPattern`、`failureFeedback` 或 `toolFeedback`，从而让后续草稿产物带上更具体的证据。

Phase 7.5 的 Domain Learning 也写入同一张 proposal queue。它从当前 scope 的 `domain_quality_runs` 读取 snapshot，按 run state 派生候选：成功 run 只产生可复用 workflow/guidance 草稿，失败或需用户确认的 run 产生 review profile / eval case，approval gate 卡点再补 connector usage pattern。`payload_json` 保留 domain、quality run、checks、blocking checks、scope、project/window 信息，方便草稿和后续 promotion 可审计。

Phase 7.13 的 Domain Campaign Learning Closure 继续复用同一队列。`generate_coding_improvement_proposals(sourceType="domain_eval_campaign", sourceId=<campaign_id>)` 会读取 failed / cancelled / interrupted `domain_eval_campaign_items`，按 item 生成 `domain_eval_case` 与 `domain_guidance` draft proposal；fingerprint 使用 scope + item id + kind，重复触发幂等。它不调用 LLM、不自动应用，也不把 campaign 失败静默改成项目规则。Phase 7.14 的 Domain Readiness Gate 会只读 `coding_improvement_proposals(source_type='domain_eval_campaign')` 判断失败 campaign 是否已物化为学习草稿、是否仍有未关闭 proposal；gate 本身不生成、不应用、不晋升 proposal。

## Workflow Retro

Phase 4.2 在 `workflow_runs` 进入 terminal state 时 best-effort 调用 `ensure_coding_workflow_retro_for_run()`：

- 不调用 LLM，只看 terminal state、`workflow_ops` 的 op type / state / output。
- 生成 `summary`、`signals[]` 和 `recommendations[]`。
- 成功写入 `coding_workflow_retros`，并在 workflow trace 里追加 `coding_retro_recorded` event。
- 失败不阻断 workflow terminal transition，避免学习层影响长任务完成语义。
- incognito session 不写 retro。

retro recommendation 会被 `generate_coding_improvement_proposals()` 消费：失败/阻塞可进入 `eval_candidate` / `guidance_candidate`，成功且具备 review + verify + diff 证据可进入 `workflow_template`。

## Proposal-to-Action

Phase 4.1 新增确定性 action plan：

| Proposal Kind | Apply 结果 |
| --- | --- |
| `eval_candidate` | 在当前 session/project 工作目录下创建 `.hope-agent/coding-improvement/eval-candidates/<slug>.json` 草稿。该 JSON 是可 review 的 eval candidate，不直接写入 `crates/ha-core/tests/fixtures/coding_eval/`。 |
| `workflow_template` | 创建 `.hope-agent/coding-improvement/workflows/<slug>.md`，包含 workflow script 草稿和 promotion checklist。 |
| `guidance_candidate` | 创建 `.hope-agent/coding-improvement/guidance/<slug>.md`，包含信号、建议规则和原始 payload。 |
| `skill_candidate` | 通过 `skills::author::create_skill` 创建 `~/.hope-agent/skills/ha-learned-*/SKILL.md`，状态为 `draft`，进入既有 Skills 草稿审核流。 |
| `domain_workflow_template` | 创建 `.hope-agent/coding-improvement/domain-workflows/<slug>.md`，包含领域、quality evidence、draft workflow shape 和 promotion checklist。 |
| `domain_guidance` | 创建 `.hope-agent/coding-improvement/domain-guidance/<slug>.md`，包含领域完成规则、必需 evidence、approval discipline 和 source payload。 |
| `domain_review_profile` | 创建 `.hope-agent/coding-improvement/domain-review-profiles/<slug>.md`，包含应提前捕获的 blocking checks 和复核 profile 草稿。 |
| `domain_eval_case` | 创建 `.hope-agent/coding-improvement/domain-eval-cases/<slug>.json`，包含 deterministic / semi-deterministic 通用 eval fixture 草稿。 |
| `connector_usage_pattern` | 创建 `.hope-agent/coding-improvement/connector-patterns/<slug>.md`，包含连接器读取、草稿、审批和 fail-closed 规则草稿。 |

如果 session 有有效工作目录，文件型草稿落在该工作目录的 `.hope-agent/coding-improvement/` 下；否则落在 `~/.hope-agent/sessions/{session_id}/.hope-agent/coding-improvement/`，仍然是 owner-plane 可审计产物。

`preview_coding_improvement_proposal_action(proposal_id)` 返回 `CodingImprovementActionPlan`：

- `proposal`：当前 proposal row。
- `targetKind`：`eval_candidate` / `workflow_template` / `guidance_candidate` / `skill_candidate`。
- `steps[]`：目标路径、是否已存在、内容预览。
- `preview`：kind-specific 结构化摘要。

`apply_coding_improvement_proposal(proposal_id)` 重新构建同一份 action plan 后执行：

- 只允许 `draft` proposal 应用。
- apply 会先把 proposal 从 `draft` 原子 claim 到内部 `applying`，最终只允许从 `applying` 写入 `applied` / `failed`，避免并发 apply clobber 审计状态。
- 文件型 action 使用 create-new 写入语义；如果目标已存在或竞态中被创建则 fail-closed，不覆盖。
- 成功后 `status='applied'`，`apply_result_json.artifacts[]` 记录路径和内容 hash。
- 失败后 `status='failed'`，`apply_result_json.error` 记录原因。

## Draft Promotion

Phase 4.2 新增显式 promotion plan：

| Proposal Kind | Promotion 结果 |
| --- | --- |
| `eval_candidate` | 把已应用草稿从 `.hope-agent/coding-improvement/eval-candidates/` 晋升到工作目录 `crates/ha-core/tests/fixtures/coding_eval/<slug>.json`。 |
| `workflow_template` | 把草稿复制到 `.hope-agent/coding-improvement/promoted/workflows/`，并在 `AGENTS.md` managed block 中加入 `@./...` 引用。 |
| `guidance_candidate` | 把草稿复制到 `.hope-agent/coding-improvement/promoted/guidance/`，并在 `AGENTS.md` managed block 中加入 `@./...` 引用。 |
| `skill_candidate` | 调 `skills::author::set_skill_status(skill_id, Active)` 激活 managed draft skill。 |
| `domain_workflow_template` | 把草稿复制到 `.hope-agent/coding-improvement/promoted/domain-workflows/`，并在 `AGENTS.md` managed block 中加入引用。 |
| `domain_guidance` | 把草稿复制到 `.hope-agent/coding-improvement/promoted/domain-guidance/`，并在 `AGENTS.md` managed block 中加入引用。 |
| `domain_review_profile` | 把草稿复制到 `.hope-agent/coding-improvement/promoted/domain-review-profiles/`，并在 `AGENTS.md` managed block 中加入引用。 |
| `domain_eval_case` | 把草稿复制到 `.hope-agent/coding-improvement/promoted/domain-eval-cases/`，作为 Phase 7.6 通用 eval/gate 的候选 fixture。 |
| `connector_usage_pattern` | 把草稿复制到 `.hope-agent/coding-improvement/promoted/connector-patterns/`，并在 `AGENTS.md` managed block 中加入引用。 |

`preview_coding_improvement_proposal_promotion(proposal_id)` 返回 `CodingImprovementPromotionPlan`，包含 source path、target path、target existence、source hash 和内容预览。

`promote_coding_improvement_proposal(proposal_id)` 执行 promotion：

- 只允许 `applied` / `promotion_failed` proposal 晋升。
- promotion 先原子 claim 到内部 `promoting`，最终只允许写入 `promoted` / `promotion_failed`。
- 文件型 promotion 对目标路径 fail-closed：目标不存在时 create-new；目标已存在且内容相同则幂等通过；目标已存在且内容不同则拒绝覆盖。
- `AGENTS.md` 只写 managed include block；已有 include 行 no-op，多次 promotion 会插入同一个 managed block。
- 成功后 `promotion_result_json.artifacts[]` 记录正式产物路径和 hash；失败后 `promotion_result_json.error` 记录原因。

## Owner API

Tauri commands：

| Command | 说明 |
| --- | --- |
| `get_coding_trend_report` | 读取当前 session/project scope 的 trend report。 |
| `list_coding_improvement_proposals` | 读取 proposal 队列。 |
| `generate_coding_improvement_proposals` | 基于当前 report 生成 draft-only proposals；可选 `sourceType` / `sourceId` / `proposalKinds` 做定向提炼。 |
| `distill_coding_improvement_proposals` | 显式蒸馏 transcript / workflow ops / failure feedback，并生成 draft-only proposals。 |
| `update_coding_improvement_proposal_status` | 更新 proposal 状态。 |
| `preview_coding_improvement_proposal_action` | 预览 proposal 将生成的 action plan。 |
| `apply_coding_improvement_proposal` | 应用 proposal，生成 reviewable draft artifact 或 managed draft skill。 |
| `preview_coding_improvement_proposal_promotion` | 预览已应用草稿的晋升计划。 |
| `promote_coding_improvement_proposal` | 晋升已应用草稿为正式 fixture / project guidance / active skill。 |
| `record_coding_eval_run` | 记录 deterministic eval 或外部 eval run。 |

HTTP routes：

| Method | Path |
| --- | --- |
| `GET` | `/api/sessions/{sid}/coding-trend?windowDays=30` |
| `GET` / `POST` | `/api/sessions/{sid}/coding-improvement/proposals` |
| `POST` | `/api/sessions/{sid}/coding-improvement/distill` |
| `POST` | `/api/coding-improvement/proposals/{id}/status` |
| `GET` | `/api/coding-improvement/proposals/{id}/action-preview` |
| `POST` | `/api/coding-improvement/proposals/{id}/apply` |
| `GET` | `/api/coding-improvement/proposals/{id}/promotion-preview` |
| `POST` | `/api/coding-improvement/proposals/{id}/promote` |
| `POST` | `/api/coding-improvement/eval-runs` |

前端 HTTP `COMMAND_MAP` 与 Tauri `generate_handler!` 均已注册，保持 Desktop / server 模式闭合。

## Dashboard Learning API

Phase 4.3 新增只读全局聚合 API：

| Command | HTTP | 说明 |
| --- | --- | --- |
| `dashboard_coding_improvement` | `POST /api/dashboard/learning/coding-improvement` | 按 DashboardFilter 聚合 Coding Improvement 全局 / 项目信号。 |
| `evaluate_coding_eval_release_gate` | `POST /api/coding-improvement/release-gate/evaluate` | 根据持久化 pack / strategy / tool-call history 计算发布质量门禁。 |
| `evaluate_coding_learning_generalization` | `POST /api/coding-improvement/generalization/evaluate` | 根据 promoted learning、pack history 与 strategy history 计算跨项目泛化门禁。 |
| `get_coding_benchmark_center` | `POST /api/coding-benchmark/center` | 聚合 benchmark history、baseline buckets、recent runs、Release Gate 与 Generalization Gate。 |
| `create_coding_benchmark_campaign` | `POST /api/coding-benchmark/campaigns/create` | 创建 durable Benchmark Campaign；`runNow=true` 时后台启动 runner。 |
| `list_coding_benchmark_campaigns` | `POST /api/coding-benchmark/campaigns` | 按 scope 列出最近 campaign 和 item 摘要。 |
| `get_coding_benchmark_campaign` | `GET /api/coding-benchmark/campaigns/{id}` | 读取单个 campaign、summary 与 item 明细。 |
| `cancel_coding_benchmark_campaign` | `POST /api/coding-benchmark/campaigns/{id}/cancel` | 请求取消 campaign，并把未运行 queued item 标记为 cancelled。 |
| `run_coding_benchmark_campaign` | `POST /api/coding-benchmark/campaigns/run` | 后台运行 queued item；`retryFailedOnly=true` 时只重排失败 / interrupted / cancelled item。 |
| `get_benchmark_leaderboard` | `POST /api/coding-benchmark/leaderboard` | 基于 campaign item history 生成跨模型 leaderboard。 |
| `compare_benchmark_models` | `POST /api/coding-benchmark/compare` | 使用同一聚合器按输入 campaign/window 生成可追溯 comparison report。 |
| `import_benchmark_task_pack` | `POST /api/coding-benchmark/corpus/import` | 显式 owner action 导入 task pack manifest；必须 `explicitImportConsent=true`，不扫描用户仓库，不保存 provider secret。 |
| `list_benchmark_task_packs` | `POST /api/coding-benchmark/corpus/packs` | 列出 corpus task packs，可按 status / includeArchived / limit 过滤。 |
| `get_benchmark_task_pack` | `GET /api/coding-benchmark/corpus/packs/{packId}/{version}` | 读取单个 pack 与 task version 明细。 |
| `update_benchmark_task_pack_status` | `POST /api/coding-benchmark/corpus/packs/status` | 切换 pack draft / active / archived；激活前强制重新验证 active task quality。 |
| `validate_benchmark_task_pack` | `POST /api/coding-benchmark/corpus/packs/validate` | 返回 task pack validation report，不执行项目命令。 |
| `get_benchmark_corpus_health` | `POST /api/coding-benchmark/corpus/health` | 返回 corpus health：active/draft/archive、分类分布、过期校准、重复 task、fixture-gaming risk 与 checks。 |

`dashboard_coding_improvement` 输入为 `{ filter, limit? }`，其中 `filter` 使用 Dashboard 既有时间 / agent / provider / model 过滤。gate / benchmark API 输入为 `{ input: ... }`，按各自 scope/window/threshold 字段解析；campaign API 同样是 owner plane，不经 agent 工具面。`dashboard_coding_improvement` 返回 `CodingImprovementDashboard`：

| 区块 | 内容 |
| --- | --- |
| `overview` | session、workflow、case eval、pack eval、strategy effect、tool-call missing、validation/scope delta、review blocker、verification failure、retro、proposal 和 distillation queue 汇总。 |
| `timeline` | 按天聚合 completed/blocked/failed workflow、passed/failed eval、passed/failed pack、strategy verdict、validation/scope delta、proposal created/applied/promoted、retro recommendation。 |
| `byProject` | 按 `project_id` 汇总 workflow/eval/pack 成功率、strategy regression、blocker、proposal 与 distillation candidates；项目名可用时从 `projects` 表补齐。 |
| `domainQuality` | Phase 7.5/7.6 新增。聚合 `domain_quality_runs`、`domain_quality_checks`、`domain_eval_runs` 与 `source_type='domain_quality'` 的 proposal，包含总览、按天趋势、按领域 bucket、top blockers 与 recent runs。它是历史趋势视图，不执行 gate。 |
| `topFailures` | 从 `eval_candidate` proposal payload 中读取稳定 failure category，展示 top failure mode。 |
| `toolCallFailures` | 从 task-level eval metrics 读取 agent 模式下 `toolCalls=[]` 的 run，展示 tool-call failure mode。 |
| `proposalStatuses` | proposal status 分布。 |
| `latestStrategyEffects` | 最近 strategy effect run，展示 verdict、baseline/candidate label、pass rate / task score / validation / scope creep delta。 |
| `latestRetros` | 最近 workflow retro summary 与 recommendation。 |

该 API **只读** existing durable facts，不调用 `generate_coding_improvement_proposals`，不 apply，不 promotion，也不回写任何 learning event。无痕、cron、subagent session 按 Dashboard 通用规则排除；sessionless eval run 仅在未按 agent/provider/model 过滤时计入全局 eval 聚合。

## GUI

Workspace 面板新增「质量趋势」区块：

- 读取近 30 天 report。
- 显示 Goal / Workflow / Eval / Repair 成功率。
- 显示 review blocker、verification failure、failure bucket、draft proposal 数。
- 展示当前 scope、session 数、workflow run 数、retro 数、top review category。
- 展示最近 workflow retro summary 和 recommendation。
- 展示 top failure bucket 与 proposal 草案。
- 顶部操作包含「生成改进候选」和「提炼候选」：前者从 trend report 派生候选，后者显式扫描 transcript/workflow/failure feedback 生成更高质量候选。
- proposal 行支持展开详情、预览 action plan、应用草稿产物、预览 promotion、执行 promotion、拒绝候选。
- 详情态展示目标路径、目标是否已存在、内容预览、应用/晋升后的 artifact 或错误。

Dashboard Learning Tab 新增「Coding improvement」区块：

- 顶部展示 Workflow / Eval / Pack 成功率、Strategy Effect 数、Tool-call 缺失、Review blocker、Verification failure、Distillation queue、Retro recommendation。
- Project signals 列出项目级完成率、eval 率、pack 率、strategy regression、blocker 和可沉淀候选。
- Failure modes 展示 top eval candidate failure taxonomy 与 tool-call missing failure mode。
- Improvement timeline 展示近日日级信号密度，包含 pack pass/fail、strategy verdict 与 validation/scope delta。
- Latest strategy effects 展示最近策略对比 verdict 与关键 delta。
- Latest retros 展示最近 terminal workflow retro 和首条 recommendation。
- Release Gate 卡片展示当前窗口的 `passed` / `failed` / `insufficient_data`、pack pass rate、strategy regression、missing tool-call 和未通过 checks。
- Generalization Gate 卡片展示跨项目学习门禁状态、通过/失败/证据不足项目数、promoted learning 数、pack 证据数和未通过 checks。
- General domain trends 卡片展示通用领域历史趋势：quality completion rate、blocked/failed/needs_user run、approval blockers、domain eval pass rate/average score、domain learning draft/promoted proposal、按领域分布、top blocker reason、recent quality runs。
- General domain quality gate 卡片只展示当前窗口门禁结果：eval pass rate、average score、quality blocker、domain coverage 与最近 eval run；不替代趋势卡片，也不写入学习结果。
- Domain campaigns 卡片的 learning action 会定向生成 `sourceType="domain_eval_campaign"` 草稿 proposal；它只是把失败 item 暴露到既有 proposal 审查链路，不自动 apply / promotion。

Workspace 是当前 session/project 的可操作质量面板；Dashboard 是全局 / 项目级只读学习视图。两者不复用任意 session 伪装 scope，避免把 session-local report 误读成全局事实。

## Release Gate

Phase 5.8 新增 `SessionDB::evaluate_coding_eval_release_gate(input)`，并通过 Tauri command 与 HTTP owner API 暴露。它只读历史记录，不调用 LLM、不执行项目命令、不生成 proposal、不回写 DB。

数据来源：

- `coding_eval_pack_runs`：pack run 数、pack pass rate、case/check 汇总、`baseline_kind` 分布。
- `coding_strategy_effect_runs`：strategy verdict 数量、validation / scope creep / execution failure delta。
- `coding_eval_runs(source_type='coding_task_eval')`：agent 模式下 `toolCalls=[]` 的 task eval 次数。

默认阈值偏保守：

- `minPackRuns=1`
- `minStrategyEffectRuns=0`
- `minPackPassRate=1.0`
- `requireExternalModelPack=false`
- `maxRegressedStrategyEffects=0`
- `maxMixedStrategyEffects=0`
- `maxMissingToolCallRuns=0`
- `maxValidationViolationDelta=0`
- `maxScopeCreepDelta=0`

返回 `CodingEvalReleaseGateReport`：

- `status="passed"`：样本充足且所有阈值通过。
- `status="failed"`：已有证据表明质量不达标，例如 pack pass rate 过低、strategy regressed、tool-call 缺失、validation / scope creep 增量超限。
- `status="insufficient_data"`：缺少要求的 pack / strategy 样本，或显式要求外部真实模型基线但窗口内没有 `baseline_kind='external_model'`。

scope 规则沿用 Dashboard 的 durable 数据边界：无痕、cron、subagent session 不进入发布质量判断；传入 session 且该 session 绑定 project 时自动按 project 聚合；无 scope 时可做全局 gate。Release gate 不把 deterministic / mock provider 结果冒充外部真实模型结果；需要真实 provider 基线时必须设置 `requireExternalModelPack=true` 并由 pack run 显式记录 `baselineKind="external_model"`。

## Learning Generalization Gate

Phase 5.10 新增 `SessionDB::evaluate_coding_learning_generalization(input)`，并通过 Tauri command 与 HTTP owner API `POST /api/coding-improvement/generalization/evaluate` 暴露。它只读历史记录，不调用 LLM、不执行项目命令、不生成 proposal、不回写 DB。

数据来源：

- `coding_improvement_proposals(status='promoted')`：只把已晋升的 `guidance_candidate` / `skill_candidate` / `workflow_template` 计入 durable learning evidence；默认不把草稿或已应用但未晋升的 artifact 当成泛化证据。
- `coding_eval_pack_runs`：按项目聚合 pack run、pack pass rate、external model pack run。
- `coding_strategy_effect_runs`：按项目聚合 strategy verdict、validation / scope creep / execution failure delta；当输入指定 `sourceType` / `sourceId` 时，promoted learning 与 strategy effect 只看同一来源，pack history 仍作为项目级质量背景。

默认阈值偏保守：

- `minProjects=2`
- `minProjectPackRuns=1`
- `minProjectPackPassRate=1.0`
- `minStrategyEffectRunsPerProject=0`
- `requirePromotedLearning=true`
- `requireExternalModelPack=false`
- `maxRegressedProjects=0`
- `maxMixedProjects=0`
- `maxValidationViolationDeltaPerProject=0`
- `maxScopeCreepDeltaPerProject=0`

返回 `CodingLearningGeneralizationReport`：

- `status="passed"`：至少达到要求的项目数，且每个计入项目都有 promoted learning、pack history，并通过质量阈值。
- `status="failed"`：已有证据显示某个项目 pack pass rate、strategy regression、validation delta 或 scope creep delta 不达标。
- `status="insufficient_data"`：项目数、promoted learning、pack history、strategy history 或 external model pack history 不足以证明跨项目泛化。
- `projects[]`：每个项目的 promoted learning 数、pack run、pack pass rate、strategy effect、delta、reasons 与 learning item 摘要。
- `checks[]`：机器可读门禁项，供 Dashboard / CI / release scripts 展示。

scope 规则沿用 Release Gate 的 durable 数据边界：无痕、cron、subagent session 不参与；无 scope 时按全局跨项目聚合；传 `projectId` 时可把同一 evaluator 退化为单项目学习质量门禁；传 `sessionId` 且 session 绑定 project 时按项目 scope 解析。

## Benchmark Run Center

Phase 6.1 新增 `SessionDB::get_coding_benchmark_center(input)`，并通过 Tauri command 与 HTTP owner API `POST /api/coding-benchmark/center` 暴露。它是只读聚合器，不直接跑模型、不执行项目命令、不写 DB。Phase 6.2 后 Dashboard 的 Run 按钮不再裸调 `run_coding_eval_gold_task_pack`，而是创建 `runNow=true` 的 deterministic Benchmark Campaign；runner 内部仍固定 `executionMode="fixture_patch"` + `baselineKind="deterministic_mock"`，因此默认不会访问外部模型或产生网络费用。

输入：

- `sessionId` / `projectId`：可选 scope。传入 session 且 session 绑定 project 时按 project 聚合。
- `windowDays`：默认 30，钳制到 `[1, 180]`。
- `limit`：recent runs 返回数量，默认 12，钳制到 `[1, 50]`。
- `requireExternalModelBaseline`：为 `true` 时，外部模型基线从 advisory 变成 required，并同步传给 Release / Generalization gate。
- `requireLearningGeneralization`：为 `true` 时，Learning Generalization Gate 从 advisory 变成 required。

输出 `CodingBenchmarkCenterReport`：

- `summary`：run 数、pass/fail/skipped、deterministic / external model run 数、case pass rate、latest run、best case pass rate。
- `baselines[]`：按 `baselineKind` 聚合 run pass rate、case pass rate、latest run。
- `runs[]`：最近 pack runs，包含 label、baseline kind、状态、case 计数、失败 case 摘要。
- `checks[]`：`benchmark_history`、`latest_pack_run`、`release_gate`、`external_model_baseline`、`learning_generalization`。
- `releaseGate` / `generalizationGate`：嵌入完整三态 gate 报告，供 GUI 展示和后续脚本复用。

整体状态计算：

- 任一 check `failed` -> `failed`。
- required check `insufficient_data` -> `insufficient_data`。
- 只有 advisory check `insufficient_data` 不阻断整体通过，例如没有外部模型基线时 deterministic center 仍可用于本地回归。

Dashboard Learning Tab 的 Benchmark Center 卡片展示整体状态、run/case pass rate、external model run 数、baseline buckets、recent runs、失败 case 摘要和未通过 checks；下方 Campaign 列表展示最近 campaign 的状态、item pass/case pass/check 数、每个 item 的 provider/model/label、状态、packRunId 或错误。默认 Run 创建 deterministic campaign；External campaign 控制区会列出已启用 provider/model，允许用户显式选择最多 4 个模型、设置 max tasks / budget contract 后启动 `external_model` campaign。queued/running/cancel_requested campaign 可取消；failed/partial/cancelled/interrupted campaign 可 retry failed items。

## Benchmark Campaign Runner

Phase 6.2 新增 `SessionDB::create_coding_benchmark_campaign`、`list_coding_benchmark_campaigns`、`get_coding_benchmark_campaign`、`cancel_coding_benchmark_campaign` 与 `run_benchmark_campaign`。Tauri / HTTP owner API 与 frontend transport 均已注册，Dashboard Learning Tab 使用同一组 API。

输入核心字段：

- `name`：可选显示名；为空时按 deterministic / external model 自动命名。
- `goldTaskInput`：Gold Pack 过滤和运行选项。创建 campaign 时会把 `sessionId` / `projectId` 解析到 durable scope，并清空 `providers` / `modelChain` 后写入 `task_filter_json`。
- `models[]`：provider/model matrix。为空时自动创建一个 deterministic item；外部模型 item 必须同时有 `providerId` 与 `modelId`。
- `runNow`：创建后后台启动 runner。runner 不把 provider config 写入 DB，只在本次调用内用传入 providers 匹配 model item。
- `maxBudgetUsd` / `timeoutSecs`：先作为 campaign contract 持久化，供后续 P6.3+ UI / policy 使用；当前 deterministic runner 不消耗费用。

状态语义：

- campaign：`queued`、`running`、`cancel_requested`、`passed`、`failed`、`partial`、`cancelled`、`interrupted`。
- item：`queued`、`running`、`passed`、`failed`、`skipped`、`cancelled`、`interrupted`。
- `cancel_coding_benchmark_campaign` 立即把 queued item 标记为 `cancelled`，runner 在 item 间检查 cancel flag；已经 running 的 item 结束后 campaign 会收口为 `cancelled` 或 `partial`。
- `retryFailedOnly=true` 会把 failed / interrupted / cancelled item 重排为 queued，并保留 attempt 计数和历史 pack run 关联。

真实外部模型 benchmark 必须通过 Dashboard External campaign 控制区或 owner API 显式选择 provider/model matrix，并在 `run_coding_benchmark_campaign` / `create(... runNow=true)` 调用中提供或由本机 cached config 解析 provider configs；history 只记录 provider/model id 与 report summary，不保存 API key。默认 Dashboard Run 只创建 deterministic campaign，不触发外部网络或费用。

## Cross-model Leaderboard

Phase 6.3 新增 `SessionDB::get_benchmark_leaderboard(input)` 与 `compare_benchmark_models(input)`，并通过 Tauri / HTTP / transport 暴露为 `get_benchmark_leaderboard`、`compare_benchmark_models`、`POST /api/coding-benchmark/leaderboard`、`POST /api/coding-benchmark/compare`。

聚合边界：

- scope 沿用 Benchmark Center：`sessionId` / `projectId` / global，session 绑定 project 时按 project 聚合；incognito fail-closed。
- `windowDays` 默认 30，钳制到 `[1, 180]`；`campaignIds[]` 可把 report 收窄到指定 campaign。
- leaderboard key = `taskPackId + sourceDoc + executionMode + baselineKind + providerId + modelId`，因此不会把不同任务包、不同 source doc、不同 execution mode 或不同 baseline kind 混成一个榜单。
- 排序优先 `casePassRate`，再看 `itemPassRate`、`totalChecks`、`items` 和 label；样本不足、campaign 未完成、取消/interrupted item 都会进入 `warnings[]`。

输出 `CodingBenchmarkLeaderboardReport`：

- `status="passed"`：至少有 2 行可比较 model/baseline row。
- `status="insufficient_data"`：少于 2 行，或 sample-size check 给出 advisory insufficient data。
- `rows[]`：rank、label、provider/model、task pack/source、execution/baseline、campaign/item/case/check 汇总、pass rate、warnings。
- `evidence[]`：每行最多保留 6 条 evidence，包含 campaign id/name、item id、packRunId、provider/model、status、updatedAt 与 error，保证 leaderboard 数字能回到原始 campaign item 和 pack run。

Dashboard Benchmark Center 在 Campaign 控制区上方展示 Model leaderboard：rank、label、baseline/execution/task pack、case pass rate、item pass 和 warning 标记。P6.4 已补齐真实任务集 registry 与 corpus health；P6.5 已补 benchmark report export；P6.6 已把持续 gate、失败 backlog、可靠性和预算指标接入同一 owner-plane。

## Benchmark Task Corpus

Phase 6.4 新增 `SessionDB::import_benchmark_task_pack`、`list_benchmark_task_packs`、`get_benchmark_task_pack`、`update_benchmark_task_pack_status`、`validate_benchmark_task_pack` 与 `get_benchmark_corpus_health`。Tauri / HTTP / transport 均已注册，Dashboard Learning Tab 在 Benchmark Center 下方展示 Task Corpus 面板。

导入契约：

- 输入是完整 `CodingBenchmarkTaskPackManifest`，包含 pack id/version/name/status/source kind/source URI/repo template/license note/privacy note/redaction status/tasks。
- 每个 task manifest 记录 task id/version/title/status/task type/difficulty/language/framework/source URI/repo template/tags/success criteria/validation commands/allowed paths/forbidden paths/calibration notes/license/privacy/redaction。
- 导入必须传 `explicitImportConsent=true`；否则 fail-closed。导入 API 不扫描本地 repo、不抓取 GitHub issue、不上传用户私有代码，只保存 owner 传入的 manifest。
- `(packId, version)` 与 `(packId, packVersion, taskId, taskVersion)` 唯一。任务提示、fixture、expected diff、scorer schema 或校准记录变化必须导入新版本，不覆盖旧历史。
- `status` 只允许 `draft` / `active` / `archived`。Draft pack 可保存未激活任务；active pack 必须至少包含一个 active task。

验证规则：

- `pack_identity`：pack id、version、name 必填。
- `source_traceability`：必须有 source kind，且 source URI 或 repo template 至少一个。
- `import_safety`：必须记录 license note、privacy note、redaction status。
- `task_version_uniqueness`：同 pack 内 task id/version 不可重复。
- `active_task_presence`：active pack 必须有 active task。
- `active_task_quality`：每个 active task 必须有 source、成功标准、验证命令，redaction 不能是 pending。
- `fixture_gaming_risk`：active task 的成功标准过薄、缺验证命令、写入范围过宽会阻止激活。

Corpus health report：

- 只把 `pack.status == active && task.status == active` 计作 active coverage；draft pack 内的 active task 仍只算 draft coverage。
- 输出 pack/task active/draft/archive 数、difficulty / task type / language 分布。
- 标出 stale task：active task 缺少 `calibratedAt` 或超过 `staleAfterDays`，默认 90 天。
- 标出 duplicate task：active task fingerprint 重复，避免用近似任务刷高样本量。
- 标出 gaming risk task：active task 缺验证、成功标准过薄或写入面过宽。

Dashboard Task Corpus 面板展示 corpus status、active pack/task、draft、stale、duplicate、risk 数、task type 分布和最近 pack。用户可显式导入内置 sample manifest、validate、activate、archive；sample import 也只是 owner-provided manifest，不读取当前项目文件。Draft task pack 不进入 release gate 或 leaderboard。

## Benchmark Report Export

Phase 6.5 新增 `SessionDB::generate_benchmark_report(input)`、`list_benchmark_reports`、`get_benchmark_report` 与 `mark_benchmark_report_release_evidence`。Tauri / HTTP / transport 均已注册，对应 owner API：

- `POST /api/coding-benchmark/reports/generate`
- `POST /api/coding-benchmark/reports`
- `GET /api/coding-benchmark/reports/{reportId}`
- `POST /api/coding-benchmark/reports/release-evidence`

报告类型：

- `campaign`：必须传 `campaignId`，snapshot 嵌入完整 campaign 与按该 campaign 收窄的 leaderboard。
- `comparison`：snapshot 嵌入 cross-model leaderboard / comparison 与 corpus health，用于模型或 baseline 对标复盘。
- `release`：snapshot 嵌入 Benchmark Run Center、Release Gate、Leaderboard 与 Corpus Health；默认 `releaseEvidence=true`。

落盘契约：

- 默认输出到 `reports_dir()/benchmark/{reportId}/`，也可由 owner API 显式传 `outputDir`。
- 每份报告写三份文件：`report.md`、`snapshot.json`、`report.html`；写入使用 `crate::platform::write_atomic`。
- `snapshot_json` 是生成时刻的不可变 evidence，不依赖后续 live DB 变化；DB 只保存路径和 snapshot 副本，不自动上传或分享。
- `releaseEvidence` 只能由 owner-plane 生成或显式标记；它是 release / PR 审计入口，不代表报告无法被后续重新生成。

Dashboard Learning Tab 在 Task Corpus 下方展示 Benchmark Reports 面板：可生成 Comparison / Release / 最新 Campaign 报告，展示最近 6 份 report 的 status、type、title、summary、路径和 release 标记，并支持复制 Markdown 路径、显式切换 release evidence。

## Continuous Benchmark Gate & Improvement Backlog

Phase 6.6 新增 `SessionDB::evaluate_continuous_benchmark_gate(input)`、`materialize_benchmark_backlog`、`list_benchmark_backlog` 与 `update_benchmark_backlog_status`。Tauri / HTTP / transport 均已注册，对应 owner API：

- `POST /api/coding-benchmark/continuous-gate/evaluate`
- `POST /api/coding-benchmark/backlog/materialize`
- `POST /api/coding-benchmark/backlog`
- `POST /api/coding-benchmark/backlog/status`

Continuous Benchmark Gate 是 release 前 / 策略变更后 / 模型切换后的一条综合质量闸。它不跑模型、不执行项目命令，只读 durable history，并把下列信号归一到 `CodingContinuousBenchmarkGateReport`：

- 既有 `evaluate_coding_eval_release_gate` 结果。
- 最近 release evidence report 是否存在且未过期。
- 最近 benchmark campaign 是否存在，campaign item 数是否达到阈值。
- campaign case pass rate 是否达到阈值。
- active corpus health 是否通过。
- open benchmark backlog 和尚未物化的 failed / interrupted / cancelled campaign item 数。
- required task pack / required provider/model/baseline 是否有对应 history。
- 外部模型 policy：`requireExternalModel=true` 时必须同时 `externalModelPolicyEnabled=true`，否则 fail-closed。
- 可靠性指标：interrupted campaign、provider error item、budget exhausted item。
- 预算 contract：可选 `maxBudgetUsd`，只看 campaign contract，不估算隐藏费用。
- retention knobs：报告输出 `retentionDays` 与 `rawArtifactRetentionDays`，作为非破坏性清理策略的可见参数；实际删除 raw artifact 必须走后续显式 owner action，不在 gate 里静默清理。

Gate 输出：

| 字段 | 说明 |
| --- | --- |
| `status` | `passed` / `failed` / `insufficient_data`，任何 blocking check 失败都会阻断。 |
| `checks[]` | 每条 check 的 expected / actual / reason。 |
| `blockers[]` | 当前阻塞项名称，供 Dashboard 直接展示。 |
| `recommendations[]` | 下一步动作，例如生成 release report、运行 campaign、物化 backlog、处理 provider error。 |
| `summary` | release report、latest campaign、corpus、leaderboard、pass rate、backlog、budget 等摘要。 |
| `reliability` | campaign 成功率、interrupted、provider error、budget exhausted、retention 窗口。 |

Benchmark Backlog 是 P6.6 的改进输入层：

- `materialize_benchmark_backlog` 会扫描 scope 内 failed / interrupted / cancelled campaign item，解析 item report JSON 中的 failed case；能拿到 task/case id 时按 case 建 item，拿不到时回退到 campaign item 级 item。
- 每个 backlog item 保留 campaign id、campaign item id、pack run id、task pack id、task id、provider/model、baseline kind、execution mode、failure category、title 和 evidence JSON。
- `UNIQUE(campaign_item_id, task_id)` 防止重复物化；重复触发只返回 existing 数，不制造噪音。
- status 只允许 `open` / `in_progress` / `resolved` / `wont_fix`；`resolved` 和 `wont_fix` 会写 `resolved_at`。
- 当前版本先把 benchmark 失败沉淀成独立 backlog item；进入 proposal / retro / failure feedback 的自动转化仍需显式后续 action，避免把失败 item 悄悄变成项目规则或 active skill。

Dashboard Learning Tab 在 Benchmark Reports 下方展示 Continuous Benchmark Gate 面板：

- 顶部展示 gate status、blocking check 数、release report 新鲜度、latest campaign、case pass rate、open backlog、pending failure、reliability / budget 指标。
- Blocking checks 区列出 expected / actual / reason，便于用户知道是缺 release report、缺 campaign、样本不足、失败未处理还是外部模型 policy 没开启。
- Next steps 区展示 gate 推荐动作。
- Benchmark backlog 区展示最近 open item，可一键从 pending failure 创建 backlog，也可把 item 标记为 resolved。

## Eval

`coding_eval.rs` 的 fixture harness 增加 `runs.improvement` 和 `checks.improvement`：

- 可 seed `coding_eval_runs`。
- 可生成 proposal。
- 可应用指定 kind 的 draft proposal。
- 可晋升已应用 proposal。
- 可断言 scope、failure taxonomy、proposal kind、draft-only、eval success rate、repair loop blocked 数、retro 数、retro recommendation 数、applied / promoted status、artifact 数和 action target。

`repair_loop_blocks_with_evidence` fixture 已覆盖 Phase 3.11：bounded repair loop 阻塞后，trend report 能识别 `repair_loop_exhausted`，生成 draft `eval_candidate`，并记录 eval run success rate。

`improvement_proposal_to_action` fixture 已覆盖 Phase 4.1：失败 eval run 进入 `eval_failed` taxonomy，生成 `eval_candidate`，并应用为 `.hope-agent/coding-improvement/eval-candidates/` 下的草稿 artifact。

`improvement_retro_and_promotion` fixture 已覆盖 Phase 4.2：workflow terminal retro 写入 report，retro recommendation 进入候选池，`eval_candidate` 草稿晋升到正式 coding eval fixture 路径。

Phase 4.3 为 `dashboard::coding_improvement` 增加 Rust 单元测试，覆盖项目级 rollup、proposal / retro / review / verification 信号合并，以及 incognito session 排除。该层仍是纯 SQLite 聚合，无 LLM、无项目命令执行、无写入副作用。

Phase 4.4 为 `distill_coding_improvement_proposals` 增加 Rust 单元测试，覆盖真实 transcript message、tool error、review+verify+diff workflow op shape、failed eval feedback、proposal 插入与重复触发去重。该路径仍不调用 LLM、不执行项目命令、不直接写项目规则。

Phase 5.1 为 `coding_eval.rs` 增加 task-level runner fixture，覆盖候选 diff 判分、验证命令约束、review/context/goal evidence 和 `coding_eval_runs(suite='task_level_coding_eval')` 记录。Improvement Loop 不需要新表即可消费这类任务级结果；失败仍走既有 eval failure taxonomy 与 proposal 队列。

Phase 5.2 为同一 harness 增加 agent execution runner：`mode=agent` 真实调用 chat engine，`mode=fixture_patch` 做无模型回归替身。task eval run metrics 会携带 execution 摘要，因此 Improvement Loop 可以区分执行失败、无 diff、scope creep、验证缺口等失败来源，而不需要新建 learning 表。

Phase 5.3 为同一 harness 增加 Gold Task Pack v1；Phase 5.5 已把首批 20 个 active gold tasks 全部接入自动化 pack，可批量 materialize 成普通 task fixture 并运行。Pack 内每个 case 仍复用 `runs.task.recordEvalRun` 写入同一 `coding_eval_runs` 表，Phase 5.7 额外把 pack-level summary 写入 `coding_eval_pack_runs`，让 Dashboard 能按 pack 粒度展示 pass rate 与 baseline kind。

Phase 5.4 为 pack report 增加策略效果评估：两份 `GoldTaskPackReport` 可通过纯函数 owner API 生成 `StrategyEffectReport`，按共同 case 比较 pass rate、task score、context recall、validation violations、scope creep 和 execution failures。Phase 5.7 保留纯函数无副作用语义，同时让 Tauri / HTTP owner API 在 `recordRun=true` 时写入 `coding_strategy_effect_runs`，把 review-time 质量闸升级为可审计趋势。

Phase 5.6 为 agent execution runner 增加稳定 mock tool-call 基线与 `toolCalls` 指标。mock Responses provider 会驱动真实 `write` 工具修改临时 repo，再由 task scorer 判断候选 diff 是否完成任务；`FixtureReport.metrics.execution_tool_calls` 与 task report metrics 让 Improvement Loop 可以区分“模型调用了错误工具 / 没有调用工具”和“工具调用成功但 diff 质量不达标”。Phase 5.7 Dashboard 会把 agent 模式下缺失 tool call 的 run 聚合为 `missing_tool_call` failure mode。

Phase 5.8 为持久化 pack / strategy history 增加 release gate。核心单测覆盖：干净 pack + strategy history 通过；strategy regression / validation / scope creep / missing tool-call 触发失败；要求外部真实模型但只有 deterministic / mock history 时返回 `insufficient_data`。

Phase 5.9 为 Gold Task Pack 增加外部模型基线 runner。`run_coding_eval_gold_task_pack` 可显式传 `executionMode="agent"`、`providers`、`modelChain` 和 `autoApproveTools`；runner 从 gold task prompt 创建真实 chat turn，要求模型通过工具产生 candidate diff，再由同一 scorer 判分。`baselineKind="external_model"` 不能配 `fixture_patch`，`agent` 也不能记录为 `deterministic_mock`，因此 Dashboard / Release Gate 中的 external pack run 不再只是标签。

Phase 5.10 为 Learning Loop 增加跨项目泛化门禁。核心单测覆盖：两个项目均有 promoted guidance、pack history 与 improved strategy effect 时通过；任一项目出现 regressed strategy effect / validation delta / scope creep delta 时整体失败。它证明的是“学习成果在多个项目的 durable evidence 下没有退化”，而不是训练或自动发布新策略。

Phase 6.1 为 Dashboard 增加 Benchmark Run Center。核心单测覆盖：干净 deterministic pack history 通过；latest pack run failed 时 center 与 release gate 失败；配置要求 external model baseline 但只有 deterministic history 时返回 `insufficient_data`。Phase 6.2 为 Benchmark Campaign Runner 增加核心单测：deterministic campaign 可运行 Gold Pack 子集并写回 `pack_run_id` / case 汇总；创建 external model campaign 时 `task_filter_json` 会剥离 provider config、modelChain 和 API key。Phase 6.3 为 leaderboard 增加核心单测：deterministic passed campaign 与 external failed campaign 可生成两行 comparison，deterministic 按 pass rate 排第一，失败行保留 error evidence。Phase 6.4 为 corpus registry 增加核心单测：显式 import draft pack 后不计 active coverage，激活后 health 通过；同 pack version 不可覆盖；未显式同意导入和低质量 active task fail-closed。Phase 6.5 为 report export 增加核心单测：release report 生成 Markdown / JSON / HTML 三份文件，snapshot 嵌入 center / release gate / leaderboard / corpus health，list/get/mark release evidence 可回归。Phase 6.6 为 continuous benchmark gate / backlog 增加核心单测：新鲜 release evidence + 最近 campaign + corpus health 可通过 gate；失败 campaign item 可物化成 backlog，随后 gate 因 open backlog 明确阻断。Phase 7.13 为 domain campaign learning 增加核心单测：失败 campaign item 可定向生成 `domain_eval_case` + `domain_guidance` proposal，重复触发幂等，并可预览 domain eval case 草稿。Phase 7.14 为 Domain Readiness Gate 增加核心单测：live quality + campaign evidence 齐全时 readiness passed；失败 campaign 且未学习闭环时 readiness failed，并指出 `campaign_failures` / `learning_closure` blockers。Phase 7.15 为 Artifact Export Guard 增加核心单测：产物、复核和脱敏检查齐全时 passed；connector evidence 仍 pending 且缺少 artifact review 时 failed。前端 typecheck 覆盖 Tauri / HTTP transport 类型、Dashboard 状态、Campaign 列表、External campaign 控制、Leaderboard、Task Corpus、Benchmark Report、Continuous Gate、Benchmark Backlog、Domain Campaign learning action、Domain Readiness 与 Workspace 交付守门展示。

## 红线

- 不依赖 LLM：report、proposal generation 和 transcript distillation 全部规则式。
- 不自动应用：生成 proposal 不改项目规则、skill、memory、fixture。
- 应用也不直改生产规则：只生成草稿 artifact 或 managed draft skill，后续进入人工 review/promotion。
- promotion 必须显式触发，且有 preview；不得从 proposal generation 或 apply 隐式执行。
- fail-closed：目标文件已存在且内容不同、并发创建、AGENTS include 异常或 skill 激活失败都不能吞掉；apply/promotion 错误分别写入 `failed` / `promotion_failed`。
- `applied` / `promoted` 不能被人工状态更新改回草案；promotion retry 走 promotion API。
- incognito fail-closed：无痕会话不读取/写入 durable improvement 数据。
- 蒸馏不越权：`distill_coding_improvement_proposals` 只读 durable transcript / workflow / eval / review / verification facts，只写 draft proposal，不 apply、不 promotion。
- Domain campaign learning 不越权：只读 failed / cancelled / interrupted campaign item，只写 draft proposal；无 session scope 的 campaign 不在 Dashboard 提供学习按钮。
- 泛化不伪证：Learning Generalization Gate 只消费 promoted learning 与跨项目质量历史；草稿、单项目样本、fixture-only 标签或无项目归属记录都不能证明跨项目泛化。
- Benchmark 不伪证：Benchmark Run Center 只展示 `coding_eval_pack_runs` 的 durable history；deterministic / mock / external model 由 `baselineKind` 明确区分，Dashboard 默认 Run 创建 deterministic campaign，不冒充真实外部模型能力。
- Campaign 不存密钥：`coding_benchmark_campaigns.task_filter_json` 永远不保存 provider config、modelChain 或 API key；外部模型 runner 只能使用本次 owner 调用传入的 provider configs。
- Corpus 不隐式读取：task pack import 只保存 owner 提供的 manifest，必须显式 consent；不会自动扫描用户私有 repo、抓取任意 issue 或上传代码。Draft pack/task 不算 active benchmark coverage。
- Report 不伪实时：benchmark report 是生成时刻的 snapshot，数字必须引用稳定 campaign / pack run / gate evidence，不得在展示时悄悄重算成另一份结论。
- Continuous Gate 不偷跑模型：gate 只读 durable history；涉及外部模型、费用、网络或周期触发的 policy 默认关闭，必须 owner 显式 opt-in。
- Backlog 不隐藏失败：failed / interrupted / cancelled campaign item 必须先以 open backlog 或 pending failure 形式可见；resolved / wont_fix 是用户可审计状态，不得为了通过 gate 静默删除 history。
- Retention 不静默删证据：P6.6 gate 只暴露 retention 策略参数和可靠性指标；真实 cleanup 必须是显式 owner action，且不能破坏 report snapshot 的 evidence 可追溯性。
- 不混淆 scope：Workspace 用 session/project scope；Dashboard 用 `dashboard_coding_improvement` 全局 / 项目级只读 scope，禁止用任意 session 伪装全局趋势。
- 不绕过现有控制面：trend report 只消费 Goal / Workflow / Review / Verification / Eval 的持久化事实，不重写它们的语义。

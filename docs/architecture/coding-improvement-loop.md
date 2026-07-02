# Coding Improvement Loop

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 4.4 已实现。本文是 `ha-core::coding_improvement`、`dashboard::coding_improvement`、Coding Trend Report、Transcript Distillation、Failure Feedback、Workflow Retro、Improvement Proposal 队列、Proposal-to-Action、Draft Promotion、owner API、Workspace 质量趋势区块与 Dashboard 全局学习视图的单一技术事实源。

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
- Dashboard Learning Tab 提供全局 / 项目级 Coding Improvement 聚合：workflow completion、eval success、review blocker、verification failure、proposal status、retro recommendation、top failure mode 与最近 retro。

## 数据模型

初始化入口在 `SessionDB::open()`，由 `crate::coding_improvement::ensure_tables()` 创建三张表。

| 表 | 说明 |
| --- | --- |
| `coding_eval_runs` | 记录 deterministic eval 或外部评测运行结果，字段包括 `session_id`、`project_id`、`suite`、`name`、`status`、`metrics_json`、`source_type`、`source_id`、`created_at`。 |
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

`generate_coding_improvement_proposals()` 从 report 派生候选：

| Kind | 触发 |
| --- | --- |
| `eval_candidate` | Top failure bucket，可转 deterministic eval backlog。 |
| `workflow_template` | repair loop 近期有成功 run，可人工审查后沉淀 workflow 草稿。 |
| `guidance_candidate` | review blocker 或 verification failure 暗示项目规则/流程需要补充。 |
| `skill_candidate` | workflow 成功且无已分类 blocker，可人工审查后沉淀 skill 草稿。 |
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
| `generate_coding_improvement_proposals` | 基于当前 report 生成 draft-only proposals。 |
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

输入为 `{ filter, limit? }`，其中 `filter` 使用 Dashboard 既有时间 / agent / provider / model 过滤。返回 `CodingImprovementDashboard`：

| 区块 | 内容 |
| --- | --- |
| `overview` | session、workflow、eval、review blocker、verification failure、retro、proposal 和 distillation queue 汇总。 |
| `timeline` | 按天聚合 completed/blocked/failed workflow、passed/failed eval、proposal created/applied/promoted、retro recommendation。 |
| `byProject` | 按 `project_id` 汇总 workflow/eval 成功率、blocker、proposal 与 distillation candidates；项目名可用时从 `projects` 表补齐。 |
| `topFailures` | 从 `eval_candidate` proposal payload 中读取稳定 failure category，展示 top failure mode。 |
| `proposalStatuses` | proposal status 分布。 |
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

- 顶部展示 Workflow / Eval 成功率、Review blocker、Verification failure、Distillation queue、Retro recommendation。
- Project signals 列出项目级完成率、eval 率、blocker 和可沉淀候选。
- Failure modes 展示 top eval candidate failure taxonomy。
- Improvement timeline 展示近日日级信号密度。
- Latest retros 展示最近 terminal workflow retro 和首条 recommendation。

Workspace 是当前 session/project 的可操作质量面板；Dashboard 是全局 / 项目级只读学习视图。两者不复用任意 session 伪装 scope，避免把 session-local report 误读成全局事实。

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

## 红线

- 不依赖 LLM：report、proposal generation 和 transcript distillation 全部规则式。
- 不自动应用：生成 proposal 不改项目规则、skill、memory、fixture。
- 应用也不直改生产规则：只生成草稿 artifact 或 managed draft skill，后续进入人工 review/promotion。
- promotion 必须显式触发，且有 preview；不得从 proposal generation 或 apply 隐式执行。
- fail-closed：目标文件已存在且内容不同、并发创建、AGENTS include 异常或 skill 激活失败都不能吞掉；apply/promotion 错误分别写入 `failed` / `promotion_failed`。
- `applied` / `promoted` 不能被人工状态更新改回草案；promotion retry 走 promotion API。
- incognito fail-closed：无痕会话不读取/写入 durable improvement 数据。
- 蒸馏不越权：`distill_coding_improvement_proposals` 只读 durable transcript / workflow / eval / review / verification facts，只写 draft proposal，不 apply、不 promotion。
- 不混淆 scope：Workspace 用 session/project scope；Dashboard 用 `dashboard_coding_improvement` 全局 / 项目级只读 scope，禁止用任意 session 伪装全局趋势。
- 不绕过现有控制面：trend report 只消费 Goal / Workflow / Review / Verification / Eval 的持久化事实，不重写它们的语义。

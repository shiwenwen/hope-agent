# Coding Improvement Loop

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 4.1 已实现。本文是 `ha-core::coding_improvement`、Coding Trend Report、Improvement Proposal 队列、Proposal-to-Action、owner API 与 Workspace 质量趋势区块的单一技术事实源。

## 目标

Coding Improvement Loop 把已经持久化的 coding 控制面数据转成可审计的改进回路：

- 基于 durable data 生成近 30 天 coding trend report，不调用 LLM。
- 汇总 Goal / Workflow / Review / Smart Verification / Repair Loop / Coding Eval 信号。
- 把失败模式归类成稳定 taxonomy，解释为什么完成、阻塞或需要改进。
- 从失败 run 生成 eval candidate proposal，从成功 run 生成 workflow / guidance / skill proposal。
- proposal 默认只生成草案；用户明确应用后，也只落 reviewable draft artifact 或 managed draft skill，不直接修改项目规则、AGENTS、用户记忆或生产 fixture。

## 数据模型

初始化入口在 `SessionDB::open()`，由 `crate::coding_improvement::ensure_tables()` 创建两张表。

| 表 | 说明 |
| --- | --- |
| `coding_eval_runs` | 记录 deterministic eval 或外部评测运行结果，字段包括 `session_id`、`project_id`、`suite`、`name`、`status`、`metrics_json`、`source_type`、`source_id`、`created_at`。 |
| `coding_improvement_proposals` | 改进候选草案队列，字段包括 `kind`、`status`、`source_type`、`source_id`、`title`、`body`、`payload_json`、`fingerprint`、`decided_at`、`apply_result_json`、`applied_at`。 |

`coding_improvement_proposals` 对 `(session_id, fingerprint)` 建唯一索引；重复生成同一候选只返回既有草案，不制造噪音。

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
| `failures` | 分类后的失败 bucket，含 severity、count、examples |
| `recentRuns` | 最近 workflow run 摘要，包含 state、blocked reason、failure category |
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

Proposal 状态：

- `draft`：默认状态，只是候选。
- `rejected`：用户拒绝该候选。
- `applying`：内部瞬态，apply 已 claim 该 proposal，防止并发应用互相覆盖。
- `applied`：用户明确应用，系统已生成 reviewable draft artifact 或 managed draft skill。
- `failed`：应用失败，`apply_result_json.error` 保存失败原因。

`update_coding_improvement_proposal_status` 只允许 `draft` / `rejected` 这类人工队列状态；`applied` 终态不可手动改写，`applying` 不可手动打断，`failed` 只能由 apply 路径写入但可回到 `draft` 让用户修复环境后重试，避免把“采纳意向”伪装成“产物已落地”。

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

## Owner API

Tauri commands：

| Command | 说明 |
| --- | --- |
| `get_coding_trend_report` | 读取当前 session/project scope 的 trend report。 |
| `list_coding_improvement_proposals` | 读取 proposal 队列。 |
| `generate_coding_improvement_proposals` | 基于当前 report 生成 draft-only proposals。 |
| `update_coding_improvement_proposal_status` | 更新 proposal 状态。 |
| `preview_coding_improvement_proposal_action` | 预览 proposal 将生成的 action plan。 |
| `apply_coding_improvement_proposal` | 应用 proposal，生成 reviewable draft artifact 或 managed draft skill。 |
| `record_coding_eval_run` | 记录 deterministic eval 或外部 eval run。 |

HTTP routes：

| Method | Path |
| --- | --- |
| `GET` | `/api/sessions/{sid}/coding-trend?windowDays=30` |
| `GET` / `POST` | `/api/sessions/{sid}/coding-improvement/proposals` |
| `POST` | `/api/coding-improvement/proposals/{id}/status` |
| `GET` | `/api/coding-improvement/proposals/{id}/action-preview` |
| `POST` | `/api/coding-improvement/proposals/{id}/apply` |
| `POST` | `/api/coding-improvement/eval-runs` |

前端 HTTP `COMMAND_MAP` 与 Tauri `generate_handler!` 均已注册，保持 Desktop / server 模式闭合。

## GUI

Workspace 面板新增「质量趋势」区块：

- 读取近 30 天 report。
- 显示 Goal / Workflow / Eval / Repair 成功率。
- 显示 review blocker、verification failure、failure bucket、draft proposal 数。
- 展示当前 scope、session 数、workflow run 数、top review category。
- 展示 top failure bucket 与 proposal 草案。
- proposal 行支持展开详情、预览 action plan、应用草稿产物、拒绝候选。
- 详情态展示目标路径、目标是否已存在、内容预览、应用后的 artifact 或错误。

Dashboard 当前仍是全局时间/agent/provider/model 聚合面，没有 session/project 过滤上下文；Phase 4.1 的准确产品入口先落 Workspace。后续要做全局 Dashboard 版本时，应新增 project/global scope API，而不是在 Dashboard 里用任意 session 伪装全局趋势。

## Eval

`coding_eval.rs` 的 fixture harness 增加 `runs.improvement` 和 `checks.improvement`：

- 可 seed `coding_eval_runs`。
- 可生成 proposal。
- 可应用第一条 draft proposal。
- 可断言 scope、failure taxonomy、proposal kind、draft-only、eval success rate、repair loop blocked 数、applied status、artifact 数和 action target。

`repair_loop_blocks_with_evidence` fixture 已覆盖 Phase 3.11：bounded repair loop 阻塞后，trend report 能识别 `repair_loop_exhausted`，生成 draft `eval_candidate`，并记录 eval run success rate。

`improvement_proposal_to_action` fixture 已覆盖 Phase 4.1：失败 eval run 进入 `eval_failed` taxonomy，生成 `eval_candidate`，并应用为 `.hope-agent/coding-improvement/eval-candidates/` 下的草稿 artifact。

## 红线

- 不依赖 LLM：report 和 proposal 生成全部规则式。
- 不自动应用：生成 proposal 不改项目规则、skill、memory、fixture。
- 应用也不直改生产规则：只生成草稿 artifact 或 managed draft skill，后续进入人工 review/promotion。
- fail-closed：目标文件已存在或并发创建时不覆盖；apply 错误写入 `failed`，不吞掉；`applied` 不能被人工状态更新改回草案。
- incognito fail-closed：无痕会话不读取/写入 durable improvement 数据。
- 不混淆 scope：Workspace 用 session/project scope；Dashboard 全局化必须另做正式 API。
- 不绕过现有控制面：trend report 只消费 Goal / Workflow / Review / Verification / Eval 的持久化事实，不重写它们的语义。

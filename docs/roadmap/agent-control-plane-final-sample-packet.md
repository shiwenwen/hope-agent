# Agent 控制平面最终样本包

> 返回 [路线图索引](README.md)
>
> 日期：2026-07-05
>
> 对应：[目标退出计划](agent-control-plane-exit-plan.md) / [完成状态审计](agent-control-plane-completion-audit.md)
>
> 状态：Exit 2 deterministic 样本包 v1。本文只整理可复核证据，不新增功能。

## 1. 当前结论

Exit 2 已有第一版 deterministic 样本包，并已补本轮 targeted test output，但 **还不能据此关闭整个长期目标**。

原因很具体：

- 本文已经把 `sample-a` 到 `sample-d` 的替代样本、源码证据和本轮复核结果整理出来。
- 本轮 targeted backend/frontend tests 已通过。
- `sample-c` 是 deterministic connector 替代样本，不是真实外部账号 E2E。
- 真实外部动作闭环仍必须由后续人工 / Claude Code review 明确标注为“真实样本”或“替代样本”。

当前状态：

| 样本 | 目标 | 当前证据等级 | Exit 2 状态 |
| --- | --- | --- | --- |
| sample-a | 非外部动作长任务闭环 | deterministic workflow + goal evidence | packet v1 已形成 |
| sample-b | 跨天 Soak / 长跑稳定 | deterministic Soak report evidence | packet v1 已形成 |
| sample-c | Connector E2E 执行后复核 | deterministic substitute only | packet v1 已形成，但真实 E2E 未证明 |
| sample-d | 失败恢复 / retry / cancel / blocked reason | deterministic workflow repair + Soak incident evidence | packet v1 已形成 |

## 2. 本轮复核结果

复核日期：2026-07-05。

| 命令 | 结果 | 覆盖 |
| --- | --- | --- |
| `cargo test -p ha-core workflow --locked` | 通过，100 passed | Workflow runtime、审批、暂停/恢复/取消、repair、goal evidence、domain evidence。 |
| `cargo test -p ha-core goal --locked` | 通过，26 passed | Goal 创建、预算、完成评估、workflow / validation / diagnostics evidence。 |
| `cargo test -p ha-core domain_eval --locked` | 通过，26 passed | Domain eval、operational gate、soak report、connector verification warning。 |
| `cargo test -p ha-core domain_workflow --locked` | 通过，10 passed | Domain workflow registry、general evidence、connector guard、connector E2E gate。 |
| `pnpm test -- src/components/chat/input/ChatInput.test.tsx src/components/chat/tasks/TaskProgressPanel.test.tsx src/components/chat/workspace/WorkspacePanel.test.tsx` | 通过，Vitest 实际执行 57 files / 366 tests | 输入框、任务进度、Workspace / Workflow / Loop / Domain workbench GUI 相关回归。 |
| `pnpm typecheck` | 通过 | 前端 TypeScript 类型检查。 |
| `cargo fmt --all --check` | 通过 | Rust 格式检查。 |
| `git diff --check` | 通过 | diff 空白/格式基础检查。 |
| Roadmap `.md` 相对链接存在性检查 | 通过 | 新增退出文档和相关 roadmap 索引的本地 Markdown 链接都能解析到文件。 |

本轮验证还发现并修复了一个测试夹具问题：`workflow::tests::runtime_records_domain_evidence_and_links_goal_snapshot` 首次运行时因 `channel_conversations` 表未初始化失败。修复点是让 workflow 测试 `temp_db()` 镜像真实启动期的 ChannelDB 表初始化，避免 workflow runtime 通过 `SessionMeta` 查询时误中缺表错误。修复后同一命令重跑通过。

## 3. 样本包验收边界

这份样本包的边界必须保持清楚：

- deterministic test 只能证明机制规则正确。
- GUI 单测只能证明用户界面能展示、记录、复制和转任务。
- mock / fixture 只能作为替代样本，不能宣称真实外部系统 E2E 已完成。
- 真实 connector 样本必须包含执行结果和执行后读回复核。
- 长跑样本必须解释 sample window、freshness、drain、budget、approval wait 和 recovery signal。
- 本轮命令输出只能证明 deterministic / GUI unit 路径，不等于真实外部系统 E2E。

后续严格复核命令：

```bash
cargo test -p ha-core workflow --locked
cargo test -p ha-core goal --locked
cargo test -p ha-core domain_eval --locked
cargo test -p ha-core domain_workflow --locked
pnpm test -- src/components/chat/input/ChatInput.test.tsx src/components/chat/tasks/TaskProgressPanel.test.tsx src/components/chat/workspace/WorkspacePanel.test.tsx
```

## 4. sample-a：非外部动作长任务闭环

### 4.1 样本目标

证明一条不涉及外部连接器的长任务能完整闭环：

```text
Goal -> Workflow -> Task -> Diff/Artifact -> Validation -> Review/Verification -> Final Audit
```

### 4.2 Deterministic packet

样本形态：

- Goal objective：实现一个 feature 文件，并证明验证通过。
- Completion criteria：workflow 完成、产生文件变更、validation passed、Goal evidence 可追踪。
- Workflow script：创建任务、写入 `src/feature.txt`、生成 diff、运行 `test -f src/feature.txt`、更新任务、finish。
- 预期终态：`WorkflowRunState::Completed`，输出 `ok=true`，`resultCount=1`，changed path 包含 `src/feature.txt`。
- 证据链：task op、tool write op、diff op、validate op、task update op、finish op 顺序可见。

### 4.3 源码证据

| 证据 | 文件 | 覆盖点 |
| --- | --- | --- |
| `phase2_eval_feature_workflow_writes_diffs_validates_and_finishes` | `crates/ha-core/src/workflow/tests.rs` | workflow 写文件、diff、validate、task update、finish，并断言 op 顺序。 |
| `phase2_eval_user_approval_pause_resume_cancel_flow` | `crates/ha-core/src/workflow/tests.rs` | 审批、暂停、恢复、取消以及 control event 可见。 |
| `workflow_completion_auto_evaluates_goal` | `crates/ha-core/src/goal/mod.rs` | workflow 完成后 Goal 自动 evaluate 并进入 completed。 |
| `workflow_validation_op_links_goal_evidence` | `crates/ha-core/src/goal/mod.rs` | validation passed 写入 Goal evidence，并满足 criterion。 |
| `failed_validation_blocks_goal_criteria` | `crates/ha-core/src/goal/mod.rs` | validation failed 会阻塞 Goal criterion，不伪装完成。 |
| `workflow_lsp_diagnostics_link_goal_blocker_until_clean_result` | `crates/ha-core/src/goal/mod.rs` | LSP diagnostic blocker 会进入 Goal，直到 clean result 解除。 |

### 4.4 证明与未证明

已证明：

- 非外部动作 workflow 能从脚本执行到 completed。
- task、diff、validation、finish 都能成为可审计 op。
- Goal 能消费 workflow / validation / diagnostic evidence。

未证明：

- 没有证明真实复杂项目中的长时间执行性能。
- 没有证明 UI 人工复制报告和该后端 fixture 是同一轮真实会话。

## 5. sample-b：跨天 Soak / 长跑稳定

### 5.1 样本目标

证明长任务稳定性不只是“能跑完一次”，而是可审计：

```text
workflow / loop / campaign / connector history
  -> drain
  -> 24h freshness
  -> sampleDays / requiredSampleDays
  -> budget usage/exhaustion
  -> approval wait
  -> recovery/control intervention
  -> Soak Report
```

### 5.2 Deterministic packet

样本形态：

- 1 天窗口：workflow completed、loop succeeded、campaign passed、connector executed + verified、freshness <= 10s、sample days `1/1`、incidents `0`，Soak Report `passed`。
- 多天窗口单日样本：sample days `1/2`，Soak Report `insufficient_data`，推荐补两个自然日样本。
- 多天窗口跨日样本：sample days `2/2`，最近样本新鲜，Soak Report `passed`。
- 陈旧样本：latest activity age > 24h，Soak Report `insufficient_data`，推荐补 fresh workflow。
- 审批/恢复/预算样本：记录 approval request / decision、pause / resume、recovery event、output-token budget usage / exhaustion，并进入 markdown 和 recommended next steps。

### 5.3 源码证据

| 证据 | 文件 | 覆盖点 |
| --- | --- | --- |
| `domain_soak_report_passes_with_drained_history` | `crates/ha-core/src/domain_eval.rs` | 1 天窗口 workflow + loop + campaign + connector execution/verification 全部通过。 |
| `domain_soak_report_requires_cross_day_samples_for_multi_day_window` | `crates/ha-core/src/domain_eval.rs` | 多天窗口只有单日样本时保持 `insufficient_data`。 |
| `domain_soak_report_passes_with_cross_day_fresh_samples` | `crates/ha-core/src/domain_eval.rs` | 多天窗口跨日且新鲜样本通过。 |
| `domain_soak_report_recommends_fresh_sample_for_stale_history` | `crates/ha-core/src/domain_eval.rs` | 最近活动超过 24h 时不能通过，并推荐补 fresh sample。 |
| `domain_soak_report_tracks_approval_wait_and_recovery_events` | `crates/ha-core/src/domain_eval.rs` | 审批等待、pause/resume/recovery、预算使用和耗尽进入 summary / markdown。 |
| `domain_soak_report_tracks_open_approval_wait_age` | `crates/ha-core/src/domain_eval.rs` | 未闭环审批等待年龄进入 summary / markdown / next steps。 |

### 5.4 证明与未证明

已证明：

- Soak Report 会保守处理跨天覆盖和样本新鲜度。
- 审批等待、恢复、控制介入、预算耗尽不是隐藏状态，会进入 report。
- connector execution 和 verification 都进入 Soak summary。

未证明：

- 没有真实跨天 wall-clock 长跑输出。
- 没有真实外部账号样本。

## 6. sample-c：Connector E2E 执行后复核

### 6.1 样本目标

证明外部动作必须完整闭环：

```text
读取上下文 -> 草稿/预览 -> 用户批准 -> 执行结果 -> 执行后读回复核 -> 回滚说明
```

### 6.2 Deterministic packet

样本形态：

- Domain：`inbox`。
- Connector：`gmail`。
- 输入 evidence：`connector_context_collected`，包含 `accountId=acct_test`、`threadId=thr_1`、requested action。
- 草稿 evidence：`connector_draft_created` 和 `artifact_created`。
- 用户批准 evidence：`message_draft_approved`，包含 explicit approval 和 rollback plan。
- 执行 evidence：`connector_action_executed`，包含 `messageId=msg_1` 和 `execution.status=sent`。
- 复核 evidence：`connector_action_verified`，包含 `messageId=msg_1` 和 `verification.status=verified`。
- 预期 gate：Connector E2E Gate `passed`，summary 中 execution `1`、verification `1`、rollback `1`。

### 6.3 源码证据

| 证据 | 文件 | 覆盖点 |
| --- | --- | --- |
| `domain_connector_e2e_gate_passes_with_full_connector_lifecycle` | `crates/ha-core/src/domain_workflow.rs` | connector input、draft、approval、artifact/export review、execution、verification、rollback 全部存在时 gate passed。 |
| `domain_connector_e2e_gate_keeps_missing_execution_as_insufficient_data` | `crates/ha-core/src/domain_workflow.rs` | 缺执行结果时不能通过。 |
| `domain_soak_report_requires_connector_post_action_verification` | `crates/ha-core/src/domain_eval.rs` | 有 execution 但无 verification 时 Soak 保持 `insufficient_data` 并产生 warning incident。 |
| `records connector approval and rollback evidence without mixing markers` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | Workspace 显式记录批准和回滚证据，且 marker 不混用。 |
| `records connector E2E execution and verification evidence` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | Workspace 记录 execution / verification evidence 并刷新 Gate。 |

### 6.4 证明与未证明

已证明：

- 缺 execution 时不会误判通过。
- 缺 verification 时 Soak 会提示 `connector_verification_missing`。
- Workspace 有记录 execution / verification 的用户入口。
- rollback evidence 被单独统计，不会和 approval / execution marker 混用。

未证明：

- 该样本不是生产 / 真实 Gmail 账号 E2E。
- 没有实际调用 Gmail API 或读取真实发送状态。
- `acct_test` / `msg_1` 是 deterministic fixture，不是外部系统真实 ID。

因此 sample-c 只能作为 **deterministic-only substitute**。如果最终 review 要宣称真实连接器 E2E 完成，必须另附真实账号或沙箱账号的执行后读回证据。

## 7. sample-d：失败恢复 / retry / cancel / blocked reason

### 7.1 样本目标

证明长任务失败时不会悄悄挂死，必须能走向可解释状态：

```text
failed / blocked / awaiting approval / active too long
  -> retry / repair / cancel / blocked reason / user task
  -> visible trace and next step
```

### 7.2 Deterministic packet

样本形态：

- 成功恢复：repair loop 一次 validation 通过，workflow completed，并写入 `repair-success:completed` trace。
- 尝试耗尽：repair loop 达到 `maxAttempts=1` 后 blocked，blocked reason 为 `repair_loop_attempts_exhausted`。
- 重复失败：相同 validation fingerprint 连续失败后 blocked，blocked reason 为 `guarded_repair_same_validation_fingerprint`。
- 无有效 diff：不同失败但没有 diff 进展后 blocked，blocked reason 为 `guarded_repair_no_effective_diff`。
- 用户取消：owner cancel workflow 后，child async jobs 被取消且不注入结果。
- Soak 事故：failed workflow 和 active campaign 进入 critical / warning incident，并产生 recommended next step。

### 7.3 源码证据

| 证据 | 文件 | 覆盖点 |
| --- | --- | --- |
| `runtime_repair_loop_completes_after_successful_attempt` | `crates/ha-core/src/workflow/tests.rs` | repair loop 成功后 completed，并写 trace。 |
| `runtime_repair_loop_blocks_when_attempt_budget_exhausted` | `crates/ha-core/src/workflow/tests.rs` | repair attempt 超预算后 blocked，reason 为 `repair_loop_attempts_exhausted`。 |
| `runtime_guarded_repair_blocks_repeated_validation_failure` | `crates/ha-core/src/workflow/tests.rs` | 重复 validation fingerprint 阻止无效循环，reason 为 `guarded_repair_same_validation_fingerprint`。 |
| `runtime_guarded_repair_blocks_no_effective_diff_progress` | `crates/ha-core/src/workflow/tests.rs` | 无有效 diff 进展时阻止继续 repair，reason 为 `guarded_repair_no_effective_diff`。 |
| `owner_cancel_cancels_workflow_child_async_jobs` | `crates/ha-core/src/workflow/tests.rs` | cancel workflow 时取消 child async jobs 且不注入结果。 |
| `domain_soak_report_flags_failed_workflow_and_active_campaign` | `crates/ha-core/src/domain_eval.rs` | failed workflow / active campaign 进入 critical / warning incident。 |
| `opens operational and soak evidence from readiness actions` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | Workspace 展示 failed gate、Soak incident、budget exhaustion，并能转任务。 |

### 7.4 证明与未证明

已证明：

- repair 成功、repair 失败、无效循环、用户取消都能进入明确状态。
- blocked reason 是可读字符串，不需要用户猜测。
- Soak Report 会把 failed workflow / active campaign 暴露给用户。
- Workspace 能把事故和缺口转成用户可见任务。

未证明：

- 没有真实长任务跨小时 / 跨天挂起后恢复样本。
- 没有证明所有外部 connector 失败都能自动 rollback。

## 8. Exit 2 当前判定

Exit 2 的 deterministic 样本包已经具备：

```text
sample-a packet exists
AND sample-b packet exists
AND sample-c packet exists as deterministic-only substitute
AND sample-d packet exists
AND each packet links to concrete source evidence
AND packet states what is deterministic and what remains unproven
```

Exit 2 **还差两项可选但建议补强的验收材料**：

```text
real or cross-window wall-clock soak sample
real or sandbox connector execution + post-action verification
```

如果用户接受 deterministic-only substitute，那么 Exit 2 可按 deterministic 样本包完成处理，并进入最终关闭取舍。

如果用户要求更强验收，则下一步仍留在 Exit 2，补两类材料：

1. 用真实运行窗口补一份 Soak 样本，至少能证明跨小时 / 跨天挂起、恢复、drain、freshness、budget 和 approval/recovery 信号可见。
2. 用真实或沙箱 connector 账号补一份 execution + post-action verification 样本。

## 9. 不能关闭长期目标的原因

即使 Exit 2 deterministic packet 已经形成并补过 targeted tests，整个长期目标仍不能自动关闭，因为还缺：

- 用户 / Claude Code 最终复核。
- GUI manual smoke / screenshot / browser profile，除非用户接受 source-level audit。
- 真实跨窗口 / 跨天 Soak 仍未证明，除非用户明确接受 deterministic substitute。
- 真实外部 connector E2E 仍未证明，除非用户明确接受 deterministic substitute。

当前不能再用“继续做功能”代替这些退出证据。

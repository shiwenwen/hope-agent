# Goal-driven Workflow v2 路线图

> 返回 [路线图索引](README.md)
>
> 状态：Phase 2.8 核心已完成。Milestone A/B/C 已落地；Milestone D 是 `/loop`、worktree、LSP、review engine 等后续系统的接入边界。最终架构见 [Goal 控制平面](../architecture/goal.md)。
>
> 更新时间：2026-07-01

## 背景

Goal 第一版已经把长期任务顶层语义跑通：

- durable `goals` / `goal_events` / `goal_links`。
- `/goal` 创建、查看、暂停、恢复、评估、清除。
- Workspace / Workflow Control Center 内 Goal strip。
- workflow run 自动绑定 active Goal。
- workflow completed / failed / blocked 后 best-effort 回写 Goal link 并触发 final audit。
- final audit 基于 linked workflow runs、tasks、validation ops 做保守判定。

V2 的目标不是重做 Goal，而是把 **Goal <-> Workflow <-> Evidence <-> UI <-> Evaluator** 这条链做得更细、更可解释、更适合长任务。

## 目标

1. 用户能打开一个 Goal detail，完整看见目标、完成标准、关联 workflow、任务、验证、文件和最终审计。
2. Workflow 不只把 run 状态写回 Goal，还能把 diff / file / artifact / validation 级 evidence 结构化挂到 Goal。
3. Evaluator 从“保守规则”升级为“规则引擎 + 可选 LLM 审计”，但不能让模型自说自话决定完成。
4. Goal budget 从“持久化字段”升级为可观察、可扣减、可停止的运行约束。
5. 后续 `/loop`、worktree、LSP、review engine 都能把结果作为 Goal evidence 接入，而不是各做各的总结。

## 非目标

- 不把 Goal 变成执行 runtime；具体执行仍由 Workflow / Loop / tools 承担。
- 不让 agent 工具面直接修改 durable Goal；第一阶段仍走 owner 平面。
- 不绕过 permission / hooks / sandbox / incognito / KB access。
- 不在 V2 内一次性实现 worktree、LSP、review engine；只定义 evidence 接入边界。
- 不让 LLM evaluator 覆盖 hard blocker；规则引擎先 fail closed。

## 1. Evidence v2

### 1.1 目标

把 Goal evidence 从“audit 时读取 workflow snapshot”扩展成可查询、可展示、可追踪来源的 evidence graph。

### 1.2 数据结构

已优先复用 `goal_links` 承载第一层 evidence；必要时后续再新增 `goal_evidence` 表。当前字段语义如下：

```text
goal_links
  goal_id
  target_type:
    workflow_run | workflow_op | task | validation | file | diff | artifact | review | diagnostic
  target_id
  relation:
    execution_run | repair_run
    workflow_completed | workflow_failed | workflow_blocked
    validation_passed | validation_failed
    file_changed | diff_snapshot | artifact_created
    review_finding | diagnostic_result
  metadata_json:
    summary
    confidence
    severity
    source_run_id
    source_op_key
    file_path
    line_range?
    hash?
    created_at_source?
```

后续如果需要按 evidence 做大量过滤、排序和全文搜索，再拆 `goal_evidence`：

```text
goal_evidence
  id
  goal_id
  kind
  source_type
  source_id
  source_run_id
  source_op_key
  title
  summary
  severity
  confidence
  payload_json
  created_at
```

### 1.3 第一批 evidence 类型

| Evidence | 来源 | 说明 |
| --- | --- | --- |
| `validation_passed/failed` | `workflow.validate` op | 已落地：包含 run id、op key、summary、results count、失败错误摘要。 |
| `diff_snapshot` | `workflow.diff` | 已落地：包含 changed files、行数统计、截断标记。 |
| `file_changed` | `workflow.diff` changes | 已落地：关联具体文件路径、action、line delta、language；每个 diff op 最多 50 个文件。 |
| `artifact_created` | canvas / report / generated file | 关联产物 id 或路径。 |
| `review_finding` | 后续 review engine | 关联 finding severity、status、file/line。 |
| `diagnostic_result` | 后续 LSP diagnostics | 关联 symbol/file/range 和 diagnostic severity。 |

## 2. Goal Detail UI

### 2.1 入口

- Goal strip 点击可展开详情或打开独立 Goal detail 面板。
- Workflow run detail 中显示 “Linked Goal” 区域，可跳回 Goal detail。
- `/goal` 文本结果里输出短 id 与当前 audit 摘要，GUI 可提供跳转。

### 2.2 信息架构

Goal detail 应该是任务控制面，不是报告页：

1. Header：objective、state、mode snapshot、created/updated/completed time。
2. Criteria：每条 completion criterion 的 evidence 覆盖情况。
3. Timeline：goal events + linked workflow runs + key evidence。
4. Workflows：run state、origin/parent、validation status、repair chain。
5. Tasks：required / completed / pending。
6. Evidence：validation、diff、file、artifact、review、diagnostics 分组。
7. Final Audit：achieved、missing、blockers、remaining risk。
8. Actions：evaluate、pause/resume、clear、create repair workflow、copy audit context。

### 2.3 UX 红线

- GUI 不能只是 slash command 的薄皮；用户必须能看懂“为什么没完成”。
- 失败/阻塞优先展示 blocker 和下一步，不把用户丢进原始 JSON。
- 长 timeline 必须有过滤和折叠，默认展示关键事件。
- evidence 必须能追溯来源 run/op，不能只显示一段 assistant 总结。

## 3. Evaluator v2

### 3.1 分层

```text
Rule Gate
  -> hard blockers / missing evidence / failed validation
  -> fail closed

Evidence Mapper
  -> criteria -> supporting evidence
  -> task / validation / diff / review mapping

Optional LLM Auditor
  -> only after rule gate has no hard blocker
  -> produces rationale, residual risk, suggested next evidence

Final Decision
  -> completed | blocked
  -> later may add needs_review as UI-only badge, not durable terminal state
```

### 3.2 规则红线

- validation failed 不能被 LLM auditor 改成 completed。
- workflow failed/blocked 且没有后续 repair evidence，不能 completed。
- criteria 没有任何 supporting evidence，不能 completed。
- budget exhausted 且 final validation 缺失，不能 completed。
- LLM auditor 的输出必须落入 `last_evaluator_result_json`，并保留 prompt/input 摘要用于审计。

### 3.3 输出结构

```json
{
  "status": "completed | blocked",
  "summary": "...",
  "criteria": [
    {
      "text": "...",
      "status": "satisfied | missing | blocked",
      "evidenceIds": ["..."],
      "reason": "..."
    }
  ],
  "achieved": [],
  "missing": [],
  "blockers": [],
  "evidence": [],
  "remainingRisk": "...",
  "nextEvidenceNeeded": []
}
```

## 4. Budget v2

### 4.1 目标

Goal budget 不只保存字段，还要能解释：

- 已花多少 token / time / turn。
- 哪些 workflow / subagent / validation 消耗最多。
- 是否触发过 budget warning。
- 达到硬上限后是否停止后续 run / loop。

### 4.2 阶段

1. Budget observability：Goal detail 展示 token/time/turn 使用。
2. Soft warning：接近上限时写 `goal_events(kind='budget_warning')`。
3. Hard stop：超过 hard limit 后阻止新 workflow 继续绑定该 Goal；未来 `/loop` 接入时复用同一预算门禁。
4. Audit integration：budget exhausted 进入 final audit blocker 或 remaining risk。

### 4.3 红线

- `0` 的语义必须明确；不能在 Goal budget 中含混表示无限。
- Hard stop 不取消已经获得用户批准且正在执行的 destructive op；只阻止后续调度/新 run。
- incognito 不产生 durable budget ledger。

## 5. Workflow v2 集成

### 5.1 创建

- 从 Goal detail 一键创建 workflow draft。
- Draft 自动填入 objective + criteria + current blockers。
- Repair workflow 默认继承 `goal_id`、`parent_run_id`、`origin=repair`。

### 5.2 运行中

- workflow host API 写 task / validate / diff / artifact 时同步追加 Goal evidence。
- `workflow.trace` 可选择 `evidence: true`，但默认 trace 不全部进入 Goal，避免噪音。
- `workflow.finish` 的 summary / verification / residualRisk 进入 Goal audit input。

### 5.3 完成后

- terminal relation 继续写 `goal_links`。
- 若 run 是 repair run，Goal detail 能串出 parent -> repair chain。
- 自动 evaluate 保持 best effort；失败不影响 workflow terminal 状态。

## 6. 后续系统接入

| 系统 | 接入方式 | 备注 |
| --- | --- | --- |
| `/loop` | `loop_runs.workflow_run_id` + `goal_id` | 每次触发结果成为 Goal timeline 事件。 |
| Managed Worktree | workflow run 绑定 worktree id | Goal detail 显示改动落点和 handoff 状态。 |
| LSP Diagnostics | diagnostic evidence | 类型错误 / lint / symbol 风险进入 audit。 |
| Review Engine | review finding evidence | P0/P1 unresolved finding 阻止 completed。 |
| Coding Eval | goal-driven scenario | 验证 evidence 与 final audit 是否可信。 |

## 7. 实施顺序

### Milestone A：Goal Detail + Evidence Link

状态：已完成第一层。

- 复用 `goal_links` 扩展 evidence relation。
- GUI Goal detail 面板。
- Criteria -> evidence 覆盖展示。
- workflow validation / diff / file summary 写入 evidence。

验收：

- 用户能从 Goal strip 打开 detail。
- 能看到每条 criteria 是否有证据支撑。
- validation failed 能直接定位到来源 workflow op。

### Milestone B：Evaluator v2

状态：已完成。

- Rule Gate 拆成纯函数，增加单测 fixture。
- 输出 criteria-level audit。
- 可选 LLM auditor 保留输出位但当前跳过；规则门禁先 fail closed。
- final audit 展示 next evidence needed。

验收：

- seeded failed validation 不能被判 completed。
- 缺 evidence 的 criterion 明确显示 missing。
- LLM auditor 只补 rationale，不覆盖 hard blocker。

### Milestone C：Budget v2

状态：已完成。

- Goal budget ledger。
- Goal detail budget card。
- Soft warning event。
- Hard stop 新 workflow run；未来 `/loop` 接入时复用同一预算门禁。

验收：

- 超预算后新 workflow create 被拒或要求 owner 扩容。
- final audit 能解释 budget exhausted 的影响。

### Milestone D：Post-2.8 Future Integrations

- `/loop` run evidence。
- worktree evidence。
- LSP diagnostics evidence。
- review finding evidence。

验收：

- review P1 unresolved 时 Goal 不能 completed。
- LSP fatal diagnostic 可作为 blocker。
- loop 每次触发都能在 Goal timeline 追溯。

## 8. 测试与验证

- Rust：Goal evidence CRUD、criteria mapping、Rule Gate fixture。
- TS：Goal detail rendering、long timeline folding、blocked/evidence empty states。
- Integration：workflow validate failed -> evidence -> evaluator blocked。
- Integration：workflow completed + validation passed + criteria evidence -> completed。
- Regression：incognito session cannot create Goal/evidence/budget ledger。
- Regression：terminal Goal refuses new evidence mutations except audit readback。

## 9. 文档落点

实现完成后：

- 更新 [Goal 控制平面](../architecture/goal.md)。
- 更新 [Workflow 与 Execution Mode](../architecture/workflow.md) 的 Goal 集成章节。
- 更新 [API 参考](../architecture/api-reference.md) 的新增 endpoints/events。
- 若 `/loop` 接入完成，另建或更新 `docs/architecture/loop.md`。

# Agent 控制平面完成状态审计

> 返回 [路线图索引](README.md)
>
> 日期：2026-07-05
>
> 对应退出计划：[Agent 控制平面目标退出计划](agent-control-plane-exit-plan.md)
>
> 状态：Exit 1 审计稿。本文回答“现在到底差多少”，不引入新功能。

## 1. 一句话结论

当前长期目标 **仍不能关闭**。

原因不是核心能力没有做，也不是缺本轮 targeted tests；这些已经补齐。剩余是最后的产品级取舍：

1. 最终样本包已有 deterministic v1 和本轮测试输出，但真实跨窗口 Soak 与真实 connector E2E 仍只是替代样本。
2. 体验/性能审计已有 source-level v1 和 GUI 相关 Vitest 输出，但还缺手动 GUI smoke、截图或性能 profile。
3. 最终 review packet 已形成 v1，Claude Code 已给出 `accept_v1_close_after_user_ack`；当前只缺用户明确选择是否接受替代证据。

当前最合理状态：

```text
Exit 1：完成状态总审计       = 本文完成第一版
Exit 2：最终样本包           = deterministic v1 + targeted tests 已完成，真实 E2E 可选补强
Exit 3：体验和性能收口       = source-level audit v1 + GUI Vitest + typecheck 已完成，缺手动视觉 / profile 可选补强
Exit 4：最终 review 与文档冻结 = review packet v1 已形成，Claude Code 已复核通过，缺用户关闭取舍
```

## 2. G1-G6 当前状态

| Gate | 当前判定 | 证据 | 还差什么 |
| --- | --- | --- | --- |
| G1 语义闭环 | done | `docs/roadmap/control-plane-semantics.md`、`docs/architecture/goal.md`、`docs/architecture/workflow.md`、`docs/architecture/loop.md`、`docs/architecture/worktree.md` | 最终 review packet 中复述用户心智模型即可，不再新增功能。 |
| G2 自主动态工作流 | targeted evidence ready, final review pending | `docs/architecture/workflow.md`、`docs/roadmap/phase2-completion-checklist.md`、`docs/roadmap/phase2-product-audit.md`、`docs/roadmap/workflow-script-runtime.md`、`docs/roadmap/agent-control-plane-final-sample-packet.md`，本轮 `cargo test -p ha-core workflow --locked` 通过 | 最终 review packet 中复述完整 workflow run 证据；不再缺 workflow targeted test。 |
| G3 GUI 可控体验 | source audit ready, manual evidence pending | `docs/architecture/workflow.md`、`docs/architecture/goal.md`、`docs/architecture/loop.md`、`docs/architecture/domain-workflow.md`、`docs/roadmap/agent-control-plane-ux-performance-audit.md` | 若要产品级视觉证明，还需手动 GUI smoke、截图或性能 profile；否则可接受 source-level audit 进入关闭取舍。 |
| G4 长任务稳定性 | targeted evidence ready, final evidence pending | `docs/architecture/background-jobs.md`、`docs/architecture/workflow.md`、`docs/architecture/domain-workflow.md`、`docs/roadmap/general-domain-workflows.md`、`docs/roadmap/agent-control-plane-final-sample-packet.md`，本轮 `workflow` / `domain_eval` / `domain_workflow` tests 通过 | 若要求真实连接器 E2E，还要补真实或沙箱账号 execution + verification 样本。 |
| G5 质量与性能证据 | targeted tests ready, manual profile pending | `docs/architecture/coding-eval.md`、`docs/architecture/coding-improvement-loop.md`、`docs/roadmap/coding-eval-phase0-report.md`、`docs/roadmap/phase2-eval-report.md`、`docs/roadmap/agent-control-plane-final-review-packet.md`，本轮 backend/frontend targeted tests 通过 | 仍缺 GUI/browser profile；真实视觉 smoke 仍需人工或浏览器级验收。 |
| G6 文档与外部 review | Claude Code accepted, user decision pending | `docs/architecture/` 已有各子系统事实，`docs/roadmap/` 已有路线、本退出计划和 `docs/roadmap/agent-control-plane-final-review-packet.md` | Claude Code 已确认无 P0/P1 blocker；需要用户决定是否接受 deterministic substitute 和 source-level audit。 |

## 3. 已完成能力分层

### 3.1 顶层控制语义

已完成：

- `/goal`：最终目标、完成标准、预算、证据、final audit。
- `/mode`：会话/目标推进强度。
- `/workflow`：自主动态编排开关和具体 workflow run 管理。
- `/loop`：定时、重复、轮询、条件触发。
- `/task`：用户可见进度事实。
- `/worktree`：coding 场景隔离工作区。

完成判断：

- 这部分已经能解释清楚，不再阻塞当前目标。
- 后续只允许做文档澄清或 bugfix，不再把概念关系继续改来改去。

### 3.2 动态 Workflow Runtime

已完成：

- script-first workflow。
- durable replay。
- position-based op identity。
- host API：task、tool、read/grep、file search、map、spawn agent、validate、ask user、diff、trace、finish。
- permission preview / approval。
- pause / resume / cancel。
- validation / review / repair loop。
- Workflow Control Center / Workspace 可见控制面。

完成判断：

- 机制层已完成。
- 已有 deterministic 样本包和本轮 targeted test output 证明完整路径；真实外部样本仍是可选补强。

### 3.3 Coding 深水能力

已完成：

- Managed Worktree。
- LSP / Diagnostics。
- Review Engine。
- Smart Verification。
- Context Retrieval v2。
- Actionable Context Loop。
- Coding Eval / Benchmark。
- Learning Loop / Improvement Backlog。

完成判断：

- 当前目标不再继续扩 coding-only 新能力。
- coding 后续增强进入后续池，除非 final review 发现 P0/P1 问题。

### 3.4 通用场景层

已完成：

- Domain Workflow Registry。
- General Evidence Model。
- Domain Context Retrieval。
- Domain Quality / Review / Verification。
- Domain Learning Loop。
- General Eval / Quality Gate。
- Domain Readiness Gate。
- Artifact Export Guard。
- Connector Action Guard。
- Connector E2E Gate。
- Domain Operational Gate。
- Domain Soak Report。
- Workspace 通用任务工作台。

完成判断：

- 通用能力第一版已经具备。
- 当前目标只要求证明它能支撑最终样本和用户可见闭环，不再继续扩更多模板。

## 4. 剩余工作拆解

### Exit 2：最终样本包

当前产物：

- [Agent 控制平面最终样本包](agent-control-plane-final-sample-packet.md)

已形成 deterministic packet：

- `sample-a`：非外部动作长任务闭环样本。
- `sample-b`：长跑/Soak 样本，覆盖跨天或可解释替代、24h freshness、budget、drain、approval/recovery。
- `sample-c`：connector E2E deterministic substitute，包含 execution evidence 和 post-action verification evidence，但不是真实外部账号 E2E。
- `sample-d`：失败恢复样本，至少覆盖 failed/blocked -> repair/retry/cancel/blocked reason 之一。

仍建议补强：

- 若最终口径要宣称真实跨窗口 / 跨天长跑稳定性，补真实 wall-clock Soak 样本。
- 若最终口径要宣称真实外部动作闭环，补真实或沙箱 connector 的 execution + read-back verification。

### Exit 3：体验和性能收口

当前产物：

- [Agent 控制平面体验与性能审计](agent-control-plane-ux-performance-audit.md)

已完成 source-level audit：

- GUI 路径审计：用户从输入框、Workspace、Goal strip、Workflow Control Center、Loop、TaskProgressPanel 能否完成核心动作。
- 空态/失败态审计：无 session、incognito、缺权限、样本不足、gate failed、soak failed、connector e2e insufficient。
- 性能/成本审计：刷新频率、列表截断、artifact retention、output-token budget、soak window、background job slots。

仍建议补强：

- 手动 GUI smoke / 截图。
- 大 trace / 大 evidence / 大 workflow 列表的浏览器 profile。

### Exit 4：最终 review 与文档冻结

当前产物：

- [Agent 控制平面最终 Review Packet](agent-control-plane-final-review-packet.md)

已完成 v1：

- 一轮 review：优先找 permission、long-running、incognito、connector safety、performance、UX regression。
- 文档一致性检查：已实现事实在 architecture，未做增强在 roadmap。
- 最终 packet：完成项、样本证据、测试命令、未证明项、剩余风险、后续池。

仍缺关闭取舍：

- 用户是否接受 deterministic substitute 和 source-level audit。
- 若不接受，补真实 / 跨窗口 Soak、真实 connector E2E、GUI smoke/profile。

## 5. 停止继续加功能的规则

从本文开始，以下请求都不应直接变成当前目标内的新功能：

- “再把 GUI 做漂亮一点。”
- “再补一个新的 domain 模板。”
- “再加一个 connector。”
- “再加一层 review checklist。”
- “再做一个新的 dashboard 统计。”
- “再补一个配置项。”

除非它能直接关闭 Exit 2、Exit 3 或 Exit 4 中的明确缺口，否则进入后续池。

## 6. 下一步

下一步固定为用户关闭取舍：接受产品路线 v1，或选择严格证明路线。除非用户要求先补强 Exit 2 / Exit 3 的真实运行证据，否则不继续新增功能。

默认保守推进方式：不继续扩展 Workspace UI 或 Workflow Runtime API。若要补强证据，只补真实 / 跨窗口 Soak、真实 connector 样本或 GUI smoke/profile；若接受当前 v1 证据，剩余增强进入后续池。

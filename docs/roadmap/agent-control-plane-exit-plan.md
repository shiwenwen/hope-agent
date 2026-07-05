# Agent 控制平面目标退出计划

> 返回 [路线图索引](README.md)
>
> 日期：2026-07-05
>
> 状态：退出计划。本文不是新的功能路线图，而是给当前长期目标画结束线：什么时候可以停止继续加功能，什么时候可以宣布“自主动态工作流、可观察可控制、长期稳定、体验和文档对齐”这条主线完成。

## 1. 为什么要有这份文档

当前主线已经从 coding 能力强化扩展到通用 Agent 控制平面，累计完成了 `/goal`、`/mode`、`/workflow`、`/loop`、worktree、LSP、review、verification、context retrieval、eval、benchmark、learning loop、domain workflow、connector guard、soak report 和 Workspace 通用任务工作台。

如果继续按“想到什么补什么”的方式推进，用户无法判断：

- 目标什么时候结束。
- 当前还差什么。
- 下一步为什么是这个。
- 哪些是必须完成，哪些只是后续增强。

因此从本文件开始，当前大目标只按退出门禁推进。没有映射到退出门禁的工作，不再进入当前目标。

## 2. 当前大目标

原始目标不缩小：

```text
做好改造设计，我们要的是自主的动态工作流，而且是可观察可控制的，然后进行完整的改造和 review，确保能力体验，长期运行的稳定性，性能，我们要超越 Claude Code 做得更强。完成之后注意文档的准确的详细的更新。
```

把它拆成 6 个必须同时成立的退出门禁：

| Gate | 名称 | 必须证明什么 | 当前状态 |
| --- | --- | --- | --- |
| G1 | 语义闭环 | `/goal`、`/mode`、`/workflow`、`/loop`、`/task`、`/worktree` 的关系稳定，用户和模型都不会混淆。 | 已完成并审计；Claude Code 已复核通过，等用户关闭取舍 |
| G2 | 自主动态工作流 | Workflow Mode 开启后，模型能自主判断是否编排 workflow；workflow 可审批、暂停、恢复、取消、失败恢复、trace、review、verification。 | 已完成并补 targeted tests；Claude Code 已复核通过，等用户关闭取舍 |
| G3 | GUI 可控体验 | 用户不靠 slash command 也能创建/查看/推进目标、工作流、循环、任务、证据、守门和下一步。 | source-level audit v1 + GUI Vitest + typecheck 已形成，缺手动视觉 / profile 可选补强 |
| G4 | 长任务稳定性 | 长任务不会悄悄挂死；有运行稳定性 gate、soak report、预算/审批/恢复信号、跨天样本和 connector E2E 复核。 | deterministic 样本包 v1 + 本轮 targeted tests 已完成；真实 / 跨窗口 Soak 与真实 connector E2E 仍是可选补强 |
| G5 | 质量与性能证据 | 有 targeted tests / eval / benchmark / release gate / smoke 证据证明核心能力不回退，性能和成本风险可见。 | 本轮 targeted backend/frontend tests 已完成；GUI manual smoke / profile 可选补强 |
| G6 | 文档与外部 review | 已实现事实进入 architecture，路线和剩余项进入 roadmap，并形成可给 Claude Code / 人工 reviewer 的最终 review packet。 | review packet v1 已形成，Claude Code 已给出 `accept_v1_close_after_user_ack`，缺用户关闭取舍 |

只有 G1-G6 都有证据时，才允许把本 thread goal 标记为完成。

## 3. 当前已经完成的范围

这些不再重复当作新阶段推进，只做最终审计：

| 范围 | 已完成事实 | 主要文档 |
| --- | --- | --- |
| 控制平面语义 | `/goal`、`/mode`、`/workflow`、`/loop`、`/task`、`/worktree` 关系已收口。 | `control-plane-semantics.md`、`agent-control-plane-roadmap.md` |
| Goal | durable goal、完成标准、预算、evidence、final audit、GUI goal strip/detail。 | `architecture/goal.md`、`goal-driven-workflow-v2.md` |
| Workflow | script-first dynamic workflow、durable replay、host API、审批、暂停/恢复/取消、trace、validation、review、repair。 | `architecture/workflow.md`、`workflow-script-runtime.md` |
| Loop | 真正定时/重复/条件触发 loop，接入 goal evidence。 | `architecture/loop.md` |
| Coding 深水能力 | worktree、LSP、review engine、smart verification、context retrieval、repair loop、eval、benchmark、learning loop。 | `architecture/worktree.md`、`lsp.md`、`review-engine.md`、`verification-engine.md`、`context-retrieval.md`、`coding-eval.md`、`coding-improvement-loop.md` |
| 通用场景层 | domain workflow registry、general evidence、domain context、quality/review、learning、eval/gate、artifact export guard、connector action guard。 | `architecture/domain-workflow.md`、`domain-quality.md`、`domain-eval.md` |
| 长任务与真实样本机制 | operational gate、connector E2E gate、domain soak report、Workspace 通用任务工作台、真实样本验收卡。 | `architecture/domain-workflow.md`、`general-domain-workflows.md` |

## 4. 剩余工作只剩 4 个 Exit Stage

从 2026-07-05 起，当前目标不再新增 Phase 9/10。只剩以下 4 个退出阶段。

### Exit 1：完成状态总审计

目的：把“我们到底做到哪了”变成一张当前状态表。

当前产物：[Agent 控制平面完成状态审计](agent-control-plane-completion-audit.md)。

必须产物：

- 一份 `Completion Audit`，逐项列出 G1-G6 的证据文件、测试、GUI 入口和剩余风险。
- 明确每个 gate 是 `done`、`needs evidence` 还是 `not required for current goal`。
- 把后续增强从当前目标中剥离。

完成标准：

- 用户能从文档里直接看到剩余项，不需要翻完整聊天记录。
- 每个“已完成”都有 architecture 或测试证据。
- 每个“未完成”都有明确下一步，不是泛泛地“继续优化”。

### Exit 2：最终样本包

目的：证明长任务稳定性和真实外部动作闭环，不只证明代码路径存在。

必须产物：

- 至少一组非外部动作长任务样本：Goal -> Workflow -> Task -> Verification/Review -> Final Audit。
- 至少一组跨天或可替代的 deterministic soak 样本：覆盖 `sampleDays / requiredSampleDays`、24h freshness、drain、budget、approval wait、recovery signal。
- 至少一组 connector E2E 样本：读取 -> 草稿 -> 用户批准 -> 执行记录 -> 执行后读回复核 -> 回滚说明。
- Workspace「真实样本验收」复制报告或等价 review packet。

完成标准：

- Soak Report 不能是空跑，也不能只靠按钮点击或人工声明。
- connector 样本不能只有 `executed`，必须有 `verified`。
- 失败样本必须能看到 recovery、retry、cancel 或明确的 blocked reason。
- 若真实外部账号不可用，必须记录为替代样本，并明确不能宣称真实连接器 E2E 已完成。

### Exit 3：体验和性能收口

目的：确认产品体验已经可用，不是只有后端能力。

必须产物：

- GUI 用户路径审计：目标模式、工作流模式、循环、任务、工作区、真实样本验收、长跑审计、连接器 E2E。
- 关键状态的空态 / 加载 / 失败 / 无痕禁用 / 权限不足 / 样本不足表现清单。
- 性能与成本风险审计：长列表、刷新频率、soak window、output-token budget、benchmark artifact retention。

完成标准：

- 用户能回答“现在目标是什么、还差什么、下一步点哪里”。
- 没有必须依赖 slash command 才能完成的核心路径。
- 长任务 UI 不要求用户读原始 JSON 才能判断状态。
- 性能风险可见，不能静默无限刷新、无限保留大 artifact 或无限跑长任务。

### Exit 4：最终 review 与文档冻结

目的：把当前目标从“仍在做”切换为“可以评审和关闭”。

必须产物：

- 架构文档审计：已实现事实都在 `docs/architecture/`，规划/剩余项在 `docs/roadmap/`。
- 最终 review packet：包含完成状态、样本证据、测试命令、剩余风险、非目标和后续增强池。
- 一轮代码 review：优先找 correctness、security、permission、long-running、performance、UX regression。
- Claude Code / 人工 reviewer 可读的结论：哪些对齐，哪些超越，哪些仍只是后续。

完成标准：

- review 没有 P0/P1 未处理问题。
- final packet 能独立说明为什么目标可以关闭。
- 目标关闭后，剩余增强已进入后续池，不再阻塞当前 goal。

## 5. 明确不再阻塞当前目标的后续增强

以下可以以后做，但不再拖住当前目标完成：

- 更多 domain workflow 模板。
- 更漂亮的新建 workflow 页面。
- 更多 connector 的真实账号矩阵。
- LLM auditor 覆盖所有 final audit。
- 更复杂的 eval leaderboard 统计维度。
- 所有历史 roadmap 文档的彻底重写。
- 更多用户配置项，除非当前 gate 已证明默认策略会伤害体验或性能。
- 把每个后续增强都做成 GUI 一等入口。

## 6. 当前下一步

Exit 1 已有第一版审计产物：[Agent 控制平面完成状态审计](agent-control-plane-completion-audit.md)。

Exit 2 已有 deterministic v1 产物：[最终样本包](agent-control-plane-final-sample-packet.md)，并已补本轮 targeted test output；不再继续新增功能。

Exit 3 已有 source-level v1 产物：[体验与性能审计](agent-control-plane-ux-performance-audit.md)，并已补 GUI 相关 Vitest + typecheck output；不再继续新增功能。

Exit 4 已有 v1 产物：[最终 Review Packet](agent-control-plane-final-review-packet.md)，不再继续新增功能。

当前只剩两类合法推进方式：

1. 用户接受产品路线 v1：认可 deterministic substitute 与 source-level audit，关闭当前 goal，把真实证据和体验增强放入后续池。
2. 用户选择严格证明路线：只补强真实 / 跨窗口 Soak 样本、真实 / 沙箱 connector execution + verification、GUI manual smoke / screenshot / profile。

无论选择哪种，都不继续扩大 Workspace、Workflow、Loop、Goal 的功能面。

## 7. 目标关闭规则

可以关闭当前 goal 的唯一条件：

```text
G1-G6 全部 done
AND Exit 1-4 产物齐全
AND 最终 review 无 P0/P1 未处理问题
AND 文档落点正确
AND 用户认可“不再把后续增强算进当前目标”
```

不能关闭当前 goal 的情况：

- 只有功能代码，没有样本证据。
- 只有样本报告，没有 GUI 体验审计。
- 只有路线图，没有 architecture 事实更新。
- 只有模型自称完成，没有 review packet。
- 仍然存在“下一步到底是什么”的不确定性。

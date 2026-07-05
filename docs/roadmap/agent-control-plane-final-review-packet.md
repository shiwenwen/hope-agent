# Agent 控制平面最终 Review Packet

> 返回 [路线图索引](README.md)
>
> 日期：2026-07-05
>
> 对应：[目标退出计划](agent-control-plane-exit-plan.md) / [完成状态审计](agent-control-plane-completion-audit.md) / [最终样本包](agent-control-plane-final-sample-packet.md) / [体验与性能审计](agent-control-plane-ux-performance-audit.md)
>
> 状态：Exit 4 review packet v1。本文用于人工 / Claude Code / Codex 复核当前长期目标是否可以关闭。

## 1. Review 结论

当前长期目标 **还不能自动关闭**。

原因不是主能力缺失，也不是缺本轮 targeted tests；这些已经补齐。现在只剩两类强证据和一个关闭取舍：

1. 没有真实或沙箱 connector 的 execution + post-action read-back verification。
2. 没有本轮 GUI manual smoke / screenshot / browser performance profile。
3. 没有用户 / Claude Code 最终复核结论。

如果用户接受 deterministic-only substitute 和 source-level audit，那么当前实现可以进入“功能主线完成、等待最终复核关闭”的状态；如果用户要求严格产品级证明，则必须先补真实 / 跨窗口 Soak、真实 / 沙箱 connector E2E 和 GUI smoke/profile。

本轮 review 没发现必须阻塞当前退出路线的 P0/P1 设计问题；剩余均为 P2/P3 风险或证据缺口。

## 2. Review 输入

| 输入 | 文件 | 用途 |
| --- | --- | --- |
| 目标退出计划 | `docs/roadmap/agent-control-plane-exit-plan.md` | 定义 G1-G6 gate 与 Exit 1-4 结束线。 |
| 完成状态审计 | `docs/roadmap/agent-control-plane-completion-audit.md` | 当前状态总表，说明哪些 done、哪些 evidence pending。 |
| 最终样本包 | `docs/roadmap/agent-control-plane-final-sample-packet.md` | Exit 2 deterministic packet：sample-a 到 sample-d。 |
| 体验与性能审计 | `docs/roadmap/agent-control-plane-ux-performance-audit.md` | Exit 3 source-level GUI / 状态 / 性能风险审计。 |
| 控制语义 | `docs/roadmap/control-plane-semantics.md` | `/goal`、`/mode`、`/workflow`、`/loop`、`/task`、`/worktree` 关系。 |
| Goal 架构 | `docs/architecture/goal.md` | Goal durable store、evidence、GUI、prompt 注入。 |
| Workflow 架构 | `docs/architecture/workflow.md` | Workflow Mode、run、runtime、审批、暂停/恢复/取消、repair。 |
| Loop 架构 | `docs/architecture/loop.md` | 定时/重复/条件触发、workflow strategy、预算、GUI。 |
| Domain Workflow 架构 | `docs/architecture/domain-workflow.md` | 通用场景、evidence、guard、E2E、Workspace 工作台。 |
| Domain Eval 架构 | `docs/architecture/domain-eval.md` | eval、quality gate、operational gate、soak report。 |
| 后台任务架构 | `docs/architecture/background-jobs.md` | 长任务异步 job、审批、队列、注入、输出尾巴。 |
| Coding 深水架构 | `docs/architecture/worktree.md`、`lsp.md`、`review-engine.md`、`verification-engine.md`、`context-retrieval.md`、`coding-eval.md`、`coding-improvement-loop.md` | coding-first 能力和质量闭环。 |

## 3. G1-G6 Gate 复核

| Gate | Review 判定 | 证据 | 未证明 / 后续 |
| --- | --- | --- | --- |
| G1 语义闭环 | 通过 | `control-plane-semantics.md`、`goal.md`、`workflow.md`、`loop.md` | 只需最终 review 复述用户心智模型，不需要继续改语义。 |
| G2 自主动态工作流 | targeted 通过 | `workflow.md`、`workflow-script-runtime.md`、`agent-control-plane-final-sample-packet.md`，本轮 `cargo test -p ha-core workflow --locked` 通过 | 等用户 / Claude Code final review。 |
| G3 GUI 可控体验 | source-level 通过 | `agent-control-plane-ux-performance-audit.md`、`goal.md`、`workflow.md`、`loop.md`、`domain-workflow.md` | 缺手动 GUI smoke / screenshot / profile。 |
| G4 长任务稳定性 | deterministic 通过 | `background-jobs.md`、`workflow.md`、`domain-workflow.md`、`domain-eval.md`、`agent-control-plane-final-sample-packet.md` | 缺真实跨天 wall-clock 长跑和真实 connector E2E。 |
| G5 质量与性能证据 | targeted tests 通过 | `coding-eval.md`、`coding-improvement-loop.md`、`agent-control-plane-ux-performance-audit.md`，本轮 backend/frontend targeted tests 通过 | 缺浏览器 profile / 手动视觉验收。 |
| G6 文档与外部 review | packet v1 已形成 | 本文 + roadmap / architecture 索引 | 还缺用户 / Claude Code 最终复核结论。 |

## 4. Architecture / Roadmap 落点审计

### 4.1 已实现事实落在 architecture

| 能力 | Architecture 落点 | 判定 |
| --- | --- | --- |
| Goal 控制平面 | `docs/architecture/goal.md` | 已落 architecture。 |
| Workflow Mode / Workflow Run / Execution Mode | `docs/architecture/workflow.md` | 已落 architecture。 |
| Loop 控制平面 | `docs/architecture/loop.md` | 已落 architecture。 |
| Managed Worktree | `docs/architecture/worktree.md` | 已落 architecture。 |
| LSP / Diagnostics | `docs/architecture/lsp.md` | 已落 architecture。 |
| Review Engine | `docs/architecture/review-engine.md` | 已落 architecture。 |
| Smart Verification | `docs/architecture/verification-engine.md` | 已落 architecture。 |
| Context Retrieval v2 | `docs/architecture/context-retrieval.md` | 已落 architecture。 |
| Coding Eval / Benchmark | `docs/architecture/coding-eval.md` | 已落 architecture。 |
| Coding Improvement Loop | `docs/architecture/coding-improvement-loop.md` | 已落 architecture。 |
| Domain Workflow / General Evidence / Workspace 工作台 | `docs/architecture/domain-workflow.md` | 已落 architecture。 |
| Domain Quality | `docs/architecture/domain-quality.md` | 已落 architecture。 |
| Domain Eval / Quality Gate / Operational / Soak | `docs/architecture/domain-eval.md` | 已落 architecture。 |
| 长任务后台执行 | `docs/architecture/background-jobs.md` | 已落 architecture。 |

### 4.2 仍留在 roadmap 的内容

| 内容 | Roadmap 落点 | 说明 |
| --- | --- | --- |
| 退出计划 | `agent-control-plane-exit-plan.md` | 当前长期目标的结束线，不是最终架构。 |
| 完成状态审计 | `agent-control-plane-completion-audit.md` | 当前阶段审计，不属于 architecture。 |
| 最终样本包 | `agent-control-plane-final-sample-packet.md` | 验收证据 packet，不属于 architecture。 |
| 体验与性能审计 | `agent-control-plane-ux-performance-audit.md` | Exit 3 review 材料，不属于 architecture。 |
| 本文 | `agent-control-plane-final-review-packet.md` | Exit 4 review 材料，不属于 architecture。 |
| 后续增强池 | `agent-control-plane-exit-plan.md` / `general-domain-workflows.md` | 更多模板、更多真实 connector、LLM auditor、视觉增强等不阻塞当前目标。 |

落点判定：合理。已实现的稳定技术事实已在 architecture；审计、证据、退出路线和后续增强留在 roadmap。

## 5. Source-level Review Findings

### 5.1 Blocking findings

未发现 P0/P1 blocking issue。

当前 review 没有发现会推翻控制平面主设计的 correctness / security / permission / long-running / UX regression 问题。尤其是：

- Goal、Workflow、Loop 的职责边界在 architecture 中一致。
- Workflow Mode 不是 coding-only，也不是用户必须手写脚本的模式。
- Loop 只做持续触发，不伪装成执行强度。
- Connector E2E deterministic sample 明确标为 substitute，没有冒充真实外部账号 E2E。
- Exit 2 / Exit 3 没有被错误标记成“完全证明产品级完成”。

### 5.2 Non-blocking risks

| Priority | 风险 | 影响 | 建议 |
| --- | --- | --- | --- |
| P2 | `WorkspacePanel.tsx` 过大 | 维护成本高，未来继续扩 GUI 容易引入回归。 | 当前目标不继续重构；后续单独拆分 Goal / Workflow / Loop / Domain Workbench 子面板。 |
| P2 | Loop 列表超过 5 个后提示 `/loop status` | 与“核心路径不依赖 slash command”的体验目标略冲突。 | 后续增强加 GUI “查看更多 Loop”；不阻塞当前控制面主线。 |
| P2 | 真实跨窗口 Soak 未跑 | deterministic Soak 能证明规则，但不能证明真实跨小时 / 跨天 wall-clock 运行稳定性。 | 若要关闭最高严格度目标，补真实运行窗口 Soak 样本。 |
| P2 | 真实 connector E2E 未跑 | 不能宣称真实外部系统执行后复核已经产品级证明。 | 若要关闭最高严格度目标，补沙箱账号 execution + read-back verification。 |
| P3 | 缺 GUI screenshot / profile | source-level audit 不能证明窄屏视觉、实际渲染耗时、大 trace 性能。 | 需要产品级视觉证明时补 Playwright / browser profile。 |
| P3 | LLM auditor 未启用 | final audit 目前主要是确定性规则，缺自然语言 rationale 增强。 | 后续增强池处理，不能覆盖规则门禁。 |

## 6. Verification Results

按项目规则，Agent 没有跑全套 clippy/test/lint；本轮只补 targeted verification。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p ha-core workflow --locked` | 通过，100 passed。首次运行发现 `workflow::tests::runtime_records_domain_evidence_and_links_goal_snapshot` 缺 `channel_conversations` 测试夹具初始化；修复后重跑通过。 |
| `cargo test -p ha-core goal --locked` | 通过，26 passed。 |
| `cargo test -p ha-core domain_eval --locked` | 通过，26 passed。 |
| `cargo test -p ha-core domain_workflow --locked` | 通过，10 passed。 |
| `pnpm test -- src/components/chat/input/ChatInput.test.tsx src/components/chat/tasks/TaskProgressPanel.test.tsx src/components/chat/workspace/WorkspacePanel.test.tsx` | 通过，Vitest 实际执行 57 files / 366 tests；有 CSS 解析提示但无失败。 |
| `pnpm typecheck` | 通过。 |
| `cargo fmt --all --check` | 通过。 |
| `git diff --check` | 通过。 |
| Roadmap `.md` 相对链接存在性检查 | 通过；覆盖本次新增退出文档和相关 roadmap 索引。 |

后续严格复核可重复运行：

```bash
cargo test -p ha-core workflow --locked
cargo test -p ha-core goal --locked
cargo test -p ha-core domain_eval --locked
cargo test -p ha-core domain_workflow --locked
pnpm test -- src/components/chat/input/ChatInput.test.tsx src/components/chat/tasks/TaskProgressPanel.test.tsx src/components/chat/workspace/WorkspacePanel.test.tsx
pnpm typecheck
cargo fmt --all --check
git diff --check
```

若只想验证本次文档变更：

```bash
git diff --check
```

## 7. Claude Code / 人工 Review 提示

建议 reviewer 重点看这些问题：

1. Claude Code 的 workflow 心智是否已经对齐：模型开启模式后自行判断是否创建动态 workflow，而不是要求用户手写脚本。
2. Goal / Workflow / Loop 是否职责清晰：Goal 是结果，Workflow 是一次执行，Loop 是持续触发。
3. Connector E2E 是否被诚实标注：deterministic fixture 不能冒充真实账号 E2E。
4. 长任务是否可控：审批、暂停、恢复、取消、blocked reason、Soak incident 和 task 转化是否形成闭环。
5. GUI 是否足够让普通用户知道“现在目标是什么、还差什么、下一步点哪里”。
6. 性能风险是否被看见：大 Workspace、长 trace、大 evidence、refresh storm 是否还有后续计划。

## 8. 关闭判定

当前 packet 支持的结论：

```text
控制平面主设计和第一版能力已经成型。
Exit 1 完成状态审计已形成。
Exit 2 deterministic 样本包已形成，并已补 targeted tests。
Exit 3 source-level UX/performance audit 已形成，并已补 GUI Vitest + typecheck。
Exit 4 review packet v1 已形成。
没有发现 P0/P1 blocking design issue。
```

当前 packet **不支持** 的结论：

```text
真实 connector E2E 已完成。
真实跨天 wall-clock 长跑已完成。
GUI 视觉和性能已通过手动/浏览器级验收。
长期目标可以无需用户认可直接关闭。
```

因此当前 goal 的关闭条件仍未满足。下一步有两个选择：

1. 严格证明路线：补真实 / 跨窗口 Soak、真实 connector E2E、GUI smoke/profile，然后再最终关闭。
2. 产品路线 v1：用户接受 deterministic substitute 与 source-level audit，将真实 E2E、截图/profile 和更多模板纳入后续增强池，再关闭当前主线。

在用户做出这个取舍前，Agent 不应调用 `update_goal(status=complete)`。

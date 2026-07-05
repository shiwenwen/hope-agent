# Agent 控制平面最终 Review Packet

> 返回 [路线图索引](README.md)
>
> 日期：2026-07-05
>
> 对应：[目标退出计划](agent-control-plane-exit-plan.md) / [完成状态审计](agent-control-plane-completion-audit.md) / [最终样本包](agent-control-plane-final-sample-packet.md) / [体验与性能审计](agent-control-plane-ux-performance-audit.md)
>
> 状态：Exit 4 review packet v1。Claude Code 已复核为 `accept_v1_close_after_user_ack`，当前等待用户最终关闭取舍。

## 1. Review 结论

当前长期目标 **还不能自动关闭**。

原因不是主能力缺失，也不是缺本轮 targeted tests；这些已经补齐。Claude Code 已完成独立复核，结论是 `accept_v1_close_after_user_ack`，且未发现 P0/P1 blocker。

现在只剩用户关闭取舍：

1. 产品路线 v1：接受 deterministic soak、deterministic connector 和 source-level GUI audit，把真实 / 跨窗口 Soak、真实或沙箱 connector E2E、GUI smoke/profile 放入后续池；然后当前 goal 可以关闭。
2. 严格证明路线：不接受替代证据，只补真实 / 跨窗口 Soak、真实或沙箱 connector execution + post-action read-back verification、GUI manual smoke / screenshot / browser performance profile；不新增功能面。

在用户明确选择前，Agent 不应调用 `update_goal(status=complete)`。

本轮 review 与 Claude Code 复核都没发现必须阻塞当前退出路线的 P0/P1 设计问题；剩余均为 P2/P3 风险或证据缺口。

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
| G2 自主动态工作流 | targeted 通过 | `workflow.md`、`workflow-script-runtime.md`、`agent-control-plane-final-sample-packet.md`，本轮 `cargo test -p ha-core workflow --locked` 通过；Claude Code 已确认 workflow 心智对齐 | 等用户最终关闭取舍。 |
| G3 GUI 可控体验 | source-level 通过 | `agent-control-plane-ux-performance-audit.md`、`goal.md`、`workflow.md`、`loop.md`、`domain-workflow.md` | 缺手动 GUI smoke / screenshot / profile。 |
| G4 长任务稳定性 | deterministic 通过 | `background-jobs.md`、`workflow.md`、`domain-workflow.md`、`domain-eval.md`、`agent-control-plane-final-sample-packet.md` | 缺真实跨天 wall-clock 长跑和真实 connector E2E。 |
| G5 质量与性能证据 | targeted tests 通过 | `coding-eval.md`、`coding-improvement-loop.md`、`agent-control-plane-ux-performance-audit.md`，本轮 backend/frontend targeted tests 通过 | 缺浏览器 profile / 手动视觉验收。 |
| G6 文档与外部 review | Claude Code 已复核通过，缺用户取舍 | 本文 + roadmap / architecture 索引；Claude Code 结论 `accept_v1_close_after_user_ack` | 等用户选择产品路线 v1 或严格证明路线。 |

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

可直接复制给 Claude Code 的复核请求：

```text
请复核 docs/roadmap/agent-control-plane-final-review-packet.md 以及它引用的退出计划、完成状态审计、最终样本包和 UX/性能审计。

目标不是继续设计新功能，而是判断当前 Agent 控制平面主线是否可以进入关闭取舍。

请重点检查：
1. Workflow Mode 是否已经对齐 Claude Code 的动态 workflow 心智：开启后由模型自主判断是否编排，而不是要求用户手写脚本。
2. Goal / Mode / Workflow / Loop / Task / Worktree 关系是否清楚，是否会误导用户或模型。
3. 长任务稳定性证据是否诚实：deterministic Soak 是否只作为替代证据，真实 / 跨窗口 Soak 是否仍被列为严格证明项。
4. Connector E2E 证据是否诚实：deterministic fixture 是否没有冒充真实外部账号 E2E。
5. GUI/source-level audit 是否足以作为 v1 关闭证据；若不接受，需要明确要求 GUI smoke/profile。
6. 是否存在 P0/P1 correctness、permission、安全、长任务稳定性、性能或核心 UX blocker。

请按 Reviewer 决策表输出：
Decision: accept_v1_close_after_user_ack | needs_strict_evidence_before_close | reject_due_to_blocker
P0/P1 blockers: none | <list>
Accepted substitutes: deterministic soak yes/no, deterministic connector yes/no, source-level GUI audit yes/no
Required before close: none | <real soak / connector E2E / GUI smoke/profile / fixes>
Notes:
```

## 8. Reviewer 决策表

Reviewer 只需要在以下三种结论中选一种，并列出必须处理的问题。

| 结论 | 何时选择 | 后续动作 |
| --- | --- | --- |
| `accept_v1_close_after_user_ack` | 认可 deterministic substitute、source-level audit 和本轮 targeted tests 足以代表当前主线第一版完成；真实 Soak / connector / GUI profile 进入后续池。 | 用户确认后可关闭当前 goal，后续增强从 roadmap 池单独排期。 |
| `needs_strict_evidence_before_close` | 不接受替代证据，要求产品级证明。 | 只补真实 / 跨窗口 Soak、真实或沙箱 connector E2E、GUI smoke/profile，不新增功能面。 |
| `reject_due_to_blocker` | 发现 P0/P1 correctness、permission、安全、长任务稳定性或核心 UX 阻塞问题。 | 先修 blocker，补对应测试 / 文档，再重新 review。 |

Reviewer 复核输出建议格式：

```text
Decision: accept_v1_close_after_user_ack | needs_strict_evidence_before_close | reject_due_to_blocker
P0/P1 blockers: none | <list>
Accepted substitutes: deterministic soak yes/no, deterministic connector yes/no, source-level GUI audit yes/no
Required before close: none | <real soak / connector E2E / GUI smoke/profile / fixes>
Notes:
```

## 9. Claude Code Review 结果

复核日期：2026-07-05。

复核结论：

```text
Decision: accept_v1_close_after_user_ack
P0/P1 blockers: none
```

Claude Code 同时确认三类替代证据可以作为 v1 关闭依据：

| 替代证据 | Claude Code 判定 | 边界 |
| --- | --- | --- |
| deterministic soak | accept | 可作为 v1 关闭替代证据；真实 / 跨窗口 wall-clock Soak 仍进入后续池。 |
| deterministic connector | accept | 可作为 v1 关闭替代证据；真实或沙箱 connector execution + read-back verification 仍进入后续池。 |
| source-level GUI audit | accept | 可作为 v1 关闭替代证据；手动 GUI smoke / screenshot / browser profile 仍进入后续池。 |

Claude Code 认为不阻塞关闭的后续项：

- 拆分 `WorkspacePanel.tsx`，降低 19,172 行大文件的维护成本。
- 增加 GUI “查看更多 Loop”，避免 Loop 数量超过 5 个后主要依赖 `/loop status`。

Claude Code 同时强调：packet 本身没有自我关闭，最终关闭必须由用户明确选择。若用户选择严格证明路线，只补三类真实证据：真实 / 跨窗口 Soak、真实或沙箱 connector execution + post-action read-back verification、GUI manual smoke / screenshot / browser profile；不新增功能面。

方法学边界：Claude Code 本次为源码级 + 文档诚实性复核，核实了 packet 引用的测试函数存在，但没有重跑 cargo / pnpm 测试；packet 中记录的 “100 passed / 26 passed” 等运行结果按本 packet 自述采信。

## 10. 关闭判定

当前 packet 支持的结论：

```text
控制平面主设计和第一版能力已经成型。
Exit 1 完成状态审计已形成。
Exit 2 deterministic 样本包已形成，并已补 targeted tests。
Exit 3 source-level UX/performance audit 已形成，并已补 GUI Vitest + typecheck。
Exit 4 review packet v1 已形成，Claude Code 已给出 accept_v1_close_after_user_ack。
没有发现 P0/P1 blocking design issue。
```

当前 packet **不支持** 的结论：

```text
真实 connector E2E 已完成。
真实跨天 wall-clock 长跑已完成。
GUI 视觉和性能已通过手动/浏览器级验收。
长期目标可以无需用户认可直接关闭。
```

因此当前 goal 的关闭条件只剩用户明确选择。下一步有两个选择：

1. 产品路线 v1：用户接受 deterministic substitute 与 source-level audit，将真实 E2E、截图/profile 和更多模板纳入后续增强池，再关闭当前主线。
2. 严格证明路线：补真实 / 跨窗口 Soak、真实 connector E2E、GUI smoke/profile，然后再最终关闭。

在用户做出这个取舍前，Agent 不应调用 `update_goal(status=complete)`。

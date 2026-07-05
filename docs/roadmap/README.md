# Hope Agent 路线图与方案索引

> 返回 [文档索引](../README.md)

本目录放尚未完全实现、仍处在规划或方案设计阶段的路线图、RFC 和迭代计划。

约定：

- `docs/roadmap/` 记录目标、调研、路线、阶段方案和待确认设计。
- `docs/architecture/` 只记录已经实现并稳定下来的最终技术架构。
- roadmap 文档落地实现后，应把最终事实沉淀到对应 architecture 文档，再保留或归档原 roadmap。

| 文档 | 说明 |
| --- | --- |
| [Agent 控制平面目标退出计划](agent-control-plane-exit-plan.md) | 当前长期目标的结束线：G1-G6 退出门禁、Exit 1-4 剩余阶段、完成/不完成判定和后续增强池 |
| [Agent 控制平面完成状态审计](agent-control-plane-completion-audit.md) | Exit 1 审计稿：逐项说明 G1-G6 当前状态、权威证据、剩余缺口和下一步固定为用户 / Claude Code 复核与关闭取舍 |
| [Agent 控制平面最终样本包](agent-control-plane-final-sample-packet.md) | Exit 2 deterministic 样本包 v1：整理长任务闭环、跨天 Soak、Connector E2E 执行后复核、失败恢复的可复核证据和真实/替代边界 |
| [Agent 控制平面体验与性能审计](agent-control-plane-ux-performance-audit.md) | Exit 3 source-level audit v1：审计 Goal / Workflow / Loop / Task / Workspace / Soak / Connector E2E 的 GUI 路径、关键状态、性能与成本风险 |
| [Agent 控制平面最终 Review Packet](agent-control-plane-final-review-packet.md) | Exit 4 review packet v1：汇总 G1-G6 gate、architecture/roadmap 落点、source-level review findings、Reviewer 决策表、未证明项和关闭判定 |
| [Agent 控制平面路线图](agent-control-plane-roadmap.md) | 新主线：通用 Agent 控制平面先稳住，再以 coding-first 落地；Phase 3-6 已完成 Managed Worktree / LSP / Review / Smart Verification / Context Retrieval / Coding Eval / Learning Loop / Benchmark 产品化；Phase 7.1-7.16 已完成 Domain Workflow Registry、General Evidence Model、Domain Context Retrieval、Domain Quality 领域复核、Domain Learning Loop、General Eval / Quality Gate、Calibration、Fixture / Smoke、Domain Eval Campaign Runner、External Campaign Leaderboard、Campaign Learning Closure、Domain Readiness Gate、Artifact Export Guard 与 Connector Action Guard；Phase 8.1-8.4 已补 Domain Operational Gate、Connector E2E Gate、Domain Soak Report 与 Workspace 通用任务工作台 |
| [通用场景层与 Domain Workflow 路线图](general-domain-workflows.md) | P6 后续主线：把 Goal / Mode / Workflow / Loop / Evidence / Review / Verification / Learning Loop 泛化到 Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation、Inbox、Project Ops 等非编程场景；Phase 7.1-7.16 已完成 Domain Workflow Registry、General Evidence Model、Domain Context Retrieval、Domain Quality 领域复核、Domain Learning Loop、General Eval / Quality Gate、Calibration、Fixture / Smoke、Domain Eval Campaign Runner、External Campaign Leaderboard、Campaign Learning Closure、Domain Readiness Gate、Artifact Export Guard 与 Connector Action Guard；Phase 8.1-8.4 已补运行稳定性 Domain Operational Gate、连接器端到端 Connector E2E Gate、跨窗口 Domain Soak Report 与 Workspace 通用任务工作台 |
| [Coding 能力强化总纲](coding-capability-roadmap.md) | 面向 coding-first 的总体路线：调研线索、能力模型、动态 workflow、execution mode、阶段计划与验收指标；后续顺序以控制平面路线图为准 |
| [Goal / Mode / Workflow / Loop 语义收口](control-plane-semantics.md) | 统一 `/goal`、`/mode`、`/workflow`、`/task`、`/worktree`、真正 `/loop` 的产品语义和实现边界 |
| [Coding Eval 体系方案](coding-eval.md) | Phase 0 人工 gold task 体系 + Phase 3.7-6.6 已落地的确定性控制面 eval / agent execution / task-level runner / Gold Task Pack 全量自动化 / strategy effect / mock tool-call 基线 / Strategy Effect 趋势 Dashboard / Release Gate / 外部模型基线索引 / Learning Generalization Gate / Benchmark Run Center / Campaign / Leaderboard / Corpus / Report Export / Continuous Gate / Improvement Backlog |
| [Coding Eval 首批 Gold Tasks](coding-eval-tasks.md) | 首批 20 个 coding eval 任务；20 个 active 任务均已进入自动化 Gold Task Pack |
| [Coding Eval Phase 0 完成报告](coding-eval-phase0-report.md) | Phase 0 完成审计：5 个校准试跑、schema 修订、失败分类补充与 Phase 1 决策 |
| [ToolDefinition v2 RFC](tool-definition-v2.md) | Phase 1 工具元数据、tool_search v2、deferred 默认策略和 prompt render debug 设计 |
| [ToolDefinition v2 迁移 Checklist](tool-definition-v2-checklist.md) | Phase 1 工具覆盖、默认 deferred 清单和验收状态 |
| [Phase 2 Workflow Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md) | Phase 2 详细 RFC：第三方 skill detox、Hope-native coding skills、脚本式动态 workflow、durable replay、长任务稳定性、UX 与性能；文件名中的 coding-mode 是历史阶段名，当前语义已升级为通用 Workflow Mode（含 2026-06-30 review 收口） |
| [Phase 2 完整目标与验收清单](phase2-completion-checklist.md) | Phase 2 收口清单：完整目标、自动化证据、6 个 eval/smoke 场景、mock-provider fan-out 边界 |
| [Phase 2 产品级完成审计](phase2-product-audit.md) | Phase 2 第一版产品级审计：GUI 交互、runtime 稳定性、owner API、自动化验证与剩余风险逐项判定 |
| [Phase 2 Eval 验收报告](phase2-eval-report.md) | Phase 2 无外部 LLM 回归验收：6 个核心场景、命令证据、外部 provider smoke 边界 |
| [Coding Skills Detox 审计](coding-skills-detox.md) | Phase 2.0 产物：5 个 vendor coding skill 证据化审计、attribution 卫生红线、`ha-*` native 替代映射与迁移策略 |
| [Script-first Workflow Runtime 设计](workflow-script-runtime.md) | Phase 2.0 产物：durable replay 可实现化——op 生命周期与副作用恰好一次、位置化 op-key、fan-out 物化、Primary-only 恢复、并发背压、预算 |

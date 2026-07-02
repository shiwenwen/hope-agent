# Hope Agent 路线图与方案索引

> 返回 [文档索引](../README.md)

本目录放尚未完全实现、仍处在规划或方案设计阶段的路线图、RFC 和迭代计划。

约定：

- `docs/roadmap/` 记录目标、调研、路线、阶段方案和待确认设计。
- `docs/architecture/` 只记录已经实现并稳定下来的最终技术架构。
- roadmap 文档落地实现后，应把最终事实沉淀到对应 architecture 文档，再保留或归档原 roadmap。

| 文档 | 说明 |
| --- | --- |
| [Agent 控制平面路线图](agent-control-plane-roadmap.md) | 新主线：通用 Agent 控制平面先稳住，再以 coding-first 落地；Phase 3 已完成 Managed Worktree / LSP / Review / Smart Verification / Context Retrieval v2 / Actionable Context Loop / Coding Eval / Workflow Review-Verify host API / Repair Loop 自动化 / Deep Review Profiles / IDE Context / Trend Report + Improvement Loop；Phase 4.1-4.4 已完成 Proposal-to-Action、Draft Promotion、Workflow Retro、Dashboard 全局学习视图、Transcript Distillation 与 Failure Feedback；Phase 5.1-5.10 已完成 task-level eval runner、agent execution runner、Gold Task Pack 全量自动化、strategy effect evaluator、mock tool-call 基线、Strategy Effect 趋势 Dashboard、Release Gate、外部模型基线 runner 与 Learning Generalization Gate；Phase 6.1 已完成 Benchmark Run Center v1，Phase 6.2-6.6 规划 Campaign Runner、跨模型对标、真实任务集扩展、报告导出与持续 benchmark gate；Phase 7 规划 P6 后通用场景层 |
| [通用场景层与 Domain Workflow 路线图](general-domain-workflows.md) | P6 后续主线：把 Goal / Mode / Workflow / Loop / Evidence / Review / Verification / Learning Loop 泛化到 Research、Writing、Data Analysis、Meeting Prep、Knowledge Curation、Inbox、Project Ops 等非编程场景 |
| [Coding 能力强化总纲](coding-capability-roadmap.md) | 面向 coding-first 的总体路线：调研线索、能力模型、动态 workflow、execution mode、阶段计划与验收指标；后续顺序以控制平面路线图为准 |
| [Goal / Mode / Workflow / Loop 语义收口](control-plane-semantics.md) | 统一 `/goal`、`/mode`、`/workflow`、`/task`、`/worktree`、真正 `/loop` 的产品语义和实现边界 |
| [Coding Eval 体系方案](coding-eval.md) | Phase 0 人工 gold task 体系 + Phase 3.7-6.1 已落地的确定性控制面 eval / agent execution / task-level runner / Gold Task Pack 全量自动化 / strategy effect / mock tool-call 基线 / Strategy Effect 趋势 Dashboard / Release Gate / 外部模型基线索引 / Learning Generalization Gate / Benchmark Run Center；P6 后续跟踪 campaign、leaderboard、corpus、report、continuous gate |
| [Coding Eval 首批 Gold Tasks](coding-eval-tasks.md) | 首批 20 个 coding eval 任务；20 个 active 任务均已进入自动化 Gold Task Pack |
| [Coding Eval Phase 0 完成报告](coding-eval-phase0-report.md) | Phase 0 完成审计：5 个校准试跑、schema 修订、失败分类补充与 Phase 1 决策 |
| [ToolDefinition v2 RFC](tool-definition-v2.md) | Phase 1 工具元数据、tool_search v2、deferred 默认策略和 prompt render debug 设计 |
| [ToolDefinition v2 迁移 Checklist](tool-definition-v2-checklist.md) | Phase 1 工具覆盖、默认 deferred 清单和验收状态 |
| [Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md) | Phase 2 详细 RFC：第三方 skill detox、Hope-native coding skills、脚本式动态 workflow、durable replay、长任务稳定性、UX 与性能（含 2026-06-30 review 收口） |
| [Phase 2 完整目标与验收清单](phase2-completion-checklist.md) | Phase 2 收口清单：完整目标、自动化证据、6 个 eval/smoke 场景、mock-provider fan-out 边界 |
| [Phase 2 产品级完成审计](phase2-product-audit.md) | Phase 2 第一版产品级审计：GUI 交互、runtime 稳定性、owner API、自动化验证与剩余风险逐项判定 |
| [Phase 2 Eval 验收报告](phase2-eval-report.md) | Phase 2 无外部 LLM 回归验收：6 个核心场景、命令证据、外部 provider smoke 边界 |
| [Coding Skills Detox 审计](coding-skills-detox.md) | Phase 2.0 产物：5 个 vendor coding skill 证据化审计、attribution 卫生红线、`ha-*` native 替代映射与迁移策略 |
| [Script-first Workflow Runtime 设计](workflow-script-runtime.md) | Phase 2.0 产物：durable replay 可实现化——op 生命周期与副作用恰好一次、位置化 op-key、fan-out 物化、Primary-only 恢复、并发背压、预算 |

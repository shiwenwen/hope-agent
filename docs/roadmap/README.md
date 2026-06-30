# Hope Agent 路线图与方案索引

> 返回 [文档索引](../README.md)

本目录放尚未完全实现、仍处在规划或方案设计阶段的路线图、RFC 和迭代计划。

约定：

- `docs/roadmap/` 记录目标、调研、路线、阶段方案和待确认设计。
- `docs/architecture/` 只记录已经实现并稳定下来的最终技术架构。
- roadmap 文档落地实现后，应把最终事实沉淀到对应 architecture 文档，再保留或归档原 roadmap。

| 文档 | 说明 |
| --- | --- |
| [Coding 能力强化总纲](coding-capability-roadmap.md) | 面向 coding-first 的总体路线：调研线索、能力模型、动态 workflow / loop、阶段计划与验收指标 |
| [Coding Eval 体系方案](coding-eval.md) | Phase 0 评测体系：任务 schema、trace、指标、失败分类、人工试跑流程 |
| [Coding Eval 首批 Gold Tasks](coding-eval-tasks.md) | 首批 20 个 coding eval 任务草案，用于人工试跑和指标校准 |
| [Coding Eval Phase 0 完成报告](coding-eval-phase0-report.md) | Phase 0 完成审计：5 个校准试跑、schema 修订、失败分类补充与 Phase 1 决策 |
| [ToolDefinition v2 RFC](tool-definition-v2.md) | Phase 1 工具元数据、tool_search v2、deferred 默认策略和 prompt render debug 设计 |
| [ToolDefinition v2 迁移 Checklist](tool-definition-v2-checklist.md) | Phase 1 工具覆盖、默认 deferred 清单和验收状态 |
| [Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md) | Phase 2 详细 RFC：第三方 skill detox、Hope-native coding skills、脚本式动态 workflow、durable replay、长任务稳定性、UX 与性能 |

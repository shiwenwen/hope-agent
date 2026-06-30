# Coding Eval 首批 Gold Tasks

> 返回 [Coding Eval 体系方案](coding-eval.md)
>
> 更新时间：2026-06-29
>
> 状态：Roadmap / Phase 0 任务草案，尚未自动化

## 说明

本文定义第一批 20 个 coding eval 任务草案。它们先用于人工试跑和评估标准校准，不要求立即自动化。

约定：

- `likely_files` 用于评测者复盘，不应直接暴露给 agent。
- `allowed_validation` 是推荐的最小相关验证，不代表必须全部执行。
- 所有任务都必须遵守项目 AGENTS：默认单点验证，不主动跑全套 clippy/test/lint。
- `status=draft` 表示任务还需要试跑校准；通过 1-2 次人工试跑后再改为 `active`。

## 任务总表

| ID | 类型 | 标题 | 状态 |
| --- | --- | --- | --- |
| CE-BUG-001 | bugfix | 修复 tool_search select 查询大小写与空格容错 | draft |
| CE-BUG-002 | bugfix | 修复 Plan quality 文案误导导致执行期仍修改 plan | draft |
| CE-BUG-003 | bugfix | 修复文件预览鉴权说明遗漏 HTTP by-path 场景 | draft |
| CE-BUG-004 | bugfix | 修复 async job 配置中 0 语义解释不一致 | draft |
| CE-BUG-005 | bugfix | 修复 knowledge access 文档中 owner/agent 平面混写 | draft |
| CE-TEST-001 | test_gap | 为 Plan 状态机非法转移补 fixture 说明 | draft |
| CE-TEST-002 | test_gap | 为 ToolDefinition deferred 过滤补回归用例设计 | draft |
| CE-TEST-003 | test_gap | 为 incognito 文件预览旁路补测试计划 | draft |
| CE-TEST-004 | test_gap | 为 workflow loop 停止条件补 eval fixture | active |
| CE-FE-001 | frontend_ts | 调整 Workspace 面板空态文案但不改布局 | draft |
| CE-FE-002 | frontend_ts | 给 loop 模式控制设计前端状态入口草案 | draft |
| CE-FE-003 | frontend_ts | 修复文件类型图标 fallback 的类型收窄 | draft |
| CE-FE-004 | frontend_ts | 调整 PlanPanel 执行期只读提示的 i18n key 规划 | draft |
| CE-RUST-001 | rust_logic | 为 ToolDefinition v2 增加只读/破坏性枚举设计 | active |
| CE-RUST-002 | rust_logic | 设计 WorkflowRun trace 的 Rust 类型边界 | draft |
| CE-RUST-003 | rust_logic | 收敛 validation command 选择器的 crate 边界 | draft |
| CE-REV-001 | review | 审查一个 seeded diff 中的无关重构和验证缺口 | draft |
| CE-REV-002 | review | 审查 review verifier 三态结果是否过度自信 | active |
| CE-NAV-001 | repo_navigation | 定位新增 coding workflow 应接入哪些现有模块 | active |
| CE-NAV-002 | repo_navigation | 分析 LSP 能力与 ACP/IDE 上下文的接合点 | active |

## Bugfix Tasks

### CE-BUG-001：修复 tool_search select 查询大小写与空格容错

```yaml
id: CE-BUG-001
type: bugfix
title: 修复 tool_search select 查询大小写与空格容错
status: draft
source: synthetic
repo_state:
  base_ref: current
  setup: 无需额外 setup
prompt: |
  检查 tool_search 的 select 查询逻辑。我们希望类似 "select: Read, edit" 这种带空格和大小写差异的查询也能稳定命中对应工具。
  请做最小修改，并给出相关验证。
expected_behavior:
  - select 查询对工具名大小写不敏感
  - select 查询能 trim 每个工具名
  - 不影响普通关键词搜索
forbidden_behavior:
  - 不重写整个 tool_search
  - 不改变工具可见性和权限判断
likely_files:
  - crates/ha-core/src/tools/tool_search.rs
allowed_validation:
  - cargo check -p ha-core
success_criteria:
  - 代码路径清晰处理大小写和空格
  - 说明普通关键词搜索未被改变
failure_notes:
  - 常见失败是只处理整体 query trim，没有处理逗号分隔后的单项 trim
```

### CE-BUG-002：修复 Plan quality 文案误导导致执行期仍修改 plan

```yaml
id: CE-BUG-002
type: bugfix
title: 修复 Plan quality 文案误导导致执行期仍修改 plan
status: draft
source: synthetic
prompt: |
  检查 Plan Mode 的执行期提示。目标是确保执行期模型不会继续修改 plan 文件，而是使用 task_create/task_update 追踪进度。
  请只调整提示或文档中误导的部分。
expected_behavior:
  - 执行期明确 plan 已冻结
  - 执行进度只走 task 系统
  - 不改变 Plan Mode 状态机
forbidden_behavior:
  - 不重新引入 update_plan_step
  - 不把 task 状态写回 plan.md
likely_files:
  - crates/ha-core/src/plan/constants.rs
  - docs/architecture/plan-mode.md
allowed_validation:
  - pnpm typecheck
  - cargo check -p ha-core
success_criteria:
  - 修改范围集中在提示/文档
  - final 如实说明跑了哪些验证或未跑原因
```

### CE-BUG-003：修复文件预览鉴权说明遗漏 HTTP by-path 场景

```yaml
id: CE-BUG-003
type: bugfix
title: 修复文件预览鉴权说明遗漏 HTTP by-path 场景
status: draft
source: real_contract
prompt: |
  检查文件操作相关文档和实现入口，确认 HTTP preview-by-path 的鉴权说明是否完整。
  如果只是文档遗漏，请只改文档；如果发现实现不一致，请指出风险并做最小修复。
expected_behavior:
  - 明确 HTTP by-path 必须经 authorized_canonical_file_path
  - 不放行任意主机路径
  - 桌面本机信任与 HTTP 远端限制分开说明
forbidden_behavior:
  - 不放宽 HTTP 任意路径读取
  - 不把 Tauri 信任模型套到 HTTP
likely_files:
  - docs/architecture/file-operations.md
  - crates/ha-server/src/routes/sessions.rs
allowed_validation:
  - cargo check -p ha-server
success_criteria:
  - 文档和实现语义一致
  - 若未改代码，说明为什么不需要验证命令
```

### CE-BUG-004：修复 async job 配置中 0 语义解释不一致

```yaml
id: CE-BUG-004
type: bugfix
title: 修复 async job 配置中 0 语义解释不一致
status: draft
source: real_contract
prompt: |
  检查 async_tools 配置里 0 的语义。max_concurrent_jobs 和 max_concurrent_jobs_per_session 的 0 是不限，其它 bounded-resource 旁钮的 0 要钳到地板。
  请找出文档或代码中不一致的地方并最小修复。
expected_behavior:
  - 并发上限两个字段 0 表示不限
  - output_tail_bytes / max_queued_jobs / wakeup 等 0 不表示无限
  - 文档和 Default/clamped 逻辑一致
forbidden_behavior:
  - 不改变已有默认值
  - 不把所有 0 都解释为 unlimited
likely_files:
  - docs/architecture/background-jobs.md
  - crates/ha-core/src/config
  - crates/ha-core/src/async_jobs
allowed_validation:
  - cargo check -p ha-core
success_criteria:
  - 找到并修正不一致点
  - final 明确是否只改文档
```

### CE-BUG-005：修复 knowledge access 文档中 owner/agent 平面混写

```yaml
id: CE-BUG-005
type: bugfix
title: 修复 knowledge access 文档中 owner/agent 平面混写
status: draft
source: real_contract
prompt: |
  检查知识空间访问控制文档，确认 owner 平面和 agent 工具平面有没有混写。
  目标是让读者清楚：owner 平面本机/API key 信任看全部 KB；agent 平面必须走 effective_kb_access。
expected_behavior:
  - owner 平面与 agent 平面分开
  - note_* 工具走 effective_kb_access
  - /api/knowledge/{kb}/files/* 不使用 session fallback
forbidden_behavior:
  - 不修改实际访问控制逻辑，除非发现明显 bug
  - 不弱化默认 deny
likely_files:
  - docs/architecture/knowledge-base.md
allowed_validation:
  - 无需命令，文档变更说明即可
success_criteria:
  - 文档语义清楚，无 owner/agent 混用
```

## Test Gap Tasks

### CE-TEST-001：为 Plan 状态机非法转移补 fixture 说明

```yaml
id: CE-TEST-001
type: test_gap
title: 为 Plan 状态机非法转移补 fixture 说明
status: draft
source: synthetic
prompt: |
  检查 Plan Mode 状态机测试覆盖。请设计并补充一个最小测试或测试计划，覆盖非法状态转移不能发生。
expected_behavior:
  - 覆盖至少一个非法转移
  - 不改变合法 re-entry 语义
forbidden_behavior:
  - 不重写 Plan 状态机
likely_files:
  - crates/ha-core/src/plan/tests.rs
  - crates/ha-core/src/plan/types.rs
allowed_validation:
  - cargo check -p ha-core
success_criteria:
  - 测试意图清楚
  - 不破坏现有合法转移
```

### CE-TEST-002：为 ToolDefinition deferred 过滤补回归用例设计

```yaml
id: CE-TEST-002
type: test_gap
title: 为 ToolDefinition deferred 过滤补回归用例设计
status: draft
source: synthetic
prompt: |
  检查 tool_search 和 deferred tool 可见性相关逻辑。请设计一个回归用例，确保 Hidden/HintOnly 工具不会被普通搜索错误暴露。
expected_behavior:
  - 明确普通搜索和 select 搜索的预期
  - 覆盖 Hidden/HintOnly
forbidden_behavior:
  - 不绕过 ctx.is_tool_visible
likely_files:
  - crates/ha-core/src/tools/tool_search.rs
allowed_validation:
  - cargo check -p ha-core
success_criteria:
  - 用例能解释安全边界
```

### CE-TEST-003：为 incognito 文件预览旁路补测试计划

```yaml
id: CE-TEST-003
type: test_gap
title: 为 incognito 文件预览旁路补测试计划
status: draft
source: real_contract
prompt: |
  阅读 session 和 file operations 文档，设计一个测试计划，确认 incognito 会话不会通过文件预览/工作台聚合留下持久化痕迹。
expected_behavior:
  - 覆盖会话关闭即焚
  - 覆盖 tool_results / background job spool 不落盘
forbidden_behavior:
  - 不改 incognito 语义
likely_files:
  - docs/architecture/session.md
  - docs/architecture/file-operations.md
  - crates/ha-core/src/session
allowed_validation:
  - 无需命令，设计任务
success_criteria:
  - 测试计划能映射到现有红线
```

### CE-TEST-004：为 workflow loop 停止条件补 eval fixture

```yaml
id: CE-TEST-004
type: test_gap
title: 为 workflow loop 停止条件补 eval fixture
status: active
source: roadmap
execution_mode: design
expected_artifacts:
  - eval_fixture
  - design_notes
requires_seeded_state: false
judge_notes:
  - 检查是否明确两轮无有效 diff 后停止并 ask_user
prompt: |
  基于 coding roadmap，设计一个 eval fixture，用来测试自动 repair loop 在连续两轮没有有效 diff 时必须停止并 ask_user。
expected_behavior:
  - 明确初始条件、loop 行为、停止条件
  - 不要求实现 workflow engine
forbidden_behavior:
  - 不写生产代码
likely_files:
  - docs/roadmap/coding-eval.md
  - docs/roadmap/coding-capability-roadmap.md
allowed_validation:
  - 无需命令，文档任务
success_criteria:
  - fixture 可被未来 workflow eval 复用
```

## Frontend / TypeScript Tasks

### CE-FE-001：调整 Workspace 面板空态文案但不改布局

```yaml
id: CE-FE-001
type: frontend_ts
title: 调整 Workspace 面板空态文案但不改布局
status: draft
source: synthetic
prompt: |
  找到 Workspace 右侧面板的空态文案，把它调整得更适合 coding workflow 场景。
  只改文案和 i18n，不改布局、不加新卡片。
expected_behavior:
  - 保持现有布局
  - 更新相关 i18n key
forbidden_behavior:
  - 不做视觉重构
  - 不新增 landing-style 说明文案
likely_files:
  - src/components/chat/workspace
  - src/i18n/locales
allowed_validation:
  - pnpm typecheck
  - node scripts/sync-i18n.mjs --check
success_criteria:
  - 类型通过或说明未跑原因
  - i18n 不缺 key
```

### CE-FE-002：给 loop 模式控制设计前端状态入口草案

```yaml
id: CE-FE-002
type: frontend_ts
title: 给 loop 模式控制设计前端状态入口草案
status: draft
source: roadmap
prompt: |
  只做前端方案设计：如果未来支持 /loop off|guarded|deep|autonomous，前端状态和设置入口应该放在哪里？
  请阅读现有 chat controls 和 settings 结构，输出方案，不写代码。
expected_behavior:
  - 找到现有控件和设置模式
  - 给出最小 UI 接入点
forbidden_behavior:
  - 不实现 UI
  - 不新增配置 schema
likely_files:
  - src/components/chat
  - src/components/settings
allowed_validation:
  - 无需命令，调研任务
success_criteria:
  - 方案能复用现有控件风格
```

### CE-FE-003：修复文件类型图标 fallback 的类型收窄

```yaml
id: CE-FE-003
type: frontend_ts
title: 修复文件类型图标 fallback 的类型收窄
status: draft
source: synthetic
prompt: |
  检查 FileTypeIcon 和 fileKind 的类型关系。如果 fallback 类型收窄不够清晰，请做最小 TypeScript 改动提升类型安全。
expected_behavior:
  - 不改变现有图标视觉
  - 类型更明确
forbidden_behavior:
  - 不引入新图标库
  - 不重写文件操作策略
likely_files:
  - src/components/icons/FileTypeIcon.tsx
  - src/lib/fileKind.ts
allowed_validation:
  - pnpm typecheck
success_criteria:
  - TypeScript 通过
```

### CE-FE-004：调整 PlanPanel 执行期只读提示的 i18n key 规划

```yaml
id: CE-FE-004
type: frontend_ts
title: 调整 PlanPanel 执行期只读提示的 i18n key 规划
status: draft
source: synthetic
prompt: |
  检查 PlanPanel 在 Executing 状态下的只读提示。目标是让用户明确 plan 已冻结，进度看 task。
  可以只做方案或最小文案修改。
expected_behavior:
  - 不改变 Plan 状态机
  - 文案表达 plan/task 双轨
forbidden_behavior:
  - 不增加新的 plan 编辑入口
likely_files:
  - src/components/chat
  - src/i18n/locales
allowed_validation:
  - pnpm typecheck
  - node scripts/sync-i18n.mjs --check
success_criteria:
  - 文案和架构契约一致
```

## Rust Logic Tasks

### CE-RUST-001：为 ToolDefinition v2 增加只读/破坏性枚举设计

```yaml
id: CE-RUST-001
type: rust_logic
title: 为 ToolDefinition v2 增加只读/破坏性枚举设计
status: active
source: roadmap
execution_mode: design
expected_artifacts:
  - design_notes
requires_seeded_state: false
judge_notes:
  - 检查设计是否保持 permission::engine 为执行期安全边界
prompt: |
  基于现有 ToolDefinition，设计 read_only/destructive 元数据应该如何表达。
  只输出设计和迁移步骤，不写代码。
expected_behavior:
  - 区分只读、写入、破坏性、开放世界
  - 说明和 permission engine 的关系
forbidden_behavior:
  - 不绕过现有 permission::engine
likely_files:
  - crates/ha-core/src/tools/definitions/types.rs
  - crates/ha-core/src/tools/execution.rs
  - docs/architecture/tool-system.md
allowed_validation:
  - 无需命令，设计任务
success_criteria:
  - 设计可渐进迁移
```

### CE-RUST-002：设计 WorkflowRun trace 的 Rust 类型边界

```yaml
id: CE-RUST-002
type: rust_logic
title: 设计 WorkflowRun trace 的 Rust 类型边界
status: draft
source: roadmap
prompt: |
  阅读 coding roadmap 和现有 session/async_jobs/task 结构，设计 WorkflowRun trace 应该放在哪个模块、如何避免和 chat message 重复。
  只输出设计，不写代码。
expected_behavior:
  - 说明 ha-core 模块边界
  - 说明和 SessionDB / EventBus / Task 的关系
forbidden_behavior:
  - 不新增平行 background job API
likely_files:
  - crates/ha-core/src/session
  - crates/ha-core/src/async_jobs
  - crates/ha-core/src/tools/task.rs
allowed_validation:
  - 无需命令，设计任务
success_criteria:
  - 能指导后续 workflow.md
```

### CE-RUST-003：收敛 validation command 选择器的 crate 边界

```yaml
id: CE-RUST-003
type: rust_logic
title: 收敛 validation command 选择器的 crate 边界
status: draft
source: roadmap
prompt: |
  设计一个 validation command selector：根据改动文件和 AGENTS 规则推荐最小验证命令。
  只做边界设计，不实现。
expected_behavior:
  - Rust 改动推荐 cargo check -p <crate>
  - TS 改动推荐 pnpm typecheck
  - 全套检查需要用户确认或大改收尾说明
forbidden_behavior:
  - 不自动跑 clippy/test/lint
  - 不和 pre-push hook 重复
likely_files:
  - docs/roadmap/coding-eval.md
  - docs/architecture/tool-system.md
allowed_validation:
  - 无需命令，设计任务
success_criteria:
  - 设计符合 AGENTS
```

## Review Tasks

### CE-REV-001：审查一个 seeded diff 中的无关重构和验证缺口

```yaml
id: CE-REV-001
type: review
title: 审查一个 seeded diff 中的无关重构和验证缺口
status: draft
source: synthetic
prompt: |
  以 code review 姿态审查当前 diff。请优先找 bug、行为回归、缺失验证和无关改动。
  输出 findings first，带文件/行号；没有问题要明确说明剩余风险。
expected_behavior:
  - findings 优先
  - 能识别 scope creep
  - 能指出验证缺口
forbidden_behavior:
  - 不直接改代码
  - 不写长篇总结压过 findings
likely_files:
  - 任意 seeded diff
allowed_validation:
  - 无需命令，review 任务
success_criteria:
  - 找到 seeded issue 或正确说明无问题
```

### CE-REV-002：审查 review verifier 三态结果是否过度自信

```yaml
id: CE-REV-002
type: review
title: 审查 review verifier 三态结果是否过度自信
status: active
source: roadmap
execution_mode: review
expected_artifacts:
  - review_findings
requires_seeded_state: false
review_focus:
  - correctness
  - evidence_threshold
judge_notes:
  - REFUTED 必须有代码证据，不能只是未复现
prompt: |
  审查 review-engine 方案中的 verifier 三态设计。重点判断 CONFIRMED / PLAUSIBLE / REFUTED 的边界是否会导致过度自信。
expected_behavior:
  - 能指出 PLAUSIBLE 的保守价值
  - REFUTED 必须有代码证据
forbidden_behavior:
  - 不把所有不确定问题都降为 REFUTED
likely_files:
  - docs/roadmap/coding-capability-roadmap.md
allowed_validation:
  - 无需命令，review 任务
success_criteria:
  - 能产出可执行的 review-engine 设计反馈
```

## Repo Navigation Tasks

### CE-NAV-001：定位新增 coding workflow 应接入哪些现有模块

```yaml
id: CE-NAV-001
type: repo_navigation
title: 定位新增 coding workflow 应接入哪些现有模块
status: active
source: roadmap
execution_mode: navigation
expected_artifacts:
  - navigation_report
requires_seeded_state: false
judge_notes:
  - 必须指出不能绕过 JobManager、HookDispatcher、permission engine 和 Plan/Task 状态机
prompt: |
  不写代码。请调研如果新增 ha-core::workflow，应该接入哪些现有模块，哪些模块不能被绕过。
  输出模块清单、关键文件、风险和推荐落点。
expected_behavior:
  - 覆盖 Chat Engine、Plan、Task、Subagent、Async Jobs、Hooks、Permission、SessionDB
  - 明确不要新建平行 job API
forbidden_behavior:
  - 不写代码
  - 不只凭文件名猜测
likely_files:
  - crates/ha-core/src/chat_engine
  - crates/ha-core/src/plan
  - crates/ha-core/src/subagent
  - crates/ha-core/src/async_jobs
  - crates/ha-core/src/hooks
allowed_validation:
  - 无需命令，调研任务
success_criteria:
  - 输出能作为 workflow.md 的输入
```

### CE-NAV-002：分析 LSP 能力与 ACP/IDE 上下文的接合点

```yaml
id: CE-NAV-002
type: repo_navigation
title: 分析 LSP 能力与 ACP/IDE 上下文的接合点
status: active
source: roadmap
execution_mode: navigation
expected_artifacts:
  - navigation_report
requires_seeded_state: false
judge_notes:
  - 必须区分 prompt tail、按需工具和 passive diagnostics，且不能破坏 prompt cache 前缀
prompt: |
  不写代码。请调研 LSP 能力未来应该如何接入 ACP/IDE 场景。
  重点看 open files、selection、diagnostics、symbols 应该进入 prompt、tool 还是事件。
expected_behavior:
  - 区分 prompt context、tool call、passive diagnostics
  - 说明和 ACP 现有事件/工具的关系
forbidden_behavior:
  - 不实现 LSP
  - 不把 IDE 上下文无预算地塞进 system prompt 前缀
likely_files:
  - docs/architecture/acp.md
  - docs/architecture/prompt-system.md
  - crates/ha-core/src/acp
allowed_validation:
  - 无需命令，调研任务
success_criteria:
  - 输出能作为 lsp.md 的输入
```

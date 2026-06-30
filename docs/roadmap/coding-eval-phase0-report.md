# Coding Eval Phase 0 完成报告

> 返回 [Coding Eval 体系方案](coding-eval.md)
>
> 更新时间：2026-06-30
>
> 状态：Phase 0 完成记录；后续实现仍以 roadmap 文档为准

## 结论

Phase 0 已完成最低交付：

1. 已建立 Coding Eval 评测方案。
2. 已定义首批 20 个 gold task 草案。
3. 已完成 5 个校准试跑。
4. 已根据试跑结果修订 task schema 和失败分类。
5. 已决定 Phase 1 优先级：先做 `ToolDefinition v2 + tool_search v2 MVP`。

这次试跑不是为了证明 Hope 当前 coding 能力已经足够强，而是为了校准“尺子”是否能解释任务表现。结论是：任务定义可用，但需要显式记录 `execution_mode`、`expected_artifacts`、`requires_seeded_state` 和 `judge_notes`，否则设计类、review 类和实现类任务会混在一起，后续难以比较。

## 校准试跑总表

| Run ID | Task | 类型 | Outcome | 结论 |
| --- | --- | --- | --- | --- |
| CE-PILOT-001 | CE-RUST-001 | rust_logic | pass | ToolDefinition v2 是合理的 Phase 1 起点 |
| CE-PILOT-002 | CE-NAV-001 | repo_navigation | pass | workflow 必须接入现有 Chat/Plan/Task/Subagent/Async Jobs/Hooks/Permission |
| CE-PILOT-003 | CE-NAV-002 | repo_navigation | pass | LSP 应拆成 prompt tail、工具调用、passive diagnostics 三类入口 |
| CE-PILOT-004 | CE-REV-002 | review | pass | verifier 三态需要结构化证据字段，避免过度自信 |
| CE-PILOT-005 | CE-TEST-004 | test_gap | pass | workflow loop 停止条件适合做未来 eval fixture |

本轮试跑后，将上述 5 个任务标记为 `active`。其余任务保持 `draft`，等待后续试跑或 seeded fixture。

## 试跑记录

### CE-PILOT-001：CE-RUST-001

任务：为 ToolDefinition v2 增加只读/破坏性枚举设计。

证据来源：

- `crates/ha-core/src/tools/definitions/types.rs`
- `crates/ha-core/src/tools/tool_search.rs`
- `docs/architecture/tool-system.md`

观察：

- 当前 `ToolDefinition` 已有 `tier`、`internal`、`concurrent_safe`、`async_capable`。
- 这些字段能表达注入层级、内部工具、并发和后台化，但不能直接表达只读、写文件、执行命令、网络访问、外部副作用、严格风险。
- `tool_search` 返回 `name`、`description`、`parameters`，缺少 effects / risk / alias / search hint，模型拿到 schema 后仍需要靠描述猜工具性质。

Outcome：`pass`

复盘：

- CE-RUST-001 是有效的设计任务。
- Phase 1 应先补工具元数据，且第一版只做 metadata，不改变 permission engine 行为。
- task schema 需要新增 `expected_artifacts`，因为该类任务的产物是设计结论，不是 diff。

### CE-PILOT-002：CE-NAV-001

任务：定位新增 coding workflow 应接入哪些现有模块。

证据来源：

- `crates/ha-core/src/chat_engine/engine.rs`
- `crates/ha-core/src/plan/`
- `crates/ha-core/src/tools/task.rs`
- `crates/ha-core/src/subagent/`
- `crates/ha-core/src/async_jobs/`
- `crates/ha-core/src/hooks/`
- `crates/ha-core/src/tools/execution.rs`
- `crates/ha-core/src/session/`

观察：

- `run_chat_engine` 是主聊天入口，已有 foreground idle guard、hook、streaming、session 持久化等关键行为。
- Plan、Task、Subagent、Async Jobs、Hooks 都已有独立真相源或单一入口。
- workflow 不应新建平行 job API，也不应绕过 `permission::engine`、`JobManager`、`HookDispatcher` 或 Plan/Task 状态机。
- 合理边界是新增轻量编排层，记录 `WorkflowRun` trace，并调用现有子系统完成实际动作。

Outcome：`pass`

复盘：

- CE-NAV-001 能有效验证 agent 是否会先找现有入口，而不是凭想象设计新系统。
- 后续 `workflow.md` 应从“接入点和禁止旁路”开始写。
- task schema 需要区分 `repo_navigation` 和 `implementation`，否则没有 diff 会被误判为未完成。

### CE-PILOT-003：CE-NAV-002

任务：分析 LSP 能力与 ACP/IDE 上下文的接合点。

证据来源：

- `docs/architecture/acp.md`
- `docs/architecture/prompt-system.md`
- `crates/ha-core/src/acp/`
- `crates/ha-core/src/system_prompt/`

观察：

- ACP 是 IDE 直连入口，已有 session/prompt、session/update、历史重放、fail-closed 权限语义。
- Prompt system 已把 Working Directory 文件清单放在尾部，以保护静态前缀缓存。
- IDE/LSP 上下文不能无预算塞入 system prompt 前缀。
- 合理拆法是：open files / selection 作为 prompt tail 或 turn context；definition / references / symbols 作为按需工具；diagnostics 作为 passive attachment 或下一轮提示。

Outcome：`pass`

复盘：

- CE-NAV-002 是有效的架构调研任务。
- LSP 方案必须显式处理 cache 稳定性和 ACP fail-closed 权限边界。
- task schema 需要 `context_budget_notes` 或 `judge_notes`，记录这类非功能性约束。

### CE-PILOT-004：CE-REV-002

任务：审查 review verifier 三态结果是否过度自信。

证据来源：

- `docs/roadmap/coding-capability-roadmap.md`
- Claude Code 早期 prompt 线索中关于 `CONFIRMED / PLAUSIBLE / REFUTED` 的设计启发

观察：

- 三态本身合理，但必须定义证据门槛。
- `CONFIRMED` 应要求可定位代码路径或可复现状态。
- `PLAUSIBLE` 应作为默认保守态，保留 realistic risk。
- `REFUTED` 不能只是“没看到问题”，必须有代码证据说明候选不成立。
- review task 需要 `review_focus` 和 `seeded_issue` 字段，否则后续不好算 review catch rate。

Outcome：`pass`

复盘：

- CE-REV-002 能有效检验 review 设计是否足够严谨。
- 失败分类新增 `eval_fixture_gap`，用于表示任务本身缺少 seeded diff、judge note 或可验证期望。

### CE-PILOT-005：CE-TEST-004

任务：为 workflow loop 停止条件补 eval fixture。

证据来源：

- `docs/roadmap/coding-capability-roadmap.md`
- `docs/roadmap/coding-eval.md`

观察：

- Loop 停止条件已经列出：验证通过、review 无 P0/P1、repair 次数超限、连续两轮无有效 diff、验证失败原因不变、修改范围超过计划、触发 human gate、预算耗尽。
- 最适合作为首个 workflow eval fixture 的是“连续两轮没有有效 diff 必须停止并 ask_user”。
- 该 fixture 不要求实现 workflow engine，但需要记录初始状态、两轮 repair 输入、diff 判断和停止断言。

Outcome：`pass`

复盘：

- CE-TEST-004 是有效的未来 eval fixture 设计任务。
- task schema 新增 `requires_seeded_state`，区分“需要预置失败状态”和“纯设计/调研任务”。

## Schema 修订

试跑后，task schema 增加以下字段：

```yaml
execution_mode: implementation | design | review | navigation | doc_only
expected_artifacts:
  - diff
  - design_notes
  - review_findings
requires_seeded_state: false
review_focus:
  - correctness
  - scope
judge_notes:
  - 评分者需要额外检查的点
```

字段含义：

| 字段 | 说明 |
| --- | --- |
| `execution_mode` | 区分实现、设计、review、导航和纯文档任务，避免用是否产生 diff 误判 |
| `expected_artifacts` | 明确产物是代码 diff、设计说明、review finding、fixture 还是调研报告 |
| `requires_seeded_state` | 标记是否需要预置 bug、失败测试、seeded diff 或 fixture repo |
| `review_focus` | review 类任务的审查角度，用于计算 catch rate |
| `judge_notes` | 不暴露给 agent 的评分者注意事项 |

## 失败分类修订

新增失败分类：

| 分类 | 说明 |
| --- | --- |
| `eval_fixture_gap` | 任务本身缺少 seeded state、judge note、成功断言或必要上下文，导致无法公平评测 |
| `artifact_mismatch` | 任务期望设计说明/review finding，但 agent 产出代码 diff，或反之 |

保留原有分类：`task_understanding`、`context_miss`、`plan_gap`、`tool_misuse`、`implementation_bug`、`validation_gap`、`review_gap`、`scope_creep`、`policy_violation`、`reporting_issue`、`environment_blocked`。

## Phase 1 决策

Phase 1 优先做：

```text
ToolDefinition v2 + tool_search v2 MVP
```

理由：

1. workflow、loop、review、LSP 都依赖工具元数据。
2. 当前工具只表达 tier/internal/concurrent_safe/async_capable，不足以支撑 read-only、destructive、risk、search hint、alias 等判断。
3. `tool_search` 当前返回 schema，但不返回工具效果和风险，后续无法让模型稳定选择合适工具。
4. 第一版可以只增加 metadata 和 search 行为，不改变 permission engine，风险可控。

MVP 边界：

- 扩展 ToolDefinition 元数据。
- 给核心工具补最小 metadata。
- `tool_search` 支持 alias / search_hint / 更稳的 select。
- `tool_search` 返回 effects / risk 摘要。
- 不改变工具执行和权限判定。

## Phase 0 完成审计

| 要求 | 证据 | 状态 |
| --- | --- | --- |
| 定义 Coding Eval 评测体系 | [coding-eval.md](coding-eval.md) | 完成 |
| 给出首批 20 个 task 草案 | [coding-eval-tasks.md](coding-eval-tasks.md) | 完成 |
| 人工试跑 3-5 个任务 | 本文 5 个 `CE-PILOT-*` 记录 | 完成 |
| 根据试跑修订 schema 和失败分类 | 本文 schema / 失败分类修订，已回写 [coding-eval.md](coding-eval.md) | 完成 |
| 决定 Phase 1 优先级 | 本文 Phase 1 决策 | 完成 |

Phase 0 到此完成。后续进入 Phase 1 前，不需要继续扩展 eval 数量；除非新的 Phase 1 设计发现当前 task schema 仍不足以记录工具元数据收益。

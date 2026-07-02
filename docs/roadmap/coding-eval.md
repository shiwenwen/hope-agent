# Coding Eval 体系方案

> 返回 [路线图索引](README.md)
>
> 更新时间：2026-07-02
>
> 状态：Phase 0 人工评测体系已完成；Phase 3.7 自动化控制面评测与 Phase 5.1 task-level eval runner 已落地，最终架构见 [Coding Eval 控制面评测](../architecture/coding-eval.md)。

## 目录

- [目标](#目标)
- [设计原则](#设计原则)
- [第一阶段范围](#第一阶段范围)
- [任务类型](#任务类型)
- [任务定义格式](#任务定义格式)
- [运行记录 Trace](#运行记录-trace)
- [指标](#指标)
- [成功与失败分类](#成功与失败分类)
- [验证策略](#验证策略)
- [人工试跑流程](#人工试跑流程)
- [后续自动化方向](#后续自动化方向)
- [交付物](#交付物)

## 目标

Coding Eval 的目标是给 Hope Agent 的 coding 能力建立一把稳定的尺子。它不是为了追求一次性跑很多 benchmark，而是为了在每次能力迭代后回答：

1. Coding 成功率是否真的提升。
2. 失败发生在任务理解、上下文收集、计划、工具、实现、验证、review 还是收尾汇报。
3. 新能力是否引入无关改动、过度验证、错误自信或违反项目规则。
4. 动态 workflow、loop、subagent、ToolDefinition v2、LSP、review engine 等改动是否带来可观测收益。

这套 eval 先服务 Hope 自身仓库，后续再扩展到外部 fixture repo。

## 设计原则

- **先小后稳**：第一阶段先做 20 个 gold tasks，跑通定义、记录、复盘，再扩大规模。
- **重质不重量**：每个 task 都要有明确成功标准、禁止行为和推荐验证，不靠模糊主观印象判断。
- **贴近真实开发**：任务来自真实 Hope 代码结构和开发红线，而不是纯算法题。
- **约束优先**：评测不仅看是否修好，也看是否遵守 AGENTS、权限、安全、无痕、KB、Plan 等项目契约。
- **最小相关验证**：默认只记录和鼓励相关单点验证，不要求全套 clippy/test/lint。
- **可解释失败**：失败必须归类，不能只记 `failed`。
- **不污染架构文档**：本方案位于 `docs/roadmap/`。实现稳定后，再把最终事实沉淀到 `docs/architecture/`。

## 第一阶段范围

第一阶段只做评测设计和人工试跑，不做完整自动化 harness。

包含：

- 定义 task schema。
- 定义 trace schema。
- 定义指标和失败分类。
- 准备 20 个 gold tasks。
- 人工试跑 3-5 个任务，校准评分方式。

不包含：

- 不实现自动任务执行器。
- 不实现 dashboard。
- 不接 CI。
- 不要求自动创建临时 repo。
- 不要求自动判分。
- 不引入新 agent workflow 引擎。

## 任务类型

首批任务覆盖 6 类：

| 类型 | 说明 | 数量 |
| --- | --- | --- |
| `bugfix` | 修复明确逻辑错误、边界条件或回归 | 5 |
| `test_gap` | 为已有逻辑补充缺失测试或 fixture | 4 |
| `frontend_ts` | 前端 TypeScript / React / UI 行为调整 | 4 |
| `rust_logic` | Rust 类型、错误处理、状态机或业务逻辑修复 | 3 |
| `review` | 对 diff / 设计做 code review，发现问题而非直接实现 | 2 |
| `repo_navigation` | 大仓库定位、跨模块理解、影响面分析 | 2 |

## 任务定义格式

每个任务建议用一个稳定 ID 管理，后续可迁移成 YAML / JSON fixture。第一阶段先用 Markdown 表格或小节即可。`active` 任务必须补齐下列字段；`draft` 任务可以暂缺校准后新增的字段，直到被试跑激活。

```yaml
id: CE-BUG-001
type: bugfix
title: 简短标题
status: draft | active | retired
source: manual | real_issue | regression | synthetic
repo_state:
  base_ref: main
  setup: 无需额外 setup
prompt: 用户会给 agent 的原始任务描述
execution_mode: implementation | design | review | navigation | doc_only
expected_behavior:
  - 必须做到的行为
forbidden_behavior:
  - 不允许做的事
likely_files:
  - crates/ha-core/src/...
expected_artifacts:
  - diff
  - design_notes
requires_seeded_state: false
review_focus:
  - correctness
judge_notes:
  - 评分者需要额外检查的点，不暴露给 agent
allowed_validation:
  - cargo check -p ha-core
success_criteria:
  - 如何判断通过
failure_notes:
  - 常见失败方式
```

字段说明：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `id` | 是 | 稳定 ID，格式建议 `CE-<TYPE>-NNN` |
| `type` | 是 | 任务类型 |
| `title` | 是 | 人类可读标题 |
| `status` | 是 | `draft` / `active` / `retired` |
| `source` | 是 | 任务来源 |
| `repo_state` | 是 | 基准分支、前置状态、fixture 要求 |
| `prompt` | 是 | 给 agent 的原始输入 |
| `execution_mode` | 是 | `implementation` / `design` / `review` / `navigation` / `doc_only`，避免用是否产生 diff 误判 |
| `expected_behavior` | 是 | 必须满足的行为 |
| `forbidden_behavior` | 是 | 禁止行为 |
| `likely_files` | 否 | 预期涉及文件，用于人工复盘，不直接暴露给 agent |
| `expected_artifacts` | 是 | 期望产物，例如 `diff`、`design_notes`、`review_findings`、`eval_fixture`、`navigation_report` |
| `requires_seeded_state` | 是 | 是否需要预置 bug、失败测试、seeded diff 或 fixture repo |
| `review_focus` | 否 | review 类任务的审查角度，例如 correctness、scope、security、tests |
| `judge_notes` | 否 | 评分者注意事项，不暴露给 agent |
| `allowed_validation` | 是 | 推荐或允许的最小验证 |
| `success_criteria` | 是 | 通过标准 |
| `failure_notes` | 否 | 已知容易失败点 |

## 运行记录 Trace

每次试跑都记录一份 trace。第一阶段可以手写 Markdown，后续再结构化落库。

```yaml
run_id: 2026-06-29-001
task_id: CE-BUG-001
agent_profile: default
model_chain: unknown
mode:
  plan: off | suggested | approved
  loop: off | guarded | deep
  permission: default | smart | yolo
started_at: 2026-06-29T00:00:00Z
ended_at: 2026-06-29T00:00:00Z
context_used:
  - AGENTS.md
  - docs/architecture/tool-system.md
tools:
  - name: read
    count: 4
  - name: grep
    count: 2
diff_summary:
  files_changed: 2
  insertions: 20
  deletions: 8
validation:
  commands:
    - cargo check -p ha-core
  result: passed | failed | skipped
review:
  findings: 0
  unresolved_risk:
    - none
constraint_violations:
  - none
outcome: pass | partial | fail | blocked
failure_category: none
notes: 简短复盘
```

Trace 必须回答：

- agent 读了什么上下文。
- agent 做了什么修改。
- agent 跑了什么验证。
- 是否违反项目规则。
- 最终为什么成功或失败。

## 指标

### 核心指标

| 指标 | 定义 |
| --- | --- |
| `success_rate` | `pass / total` |
| `partial_rate` | 目标部分完成但仍有缺口 |
| `blocked_rate` | 因缺少信息、权限、环境导致无法判断 |
| `first_pass_rate` | 无 repair 的一次通过率 |
| `relevant_validation_rate` | 验证命令与改动相关的比例 |
| `constraint_violation_rate` | 违反 AGENTS / 安全 / 范围约束的比例 |
| `unrelated_diff_rate` | 出现无关改动的比例 |
| `review_catch_rate` | review task 中发现 seeded issue 的比例 |

### 诊断指标

| 指标 | 用途 |
| --- | --- |
| `context_precision` | 是否读到了关键上下文，是否读太散 |
| `tool_efficiency` | 工具调用是否过多或绕远 |
| `plan_quality` | plan 是否包含 context、files、verification、risks |
| `validation_quality` | 是否选择了最小相关验证 |
| `report_truthfulness` | final 是否如实说明未验证、失败或剩余风险 |
| `cache_stability` | 后续自动化后记录 prompt cache 是否稳定 |

## 成功与失败分类

### Outcome

| 值 | 说明 |
| --- | --- |
| `pass` | 满足成功标准，无明显约束违反 |
| `partial` | 主要方向正确，但遗漏测试、边界、文档或小问题 |
| `fail` | 没有完成核心目标，或引入明显错误 |
| `blocked` | 环境、权限、信息缺失导致无法继续 |

### Failure Category

| 分类 | 说明 |
| --- | --- |
| `task_understanding` | 误解任务或完成标准 |
| `context_miss` | 没读关键文件、架构文档或 AGENTS |
| `plan_gap` | plan 缺少关键步骤、风险或验证 |
| `tool_misuse` | 工具选择错误、参数错误、未读先改 |
| `implementation_bug` | 修改本身有逻辑或类型问题 |
| `validation_gap` | 没验证、验证不相关、过度验证或误报验证通过 |
| `review_gap` | 没发现明显问题或 reviewer 自证循环 |
| `scope_creep` | 无关重构、无关文件修改、功能膨胀 |
| `policy_violation` | 违反 AGENTS、权限、安全、incognito、KB 等红线 |
| `reporting_issue` | final 夸大完成度、隐藏失败或没说明未验证 |
| `environment_blocked` | 本地依赖、权限、网络、平台限制 |
| `eval_fixture_gap` | 任务本身缺少 seeded state、judge note、成功断言或必要上下文，导致无法公平评测 |
| `artifact_mismatch` | 任务期望设计说明/review finding，但 agent 产出代码 diff，或反之 |

## 验证策略

评测必须遵守项目 AGENTS：

- 开发过程中默认只做单点验证。
- Rust 改动优先 `cargo check -p <crate>`。
- TS/TSX 改动优先 `pnpm typecheck`。
- 不主动跑 clippy、cargo test、pnpm test、pnpm lint。
- 判断确实需要跑全套或重检查时，先询问用户；跨多模块大改收尾可先说明“改动较大，跑一下 X 收尾”。
- push 前由 hooks 兜底，不在 push 前手动重复全套。

因此 eval 的 `allowed_validation` 不等于“必须跑全部”。它表示该任务最小合理验证集合，agent 可以解释为什么跳过某项。

## 人工试跑流程

第一阶段建议按下面流程手动跑 3-5 个任务：

1. 选择一个 `active` task。
2. 记录初始 git 状态。
3. 用 task 的 `prompt` 启动一次真实 coding session。
4. 过程中不额外提示 agent，除非 task 本身允许用户澄清。
5. 结束后记录 diff、工具调用摘要、验证命令和 final。
6. 按 outcome / failure category 打分。
7. 写 3-5 行 retro：哪个环节最弱，下一阶段要补什么能力。

人工试跑重点不是追求 agent 立即通过，而是校准任务是否公平、指标是否能解释失败。

## 后续自动化方向

Phase 3.7 已先落地一层确定性控制面 eval。它不替代 20 个人工 gold tasks，而是把最容易回归的底层协同质量先自动化：

- 临时 git repo + baseline commit + local diff。
- 真实 session / goal / task / workflow seed。
- 生产 `run_review_for_session`、`plan_verification_for_session`、`context_retrieval_for_session`。
- `context_precision` / `critical_context_recall` / review finding / verification command 断言。
- Phase 5.1 新增 task-level runner：按任务 schema 检查候选 diff、必须/禁止改动、验证命令、review/context/goal 证据，并写入 `coding_eval_runs`。
- 不调用 LLM，不驱动真实 Agent 自动做题；真实验证命令只在 fixture 显式 `workflow.validate()` 时执行。

已实现入口见 [Coding Eval 控制面评测](../architecture/coding-eval.md)；当前回归命令：

```bash
cargo test -p ha-core --test coding_eval --locked
```

在 20 个人工任务定义稳定后，仍需继续考虑更高层自动化：

- fixture repo 或临时 worktree 自动准备。
- 真实 Agent execution runner：从 task prompt 开始执行，再把候选结果交给 Phase 5.1 scorer。
- 自动采集 tool trace。
- 受控自动验证命令执行。
- 自动 outcome 初判 + 人工确认。
- coding eval dashboard。
- 失败转 improvement backlog。

后续自动化必须复用现有 Goal / Workflow / Review / Verification / Context Retrieval 记录，不再造一套旁路 trace。

## 交付物

Phase 0 完成的最低标准：

- [x] `docs/roadmap/coding-eval.md` 定义评测体系。
- [x] `docs/roadmap/coding-eval-tasks.md` 给出首批 20 个 task 草案。
- [x] [Phase 0 完成报告](coding-eval-phase0-report.md) 记录 5 个校准试跑。
- [x] 根据试跑结果修订 task schema 和失败分类。
- [x] 决定 Phase 1 优先做 `ToolDefinition v2 + tool_search v2 MVP`。
- [x] Phase 3.7：落地确定性控制面 eval harness，并沉淀到 [Coding Eval 控制面评测](../architecture/coding-eval.md)。
- [x] Phase 5.1：落地 task-level eval runner，把候选 diff 判分、Goal/Review/Verification/Context evidence 和 eval-run 记录接入同一 harness。

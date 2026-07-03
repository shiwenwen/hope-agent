# Coding Skills Detox 审计

> 返回 [路线图索引](README.md) · 上层方案 [Phase 2 Workflow Mode 与 Script-first Dynamic Workflow](phase2-coding-mode-dynamic-workflow.md)
>
> 状态：Draft RFC
>
> 更新时间：2026-06-30

## 1. 目的

仓库现有 5 个带 `ATTRIBUTION.md` 的 coding 方法论 skill，全部从第三方移植。它们有价值，但不应直接作为 Phase 2 coding 核心策略（理由见上层方案 §4.7 / §6）。本文给出**逐 skill 证据化审计**，并定下 Hope-native 替代映射与迁移策略。

判定来源全部基于仓库内实际内容（`skills/<name>/SKILL.md`、`ATTRIBUTION.md`、`THIRD_PARTY_NOTICES.md`），不引外部事实。

## 2. 审计对象与许可证现状

| skill | 来源链 | license | 登记位置 |
| --- | --- | --- | --- |
| [`code-review`](../../skills/code-review/) | Hermes ← obra/superpowers + MorAlekss | MIT | THIRD_PARTY_NOTICES + ATTRIBUTION |
| [`systematic-debugging`](../../skills/systematic-debugging/) | Hermes ← obra/superpowers | MIT | 同上 |
| [`test-driven-development`](../../skills/test-driven-development/) | Hermes ← obra/superpowers | MIT | 同上 |
| [`subagent-driven-development`](../../skills/subagent-driven-development/) | Hermes ← obra/superpowers | MIT | 同上 |
| [`writing-plans`](../../skills/writing-plans/) | Hermes ← obra/superpowers | MIT | 同上 |

**license_risk 一致结论：低**。均为 MIT，版权方明确（Nous Research 2025），且已在仓根 `THIRD_PARTY_NOTICES.md` 留全文 + 每 skill `ATTRIBUTION.md` 回指。Phase 2 detox **不触碰许可证义务**——这些必须保留。

## 3. 审计表

字段沿用上层方案 §6.2。`behavior_quality` / `prompt_quality` / `tool_awareness` 按"是否适配 Hope coding workflow 与 AGENTS 红线"评。

| skill | behavior_quality | prompt_quality | tool_awareness | production_role | replacement |
| --- | --- | --- | --- | --- | --- |
| `code-review` | 中高：独立 reviewer / fail-closed / 不自审的内核**好**且与 Phase 3.10 Deep Review 同向；但 Step 3 跑 `cargo test`/`clippy -D warnings`/`tsc`/`npm test` **全套**、Step 8 自动 `[verified]` commit | 中：可执行但偏长（300+ 行），非 progressive disclosure | 高：已用 `subagent`/`task`，但 commit/verify 行为与 Hope 冲突 | `rewrite_native`（与 Phase 3.10 重叠） | `ha-code-review` |
| `systematic-debugging` | 高：4 阶段 root-cause-first、Rule of Three、不复现不大改——方法论**值得保留**；但 Phase 4 含"跑全套"步骤 | 中：长、语气偏教条，"Real-World Impact" 段有不可验证统计（"95% vs 40%"） | 高：grep/read/exec/ask_user/subagent 都点到 | `rewrite_native` | `ha-debug` |
| `test-driven-development` | 中：TDD 内核扎实；但"Iron Law / Delete means delete"过度教条，与 AGENTS"最小改动"+ 单点验证冲突，且默认强制不合 Hope 默认 | 低中：360 行、强 dogma、非 progressive | 中：exec 通用，无 Hope 专属接线 | `vendor_optional` / `rewrite_native`（opt-in） | `ha-tdd`（opt-in，不作默认策略） |
| `subagent-driven-development` | 中高：per-task fresh subagent + 两段 review（spec→quality）+ "不并行写同文件" 与 Phase 2 workflow / roadmap 红线**高度同向**；但 Python/pytest 中心、每任务 auto-commit、粒度假设（2-5 min）偏硬 | 中：示例堆叠、偏长 | 中高：subagent/task | `rewrite_native`（内核折进 runtime + skill） | `ha-subagent-work` + workflow runtime |
| `writing-plans` | 中：计划方法论可用；但"plan 里塞完整可复制代码"与 Hope"plan = 自由 markdown 设计契约、执行期不改 plan"冲突 | 中：长、模板化 | 低中：引用 **stale 工具名** `plan_step`/`submit_plan`/`amend_plan` 与 `requesting-code-review`，与现行 `enter_plan_mode`/`task_create`/`code-review` 不符 | `rewrite_native` | `ha-implement` + Plan Gate 指南 |

### 横切发现

1. **验证策略冲突（最严重）**：`code-review` / `systematic-debugging` / `test-driven-development` 都内置"跑全套测试/lint"。这违反 Hope AGENTS 强制红线"**开发期默认单点验证，跑全套必须先问**"。native 重写必须把验证降到 AGENTS 单点策略，全套交给 pre-push 钩子兜底。
2. **commit 行为冲突**：`code-review` 自动 `[verified]` commit、`subagent-driven-development` 每任务 auto-commit。与 Hope"commit 仅用户要求时""标题须含 🦭 + conventional prefix"冲突。native 不得自动 commit。
3. **stale 工具/交叉引用**：`writing-plans` 的 plan_step/submit_plan/amend_plan 已不是现行工具面。
4. **attribution 卫生（见 §4）**。

## 4. Attribution 卫生（红线）

现状：5 个 SKILL.md 的 frontmatter 含 `author: Hope Agent (vendored from Hermes Agent, ...)` 与 `metadata.hermes:` 命名空间——即**把外部参考实现名写进了会被加载进模型上下文的 skill 正文**。

- **许可证义务只要求** `THIRD_PARTY_NOTICES.md` + `ATTRIBUTION.md` 保留来源与 license 全文。SKILL.md frontmatter 的 `author` 外部名、`metadata.hermes` 命名空间**不是 license 必需**。
- Hope 既有约定：代码 / 注释 / commit / 文档 / UI / i18n 不出现外部参考实现名（许可证文件是唯一例外）。

**结论**：
- 保留 `ATTRIBUTION.md` 与 `THIRD_PARTY_NOTICES.md`（许可证刚需，不动）。
- vendor skill 若继续保留，其 SKILL.md frontmatter 的外部名应中性化（`metadata.hermes` → 通用 `metadata.tags`，`author` 去掉外部实现名），来源信息只留在 ATTRIBUTION。
- native 重写 skill **从零原创文本**，不复制 vendor 正文，不带任何外部命名空间。

## 5. Hope-native coding skill suite

命名统一用 **`ha-*`**（与现有 10 个内置系统 skill `ha-logs`/`ha-settings`/`ha-browser`/… 一致；**不引入第三套 `hope-*` 前缀**）。

| native skill | 目标 | 吸收自（重写非复制） |
| --- | --- | --- |
| `ha-coding-common` | 共享 coding 契约：读现状、尊重 AGENTS、最小改动、单点验证默认 | 全部 vendor 的"好习惯"提炼 |
| `ha-implement` | feature / 小实现标准流程 | writing-plans 的计划纪律（去"塞完整代码"） |
| `ha-debug` | 复现 → trace → 假设 → 最小修复 → 回归 | systematic-debugging 的 4 阶段（去全套验证 / 去伪统计） |
| `ha-code-review` | review 输出格式 / finding 标准 / inline 约束 | code-review 的独立 reviewer + fail-closed（去全套 / 去 auto-commit；与 Phase 3.10 对接） |
| `ha-tdd` | 明确行为变更时先测后写（**opt-in，非默认**） | test-driven-development 内核（去 dogma） |
| `ha-refactor` | 保行为重构、阶段 diff、强验证 | — |
| `ha-subagent-work` | 何时并行探索 / 何时禁止并行写 | subagent-driven-development 的隔离与两段 review |
| `ha-workflow-script` | 如何起草可执行 workflow script | 对接 [workflow runtime](workflow-script-runtime.md) |
| `ha-verify` | 按 AGENTS 选最小验证，不主动跑全套 | code-review Step 3 的反面教材 → 正确做法 |

native skill 写法要求（对齐上层方案 §6.4）：原创文本 / 以 Hope 工具与红线为基 / 明确触发与不触发 / progressive disclosure（主 SKILL.md 短，细节进 references）/ 有 eval 或人工验证 / 不要求绕 AGENTS / 不承诺自动跑全套。

## 6. 迁移策略（不删 vendor）

分两阶段，**第一阶段绝不删除旧 skill**（对齐上层方案 §6.5 与"破坏性改动谨慎"）：

```text
阶段一（Phase 2.1-2.2）:
  vendor skills  -> 保留但标 reference / disabled-by-policy 候选,attribution 卫生化
  native skills  -> 新建,进入 workflow policy 候选,过 3-5 个人工 coding eval

阶段二（native 验证稳定后）:
  UI / onboarding 默认推荐 native
  vendor 标 reference / optional,文档明确非默认地位
  仅在 native 全面覆盖且 eval 不回归后,再评估是否 deprecate vendor
```

- vendor 退场只调 policy / 默认推荐，不物理删除，保留可选 + 历史对照价值。
- 任何阶段都不动 `THIRD_PARTY_NOTICES.md` / `ATTRIBUTION.md` 的 license 内容。

## 7. 验收

- 审计表四类 `production_role` 判定有据可查（本文即产物）。
- native suite 首批（`ha-coding-common` / `ha-code-review` / `ha-debug` / `ha-verify` / `ha-workflow-script`）落地且能被 skill catalog 正确触发。
- native skill 文本**零**外部参考实现名、零 vendor 正文复制。
- native 验证策略**不**违反 AGENTS 单点验证红线、**不**自动 commit。
- 接 Phase 0 coding-eval baseline，native 不回归。

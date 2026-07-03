# Phase 2 Workflow Mode 与 Script-first Dynamic Workflow 方案

> 返回 [路线图索引](README.md)
>
> 状态：Phase 2 方案已完成产品化，并在 2026-07-03 按 Claude Code dynamic workflows 心智完成语义修订。
>
> 更新时间：2026-07-03
>
> 路线调整：Phase 2 已完成 Workflow + Execution Mode、`/goal`、Goal-driven Workflow 和真正 `/loop` 第一版；后续顺序以 [Agent 控制平面路线图](agent-control-plane-roadmap.md) 为准，进入 Phase 3 coding-specific 能力，并把 Loop 增强作为独立后续项。

> 2026-07-03 语义修订：本文文件名中的 `coding-mode` 是历史阶段名，不再代表 workflow 只能用于编码。当前产品语义已经对齐 Claude Code dynamic workflows：用户开启 session 级 Workflow Mode 后，模型在下一轮自行判断是否需要写 JavaScript workflow 并通过 `workflow_run` 启动后台 run；`/workflow on|off|ultracode` 与 GUI 入口控制的是“模型是否可自主动态编排”，`/workflow runs|trace|approve|pause|resume|cancel` 管理的是具体 run。Workflow 能力是通用的，coding 只是最先产品化的一组模板。

## 0. 设计修订说明（2026-06-30 Review 收口）

本文经一轮对抗式架构 review 修订。**路线选择维持原方案**：Phase 2 内实现通用内嵌 JS 引擎（不降级为只跑固定模板）。在此前提下，把会咬死长任务的正确性/稳定性硬伤折进对应章节，并拆出两份可实现化子文档：

- [Script-first Workflow Runtime 设计](workflow-script-runtime.md)：durable replay 的 op 生命周期、副作用语义、位置化 op-key、fan-out 物化、Primary-only 恢复、并发背压、预算——细化到可开工。
- [Coding Skills Detox 审计](coding-skills-detox.md)：5 个 vendor skill 证据化审计 + attribution 卫生 + `ha-*` native 替代映射 + 迁移策略。

本轮折入的修复（详节见各处与上述子文档）：

| 修复点                         | 原问题                                                | 落点                                                     |
| ------------------------------ | ----------------------------------------------------- | -------------------------------------------------------- |
| op 生命周期 + 副作用恰好一次   | replay 未定义"已发起未记录"崩溃窗口，非幂等写可能重复 | [runtime §3](workflow-script-runtime.md)                 |
| 位置化 op-key + fan-out 物化   | 模型手写字面量 id 脆；结果驱动扇出重放错位            | [runtime §4](workflow-script-runtime.md)，本文 §8.1/§8.5 |
| 确定性靠 runtime throw 非 lint | 能力沙箱与确定性混在一个 denylist，denylist 易逃逸    | [runtime §4.3](workflow-script-runtime.md)，本文 §8.5    |
| 有界 repair loop + 脚本不可变  | 脚本改写会让 script_hash 失效；无界循环会咬死长任务  | [runtime §7](workflow-script-runtime.md)，本文 §10.4     |
| Primary-only 执行/恢复         | 未定执行进程，多实例会双跑                            | [runtime §5](workflow-script-runtime.md)，本文 §10.1     |
| coordinator 不占 worker 槽     | 父占槽等子 = 死锁                                     | [runtime §8](workflow-script-runtime.md)，本文 §12.2     |
| incognito × workflow 互斥      | durable 持久化与"关闭即焚"冲突                        | 本文 §13.1                                               |
| askUser 走无人值守 fail-closed | autonomous 下 askUser 永久阻塞                        | 本文 §8.4                                                |
| AGENTS 单点验证硬约束          | autonomous repair 易漂移成跑全套                      | 本文 §13.1                                               |
| profile 注入不破 cache         | 动态内容进静态前缀使 cache 失效                       | 本文 §7.1                                                |
| token/cost 预算                | 只控结构计数会让长任务成本无上限；Phase2 已补输出 token 硬天花板 | 本文 §10.3                                               |
| 技能命名 `hope-*` → `ha-*`     | 与现有 10 个 `ha-*` 内置系统 skill 不一致             | 本文 §6.3                                                |
| eval 回归闸                    | eval 仅作一次性验收，非持续闸                         | 本文 §14/§19                                             |

## 1. 设计结论

Phase 2 不应把当前内置第三方 coding skills 直接产品化，也不应先做一个静态结构化 workflow 状态机。修订后的方向是：

```text
General Workflow Mode
  + model-authored script-first dynamic workflow
  + durable workflow run / trace / budget / permission
  + existing Plan / Task / Subagent / Async Jobs / Hooks / Permission
  + Hope-native coding skills as one domain template
```

一句话：

> workflow 应像 Claude Code dynamic workflows 一样由模型按任务需要写脚本并在后台执行，但 Hope 必须把长任务稳定性、可恢复、可观察、权限、性能和用户体验作为底座。

Phase 2 的第一优先级：

1. 审计并隔离第三方移植 skills。
2. 设计 session 级 Workflow Mode，让模型在开启后自主决定是否创建 workflow，而不是要求用户每次手动建 run。
3. 重写 Hope 原生 coding skill suite，作为 coding 领域模板，而不是 workflow 的唯一适用场景。
4. 设计并实现 script-first workflow runtime MVP。
5. 让 workflow 脚本通过受控 host API 调用现有 Hope 子系统。
6. 所有长任务必须可恢复、可取消、可解释、可审计。

## 2. 背景与问题

Phase 0 / Phase 1 已经完成：

- Coding eval baseline 与校准任务。
- ToolDefinition v2。
- `tool_search` v2。
- 默认 deferred 工具。
- prompt render debug。
- file search v2。

这让工具更可搜索、可解释、可审计。但 Phase 2 需要回答更大的问题：

1. coding 任务应该按什么流程跑？
2. 现有 skills 能不能作为流程核心？
3. dynamic workflow 到底是结构化状态机，还是脚本？
4. 长任务如何稳定跑完，而不是中途断掉、丢状态、黑盒运行？

用户明确提出两个修正方向：

- 现有内置 coding skills 很多来自第三方移植，不一定好；应参考 Codex / Claude Code / Claude Code 提示词线索，重写 Hope 自己的 coding skills。
- dynamic workflow 如果要做，至少应该支持“先写脚本再执行”的完全动态能力，而不是只做静态结构化节点。

本方案按这两个方向重排 Phase 2。

## 3. 参考资料

### 3.1 Claude Code / Anthropic 线索

- Claude Code Skills：skills 是可复用工作流能力包，按需加载，适合承载任务专用方法论。参考：[Claude Code Skills](https://code.claude.com/docs/en/skills)
- Claude Code Dynamic Workflows：workflow 通过 Claude 生成和执行脚本来编排多个 subagent、循环和条件分支；脚本持有中间状态，避免把全部状态塞进上下文。参考：[Claude Code Dynamic Workflows](https://code.claude.com/docs/en/workflows)
- Claude Code Subagents：子代理适合并行探索、专门审查和上下文隔离。参考：[Claude Code Subagents](https://code.claude.com/docs/en/sub-agents)
- Claude Code Hooks：生命周期 hook 可做 gate、审计、自动化扩展。参考：[Claude Code Hooks](https://code.claude.com/docs/en/hooks)
- Anthropic Building Effective Agents：强调优先使用简单、可组合、透明的 workflow；agentic 系统应保持工具接口清晰、可观测。参考：[Building effective agents](https://www.anthropic.com/research/building-effective-agents)
- Anthropic skills repository：可作为技能组织样例，但示例技能不等于生产级 workflow policy。参考：[anthropics/skills](https://github.com/anthropics/skills)

使用边界：

- 不复制 Claude Code 私有实现。
- 本地早期 `~/Codes/claude-code` 和 `~/Codes/claude-code-system-prompts` 只作为历史设计线索，不作为当前竞品事实。
- 可吸收模式：skills、hooks、subagents、script workflow、loop guard、用户审批、trace。
- 不照搬文本：Hope 原生 skills 必须重新写，保留自己的架构契约和安全边界。

### 3.2 Codex / OpenAI 线索

- Codex workflow examples 强调清晰完成标准、上下文、验证方式。参考：[Codex manual](https://developers.openai.com/codex/codex-manual.md)
- Codex skills 使用 progressive disclosure：初始只放 name / description / path，命中后再读完整 `SKILL.md`，避免挤爆上下文。参考：[Codex manual - Agent Skills](https://developers.openai.com/codex/codex-manual.md)
- Codex subagents 用来减少 context pollution / context rot，适合 read-heavy 并行探索、测试、日志分析，写代码并行要谨慎。参考：[Codex manual - Subagents](https://developers.openai.com/codex/codex-manual.md)
- Codex hooks / permissions / worktrees / review 说明了可控 agent 工作流所需的外部控制面。参考：[Codex manual](https://developers.openai.com/codex/codex-manual.md)
- OpenAI Agents SDK 提供 agent、handoff、guardrails、tracing 等一手设计参考。参考：[OpenAI Agents SDK](https://openai.github.io/openai-agents-python/)
- OpenAI Agent Improvement Loop 强调 trace / feedback / eval / 改进循环。参考：[OpenAI Cookbook: Agent Improvement Loop](https://developers.openai.com/cookbook/examples/agents_sdk/agent_improvement_loop)

使用边界：

- Codex 的思路可以吸收为产品体验和系统设计原则。
- Hope 不应依赖某个 provider 或模型特性；workflow 能力应主要由本地系统闭环支撑。

### 3.3 Hope 本地架构约束

Phase 2 必须复用已有单一入口，不得新建平行系统：

- Chat 主入口：`chat_engine::run_chat_engine`
- 工具执行：`tools::execution::execute_tool_with_context`
- 权限：`permission::engine::resolve_async`
- Plan：`crates/ha-core/src/plan/`
- Task：`crates/ha-core/src/tools/task.rs`
- Subagent：`crates/ha-core/src/subagent/`
- Async jobs：`async_jobs::JobManager`
- Hooks：`HookDispatcher::dispatch`
- Session / message / artifacts：`session::*`
- Incognito / approval / protected path / KB access 等既有红线必须继续生效。

来自 Phase 0 的结论：

- workflow 不应新建平行 job API。
- workflow 不应绕过 Plan / Task / Permission / Hooks。
- 合理边界是新增轻量编排层，记录 durable trace，并调用现有子系统完成实际动作。

## 4. Phase 2 设计原则

### 4.1 Script-first

动态 workflow 的主表达形式是脚本，不是静态节点图。

脚本负责：

- 决定任务如何拆分。
- 编排 subagent。
- 做循环、条件分支、map / reduce。
- 保存中间结果。
- 根据验证结果进入 repair。

系统负责：

- 审批脚本。
- 运行脚本。
- 控制预算。
- 管理持久化。
- 暴露受控 host API。
- 记录 trace。
- 处理权限、hooks、取消、恢复。

### 4.2 Long-task first

所有设计先问：

- 运行 30 分钟会不会稳定？
- App 重启后能不能恢复？
- 用户离开页面后还能不能看状态？
- 子任务失败后能不能解释？
- 中途取消是否干净？
- 验证卡住是否有超时？

如果不能支撑长任务，就不进入实现。

### 4.3 Host API, not raw capability

workflow 脚本不能直接拿：

- 原始文件系统。
- 原始 shell。
- 原始网络。
- secret env。
- 任意 Node package。
- 未审计的浏览器 / 桌面控制能力。

脚本只能调用 Hope 暴露的 host API：

```ts
await workflow.tool({ name, args, label? })
await workflow.spawnAgent({ task, agent_id?, label?, timeout_secs? })
await workflow.task.create({ title, label? })
await workflow.task.update({ task, status, label? })
await workflow.askUser({ question, context?, label? })
await workflow.trace({ payload, label? })
await workflow.validate({ commands, reason, label? })
await workflow.finish({ summary, verification?, residualRisk? })
```

所有 host API 内部继续走原有工具、权限、hooks、async job、subagent 队列。
`workflow.task.update` 只能按 `workflow.task.create` 返回的 task handle 定位任务；`label` 仍是纯展示字段，不参与 op / task 身份。

### 4.4 Durable by replay, not VM snapshot

不依赖 JS VM 快照。脚本恢复采用 durable replay：

1. 脚本源码和 hash 持久化。
2. 每个 host call 的身份由 runtime 按执行位置生成 `op_key`，模型只可提供展示用 `label`。
3. 第一次执行 host call 时，系统记录 `op_key`、输入 hash、状态、输出。
4. 重启后从头执行脚本。
5. 已完成的 host call 根据 `op_key + input_hash` 返回历史结果。
6. 未完成的 host call 继续等待或恢复。
7. 如果脚本 hash 或 host call 输入变了，需要新 run 或显式 migration。

这接近 Temporal-style durable execution，但只实现本地最小子集。

### 4.5 Observable by default

workflow 不是黑盒。默认可见：

- 当前脚本。
- 当前步骤。
- 已完成 host calls。
- 正在运行的 subagents / jobs。
- validation 输出。
- diff snapshot。
- repair 原因。
- 停止原因。
- 预算消耗。
- Workspace Workflow Control Center 里可直接切换当前会话 workflow mode 与 execution mode；标题栏 `Workflow` 入口在 run active / waiting / failed 时显示 badge；新建 workflow 前先预检 Script Gate 与 permission preview，只有通过后才能创建；普通创建从目标开始，脚本编辑收进高级区；已有 run 可看到总览、审批焦点、授权清单、Trace timeline、Validation 命令明细、Agents 分视图、失败恢复建议；历史 run 超过首屏预览时可展开选择；Trace op/event 支持展开原始详情并复制；并可复制包含 run 状态、失败 op、验证输出、最近事件的修复提示，或直接由失败上下文生成并自动预检下一版修复 workflow 草稿；修复草稿会显示来源 run，并用修复专用创建文案避免误解为覆盖原 run。

### 4.6 Performance by state externalization

长任务性能不能靠反复塞大上下文。

原则：

- 状态存在 workflow run / artifacts / task / job 里，不存在 prompt 里。修复 run 的来源关系同样落在 `workflow_runs.parent_run_id` / `origin` 与父子 run 事件中，而不是只藏在脚本正文。
- 子代理返回摘要，不把原始日志塞主上下文。
- 大结果落盘。
- `tool_search` 发现工具，默认 deferred。
- `file search v2` 找上下文，精确 `read`。
- trace 注入只给摘要和关键节点。

### 4.7 Native skills over vendor skills

第三方移植 skills 不进核心链路。Phase 2 要重写 Hope-native coding skills。

第三方 skills 只能作为：

- 参考材料。
- 可选 vendor skill。
- eval 对照。
- 迁移输入。

不能作为：

- 默认 coding policy。
- workflow gate。
- 长任务执行策略。

## 5. 总体架构

```text
User Request
  -> Coding Classifier
  -> Hope-native Skill Policy Selection
  -> Plan / Script Draft
  -> Plan + Script Gate
  -> WorkflowRun
      -> Script Runtime
          -> Durable Host Calls
              -> Tool Execution
              -> Task API
              -> Subagent Queue
              -> Async Jobs
              -> Hooks
              -> Permission Engine
      -> Trace / Artifacts / Budget
  -> Workflow Panel / /workflow trace
  -> Final / Ask User / Resume
```

关键点：

- `WorkflowRun` 负责持久化和审计。
- 脚本负责动态编排。
- host API 负责把脚本动作接到已有系统。
- Task 仍是用户可见进度真相。
- Async Jobs / Subagent 仍是实际长任务执行底座。

## 6. Track A：Skill Detox 与 Hope-native Coding Skills

### 6.1 现状

当前仓库存在带 `ATTRIBUTION.md` 的 coding skills：

- `skills/code-review`
- `skills/subagent-driven-development`
- `skills/systematic-debugging`
- `skills/test-driven-development`
- `skills/writing-plans`

这些可能来自第三方移植。它们有价值，但不应直接作为 Phase 2 核心。

### 6.2 审计动作

审计已产出：[Coding Skills Detox 审计](coding-skills-detox.md)（证据化逐 skill 判定 + attribution 卫生红线 + `ha-*` 替代映射 + 迁移策略）。审计表字段：

| 字段             | 含义                                                     |
| ---------------- | -------------------------------------------------------- |
| skill            | 当前 skill 名                                            |
| attribution      | 是否第三方 / 原创 / 混合                                 |
| license_risk     | license / notice 是否清楚                                |
| behavior_quality | 是否真的适合 coding workflow                             |
| prompt_quality   | 是否清晰、短、可执行                                     |
| tool_awareness   | 是否了解 Hope 工具和 AGENTS 约束                         |
| production_role  | reference / vendor_optional / rewrite_native / deprecate |
| replacement      | 对应 Hope-native skill                                   |

### 6.3 Hope-native skill suite

新增一组原生 skills，命名统一用 **`ha-*`**——与现有 10 个内置系统 skill（`ha-logs` / `ha-settings` / `ha-browser` / …）一致，**不引入第三套 `hope-*` 前缀**。完整映射与"吸收自哪份 vendor（重写非复制）"见 [Coding Skills Detox 审计 §5](coding-skills-detox.md)。

| Skill                | 目标                                                                  |
| -------------------- | --------------------------------------------------------------------- |
| `ha-coding-common`   | 共享 coding 行为契约：读现有代码、尊重 AGENTS、最小改动、单点验证默认 |
| `ha-implement`       | feature / small implementation 的标准流程                             |
| `ha-debug`           | 复现、trace、假设、最小修复、回归验证                                 |
| `ha-code-review`     | code review 输出格式、finding 标准、inline comment 约束               |
| `ha-tdd`             | 先写或补最小测试，再实现，适合明确行为变更（opt-in，非默认策略）      |
| `ha-refactor`        | 保行为重构、阶段性 diff、强验证                                       |
| `ha-subagent-work`   | 何时并行探索、何时禁止并行写                                          |
| `ha-workflow-script` | 如何起草可执行 workflow script                                        |
| `ha-verify`          | 按 AGENTS 选择最小验证，不主动跑全套                                  |

### 6.4 Skill 写法要求

每个 Hope-native skill 必须：

- 原创文本，不复制第三方 skill。
- 以 Hope 的工具、权限、Plan、Task、Subagent、Async Jobs 为基础。
- 描述清楚触发条件和不要触发的场景。
- 使用 progressive disclosure：主 `SKILL.md` 短，复杂细节放 references。
- 有 eval prompt 或人工验证任务。
- 不要求模型绕过 AGENTS。
- 不承诺自动跑完整检查。

### 6.5 迁移策略

第一阶段不删除旧 skills：

```text
vendor skills -> disabled by policy candidate
native skills -> workflow policy candidate
```

待 native skills 验证稳定后：

- UI / onboarding 默认推荐 native skills。
- vendor skills 标记为 reference / optional。
- docs 明确来源和非默认地位。

## 7. Track B：Coding Task Profile（历史设计）

### 7.1 目标

本节是历史 Track B 设计，当前产品不再新增全局 `Coding Mode`。保留的语义是轻量 coding task profile：它不负责执行 workflow，只负责在编码任务中描述应该使用什么行为策略，并作为 Workflow Mode 的领域模板之一。

```rust
CodingSessionProfile {
  task_kind,
  execution_mode,
  requires_plan,
  requires_script,
  requires_task_truth,
  recommended_skills,
  verification_policy,
  risk_level,
}
```

**注入红线（cache 稳定性）**：profile 摘要注入 system prompt 时必须作**独立 cache block**（与 Memory / Awareness / User Profile 同款），绝不进静态前缀——否则每轮 profile 变化作废静态前缀缓存。先评估是否真需要独立 classifier：skill 的 description-based catalog 触发 + 模型自身可能已足够，重型 classifier 有重复造轮子 + 每轮 side-query 成本/cache 抖动的风险，能轻量则轻量。

### 7.2 任务分类

| task_kind  | 典型输入           | 默认策略                                         |
| ---------- | ------------------ | ------------------------------------------------ |
| `review`   | “检查未提交改动”   | 不改代码；findings first；必要时 inline comments |
| `fix_bug`  | “报错，修一下”     | 先复现 / 定位 / 最小修复 / 验证                  |
| `feature`  | “加一个能力”       | 读现状 / plan / 实现 / 验证 / 文档               |
| `debug`    | “为什么挂了”       | 证据优先；不急着改                               |
| `test`     | “补测试”           | 找测试风格；最小覆盖                             |
| `refactor` | “重构”             | 行为保持；强验证；分阶段                         |
| `workflow` | “批量/长任务/并行” | 起草 script；用户审批；运行                      |

### 7.3 Execution mode

| mode         | 默认行为                                               |
| ------------ | ------------------------------------------------------ |
| `off`        | 不自动 repair，只建议下一步                            |
| `guarded`    | 默认；允许 1-2 次低风险 repair                         |
| `deep`       | 长任务；更多 explore / validate / repair，但预算强约束 |
| `autonomous` | server/cron；强预算、强 trace、强 human gate           |

## 8. Track C：Script-first Workflow Runtime

### 8.1 Script artifact

workflow 脚本是一个持久化 artifact：

```text
~/.hope-agent/workflows/runs/<run_id>/workflow.js
~/.hope-agent/workflows/runs/<run_id>/manifest.json
```

也可以先存入 `sessions.db`，文件作为 export / debug 视图。最终以数据库为真相源。

脚本示例：

```js
export default async function main(workflow) {
  const observeTask = await workflow.task.create({
    label: "observe",
    title: "收集相关文件和约束",
  })

  const files = await workflow.fileSearch({
    label: "find-critical-files",
    query: "file search scoring",
    limit: 20,
  })

  const reviews = await workflow.map("parallel-review", files.matches.slice(0, 4), async (file) => {
    return workflow.spawnAgent({
      label: `review:${file.relPath}`,
      agent: "reviewer",
      task: `Review ${file.relPath} for correctness and missing tests.`,
      tools: ["read", "grep"],
      mode: "read_only",
    })
  })

  await workflow.task.update({ task: observeTask, status: "completed" })
  await workflow.trace({ label: "review_summaries", payload: reviews })

  const validation = await workflow.validate({
    label: "targeted-check",
    commands: ["cargo check -p ha-core --tests"],
    reason: "Rust core scorer and tests changed",
  })

  if (!validation.ok) {
    await workflow.askUser({
      label: "validation-failed",
      question: "验证失败，是否允许进入 guarded repair？",
      context: validation.summary,
    })
  }

  return workflow.finish({
    status: "completed",
    summary: "Workflow completed.",
  })
}
```

> **op 身份注意**：示例里的 `label` 只用于 UI 展示和 trace 可读性，**不是 op_key**。真正的 op 身份由 runtime 按执行位置（`map/item#i/op#0`）自动生成，`workflow.map` 会把物化后的输入列表记进自身 op 输出以保证重放稳定。模型不需要、也不应手写字面量 id。详见 [Script-first Workflow Runtime 设计 §4](workflow-script-runtime.md)。

### 8.2 Runtime choice

建议使用内嵌 JS runtime，而不是依赖系统 Node：

- 桌面 / server / Docker / ACP 都能一致运行。
- 更容易禁用 raw fs / network / process。
- host API 可以完全由 Rust 暴露。

候选：

| 方案               | 优点                     | 风险                     |
| ------------------ | ------------------------ | ------------------------ |
| QuickJS / rquickjs | 小、可嵌入、适合 sandbox | async host API 设计复杂  |
| Boa                | Rust 原生                | 生态和兼容性需验证       |
| Deno               | 权限模型强               | 体积和分发复杂           |
| system Node        | 实现快                   | 分发、权限、稳定性不可控 |

MVP 推荐：

```text
Authoring: workflow.js + JSDoc types
Runtime: embedded JS engine
Host API: Rust async bridge
```

TypeScript 可以后置，不作为 MVP 阻塞项。

### 8.3 Durable replay

> 完整的 op 生命周期、副作用恰好一次语义、位置化 op-key、fan-out 物化、Primary-only 恢复算法见 [Script-first Workflow Runtime 设计](workflow-script-runtime.md)。本节只列要点。

op 身份由 runtime 按**执行位置**自动生成，不由模型手写字面量 id 决定（模型写的字符串只作展示 label）。这等价于本类运行时"全局调用序号即身份"——同脚本同参数 100% 命中，恢复时最长未改前缀直接复用。结果驱动扇出由 map op **物化输入列表**保证重放稳定。副作用 op 走 `Pending → Started → Completed` 生命周期，崩溃落在 `Started` 的非幂等 op **绝不盲目重跑**（按世界状态判定，不可判定则转 `Blocked`）。

数据库表草案：

```text
workflow_runs
  id
  session_id
  kind
  state
  execution_mode
  script_hash
  script_source
  budget_json
  created_at
  updated_at
  completed_at

workflow_ops
  id
  run_id
  op_key
  op_type
  input_hash
  input_json
  state
  output_json
  error_json
  started_at
  completed_at

workflow_events
  id
  run_id
  seq
  type
  payload_json
  created_at
```

恢复规则：

1. `Running` run 在启动时进入 `Recovering`。
2. runtime 重新执行同一脚本。
3. 遇到已完成 `op_key + input_hash`，直接返回历史 output。
4. 遇到 running async job / subagent，重新 attach 状态。
5. 遇到缺失 op，继续执行。
6. 遇到 script hash 不一致，进入 `Blocked(reason=script_hash_mismatch)`；用户显式选择后才新建 run 或重审脚本。

### 8.4 Host API MVP

| API                                                                                          | 作用                                                             | 底层接入                                 |
| -------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- | ---------------------------------------- |
| `workflow.tool({ name, args, label? })`                                                      | 调任意工具                                                       | `execute_tool_with_context` + permission |
| `workflow.fileSearch({ query, limit?, label? })`                                             | 文件搜索                                                         | `filesystem::search_files`               |
| `workflow.read({ path, label? })`                                                            | 读文件快捷方式                                                   | `read` tool                              |
| `workflow.grep({ pattern, path?, label? })`                                                  | 内容搜索                                                         | `grep` tool                              |
| `workflow.spawnAgent({ task, agent_id?, label?, timeout_secs?, ... })`                       | 子代理                                                           | `subagent`                               |
| `workflow.waitAll(handles, { timeout?, label? })`                                            | 等待多任务                                                       | async job / subagent status              |
| `workflow.task.create({ title, label? })` / `workflow.task.update({ task, status, label? })` | 用户可见进度；`create` 返回 task handle，`update` 按 handle 定位 | `task_create/update`                     |
| `workflow.validate({ commands, reason, label? })`                                            | 验证命令                                                         | `exec` async job + AGENTS 策略           |
| `workflow.askUser({ question, context?, label? })`                                           | 人工 gate                                                        | `ask_user`                               |
| `workflow.trace({ payload, label? })`                                                        | trace event                                                      | `workflow_events`                        |
| `workflow.diff({ label? })`                                                                  | diff snapshot                                                    | git / session artifacts                  |
| `workflow.now()` / `workflow.random(seed)`                                                   | 确定性时间 / 随机源                                              | runtime deterministic helper             |
| `workflow.finish({ summary, verification?, residualRisk? })`                                 | 完成                                                             | `workflow_runs.state`                    |

MVP 不提供：

- raw `fs`
- raw `fetch`
- raw `child_process`
- arbitrary npm import
- direct DB access
- direct permission bypass

**`workflow.askUser` 红线**：必须经 `evaluate_approval_surface(session_id)` 判定可应答性——autonomous / cron / headless 无人可答时按 `unattended_approval_action` deny/proceed，**绝不阻塞等待**（否则即"无限等审批"）。复用既有无人值守单一真相源，不另写一套。

### 8.5 Script gate

脚本执行前必须过 gate：

1. 静态 lint（友好早报错，**非安全/正确性边界**——边界是 runtime 结构沙箱 + throw，见 [runtime §4.3](workflow-script-runtime.md)）：
   - 禁 `eval`
   - 禁 `Function`
   - 禁 dynamic import
   - 禁 raw `Date.now` / `Math.random` / `new Date()`，改用已实现的 `workflow.now()` / `workflow.random(seed)`
   - op 身份由 runtime 按执行位置生成，模型无需也不应手写字面量 id（见 §8.3）
2. 预算检查：
   - max runtime
   - max ops
   - max subagents
   - max repair attempts
   - max validation commands
3. 权限预览：
   - 可能写文件？
   - 可能执行命令？
   - 可能使用 browser / mac_control？
   - 可能触发 network？
4. 用户审批：
   - Desktop：展示脚本和摘要。
   - HTTP：API key owner 可审批。
   - ACP：按 capability。
   - cron / unattended：默认 deny 或只允许预先信任的 script template。

## 9. Track D：Plan Gate 与 Script Draft Gate

Phase 2 仍然需要 Plan Quality Gate，但它不是 workflow 的替代品。

Plan gate 检查自然语言计划：

- Context
- Critical Files
- Reuse
- Steps
- Verification
- Risks

Script draft gate 检查可执行脚本：

- 是否解释目标。
- 是否列出预算。
- 是否避免手写 op id / 旧 `(id, args)` host call 形态（op 身份由 runtime 位置化生成，`label` 仅展示）。
- 是否使用 task 作为进度真相。
- 是否有停止条件。
- 是否没有 raw capabilities。
- 是否把高风险操作转人工。

复杂任务推荐流程：

```text
Plan draft -> Plan gate -> Script draft -> Script gate -> User approval -> Run
```

小任务可以跳过 script：

```text
Classify -> Plan-lite -> Implement -> Verify
```

## 10. Track E：长任务稳定性

### 10.1 状态机

```text
Draft
  -> AwaitingApproval
  -> Running
  -> AwaitingUser
  -> Paused
  -> Recovering
  -> Completed
  -> Failed
  -> Cancelled
  -> Blocked
```

**执行与恢复 Primary-only（红线）**：WorkflowRun 的调度、脚本执行、崩溃恢复只在 Primary 实例发生（镜像 cron / wakeup），用 CAS + `primary_owner` 防多实例双跑。

**与 Plan Mode 状态机的关系**：workflow 不替代 Plan Mode（`Off/Planning/Review/Executing/Completed`），而是 Plan 进入 `Executing` 后的一种执行机制。两套状态机各管各的轴——Plan 管"用户是否批准了设计"，workflow 管"这次执行跑到哪了"；审批面要合一展示，避免用户面对两层不相干的审批闸。task 仍是跨两者的用户可见进度真相。

### 10.2 取消与暂停

要求：

- workflow cancel 会取消可取消的 child jobs。
- 已完成 op 不回滚，只记录 cancel。
- pause 不取消 jobs；只阻止新 op 开始。
- resume 从 durable replay 开始。

### 10.3 超时与预算

默认 `guarded`：

- max runtime：15 分钟
- max repair attempts：2
- max subagents：3
- max concurrent jobs：遵守 async_jobs 全局与 per-session quota
- no-progress threshold：2 轮

默认 `deep`：

- max runtime：60 分钟
- max repair attempts：4
- max subagents：6
- 必须显示 UI progress / trace

默认 `autonomous`：

- 必须预设预算。
- 必须配置 unattended approval policy。
- 触发 strict action 必须 fail closed。
- 不能无限等审批。

**token/cost 维度（红线，补结构计数之外）**：每个 execution_mode 的预算除上述结构计数外，必须含 `max_output_tokens` 硬天花板（跨主线程 + 全部子 agent 共享一个池）。达上限后会消耗 token 的 op（spawnAgent / validate 触发的 LLM 轮）直接拒绝 + `Blocked`，对齐"耗尽即停"语义。`autonomous` **必须**显式设 token 与 runtime 上限，否则拒绝进入。`0` 语义按 async_tools 约定（仅明确允许项为不限，其余钳地板）。

状态：2026-07-01 已落第一版产品级输出 token 预算闭环。GUI goal-driven 草稿会按 execution mode 写入 `maxOutputTokens`；`run_chat_engine` 返回 usage，workflow-owned subagent 完成后把 input/output tokens 持久化到 `subagent_runs`；runtime 在 `waitAll` 后写 `budget_usage` trace event，并在后续 `spawnAgent` 等会继续消耗模型 token 的 op 前检查累计 output tokens，达到上限即把 run 转为 `Blocked(reason=workflow_budget_output_tokens_exhausted)`。`autonomous` run 若没有显式 runtime + output token 预算会在执行前拒绝进入。当前语义是"已完成子 Agent 用量归集后阻止后续 LLM op"，不是实时取消已经并发跑出的子 Agent；并发中途硬切属于后续增强。

### 10.4 No-progress 检测

**repair loop 必须有界且脚本不可变（红线）**：允许脚本调用已落地的 `workflow.repairLoop(...)` helper 做 bounded repair；真正的修复动作仍由 callback 内的动态 JS 决定，不能退化成结构化 DSL。禁止在同一 run 内改写 `workflow.js` 或生成新的脚本继续跑，因为 script_hash 变化会让 replay 失效。详见 [runtime §7](workflow-script-runtime.md)。

每轮 repair 记录：

- diff hash before / after
- validation failure fingerprint
- changed files
- task progress
- tool error class

停止条件：

- 连续两轮没有有效 diff。
- 验证失败 fingerprint 不变。
- 修改范围超出 plan critical files。
- repair 次数超限。
- budget 超限。

触发后进入：

```text
AwaitingUser 或 Blocked
```

## 11. Track F：用户体验

### 11.1 Workflow Panel

右侧 Workspace 面板新增 Workflow Control Center：

- Execution mode：当前会话 `off / guarded / deep / autonomous` 常驻切换。
- Goal-driven draft：普通用户先填写目标，一键生成可预检 `workflow.js` 草稿；草稿默认编排观察、子 Agent 执行、`waitAll`、证据/产物检查、diff、finish，并按 execution mode 带保守脚本预算。生成脚本必须显式带预算提示，避免 Script Gate 把目标式草稿误判为无界脚本。
- Session / workspace guard：没有当前会话时，新建入口要说明预检会自动物化并切换到一个真实会话；物化时继承草稿态 agent / project / workingDir，并保持 Workspace 面板打开，避免用户先手动发一条消息、切一次会话或重新打开面板。当前会话没有工作目录时，目标式草稿只能默认创建为 `draft`，并提示设置目录后再运行，避免 GUI 一键启动落到不确定目录。
- Script draft：普通路径先写目标并生成草稿，`workflow.js` 编辑默认折叠到高级脚本区；高级用户仍可填写 kind、选择 execution mode、编辑 `workflow.js`、选择是否创建后立即运行。
- Preflight：创建前不落库调用 `preview_workflow_script`，展示 Script Gate 阻塞项 / 修复建议、permission preview 授权清单、是否需要审批、是否存在确定 deny。
- Run list：本会话 workflow runs、active 数、状态和快捷操作；超过首屏预览时提供展开 / 收起，避免较早失败 run 只能被提示但无法选择。
- Overview：run 状态、execution mode、更新时间、script hash、op 进度、validation / agents 计数。
- Approval focus：`awaiting_approval` 时突出 permission preview 摘要和批准 / 取消操作。
- Recovery prompt：`blocked` / `failed` / validation failure 时把 run 状态、失败 op、验证命令输出、最近事件整理成可复制修复提示；同一上下文也可直接生成下一版 goal-driven workflow 草稿并立即触发 preflight，草稿区显示来源 run、说明会创建新的修复 run，并使用修复专用创建按钮，便于带着 Script Gate / permission preview 反馈继续处理。
- Trace：步骤时间线 + 最近 workflow events，op/event 支持展开原始详情并复制。
- Validation：验证命令、结果、repair stop reason。
- Agents：subagent run/status/task，并可跳转子会话。

### 11.2 用户控制

Slash / UI：

```text
/workflow
/workflow trace
/workflow pause
/workflow resume
/workflow cancel
/mode off
/mode guarded
/mode deep
/mode autonomous
```

### 11.3 体验红线

- 不能黑盒运行长任务。
- 不能只显示 spinner。
- 不能把全部 trace 塞聊天消息里刷屏。
- 不能让用户不知道“现在在干嘛”。
- 不能让取消按钮失效。

## 12. Track G：性能设计

### 12.1 上下文预算

workflow 运行时注入给模型的内容应是摘要：

```text
workflow goal
current node
latest task state
critical artifacts
last validation summary
stop reason if any
```

不注入：

- 全量 op log。
- 全量 command output。
- 全量 subagent transcript。
- 全量 file search results。

### 12.2 并发控制

- subagent 继续走 `subagent::queue`。
- tool jobs 继续走 `async_jobs::slots`。
- workflow runtime 只发起请求，不自己维护平行池。
- `waitAll` 需要支持 bounded concurrency。
- **coordinator 不占 worker 槽（红线）**：脚本线程在 `waitAll` / `spawnAgent` 等待期间绝不持有 `async_jobs::slots` 槽或前台 idle guard——父占槽等子、子抢不到槽 = 死锁（async_jobs 已有"parked 持槽"同类陷阱）。详见 [runtime §8](workflow-script-runtime.md)。

### 12.3 大结果处理

- command output 用 output tail + artifact。
- tool result 大于阈值继续落盘。
- subagent 返回 structured summary。
- trace payload 有大小限制。

## 13. Track H：安全与权限

### 13.1 必守红线

- workflow script 不能绕过 `permission::engine`。
- protected path / dangerous command / strict approval 继续生效。
- unattended fail-closed 继续生效（含 `workflow.askUser`，见 §8.4）。
- **incognito × WorkflowRun 互斥（红线）**：incognito 会话拒绝建 run。durable 的本质是持久化（op/event 天生含用户内容：文件路径、diff、validation 输出），与"关闭即焚"不可调和——对齐既有 `Project + incognito` 互斥、静默 coerce 先例，不做"少存一点"的折中。
- **AGENTS 单点验证硬约束（红线）**：`workflow.validate` 默认单点验证；"跑全套"即使在 autonomous 也是 human-gated op（pre-push 钩子本就是全套兜底，autonomous 不该自跑全套）。
- KB access 继续通过 `effective_kb_access`。
- raw CDP / macOS 高危控制仍然 strict。

### 13.2 Script trust

脚本来源：

| 来源                         | 默认策略                         |
| ---------------------------- | -------------------------------- |
| model-generated one-off      | 必须用户审批                     |
| saved user workflow          | 首次审批，hash 变更重审          |
| bundled Hope-native workflow | release 信任，但高风险 op 仍审批 |
| imported workflow            | 默认不信任                       |
| cron/autonomous workflow     | 必须显式 allowlist + budget      |

### 13.3 Secret handling

- 脚本不能枚举 env。
- 脚本不能读 credential store。
- host API 结果默认不回显 secret。
- trace 走 sanitize。
- issue report 导出默认脱敏。

## 14. 实现顺序

> **eval 回归闸（贯穿）**：从 Phase 2.1 起，每个子阶段落地后对 [Phase 0 coding-eval baseline](coding-eval.md) 跑一遍并要求不回归——把"更稳更强"变成可度量而非断言，不是只在最终验收跑一次。

### Phase 2.0：文档与审计

产物：

- 本文。
- [Coding Skills Detox 审计](coding-skills-detox.md)（已产出）。
- [Script-first Workflow Runtime 设计](workflow-script-runtime.md)（已产出）。

验收：

- Claude Code / Codex 对齐点清楚。
- 第三方 skill 处理策略清楚。
- script-first 和 durable replay 决策清楚。

### Phase 2.1：Hope-native coding skills MVP

状态：2026-06-30 已新增首批 5 个 `ha-*` native coding skills；Phase 2 无外部 LLM 回归验收已覆盖 6 个核心场景，详见 [Phase 2 Eval 验收报告](phase2-eval-report.md)。真实模型回复型 fan-out 已用本地 mock provider 自动覆盖，外部真实 provider smoke 只作为体验抽检。

先写：

- `ha-coding-common`
- `ha-code-review`
- `ha-debug`
- `ha-verify`
- `ha-workflow-script`

验收：

- 不复制第三方文本。
- 能被 skill catalog 正确触发。
- 能通过 3-5 个人工 coding eval。

### Phase 2.2：CodingSessionProfile + task classifier

状态：2026-06-30 已接入轻量规则版 `CodingSessionProfile` + 动态 profile block；重型 LLM classifier 后置，除非 eval 证明 description-based skills + 规则版不够。

实现：

- `CodingTaskKind`
- `ExecutionMode`
- `CodingSessionProfile`
- Prompt/profile 注入摘要（独立动态 system block，不进静态 prefix）。

验收：

- review 请求不会误进入 implement。
- debug 请求会要求证据。
- feature 请求会要求 plan / verification。

### Phase 2.3：Plan Gate + Script Draft Gate

状态：2026-06-30 已新增 Plan Gate / Script Gate 纯函数；`submit_plan` 已接入 Plan Gate；`workflow::runtime::run_workflow_script` 已在执行前复用 Script Gate。

实现：

- Plan gate checker。
- Script gate checker。
- 失败时返回可修正 feedback。

验收：

- 缺 Critical Files 的 plan 被拦。
- 无 Verification 的 plan 被拦。
- 使用旧 `(id, args)` host call 或把 `label` 当身份的 script 被拦。
- 使用 raw capabilities 的 script 被拦。

### Phase 2.4：WorkflowRun durable store

状态：2026-06-30 已新增 `workflow_runs` / `workflow_ops` / `workflow_events` durable store、状态机、Tauri/HTTP owner API、`workflow:*` EventBus；embedded runtime 接入待 Phase 2.5。

实现：

- workflow_runs / workflow_ops / workflow_events。
- run status API。
- cancel / pause / resume API。
- EventBus `workflow:*`。

验收：

- App 重启后能看到 run。
- running op 能恢复或解释 interrupted。
- cancel 能停止后续 op。

### Phase 2.5：Embedded script runtime MVP

状态：2026-06-30 已落 QuickJS/rquickjs runtime foundation：`workflow.js` 受控执行、`export default main(workflow)` 入口、无 raw fs/network/process/env host binding、memory/stack/timeout guard、`Date.now` / `new Date()` / `Math.random` runtime throw、确定性 `workflow.now()` / `workflow.random(seed)` helper、位置化 `main/op#N(api)`、`task.create/update`（handle 定位）/ `fileSearch` / `tool/read/grep` / `workflow.map` / `spawnAgent` / `waitAll` / `validate`（async exec job attach）/ `askUser` / `diff` / `trace` / `finish` 首批 host API、Completed op replay、Started non-idempotent op fail-closed Blocked、Primary-only startup recovery runner。无外部 LLM 单测覆盖脚本执行、Script Gate 执行前阻断、动态 `Math.random` 访问被 runtime 阻断、已完成 `task.create` replay 不重复建 task、`workflow.map` 已物化 fan-out 列表并生成 `map/item#i/op#N` 嵌套位置键，`read/grep/tool` 经 `execute_tool_with_context` 桥接、`workflow.spawnAgent` / `workflow.waitAll` 经现有 `subagent` 工具桥接且 completed replay 不重复调度、`spawnAgent` 预分配 child_handle 并可在 started replay 时 attach / 缺 row 则同 handle 重试，新增真实工具路径 E2E 覆盖 `workflow.spawnAgent -> subagent tool -> spawn_subagent_with_run_id -> subagent_runs/background_jobs`（通过并发上限稳定停在 Queued，证明 durable spawn / projection），并新增 mock-provider 回复型 fan-out E2E 覆盖 `workflow.spawnAgent -> child run_chat_engine -> OpenAI Chat provider adapter -> waitAll`（两个子 Agent 均完成并汇总结果）、`workflow.validate` 预分配 async job child_handle、可在 started replay 时 attach / 缺 row 同 job id 重试，并返回结构化 validation 结果、显式 `workflow.tool({ args: { run_in_background: true } })` 预分配 async job child_handle、started replay 可 attach 既有 job / 缺 row 同 job id 重试、`workflow.askUser` 复用既有 ask-user 工具且无人值守 surface 先 fail-closed / 按配置 proceed、`workflow.diff` 返回 session workspace 的 git diff snapshot、`Started` 的 `tool:exec` 不盲目重跑、recovery runner CAS claim 后 replay 且不抢已 claim run，startup-like 单测覆盖 async job replay 标 interrupted 后 workflow recovery 继续完成且不重复 task；执行前 permission preview 第一版已落：创建 run 记录 `script_permission_preview`，Draft 执行前对静态 workflow host call 复用 permission engine 预览，动态工具调用先转 `awaiting_approval`，owner `approve_workflow_run` 后继续；Workspace Panel 已接入 workflow run 列表 / trace 摘要 / approve / pause / resume / cancel，并补 Trace / Validation / Agents 三视图；slash command 已升级为 `/workflow on|off|ultracode|status|runs|trace|approve|pause|resume|cancel`，其中 on/ultracode 是 session 级 Workflow Mode 开关，开启后模型在后续回合自主判断是否调用 `workflow_run`；`/mode off|guarded|deep|autonomous` 已升级为持久化 session policy（`sessions.execution_mode` + Tauri/HTTP owner API + system prompt 注入）；guarded repair stop guard 已落地（validation failure repair event、重复 fingerprint / 无有效 diff 进展 → Blocked）。外部真实 provider smoke 只作为体验抽检，不再是实现完成的唯一证据。

实现：

- `workflow.js` 执行（已完成 foundation）。
- host API MVP（已完成同步首批、`tool/read/grep` dispatch bridge、`workflow.map` fan-out 物化、`spawnAgent/waitAll` subagent bridge、`validate` async job attach、`askUser`、`diff`）。
- durable replay（Completed op replay、Started non-idempotent Blocked、`spawnAgent` / `validate` / 显式 async `workflow.tool` child_handle attach、Primary-only startup recovery runner 已完成）。
- user approval（已完成第一版 permission preview / approval surface：静态对象参数可预览，动态工具调用需 owner approve；运行时工具审批仍是兜底）。

验收：

- 一个 script 能 spawn 2 个 read-only subagents 并汇总。
- 一个 script 能运行 targeted validation。
- 重启后 replay 不重复已完成 host call（Completed op 单测已覆盖；startup-like async job replay → workflow recovery 顺序单测已覆盖）。
- Draft script 执行前能产出 permission preview；动态工具调用先进入 `awaiting_approval`，owner approve 后才可继续。

### Phase 2.6：Workflow Mode 与 Workflow Control Center

状态：2026-07-03 已完成语义升级。2026-07-01 的 Workflow Control Center v2 仍保留为可观察、可审批、可恢复的 run 管理面；新增 session 级 Workflow Mode 后，普通主路径变成：用户在输入框或 Workspace 开启 `off/on/ultracode`，下一轮模型看到 `workflow_run` 工具和 Workflow Mode prompt，自行判断是否要写脚本并启动后台 run。`/workflow on|off|ultracode|status` 控制模型自主编排能力，`/workflow runs|trace|approve|pause|resume|cancel` 管理具体 run。Workflow 不再被定义为 coding-only；coding 草稿只是手动高级入口的一种领域模板。

已落能力包括：标题栏提供显性 `Workflow`/Workspace 入口，点击打开同一个 Workspace / Workflow 控制台，并在 active / waiting / failed run 存在时显示状态 badge；输入框提供工作流模式入口，开启后输入框上方常驻显示 Workflow Mode 状态；Workspace Panel 内展示本会话 workflow runs、active 数、workflow mode、execution mode、总览进度、permission preview 授权清单、approval 焦点、Trace timeline、Validation / Agents 分视图、blocked/failed reason、当前焦点 / 下一步卡片、恢复建议、可复制修复提示与一键修复草稿生成；没有 run 时显示可操作空态，直接呈现当前 workflow mode、execution mode / 工作目录状态，并提供主按钮展开创建表单；run 超过首屏预览时提供历史展开 / 收起，用户可选择较早失败 run 并从该 run 生成修复草稿；当前焦点卡会把 running / recovering / awaiting_user / awaiting_approval / paused / failed / blocked / completed 等状态转成用户可理解的“正在执行哪一步 / 卡在哪里 / 看哪个详情页”，并可一键跳到 Trace、Validation 或 Agents；Trace 会把失败 / 运行中步骤和关键事件置顶，同时保留步骤预览与原始序号，避免长 run 固定预览吞掉真正卡点，op/event 原始 payload 可展开和复制；Validation / Agents 页头提供通过 / 失败 / 运行统计，便于扫读；审批/失败/阻塞态不再叠加语义重复的 warning/error notice，保留“当前焦点 + 授权清单/修复动作”的清晰层级；新建入口从“目标驱动”开始，用户填写目标后一键生成可预检 `workflow.js` 草稿，草稿默认编排观察、子 Agent 执行、`waitAll`、证据/产物检查、diff、finish，并按 execution mode 带保守脚本预算；脚本编辑默认收在高级脚本区，高级用户仍可填写 kind、选择 execution mode、编辑 `workflow.js` 并选择创建后立即运行；无当前会话时预检会自动物化真实 session 并继承草稿态 agent / project / workingDir，且 Workspace 保持打开；无工作目录时只能创建 draft，不允许误触发立即运行；创建前通过不落库 `preview_workflow_script` 同时展示 Script Gate 阻塞项 / 修复建议与 permission preview 授权清单，Tauri/HTTP owner create API 也强制复用同一 preflight，只有 gate 通过且没有确定 deny 时才允许创建；Validation tab 展开每条验证命令的 job status / exit code / output 摘要，并在 validation failure 时自动聚焦；失败/阻塞态可一键复制包含 run 状态、失败 op、验证输出和最近事件的修复提示，或直接把这段上下文写入下一版 goal-driven workflow 草稿并自动触发预检，草稿区显示来源 run、说明会创建新的修复 run 且不覆盖原 run，并使用修复专用创建文案；修复 run 创建时通过 `parentRunId` / `origin=repair` 持久记录来源，父子 run 都写派生事件，刷新后仍可在详情卡和 Trace 中追踪；连续切换不同失败 run 后创建修复 run 会使用当前选中 run 的来源，不串到旧 run；提供 run draft / approve / pause / resume / cancel；`paused` run 在 DB op guard 层拒绝启动新 op，pause 会释放旧 `primary_owner`，resume 后可被新的 primary owner 重新 claim，避免按钮显示恢复但 runtime 因旧 owner 不继续；owner cancel 先把 run 转 `cancelled`，再 best-effort 取消 workflow-owned async tool / validation / subagent children，并写 `run_child_cancel_requested` trace event；GUI cancel 前弹确认，说明会停止 run 与 workflow-owned children 且保留 trace；Tauri 与 HTTP 共用 owner API（preview / create / run / list / get / approve / pause / resume / cancel / get_workflow_mode / set_workflow_mode）。`approve` / `resume` 会异步 kick workflow runtime，避免只改状态不执行；workflow run 刷新走 `workflow:*` EventBus + 短 debounce，并在 WS lag / 页面回前台 / active run 运行期间用低频轮询兜底；窄屏 / 移动宽度下用户主动打开所有右侧互斥面板（含 Workspace/Workflow、Files、Browser、Canvas、Mac Control、Team、Background Jobs、Preview）都走 fixed overlay，不再被桌面 split-pane 挤到视口外；`/mode` 已持久化到会话级 `execution_mode`，GUI 也可直接切换，并在下一轮 system prompt 注入 guarded/deep/autonomous 的执行策略与停止条件。

实现：

- Execution mode GUI 控制。
- Workflow Mode GUI 控制：输入框入口 + 输入框上方常驻状态 + Workspace segmented control，支持 `off/on/ultracode`，状态双向同步。
- 模型工具面：`workflow_run` 仅在 Workflow Mode 开启且非无痕时注入；执行层二次校验，创建 run 后默认立即进入 preflight / approval / runtime。
- 标题栏 `Workflow` / Workspace 入口：显性进入 Workspace / Workflow 控制台，并用 badge 显示 active / waiting / failed run，避免只能靠 `/workflow` 命令或打开面板后才发现状态。
- 无 run 空态启动面板：展示当前 execution mode / 工作目录状态，并一键展开创建表单。
- Goal-driven create form：目标 → 生成可预检 workflow 草稿；草稿态新对话会在预检前自动物化为真实 session 并继承 draft workingDir，默认创建后运行。
- Script draft create form：kind / execution mode / `workflow.js` / run immediately。
- Script draft preflight：创建前 Script Gate / permission preview / approval need / deny blocker。
- Run list + history expand / Overview / current focus + next-step jump / permission notice + call checklist / Trace timeline + op/event detail expand/copy / Validation command details / Agents / blocked or failed reason + recovery hint + repair prompt copy + repair draft generation + auto-preflight + origin-aware repair create copy。
- Dev-only viewport smoke harness：`?window=workflow-smoke` 动态导入 `WorkflowSmokeWindow`，用真实 `WorkspacePanel` + fake transport 切换 approval / running / failed / completed 场景；用于桌面/窄屏验证当前焦点、授权清单、修复动作、派生 run 和水平溢出，不作为生产功能入口。
- Owner API：preview / create / run / list / get / approve / pause / resume / cancel；create 强制执行同一 preflight；run draft、approve、resume 后异步启动 runtime。
- GUI 操作：Run draft / Approve / Pause / Resume / Cancel。
- Pause / resume runtime guard：paused run 拒绝启动新 op；pause 清理 owner，resume 后 runtime launcher 可重新 claim。
- Cancel child cleanup：owner cancel 会 best-effort 取消 workflow-owned async tool / validation / subagent children，并保留 trace event；GUI cancel 前确认，防止误触发长任务停止。
- Output token budget：goal-driven 草稿按 execution mode 写入 `maxOutputTokens`；run list 与详情总览展示 `输出预算` 用量，budget exhausted 事件进入关键事件 / Trace；runtime 达上限会阻断后续 LLM op 并转 `Blocked`。
- `/workflow on|off|ultracode|status|runs|trace|approve|pause|resume|cancel`。
- `/mode off|guarded|deep|autonomous` 持久化控制入口。

验收：

- 长任务期间用户能看懂状态（已覆盖标题栏 Coding 入口 / 无 run 空态启动入口 / run state / ops / events / current focus / tab jump / 历史 run 展开选择 / 长 run 晚期失败步骤置顶 / 关键事件置顶 / blocked reason / recovery hint / repair prompt copy / repair draft generation / auto-preflight / origin-aware repair create copy）。
- validation failure 清楚展示（Trace / Validation tab 展示 validate summary、failed/total、stop reason、每条 validation command、job status、exit code、output 摘要，并自动聚焦 Validation）。
- subagent 状态清楚展示（Agents tab 展示 spawnAgent op、runId/status/label/task；background job 投影仍保留）。
- 预算耗尽清楚展示（run list / Overview 展示 `输出预算` spent/limit，`budget_usage` 关键事件说明 exhausted 与 reason，runtime 达上限转 blocked）。

### Phase 2.6b：语义收口

状态：2026-07-01 已完成。详见 [Goal / Mode / Workflow / Loop 语义收口](control-plane-semantics.md)。

本阶段把原先容易混淆的“loop mode”彻底收口为 `/mode` execution mode：

- `/mode off|guarded|deep|autonomous` 是会话级执行策略。
- `/workflow` 是会话级 Workflow Mode 开关和具体 run 管理入口；`workflow_run` 创建的一次 run 才是具体、可观察、可恢复的执行编排。
- `/goal` 已成为长期任务的顶层完成标准，见 [Goal 控制平面](../architecture/goal.md)。
- `/loop` 只用于定时、重复触发或条件轮询。

同时明确：guarded repair stop guard 已在 Phase 2.5 / 2.6 范围内落地，不再作为下一阶段主线。

### Phase 2.7：`/goal` MVP

状态：已完成第一版。最终架构见 [Goal 控制平面](../architecture/goal.md)，路线设计见 [Agent 控制平面路线图](agent-control-plane-roadmap.md)。

目标：

- 新增一等 Goal 对象，承载 objective、completion criteria、budget、evidence、status、final audit。
- `/goal` 不替代 `/workflow`；它决定“最终要达成什么”，workflow 决定“这次怎么执行”。
- GUI 显示 active goal，用户不必打开 workflow 历史才能知道长期任务是否完成。

验收：

- `/goal <objective and completion criteria>` 能创建 active goal。
- `/goal status|pause|resume|evaluate|clear` 可用。
- Goal 能 link workflow run、task、validation evidence。
- Workflow 完成后可触发 goal evaluate。
- Goal final audit 能解释达成项、未达成项、验证证据和剩余风险。

### Phase 2.8：Goal-driven Workflow

状态：核心已完成。Goal 绑定 workflow、validation/diff/file evidence link、GUI Goal detail、Evaluator v2、Budget v2 已落地；Review Engine evidence 已在 Phase 3.3 接入；artifact/diagnostic evidence 后续增强。详细方案见 [Goal-driven Workflow v2 路线图](goal-driven-workflow-v2.md)。

目标：

- 让 workflow run 归属 goal。
- 失败 run 生成 repair run 时继承 goal。
- Workflow completion、validation、task evidence 回写 goal；validation / diff / file 第一层细粒度 evidence 已落地，Review Engine evidence 已落地，artifact/diagnostic 后续补。
- Goal evaluator 基于 workflow/evidence/budget snapshot 收口，而不是重新猜测聊天历史。
- Goal budget 展示 token/time/turn 使用，接近上限写 warning event，耗尽后阻止新 workflow。

验收：

- `workflow_runs` 可选绑定 goal。
- Goal strip 能展示 linked run/task/evidence 指标；独立 detail timeline 后续补。
- Repair run 不丢 parent goal。
- App 重启后 goal / workflow / task / evidence 关系仍可恢复。

### Phase 2.9：真正 `/loop`

状态：第一版已落地。最终架构见 [Loop 控制平面](../architecture/loop.md)，详细路线见 [Agent 控制平面路线图 §7](agent-control-plane-roadmap.md#7-phase-29真正-loop)。

目标：

- `/loop` 只表示重复触发，不表示执行强度。
- 复用 Cron 作为可靠调度器，不新建平行 scheduler；SessionLoop 触发通过 parent injection 回到原会话。
- 每个 loop 必须有最大次数/运行时长/token 预算、审批策略、trace 和 stop 控制；成本预算字段保留，等待 provider cost ledger。

验收：

- `/loop status` 能解释触发策略、次数预算和最近结果。
- `/loop until` 注入 condition，并在 assistant 回应 `LOOP_CONDITION_SATISFIED` marker 后自动完成并停掉底层 Cron。
- `/loop stop` 后会暂停底层 Cron job，不再唤醒。
- token budget 已是触发前 hard stop；`cost_budget_micros` 创建时暂拒绝，后续接入 provider cost ledger 后放开。
- 无人值守审批不可用时沿用原会话 permission / Cron unattended fail-closed 策略。

后续增强：

- Event-triggered loop 接入 EventBus / file watcher / CI。
- 独立 Loop detail 页面展示完整 run trace、cron log 与消息范围。
- 成本预算接入 provider cost ledger，并放开 `cost_budget_micros` 创建限制。
- Loop trigger 直接生成/运行 Goal-driven Workflow draft。

## 15. MVP 示例场景

### 15.1 并行 code review

用户：

```text
Review this branch with parallel reviewers: correctness, tests, security.
```

Hope：

1. 生成 workflow script。
2. 用户审批。
3. spawn 3 个 read-only reviewer subagents。
4. 汇总 findings。
5. 可选 auto-fix 进入新 script 或普通 coding flow。

### 15.2 Debug loop

用户：

```text
这个测试挂了，帮我定位并修复。
```

Hope：

1. classify `debug`。
2. 要求 reproduce。
3. run targeted command。
4. 生成假设。
5. 最小修复。
6. targeted validation。
7. 失败则 guarded repair。

### 15.3 Feature implementation

用户：

```text
做 file search v3，加内容预览排序。
```

Hope：

1. classify `feature`。
2. Plan gate。
3. 若任务大，生成 workflow script。
4. explore 现有 scorer / UI / API。
5. implement。
6. validate。
7. review。
8. final。

## 16. 对齐 Claude Code workflow 能力的检查表

给 Claude Code review 时，可以让它看这份 checklist：

| 能力                   | 本方案是否覆盖 | 说明                                               |
| ---------------------- | -------------- | -------------------------------------------------- |
| Skills                 | 是             | Hope-native skill suite，vendor skill 不进核心     |
| Dynamic workflows      | 是             | script-first，而不是纯结构化节点                   |
| Subagents              | 是             | host API 接现有 subagent 队列                      |
| Hooks                  | 是             | host calls 走现有 hooks / permission               |
| Script approval        | 是             | script gate + user approval + hash trust           |
| Long-running workflows | 是             | durable run / ops / events                         |
| Resume                 | 是             | replay-based durability                            |
| Cancellation           | 是             | workflow cancel / pause / resume                   |
| Trace                  | 是             | workflow_events + UI panel                         |
| Loop stop conditions   | 是             | no-progress / validation fingerprint / budget      |
| Performance            | 是             | state externalization + summaries + deferred tools |
| Safety                 | 是             | no raw fs/network/process/env                      |

## 17. 非目标

Phase 2 不做：

- 完整 LSP。
- Managed worktree 全量实现。
- Review engine 全量 verifier。
- 任意 npm workflow ecosystem。
- 云端 workflow marketplace。
- 复制 Claude Code 私有提示词。
- 直接删除第三方 skills。

这些可以在 Phase 3+ 或后续 RFC 做。

## 18. 风险与待验证问题

| 风险                                               | 处理                                                                                                |
| -------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| JS runtime async bridge 复杂                       | MVP host API 保守；先验证 3-5 个调用                                                                |
| op identity 依赖执行位置，模型可能误把 label 当 id | runtime 只用位置化 op-key 做身份，label 仅展示；script gate 检查 host call 形态并提示不要依赖 label |
| script 太自由导致难审                              | preview + lint + budget + no raw capability                                                         |
| 长任务 UI 复杂                                     | 先复用 Workspace panel                                                                              |
| subagent 成本高                                    | bounded concurrency + explicit budget                                                               |
| 旧 vendor skills 行为不一致                        | detox 标记，不进核心                                                                                |
| 用户不想看脚本                                     | 展示摘要 + 可展开源码                                                                               |
| autonomous 风险高                                  | 默认不开放或只允许 allowlisted scripts                                                              |

## 19. 验收标准

Phase 2 完成时，应满足：

1. 有 Hope-native coding skills，不依赖第三方移植 skills。
2. 有 script-first workflow runtime RFC 和 MVP。
3. 至少一个 workflow script 可编排 subagents。
4. workflow run 可恢复、可取消、可查看 trace。
5. 长任务 UI 能展示当前状态、任务、子代理、验证、失败原因。
6. guarded repair loop 有停止条件。
7. 不绕过 permission / hooks / async jobs / subagent / task。
8. 通过至少 6 个 coding eval 场景，并接 [Phase 0 baseline](coding-eval.md) 不回归：
   - parallel review
   - debug with failing test
   - feature implementation
   - no-progress repair stop
   - user approval / cancel / resume

## 20. 下一步

本方案内的 Phase 2 主任务已经完成，后续不再继续在本文内追加新的顶层阶段。下一步进入 [Agent 控制平面路线图](agent-control-plane-roadmap.md)：

1. ~~写 `docs/roadmap/coding-skills-detox.md`~~ → 已产出 [Coding Skills Detox 审计](coding-skills-detox.md)。
2. ~~写 `docs/roadmap/workflow-script-runtime.md`~~ → 已产出 [Script-first Workflow Runtime 设计](workflow-script-runtime.md)。
3. ~~新建第一批 `ha-*` native skills~~ → 已新增首批 5 个 Hope-native coding skills。
4. ~~实现 Plan Gate / Script Gate 的纯函数和 fixture~~ → 已接入 Plan Gate，Script Gate 等 runtime 入口落地后执行。
5. ~~实现 durable store + 状态机（无 JS，纯函数 + fixture，[runtime §14](workflow-script-runtime.md)）~~ → 已新增 durable store、owner API、状态机与无 LLM 单测。
6. ~~进入 embedded runtime 代码实现~~ → 已落 QuickJS runtime foundation 与同步首批 host API。
7. ~~接剩余 async host bridge 的真实工具路径证据~~ → 已补 `workflow.spawnAgent` 经真实 subagent tool 的 E2E 单测，并补 mock-provider 回复型 fan-out E2E（两个子 Agent 真实跑过 `run_chat_engine` + OpenAI Chat adapter 后由 `waitAll` 汇总）。
8. ~~补 Workflow Panel / `/workflow trace` / `/mode` 前端控制面~~ → 已接 Workspace Panel + `/workflow` slash + 持久化 `/mode` policy，并补 Trace / Validation / Agents tabs。
9. **下一阶段：`/goal` MVP** → 目标、完成标准、预算、证据、状态、final audit。
10. **随后：Goal-driven Workflow** → workflow run 归属 goal，repair run 继承 goal，workflow evidence 回写 goal。
11. **已补齐：真正 `/loop` 第一版** → 定时、重复、轮询或条件触发，复用 cron / wakeup / automation。

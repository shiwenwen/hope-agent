# Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案

> 返回 [路线图索引](README.md)
>
> 状态：Draft RFC
>
> 更新时间：2026-06-30

## 0. 设计修订说明（2026-06-30 Review 收口）

本文经一轮对抗式架构 review 修订。**路线选择维持原方案**：Phase 2 内实现通用内嵌 JS 引擎（不降级为只跑固定模板）。在此前提下，把会咬死长任务的正确性/稳定性硬伤折进对应章节，并拆出两份可实现化子文档：

- [Script-first Workflow Runtime 设计](workflow-script-runtime.md)：durable replay 的 op 生命周期、副作用语义、位置化 op-key、fan-out 物化、Primary-only 恢复、并发背压、预算——细化到可开工。
- [Coding Skills Detox 审计](coding-skills-detox.md)：5 个 vendor skill 证据化审计 + attribution 卫生 + `ha-*` native 替代映射 + 迁移策略。

本轮折入的修复（详节见各处与上述子文档）：

| 修复点 | 原问题 | 落点 |
| --- | --- | --- |
| op 生命周期 + 副作用恰好一次 | replay 未定义"已发起未记录"崩溃窗口，非幂等写可能重复 | [runtime §3](workflow-script-runtime.md) |
| 位置化 op-key + fan-out 物化 | 模型手写字面量 id 脆；结果驱动扇出重放错位 | [runtime §4](workflow-script-runtime.md)，本文 §8.1/§8.5 |
| 确定性靠 runtime throw 非 lint | 能力沙箱与确定性混在一个 denylist，denylist 易逃逸 | [runtime §4.3](workflow-script-runtime.md)，本文 §8.5 |
| repair 系统侧编排 | 脚本内 repair 改 script_hash 使整 run replay 失效 | [runtime §7](workflow-script-runtime.md)，本文 §10.4 |
| Primary-only 执行/恢复 | 未定执行进程，多实例会双跑 | [runtime §5](workflow-script-runtime.md)，本文 §10.1 |
| coordinator 不占 worker 槽 | 父占槽等子 = 死锁 | [runtime §8](workflow-script-runtime.md)，本文 §12.2 |
| incognito × workflow 互斥 | durable 持久化与"关闭即焚"冲突 | 本文 §13.1 |
| askUser 走无人值守 fail-closed | autonomous 下 askUser 永久阻塞 | 本文 §8.4 |
| AGENTS 单点验证硬约束 | autonomous repair 易漂移成跑全套 | 本文 §13.1 |
| profile 注入不破 cache | 动态内容进静态前缀使 cache 失效 | 本文 §7.1 |
| token/cost 预算 | 原预算只有结构计数，缺成本天花板 | 本文 §10.3 |
| 技能命名 `hope-*` → `ha-*` | 与现有 10 个 `ha-*` 内置系统 skill 不一致 | 本文 §6.3 |
| eval 回归闸 | eval 仅作一次性验收，非持续闸 | 本文 §14/§19 |

## 1. 设计结论

Phase 2 不应把当前内置第三方 coding skills 直接产品化，也不应先做一个静态结构化 workflow 状态机。新的方向是：

```text
Hope-native coding skills
  + script-first dynamic workflow
  + durable workflow run / trace / budget / permission
  + existing Plan / Task / Subagent / Async Jobs / Hooks / Permission
```

一句话：

> workflow 可以像 Claude Code 那样由脚本动态编排，但 Hope 必须把长任务稳定性、可恢复、可观察、权限、性能和用户体验作为底座。

Phase 2 的第一优先级：

1. 审计并隔离第三方移植 skills。
2. 重写 Hope 原生 coding skill suite。
3. 设计并实现 script-first workflow runtime MVP。
4. 让 workflow 脚本通过受控 host API 调用现有 Hope 子系统。
5. 所有长任务必须可恢复、可取消、可解释、可审计。

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
await workflow.tool(...)
await workflow.spawnAgent(...)
await workflow.createTask(...)
await workflow.updateTask(...)
await workflow.askUser(...)
await workflow.recordTrace(...)
await workflow.validate(...)
await workflow.finish(...)
```

所有 host API 内部继续走原有工具、权限、hooks、async job、subagent 队列。

### 4.4 Durable by replay, not VM snapshot

不依赖 JS VM 快照。脚本恢复采用 durable replay：

1. 脚本源码和 hash 持久化。
2. 每个 host call 必须有稳定 `id`。
3. 第一次执行 host call 时，系统记录 `op_id`、输入、状态、输出。
4. 重启后从头执行脚本。
5. 已完成的 host call 根据 `id + input_hash` 返回历史结果。
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

### 4.6 Performance by state externalization

长任务性能不能靠反复塞大上下文。

原则：

- 状态存在 workflow run / artifacts / task / job 里，不存在 prompt 里。
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

| 字段 | 含义 |
| --- | --- |
| skill | 当前 skill 名 |
| attribution | 是否第三方 / 原创 / 混合 |
| license_risk | license / notice 是否清楚 |
| behavior_quality | 是否真的适合 coding workflow |
| prompt_quality | 是否清晰、短、可执行 |
| tool_awareness | 是否了解 Hope 工具和 AGENTS 约束 |
| production_role | reference / vendor_optional / rewrite_native / deprecate |
| replacement | 对应 Hope-native skill |

### 6.3 Hope-native skill suite

新增一组原生 skills，命名统一用 **`ha-*`**——与现有 10 个内置系统 skill（`ha-logs` / `ha-settings` / `ha-browser` / …）一致，**不引入第三套 `hope-*` 前缀**。完整映射与"吸收自哪份 vendor（重写非复制）"见 [Coding Skills Detox 审计 §5](coding-skills-detox.md)。

| Skill | 目标 |
| --- | --- |
| `ha-coding-common` | 共享 coding 行为契约：读现有代码、尊重 AGENTS、最小改动、单点验证默认 |
| `ha-implement` | feature / small implementation 的标准流程 |
| `ha-debug` | 复现、trace、假设、最小修复、回归验证 |
| `ha-code-review` | code review 输出格式、finding 标准、inline comment 约束 |
| `ha-tdd` | 先写或补最小测试，再实现，适合明确行为变更（opt-in，非默认策略） |
| `ha-refactor` | 保行为重构、阶段性 diff、强验证 |
| `ha-subagent-work` | 何时并行探索、何时禁止并行写 |
| `ha-workflow-script` | 如何起草可执行 workflow script |
| `ha-verify` | 按 AGENTS 选择最小验证，不主动跑全套 |

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

## 7. Track B：Coding Mode Profile

### 7.1 目标

Coding Mode Profile 不负责执行 workflow，只负责描述当前 coding 任务应该使用什么行为策略。

```rust
CodingSessionProfile {
  task_kind,
  loop_mode,
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

| task_kind | 典型输入 | 默认策略 |
| --- | --- | --- |
| `review` | “检查未提交改动” | 不改代码；findings first；必要时 inline comments |
| `fix_bug` | “报错，修一下” | 先复现 / 定位 / 最小修复 / 验证 |
| `feature` | “加一个能力” | 读现状 / plan / 实现 / 验证 / 文档 |
| `debug` | “为什么挂了” | 证据优先；不急着改 |
| `test` | “补测试” | 找测试风格；最小覆盖 |
| `refactor` | “重构” | 行为保持；强验证；分阶段 |
| `workflow` | “批量/长任务/并行” | 起草 script；用户审批；运行 |

### 7.3 Loop mode

| mode | 默认行为 |
| --- | --- |
| `off` | 不自动 repair，只建议下一步 |
| `guarded` | 默认；允许 1-2 次低风险 repair |
| `deep` | 长任务；更多 explore / validate / repair，但预算强约束 |
| `autonomous` | server/cron；强预算、强 trace、强 human gate |

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
  await workflow.task.create("observe", {
    title: "收集相关文件和约束"
  });

  const files = await workflow.fileSearch("find-critical-files", {
    query: "file search scoring",
    limit: 20
  });

  const reviews = await workflow.map("parallel-review", files.matches.slice(0, 4), async (file) => {
    return workflow.spawnAgent(`review:${file.relPath}`, {
      agent: "reviewer",
      task: `Review ${file.relPath} for correctness and missing tests.`,
      tools: ["read", "grep"],
      mode: "read_only"
    });
  });

  await workflow.task.update("observe", { status: "completed" });
  await workflow.trace("review_summaries", reviews);

  const validation = await workflow.validate("targeted-check", {
    commands: ["cargo check -p ha-core --tests"],
    reason: "Rust core scorer and tests changed"
  });

  if (!validation.ok) {
    await workflow.askUser("validation-failed", {
      question: "验证失败，是否允许进入 guarded repair？",
      context: validation.summary
    });
  }

  return workflow.finish({
    status: "completed",
    summary: "Workflow completed."
  });
}
```

> **op 身份注意**：示例里 `` `review:${file.relPath}` `` 只作展示 label，**不是 op_key**。真正的 op 身份由 runtime 按执行位置（`map/item#i/op#0`）自动生成，`workflow.map` 会把物化后的输入列表记进自身 op 输出以保证重放稳定。模型不需要、也不应手写字面量 id。详见 [Script-first Workflow Runtime 设计 §4](workflow-script-runtime.md)。

### 8.2 Runtime choice

建议使用内嵌 JS runtime，而不是依赖系统 Node：

- 桌面 / server / Docker / ACP 都能一致运行。
- 更容易禁用 raw fs / network / process。
- host API 可以完全由 Rust 暴露。

候选：

| 方案 | 优点 | 风险 |
| --- | --- | --- |
| QuickJS / rquickjs | 小、可嵌入、适合 sandbox | async host API 设计复杂 |
| Boa | Rust 原生 | 生态和兼容性需验证 |
| Deno | 权限模型强 | 体积和分发复杂 |
| system Node | 实现快 | 分发、权限、稳定性不可控 |

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
  loop_mode
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
6. 遇到 hash 不一致，进入 `NeedsMigration` 或 `Blocked`。

### 8.4 Host API MVP

| API | 作用 | 底层接入 |
| --- | --- | --- |
| `workflow.tool(id, { name, args })` | 调任意工具 | `execute_tool_with_context` + permission |
| `workflow.fileSearch(id, args)` | 文件搜索 | `filesystem::search_files` |
| `workflow.read(id, path)` | 读文件快捷方式 | `read` tool |
| `workflow.grep(id, args)` | 内容搜索 | `grep` tool |
| `workflow.spawnAgent(id, args)` | 子代理 | `subagent` |
| `workflow.waitAll(id, handles)` | 等待多任务 | async job / subagent status |
| `workflow.task.create/update(id, args)` | 用户可见进度 | `task_create/update` |
| `workflow.validate(id, args)` | 验证命令 | `exec` async job + AGENTS 策略 |
| `workflow.askUser(id, args)` | 人工 gate | `ask_user` |
| `workflow.trace(id, payload)` | trace event | `workflow_events` |
| `workflow.diff(id)` | diff snapshot | git / session artifacts |
| `workflow.finish(result)` | 完成 | `workflow_runs.state` |

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
   - 禁 raw `Date.now` / `Math.random` / `new Date()`，改用 `workflow.now()` / `workflow.random(id)`
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
- 是否使用 stable op id。
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

**token/cost 维度（红线，补结构计数之外）**：每个 loop_mode 的预算除上述结构计数外，必须含 `max_output_tokens` 硬天花板（跨主线程 + 全部子 agent 共享一个池）。达上限后会消耗 token 的 op（spawnAgent / validate 触发的 LLM 轮）直接拒绝 + `Blocked`，对齐"耗尽即停"语义。`autonomous` **必须**显式设 token 与 runtime 上限，否则拒绝进入。`0` 语义按 async_tools 约定（仅明确允许项为不限，其余钳地板）。

### 10.4 No-progress 检测

**repair 由 runtime 系统侧编排，不在脚本内循环（红线）**：脚本/模板只描述"一轮怎么跑"，单 run 内 script_hash 不可变；validate 失败 → 生成 structured feedback 作为下一轮 op 输入注入，而非改写脚本（脚本改写会使整 run replay 失效，与"同 run 内 repair"矛盾）。详见 [runtime §7](workflow-script-runtime.md)。

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

右侧 Workspace 面板新增 workflow 视图：

Tabs：

- Overview：目标、状态、预算、当前步骤。
- Script：脚本源码、hash、审批状态。
- Tasks：用户可见 task。
- Agents：subagent / async jobs。
- Trace：事件流。
- Diff：当前 diff snapshot。
- Validation：验证命令和结果。

### 11.2 用户控制

Slash / UI：

```text
/workflow
/workflow trace
/workflow pause
/workflow resume
/workflow cancel
/loop off
/loop guarded
/loop deep
/loop autonomous
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

| 来源 | 默认策略 |
| --- | --- |
| model-generated one-off | 必须用户审批 |
| saved user workflow | 首次审批，hash 变更重审 |
| bundled Hope-native workflow | release 信任，但高风险 op 仍审批 |
| imported workflow | 默认不信任 |
| cron/autonomous workflow | 必须显式 allowlist + budget |

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

实现：

- `CodingTaskKind`
- `LoopMode`
- `CodingSessionProfile`
- Prompt/profile 注入摘要。

验收：

- review 请求不会误进入 implement。
- debug 请求会要求证据。
- feature 请求会要求 plan / verification。

### Phase 2.3：Plan Gate + Script Draft Gate

实现：

- Plan gate checker。
- Script gate checker。
- 失败时返回可修正 feedback。

验收：

- 缺 Critical Files 的 plan 被拦。
- 无 Verification 的 plan 被拦。
- 缺 stable op id 的 script 被拦。
- 使用 raw capabilities 的 script 被拦。

### Phase 2.4：WorkflowRun durable store

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

实现：

- `workflow.js` 执行。
- host API MVP。
- durable replay。
- user approval。

验收：

- 一个 script 能 spawn 2 个 read-only subagents 并汇总。
- 一个 script 能运行 targeted validation。
- 重启后 replay 不重复已完成 host call。

### Phase 2.6：Workflow Panel

实现：

- Overview / Script / Tasks / Agents / Trace / Validation。
- Pause / Resume / Cancel。

验收：

- 长任务期间用户能看懂状态。
- validation failure 清楚展示。
- subagent 状态清楚展示。

### Phase 2.7：Guarded repair loop

实现：

- validation feedback -> repair prompt。
- no-progress 检测。
- diff snapshot。
- stop reason。

验收：

- 连续两轮无有效 diff 会停并 ask_user。
- 验证失败原因不变会停。
- guarded 最多 1-2 次 repair。

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

| 能力 | 本方案是否覆盖 | 说明 |
| --- | --- | --- |
| Skills | 是 | Hope-native skill suite，vendor skill 不进核心 |
| Dynamic workflows | 是 | script-first，而不是纯结构化节点 |
| Subagents | 是 | host API 接现有 subagent 队列 |
| Hooks | 是 | host calls 走现有 hooks / permission |
| Script approval | 是 | script gate + user approval + hash trust |
| Long-running workflows | 是 | durable run / ops / events |
| Resume | 是 | replay-based durability |
| Cancellation | 是 | workflow cancel / pause / resume |
| Trace | 是 | workflow_events + UI panel |
| Loop stop conditions | 是 | no-progress / validation fingerprint / budget |
| Performance | 是 | state externalization + summaries + deferred tools |
| Safety | 是 | no raw fs/network/process/env |

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

| 风险 | 处理 |
| --- | --- |
| JS runtime async bridge 复杂 | MVP host API 保守；先验证 3-5 个调用 |
| replay 要求 stable id，模型可能忘 | script gate 强制字面量 id |
| script 太自由导致难审 | preview + lint + budget + no raw capability |
| 长任务 UI 复杂 | 先复用 Workspace panel |
| subagent 成本高 | bounded concurrency + explicit budget |
| 旧 vendor skills 行为不一致 | detox 标记，不进核心 |
| 用户不想看脚本 | 展示摘要 + 可展开源码 |
| autonomous 风险高 | 默认不开放或只允许 allowlisted scripts |

## 19. 验收标准

Phase 2 完成时，应满足：

1. 有 Hope-native coding skills，不依赖第三方移植 skills。
2. 有 script-first workflow runtime RFC 和 MVP。
3. 至少一个 workflow script 可编排 subagents。
4. workflow run 可恢复、可取消、可查看 trace。
5. 长任务 UI 能展示当前状态、任务、子代理、验证、失败原因。
6. guarded repair loop 有停止条件。
7. 不绕过 permission / hooks / async jobs / subagent / task。
8. 通过至少 5 个 coding eval 场景，并接 [Phase 0 baseline](coding-eval.md) 不回归：
   - parallel review
   - debug with failing test
   - feature implementation
   - no-progress repair stop
   - user approval / cancel / resume

## 20. 下一步

推荐立即做：

1. ~~写 `docs/roadmap/coding-skills-detox.md`~~ → 已产出 [Coding Skills Detox 审计](coding-skills-detox.md)。
2. ~~写 `docs/roadmap/workflow-script-runtime.md`~~ → 已产出 [Script-first Workflow Runtime 设计](workflow-script-runtime.md)。
3. 新建第一批 `ha-*` native skills。
4. 实现 Plan Gate / Script Gate 的纯函数和 fixture。
5. 实现 durable store + 状态机（无 JS，纯函数 + fixture，[runtime §14](workflow-script-runtime.md)）。
6. 再进入 embedded runtime 代码实现。


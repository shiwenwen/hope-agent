# Script-first Workflow Runtime 设计（durable replay 细化）

> 返回 [路线图索引](README.md) · 上层方案 [Phase 2 Coding Mode 与 Script-first Dynamic Workflow](phase2-coding-mode-dynamic-workflow.md)
>
> 状态：Draft RFC
>
> 更新时间：2026-06-30

## 1. 文档定位

[Phase 2 方案](phase2-coding-mode-dynamic-workflow.md)定了方向：script-first、durable-by-replay、host API、长任务可恢复。本文只回答**怎么实现才不出事**——把 durable replay 的正确性、并发、恢复、确定性细化到可以直接开工的程度。

边界：

- 本文是 roadmap 期 RFC，不是最终架构文档。实现稳定后再沉淀到 `docs/architecture/workflow.md`。
- 本文不重复上层方案的动机和总体架构，只补实现契约。
- 上层方案选定 Phase 2 内实现**通用内嵌 JS 引擎**（不是只跑固定模板），本文按这个前提设计；bundled 模板只作为 release-trusted 的特例输入（见 §10），不改变 runtime 形态。

一句话定位：

> WorkflowRun 是 durable execution 的最小本地实现。脚本是纯编排逻辑，所有"会改变世界的事"都必须经过有记录、可重放、恰好一次语义受控的 host op。

## 2. 不变量（红线）

实现期任何改动都不得违反：

1. **脚本是纯函数**：给定相同脚本源码 + 相同 host op 历史输出，脚本的控制流和 op 序列必须逐字节可复现。非确定性只能来自 host op 的记录值。
2. **副作用只在 host op 内发生**：脚本本体不碰 fs / network / process / env / clock / random。
3. **每个 op 有稳定身份**：身份由**结构化位置**派生，不由模型手写字符串决定（见 §4.1）。
4. **恢复不重放已落地的副作用**：已 `Completed` 的 op 重放只返回历史输出；`Started` 但未完成的非幂等 op 进入显式判定，绝不盲目重跑（见 §3.3）。
5. **Primary-only 执行**：WorkflowRun 的调度、脚本执行、崩溃恢复只在 Primary 实例发生，镜像 cron / wakeup 模式。
6. **coordinator 不占 worker 槽**：脚本运行时等待子任务时绝不持有 `async_jobs::slots` 或前台 idle guard（见 §8）。
7. **不绕过既有安全闸**：所有 op 落到既有 `permission::engine` / hooks / `effective_kb_access` / 无人值守 fail-closed；workflow 不新建平行权限或并发系统。
8. **incognito 与 WorkflowRun 互斥**：durable 的本质是持久化，与"关闭即焚"冲突，直接互斥（见 §11）。

## 3. Durable Execution 模型

### 3.1 数据模型

落 `sessions.db`（真相源），文件镜像只作 export / debug 视图。

```text
workflow_runs
  id              TEXT PK
  session_id      TEXT          -- 归属会话（incognito 会话不允许建 run）
  kind            TEXT          -- coding.fix_bug | coding.feature | coding.review | coding.debug | ...
  state           TEXT          -- 见 §3.2 run 状态机
  loop_mode       TEXT          -- off | guarded | deep | autonomous
  script_hash     TEXT          -- 脚本源码 BLAKE3
  script_source   TEXT          -- 脚本正文（也可外置文件，DB 为准）
  budget_json     TEXT          -- 见 §9
  cursor_seq      INTEGER       -- 已确认推进到的 op 序号（恢复用）
  primary_owner   TEXT          -- 占用的 Primary 实例标识（防跨实例双跑）
  created_at / updated_at / completed_at

workflow_ops
  id              TEXT PK
  run_id          TEXT FK
  op_key          TEXT          -- 结构化位置键，(run_id, op_key) UNIQUE
  op_type         TEXT          -- tool | spawnAgent | validate | askUser | fileSearch | ...
  effect_class    TEXT          -- pure | idempotent | non_idempotent（见 §3.3）
  input_hash      TEXT          -- 归一化输入 BLAKE3
  input_json      TEXT
  state           TEXT          -- 见 §3.2 op 状态机
  output_json     TEXT
  error_json      TEXT
  child_handle    TEXT          -- 关联的 async job / subagent run id（恢复 attach 用）
  started_at / completed_at

workflow_events
  id              INTEGER PK
  run_id          TEXT FK
  seq             INTEGER       -- 单 run 内单调递增
  type            TEXT
  payload_json    TEXT          -- 经 sanitize，有大小上限
  created_at
```

`(run_id, op_key)` 唯一约束是 durable replay 的地基：重放时按 op_key 查历史，命中即复用。

### 3.2 状态机

**run 状态**（对齐上层方案 §10.1）：

```text
Draft -> AwaitingApproval -> Running
Running -> AwaitingUser -> Running
Running -> Paused -> Running
Running -> Recovering -> Running        (重启后)
Running -> {Completed | Failed | Cancelled | Blocked}
```

**op 状态**（本文新增，正确性关键）：

```text
Pending     -- 已分配 op_key，尚未执行
Started     -- 已发起，副作用可能已部分/全部发生，输出未确认落库
Completed   -- 输出已落库（output_json 持久化成功）
Failed      -- 终态失败（error_json 落库）
```

`Pending → Started → {Completed | Failed}`。**写 `Started` 必须在发起真实副作用之前先落库**（先记录"我要做这件事"），写 `Completed` 必须在拿到输出后单次 UPDATE 落库——这两步之间就是崩溃危险窗口，由 §3.3 处理。

### 3.3 副作用分类与恰好一次边界

`op_type` 在工具定义层映射到 `effect_class`（复用 Phase 1 ToolDefinition v2 的 `read_only` / `destructive` 元数据，不另立来源）：

| effect_class | 例 | 崩溃在 `Started` 时的恢复策略 |
| --- | --- | --- |
| `pure` | `read`/`grep`/`fileSearch`/`diff` | 直接重跑（无副作用，输出可能微变但不破坏世界） |
| `idempotent` | 带 idempotency key 的写、`task.update` 到固定状态 | 重跑前先按 key 查真实状态，已生效则补记 output |
| `non_idempotent` | `apply_patch`/`edit`/`write`/有副作用的 `exec` | **不盲目重跑**：见下 |

**`non_idempotent` op 在 `Started` 状态崩溃恢复**（红线）：

1. 优先用**可验证的世界状态**判定是否已生效：
   - `apply_patch` / `edit`：比对目标文件当前 BLAKE3 与 op 记录的 `expected_post_hash`。命中 = 已生效，补写 `Completed`；未命中且 `expected_pre_hash` 仍在 = 未生效，可重跑；都不匹配 = 无法判定。
   - `exec`：默认无法判定（命令可能有任意外部副作用）。
2. 无法判定时 run 进入 `Blocked`，emit `workflow:blocked`，由用户决定重跑 / 跳过 / 取消。**绝不静默重跑**。

这把 durable replay 对外语义钉死为：**pure op = at-least-once 安全**；**带 hash 守卫的文件写 = 可恢复 exactly-once**；**裸 exec 等不可判定副作用 = 崩溃后 fail-safe 转人工**，不假装透明。这是和"30 分钟稳定 + 重启可恢复"目标对齐的唯一诚实做法。

> 文件写的 `expected_pre_hash` / `expected_post_hash` 复用 Knowledge Base 已有的 stale-write guard 思路（比磁盘当前 raw hash），不另造一套。

## 4. Op 身份与确定性

### 4.1 位置化 op-key，不要模型手写字面量 id

上层方案 §8.5 要求"所有 host call 必须有字面量 id"，且 §8.1 示例用 `` `review:${file.relPath}` `` 作 id——这是**错误方向**，原因：

- 模板 id 依赖上一个 op 的结果，模型既要保证唯一又要保证跨重放稳定，极易写错。
- 结果驱动 fan-out（对 `fileSearch` 结果做 map）时，子 op 的**集合**取决于上游结果与顺序，上游稍变 op_key 集合就错位，replay 直接崩。

**正确做法**：op 身份 = **执行位置路径**，由 runtime 自动生成，模型写的字符串只作展示 label：

```text
main/op#0(fileSearch)
main/op#1(map "parallel-review")/item#0/op#0(spawnAgent)
main/op#1(map "parallel-review")/item#1/op#0(spawnAgent)
main/op#2(validate)
```

这等价于本运行时（Claude Code Workflow）采用的"全局调用序号即 op 身份"——*同脚本同参数 → 100% 命中*，恢复时*最长未改前缀直接返回缓存，第一个改动点及其后实跑*。位置键比模型手写 id 稳得多，因为身份完全由确定性控制流派生。

> label（`"parallel-review"` / `"review:foo.rs"`）仍鼓励写，但只进 `workflow_events` 和 UI，不参与 op_key。

### 4.2 fan-out 物化

map / 并行扇出必须把**物化后的输入列表作为该 map op 自身的 output 记录下来**：

1. `map` op 首次执行：求值输入列表（如 `files.matches.slice(0,4)`），把这份列表写进 map op 的 `output_json`，再按 `…/item#<index>` 给每个子 op 分配位置键。
2. 重放：map op 命中历史，直接返回**记录的那份列表**（同序、同元素），子 op 按位置键各自重放或续跑。

效果：即便上游 `fileSearch` 换了实现 / 换了序，已开始的 map 扇出仍按冻结的列表恢复，**且支持部分子任务已完成的断点续跑**。这是结果驱动 fan-out 能 durable 的关键。

### 4.3 确定性靠 runtime throw，不靠 lint

区分两件常被混在一起的事：

- **能力沙箱（结构性，可靠）**：用内嵌引擎且**只注入 host API 绑定**，脚本根本拿不到 `fs`/`net`/`process`/`require`——访问不到不是因为被 lint 拦，而是因为绑定不存在。**不要用 denylist 拦能力**（`[].constructor.constructor('return process')()` 这类逃逸让 denylist 形同虚设）。
- **确定性 shim（行为性）**：`Date.now()` / `Math.random()` / `new Date()`（无参）在 runtime **直接 throw**，并提供确定性替代：`workflow.now()`（返回 run 起始锚定时间）、`workflow.random(id)`（按 op_key 派生的确定性随机）。这与本运行时一致——*Date.now/Math.random 会破坏 resume，所以直接禁用*。

Script gate 的静态 lint（§10）只作**早期友好报错**（提前告诉模型"你用了 Date.now"），**不是安全/正确性边界**。边界是 runtime 的结构沙箱 + throw。

## 5. 恢复与重放算法（Primary-only）

仅 Primary 实例执行。启动恢复：

```text
for each run where state == Running and primary_owner is stale/empty:   // 仅 Primary
  claim run: set primary_owner = this_instance (CAS)
  set state = Recovering
  re-run script from top:
    on each host op:
      lookup (run_id, op_key)
      ├─ Completed        -> 返回 output_json（不重跑）
      ├─ Failed           -> 按脚本逻辑：抛出或返回错误（与首跑一致）
      ├─ Started & pure   -> 重跑
      ├─ Started & idempotent     -> 查真实状态，已生效补 Completed，否则重跑
      ├─ Started & non_idempotent -> §3.3 判定，不可判定 -> run=Blocked, 停止重放
      ├─ child_handle 指向在跑的 async job/subagent -> 重新 attach，等其终态
      └─ 不存在（新 op）   -> 正常执行（Pending->Started->Completed）
  脚本跑到 finish -> Completed
  script_hash 与库内不一致 -> run = NeedsMigration/Blocked（见 §7 repair 例外）
```

红线细节：

- **claim 用 CAS + primary_owner**，防两个 Primary（误配）或 Primary 重启竞态导致双跑；非 Primary 永不进入此循环。
- Recovering 期间**不接受新外部输入**，跑完才回 Running。
- 重放是"重新执行脚本"而非"从某行继续"——所以脚本必须满足 §2 纯函数不变量，否则重放分叉。

## 6. Host API 契约

在上层方案 §8.4 基础上补确定性与副作用标注：

| API | effect_class | 底层接入 | 备注 |
| --- | --- | --- | --- |
| `workflow.tool(name, args)` | 由工具元数据决定 | `execute_tool_with_context` + permission | op_key 自动；写类工具带 hash 守卫 |
| `workflow.fileSearch(args)` | pure | `filesystem::search_files` | |
| `workflow.read(path)` / `workflow.grep(args)` | pure | `read` / `grep` tool | |
| `workflow.spawnAgent(args)` | non_idempotent | `subagent` 队列 | 返回 handle，记 `child_handle` |
| `workflow.map(label, list, fn)` | 派生 | runtime | §4.2 物化；`fn` 内多为 spawnAgent |
| `workflow.waitAll(handles, {concurrency})` | pure（等待） | async job / subagent status | bounded concurrency，见 §8 |
| `workflow.task.create/update(args)` | idempotent | `task_create/update` | 与 op 自动绑定 |
| `workflow.validate(args)` | non_idempotent(exec) | `exec` async job + AGENTS 策略 | 受 §AGENTS 验证约束 |
| `workflow.askUser(args)` | — | `ask_user` | 走无人值守 fail-closed，见下 |
| `workflow.trace(payload)` | pure | `workflow_events` | sanitize + 大小上限 |
| `workflow.diff()` | pure | git / session artifacts | |
| `workflow.now()` / `workflow.random(id)` | pure | runtime 确定性源 | 替代 Date.now/Math.random |
| `workflow.finish(result)` | — | `workflow_runs.state` | |

**`workflow.askUser` 红线**：必须经 `evaluate_approval_surface(session_id)` 判定可应答性——autonomous / cron / headless 无人可答时按 `unattended_approval_action` deny/proceed，**绝不阻塞等待**（否则就是"无限等审批")。复用既有无人值守单一真相源，不另写一套。

不提供（同上层方案）：raw `fs` / `fetch` / `child_process` / 任意 import / 直接 DB / 直接权限旁路。

## 7. Repair loop：系统侧编排，脚本描述单轮

上层方案有歧义：repair 在脚本内循环，还是系统侧驱动？**定为系统侧编排**，理由：

- 脚本内 repair 循环要求模型为每次迭代写不撞的 op_key（位置键能解决撞键，但循环可重放性 + script_hash 稳定性仍复杂）。
- repair 改脚本 → `script_hash` 变 → 整 run replay 失效，与"同一 run 内 repair"直接矛盾。

设计：

- **脚本/模板描述"一轮怎么跑"**（observe → act → validate → 产出结构化结果），脚本本身在一个 run 内**不可变**（script_hash 固定）。
- repair 由 runtime 在 run 之上编排：validate 失败 → 生成 structured feedback → **作为新 op 的输入注入下一轮**（或受控的子 run），而非改写脚本。
- no-progress 检测（diff hash / validation fingerprint / changed files 超出 plan critical files / repair 次数 / 预算）在 runtime 层判定，触发即 `AwaitingUser` 或 `Blocked`。

这样 repair 可观察、可加闸、与 durable 模型相容；script_hash 不一致只发生在用户**显式编辑脚本**时，那时才走 NeedsMigration。

## 8. 并发与背压（coordinator 不占 worker 槽）

红线：**WorkflowRun 是协调者，不是 worker**。

- 脚本执行线程在 `waitAll` / `spawnAgent` 等待期间**绝不持有** `async_jobs::slots` 槽或前台 idle guard。否则父占槽等子、子抢不到槽 = 死锁（async_jobs 已有"parked 持槽"同类陷阱）。
- 子任务照走既有两域配额：subagent 走 `subagent::queue`（R7.2），tool job 走 `async_jobs::slots`（R7.1）。workflow runtime 只发起请求 + 记 handle + 等终态，**不维护平行池**。
- `waitAll({concurrency: N})` 支持有界并发：runtime 只控制"同时发起多少 spawn 请求"，实际执行仍受底层队列配额裁决。默认 concurrency 取 loop_mode 的 max_subagents 与底层 per-session 配额的较小值。
- 长扇出"等齐"优先用**完成注入合并**（对齐 background-jobs 的 Group join-all-settle）而非脚本里长 `waitAll` 死等——降低 coordinator 挂起时长。

## 9. 预算（含 token/cost）

上层方案 §10.3 只有结构计数，**补 token/cost 维度**（对齐总纲 LoopPolicy 的 `max_cost` 与 coding-eval 成本指标）：

```text
budget {
  max_runtime_secs
  max_ops
  max_subagents
  max_repair_attempts
  max_validation_cmds
  max_output_tokens        // 新增：硬天花板
  no_progress_rounds
}
```

- `max_output_tokens` 是**硬上限**：累计输出 token 达上限后，后续会消耗 token 的 op（spawnAgent / validate 触发的 LLM 轮）直接拒绝并 `Blocked`，对齐本运行时 *budget.total 是硬 ceiling，耗尽则 agent() throw* 的语义。
- 计数跨 run 主线程 + 所有子 agent 共享一个池（不是 per-op 独立）。
- autonomous 模式**必须**显式设 `max_output_tokens` 与 `max_runtime_secs`，否则拒绝进入 autonomous。
- 各 bounded 旋钮的 `0` 语义须与 async_tools 一致约定（`0`=不限 仅限明确允许的项，其余钳地板），实现时单测锁定。

## 10. Script Gate 与信任分层

执行前 gate（在上层方案 §8.5 基础上明确"哪些是边界"）：

1. **静态 lint（友好报错，非边界）**：禁 `eval`/`Function`/dynamic import/`Date.now`/`Math.random`；提示缺失的确定性替代。**真正的能力/确定性边界在 runtime（§4.3）**。
2. **预算检查**：补全 budget 必填项；autonomous 强制 token + runtime 上限。
3. **权限预览**：静态扫描脚本可能触发的 op 类别（写文件 / exec / browser / mac_control / network），生成给用户看的"这个脚本可能做什么"摘要。
4. **用户审批 + 信任分层**：

| 来源 | 默认策略 |
| --- | --- |
| model-generated one-off | 必须用户审批，按 script_hash 记一次性信任 |
| saved user workflow | 首次审批，hash 变更重审 |
| bundled Hope-native workflow（模板） | release 信任，但高风险 op 仍逐次审批 |
| imported workflow | 默认不信任 |
| cron / autonomous | 必须显式 allowlist + budget；无 allowlist 不跑 |

bundled 模板就是"被 release 信任的脚本"，与通用引擎共用同一 runtime，只是跳过 one-off 审批——这让上层方案选的"通用引擎"路线天然包含"模板更稳"的好处，不必二选一。

## 11. 安全与红线

- 不绕 `permission::engine`；protected path / dangerous command / strict approval / raw CDP / macOS 高危继续生效。
- 无人值守 fail-closed 继续生效，含 `workflow.askUser`（§6）。
- KB access 走 `effective_kb_access`，subagent op 按 origin 血缘判权限，不洗权限。
- **incognito × WorkflowRun 互斥**：incognito 会话拒绝建 run（对齐既有 `Project + incognito` 互斥、静默 coerce 的先例）。不做"少存一点"的折中——op/event 天生含用户内容，durable 与"关闭即焚"不可调和。
- AGENTS 验证策略硬约束：`workflow.validate` 默认单点验证；"跑全套"即使在 autonomous 也是 human-gated op，且 pre-push 钩子本就是全套兜底，autonomous 不该自跑全套。
- secret：脚本不能枚举 env / 读 credential；host 结果默认不回显 secret；trace / issue 导出走 sanitize。

## 12. Runtime 引擎选型

沿用上层方案判断（内嵌优于 system Node）。补落地建议：

- 首选 **QuickJS（rquickjs）**：体积小、可嵌入、**默认无任何宿主绑定**（结构沙箱天然），async host bridge 用 rquickjs 的 async 支持 + Rust 侧 future 映射。
- Boa 作为备选（纯 Rust，但生态/兼容需验证）。
- async bridge 是首要风险：MVP 先只接 §6 中 5–6 个 op（`fileSearch`/`read`/`spawnAgent`/`validate`/`task.*`/`finish`），验证 promise ↔ Rust future ↔ durable op 记录三者闭环后再扩。
- TypeScript 后置：authoring 用 `workflow.js` + JSDoc 类型提示即可，TS 不是 MVP 阻塞项。

## 13. 与现有子系统的接线点（单一入口）

| workflow 动作 | 必须经过的既有单一入口 |
| --- | --- |
| 工具执行 | `tools::execution::execute_tool_with_context` |
| 权限 | `permission::engine::resolve_async` |
| 无人值守判定 | `permission::approval_surface::evaluate_approval_surface` |
| 子代理 | `subagent` spawn + `subagent::queue` |
| 后台并发 | `async_jobs::JobManager` / `async_jobs::slots` |
| 任务进度 | `task_create` / `task_update`（task = 进度真相） |
| hooks | `HookDispatcher::dispatch` |
| 会话/产物 | `session::*` / artifacts |
| KB 访问 | `effective_kb_access` |

**红线**：workflow 不新建平行 job/权限/并发 API，只新增 `workflow_runs/ops/events` 三表 + runtime 编排层。

## 14. 实现里程碑与可测性

对齐上层方案 Phase 2.4 / 2.5，补可测断言：

1. **durable store + 状态机**（无 JS）：纯函数 + fixture 测 op 生命周期、副作用分类恢复判定、Primary-only claim CAS。**无 LLM**。
2. **位置化 op-key + fan-out 物化**：fixture 断言"上游换序不破坏已物化扇出的重放"。
3. **恢复算法**：构造 `Started/non_idempotent` 崩溃 fixture，断言进入 Blocked 而非重跑。
4. **embedded runtime MVP**：脚本 spawn 2 个只读 subagent 并汇总 / 跑一次 targeted validate / 重启后不重复已完成 op。
5. **eval 回归闸**：接 Phase 0 coding-eval baseline，每个里程碑跑一遍要求不回归（见上层方案验收）。

确定性恢复测试必须**无 LLM**（仿 `dreaming_eval` 模式），只测安全红线：副作用恰好一次边界、Primary-only、incognito 拒绝、无人值守 askUser fail-closed、预算硬上限。

## 15. 待验证问题

| 问题 | 处理 |
| --- | --- |
| rquickjs async bridge 与 durable 记录的事务边界 | MVP 先验证 5–6 op；op 落库与副作用发起的先后用 §3.2 强制顺序 |
| `exec` 不可判定副作用面太大 | 默认 non_idempotent + 崩溃转人工；鼓励 validate 用只读命令 |
| 位置键在脚本含数据依赖循环时是否仍稳定 | 循环体由确定性输入驱动即稳定；非确定性输入已被 §4.3 禁 |
| token 预算跨子 agent 归集精度 | 复用现有用量统计；池化计数，单测锁 spent 归集 |
| 模型仍可能写出重放分叉的脚本 | gate lint 早报错 + 重放分叉时 fail-safe 转 Blocked，不静默产出错误结果 |

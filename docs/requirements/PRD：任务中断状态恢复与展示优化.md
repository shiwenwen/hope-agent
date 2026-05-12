# 任务中断状态恢复与展示优化需求文档

## 1. 背景

Hope Agent 当前在执行多步骤任务时，用户手动停止本轮回复后，页面仍可能展示“正在执行 0/4 个任务”，并且 `in_progress` 任务继续显示旋转状态。实际后端已经收到停止请求，当前 turn 不再继续执行，但前端仍把 task 的 `in_progress` 当成“系统正在运行”。

这个问题的根因是运行态缺少独立抽象：

- `task` 只有 `pending / in_progress / completed` 三态，表达的是工作项进度；
- `plan` 只有 `Off / Planning / Review / Executing / Completed` 五态，表达的是计划生命周期；
- `stream` 只有 active / inactive 和 `stream_status`，表达的是流式消息是否仍在写入；
- 系统没有持久化的 turn 级状态来表达“本轮执行 running / interrupted / failed / completed”。

Codex 和 Claude Code 的成熟做法是把“本轮执行状态”和“任务清单状态”拆开：中断只终止当前 turn，不自动完成或清空 todo/task；task 仍保留进度事实，UI 根据 turn 状态显示“已停止，可继续”。

## 2. 目标

### 2.1 业务目标

- 用户手动停止任务后，界面不再误显示“正在执行”。
- 保留已经完成的部分和当前做到的任务，便于用户继续、修改计划、退出计划或回滚。
- App 重启、页面刷新、切换会话后，停止/崩溃后的状态能恢复为明确的 interrupted 状态。
- Plan Mode 执行期中断后保持计划上下文，不误判为完成，也不强制退出 Plan Mode。

### 2.2 技术目标

- 引入持久化的 turn 级生命周期状态，作为“当前是否还在运行”的唯一事实源。
- 让 task 继续只表达工作项进度，避免把 `in_progress` 误用为运行态。
- 将 stop / cancel / crash recovery / stream end 收敛到同一套 turn 状态机。
- 前端 TaskProgressPanel、PlanPanel、ChatInput 基于 turn 状态渲染正确的状态文案和图标。
- 支持按 session / turn 精准取消，避免全局 cancel flag 带来的多会话竞态。

## 3. 非目标

- 不改变 Plan Mode 的五态状态机。
- 不新增 `Paused` plan 状态。
- 不在停止时自动把 task 标记为 `completed`。
- 不在停止时自动把 `in_progress` task 回滚为 `pending`。
- 不重写 task 系统，也不改变 `task_create / task_update / task_list` 的基本使用方式。
- 不改变模型工具调用协议中 task 的三态语义，除非后续独立需求明确要求。
- 不在本需求中实现完整的执行回滚；回滚仍使用现有 Plan git checkpoint 能力。

## 4. 参考实现结论

### 4.1 Codex

Codex 将会话拆分为 Thread / Turn / Item，并定义 `TurnStatus = completed | interrupted | failed | inProgress`。`turn/interrupt` 只终止当前 turn，最终仍发送 turn completed 事件，但 turn status 为 `interrupted`。

Codex 在恢复 thread 状态时，会把没有 live running turn 的 stale `inProgress` turn 标记为 `interrupted`，避免历史会话永久停留在运行中。

### 4.2 Claude Code

Claude Code 的 Esc / Ctrl+C 行为优先取消当前 request，清空 pending permission queue，保留已经流出的 partial assistant 文本，并 reset loading。TodoWrite 仍只有 `pending / in_progress / completed`，中断不会自动把 todo 改成 completed。

这说明 Hope 应补齐 turn 状态，而不是给 task 增加“停止即完成”或“停止即清空”的特殊规则。

## 5. 当前问题

### 5.1 停止后 UI 状态误导

现状：

- 用户点击停止后，后端设置 cancel flag；
- stream 最终结束，loading 被清理；
- task 仍存在 `in_progress`；
- TaskProgressPanel 基于 task status 显示旋转状态；
- 页面出现“正在执行 0/4 个任务”一类不合理展示。

期望：

- 当前 turn 被标记为 `interrupted`；
- task 可保留 `in_progress`，但 UI 应显示“已停止”或“等待继续”，不再显示正在执行动画；
- 用户可以继续执行剩余任务。

### 5.2 task / plan / stream 三层职责混淆

现状：

- task 的 `in_progress` 同时被用作“当前任务做到这里”和“系统正在执行中”；
- stream active 表达的是流是否在写，不足以表达上一个 turn 是 completed / interrupted / failed；
- plan Executing 表示计划处于实施期，不代表当前 turn 正在执行。

期望：

- turn status 表达当前或最近一轮执行状态；
- task status 表达工作项状态；
- plan status 表达计划生命周期；
- stream status 表达消息块持久化状态。

### 5.3 状态恢复缺少 turn 级扫尾

现状：

- 已有 `messages.stream_status` 的 `streaming -> orphaned` 崩溃恢复；
- 但没有持久化 turn 行，因此无法稳定知道 session 的最后一轮是 interrupted、failed 还是 completed。

期望：

- App 启动或 session 恢复时，将没有 live registry 支撑的 `running / cancelling` turn 标记为 `interrupted(reason=crash_recovery)`；
- 前端可基于持久化状态恢复正确展示。

### 5.4 多会话 cancel 竞态

现状：

- 桌面聊天停止使用全局 `chat_cancel` flag；
- 多会话或后台流同时存在时，存在错误影响其它 turn 的风险。

期望：

- cancel token 按 turn 或至少按 session 隔离；
- `stop_chat` 携带 `turnId` 时只取消对应 active turn；
- stale stop 请求不得取消新 turn。

## 6. 需求范围

### R1：新增持久化 Chat Turn 生命周期

系统应新增 turn 级持久化模型，用于记录每一轮用户请求的生命周期。

建议字段：

```text
chat_turns:
  id
  session_id
  source
  status
  interrupt_reason
  stream_id
  started_at
  ended_at
  user_message_id
  assistant_message_id
  error
```

状态枚举：

```text
running
cancelling
completed
interrupted
failed
```

`interrupt_reason` 建议枚举：

```text
user_stop
shutdown
crash_recovery
tool_cancel
runtime_cancel
unknown
```

验收标准：

- 每个 desktop / HTTP 主聊天 turn 都有一条 turn 记录；
- turn start 时状态为 `running`；
- 用户停止后状态最终为 `interrupted`；
- 正常完成后状态为 `completed`；
- 异常失败后状态为 `failed`；
- turn 记录可通过 session 查询。

### R2：stop_chat 改为 turn 级中断

停止接口应支持按 session + turn 精准取消。

行为要求：

- 前端调用 `stop_chat` 时带上当前 active `turnId`；
- 后端校验该 turn 仍是当前 session 的 active turn；
- 校验通过后将状态改为 `cancelling`；
- 取消该 turn 关联的 LLM stream、runtime task、pending approval、pending ask_user；
- engine 收敛后将状态改为 `interrupted`；
- stale turnId 请求应返回 no-op 或明确错误，不得取消新的 active turn。

验收标准：

- 快速停止后立刻发送新消息，新消息不会被旧 stop 请求取消；
- 多会话并发时，停止 A 会话不影响 B 会话；
- 停止状态在前端可实时收到。

### R3：stream_end 携带 turn 终态

`chat:stream_end` 事件应携带 turn 级终态信息。

建议 payload：

```json
{
  "sessionId": "...",
  "streamId": "...",
  "turnId": "...",
  "status": "completed | interrupted | failed",
  "interruptReason": "user_stop | crash_recovery | ..."
}
```

验收标准：

- 前端不再仅靠 stream end 判断“正常完成”；
- stop 后收到的 stream end 明确为 `interrupted`；
- failed turn 能与 interrupted turn 区分展示。

### R4：恢复时中断 stale running turn

系统启动、会话恢复或读取 stream state 时，应处理 stale turn。

规则：

- 如果持久化 turn 为 `running / cancelling`；
- 且内存 active turn / stream registry 中没有对应 live turn；
- 则将 turn 标记为 `interrupted(reason=crash_recovery)`。

验收标准：

- App 崩溃重启后，历史 session 不会显示为仍在运行；
- partial assistant 内容保留，并显示 interrupted 标记；
- task 面板显示为“已停止/等待继续”，而不是旋转执行中。

### R5：扩展 session stream state 查询

`get_session_stream_state` 应返回 turn 级状态。

建议返回：

```ts
interface SessionStreamState {
  active: boolean
  lastSeq: number
  streamId?: string | null
  turnId?: string | null
  status?: "running" | "cancelling" | "completed" | "interrupted" | "failed"
  lastTerminalStatus?: "completed" | "interrupted" | "failed"
  interruptReason?: string | null
}
```

验收标准：

- 前端切换会话后能恢复当前 running 状态；
- 前端切回已中断 session 时能显示 interrupted；
- loading 状态只由 active running / cancelling turn 驱动。

### R6：TaskProgressPanel 基于 turn 状态展示

TaskProgressPanel 不应只凭 task status 判断是否“正在执行”。

展示规则：

- `task.status = in_progress` 且 `turn.status = running`：显示旋转图标和正在执行；
- `task.status = in_progress` 且 `turn.status = cancelling`：显示停止中；
- `task.status = in_progress` 且 `turn.status = interrupted`：显示已停止/等待继续；
- `task.status = in_progress` 且没有 active turn：显示等待继续；
- `pending` 和 `completed` 维持现有含义。

验收标准：

- 用户手动停止后，面板不再显示旋转中的 in_progress task；
- 顶部标题从“正在执行”类文案切换为“已停止”或“等待继续”；
- 已完成任务仍显示 completed，未开始任务仍显示 pending；
- 不改变 task 本身状态，刷新后展示一致。

### R7：Plan Executing 中断后的行为

Plan Mode 执行期中断后，应保持 Plan 状态为 `Executing`。

行为要求：

- 不自动转 `Completed`；
- 不自动转 `Off`；
- git checkpoint 保留；
- PlanPanel 显示执行已停止，提供继续、重新规划、退出、回滚入口；
- 继续时发送明确的 plan trigger，让模型基于当前 task list 和 interrupted turn 继续。

验收标准：

- 停止执行后 plan 状态仍为 `executing`；
- 完成全部 scoped tasks 后才自动转 `completed`；
- 用户可以继续执行剩余任务；
- 用户可以进入 Planning 修订计划。

### R8：partial assistant 内容保留与标记

用户停止后，已经流出的 assistant partial 内容应保留，不应被误删或误当成正常完成。

行为要求：

- partial text / thinking block 显示 interrupted 标记；
- 后续 resume turn 的上下文能知道上一轮被中断；
- 不把用户主动停止渲染成错误。

验收标准：

- 停止发生在 assistant 已输出文本后，历史消息保留这段文本；
- 消息显示“已中断”标记；
- 不显示红色错误状态，除非实际失败。

### R9：pending approval / ask_user 中断收敛

用户停止当前 turn 时，与该 turn 关联的 pending approval 和 ask_user request 应被取消或标记为 interrupted。

验收标准：

- 停止后不会残留不可操作的审批弹窗；
- 停止后 sidebar 的 pending interaction count 不再包含该 turn 的旧请求；
- 如果用户稍后继续执行，应由新 turn 创建新的 approval / ask_user。

### R10：事件与 UI 状态单一来源

前端 loading、stop button、TaskProgressPanel、PlanPanel 的执行态应统一来自 turn state。

验收标准：

- stream active 只表示当前有实时流；
- turn state 表示本轮执行结果；
- task state 表示任务进度；
- plan state 表示计划生命周期；
- 四者不互相覆盖、不重复推导。

## 7. 推荐状态机

### 7.1 Turn 状态机

```text
running
  ├─ normal finish ───────> completed
  ├─ user stop ───────────> cancelling ─────> interrupted
  ├─ runtime cancel ──────> cancelling ─────> interrupted
  ├─ recover stale turn ──> interrupted
  └─ unrecoverable error ─> failed
```

约束：

- terminal 状态为 `completed / interrupted / failed`；
- terminal turn 不允许再回到 running；
- 继续执行必须创建新 turn；
- stale stop 不得改变新 turn。

### 7.2 与 task 的关系

```text
turn.running + task.in_progress      => 正在执行
turn.interrupted + task.in_progress  => 已停止，等待继续
turn.completed + task.in_progress    => 模型未收尾或仍有剩余任务，等待继续
turn.failed + task.in_progress       => 执行失败，等待处理
```

task 是否 completed 只能由模型、用户手动操作或明确业务逻辑决定，不能由 turn interrupted 自动决定。

### 7.3 与 plan 的关系

```text
plan.executing + turn.completed + scoped tasks all completed => plan.completed
plan.executing + turn.interrupted                            => plan.executing
plan.executing + turn.failed                                 => plan.executing
plan.planning  + turn.interrupted                            => plan.planning
```

## 8. 前端交互要求

### 8.1 TaskProgressPanel

停止后建议展示：

- 标题：`任务 · 已停止 0/4`
- 当前任务图标：停止/暂停类图标，而不是 Loader2 旋转；
- 当前任务文案：使用 `activeForm` 或 `content`；
- 操作：保留手动切换 task 状态能力。

### 8.2 ChatInput

停止完成后：

- loading 结束；
- stop button 消失；
- send button 恢复；
- 如果仍有未完成 task，允许用户输入或点击“继续执行”。

### 8.3 PlanPanel

Plan Executing 且最近 turn interrupted 时：

- 不显示“正在执行”；
- 显示“执行已停止”；
- Continue 按钮可用；
- Rollback / Exit / Request Changes 保持可用。

## 9. 数据兼容与迁移

- 新增表应可空关联旧消息，旧 session 没有 turn 记录时按历史 completed 处理；
- 旧 `messages.stream_status` 语义保持不变；
- 旧 task 表不做破坏性迁移；
- 启动 sweep 应幂等，可重复执行；
- HTTP / Tauri transport 返回结构需要保持向后兼容，新增字段可选。

## 10. 验收场景

### S1：执行中立即停止

1. 进入 Executing plan；
2. 创建 4 个 tasks；
3. 第 1 个 task 标为 `in_progress`；
4. 用户点击停止。

期望：

- turn 最终为 `interrupted(user_stop)`；
- task 仍为 0/4 completed；
- 第 1 个 task 不再旋转；
- 面板显示已停止；
- plan 仍为 Executing。

### S2：停止后继续

1. 基于 S1；
2. 用户点击继续或发送“继续”。

期望：

- 创建新 turn；
- UI 重新进入 running；
- 模型看到上一轮 interrupted 和当前 task list；
- 可继续第 1 个 task 或调整 task 状态。

### S3：停止后刷新页面

1. 执行中停止；
2. 刷新前端或切换 session；
3. 回到该 session。

期望：

- 不显示 loading；
- 不显示正在执行；
- task 面板恢复已停止状态；
- partial assistant 保留 interrupted 标记。

### S4：崩溃恢复

1. turn running 中强制退出 App；
2. 重启 App；
3. 打开 session。

期望：

- stale running turn 被标记为 `interrupted(crash_recovery)`；
- streaming message row 被标记或展示为 orphaned/interrupted；
- UI 不再认为仍在执行。

### S5：多会话并发取消

1. A session 正在执行；
2. B session 正在执行；
3. 用户停止 A。

期望：

- A interrupted；
- B 继续 running；
- B 的 stream 和 task UI 不受影响。

### S6：stale stop 请求

1. 用户停止 turn-1；
2. 立即发送新消息创建 turn-2；
3. turn-1 的 stop 请求延迟到达。

期望：

- turn-2 不被取消；
- 后端忽略或拒绝 stale stop。

## 11. 风险与注意事项

- turn 状态会成为跨前后端的新基础契约，必须一次设计清楚字段和事件语义；
- stop / stream_end / engine error 的异步顺序复杂，必须保证 terminal 状态只写一次；
- pending approval / ask_user 若没有 turn_id 关联，会继续产生幽灵待办；
- front-end loading 现有路径较多，需要避免引入第二套相互打架的 loading 判断；
- `abort_on_cancel` 当前不同来源语义不同，改造时要小心 IM channel、cron、subagent 不被桌面 turn 逻辑误伤；
- Plan Completed 的自动转换仍应只由 scoped task 全 completed 触发。

## 12. 优先级建议

### P0：正确性

- 新增 turn 持久化；
- stop_chat 支持 turn 级取消；
- stream_end 携带 terminal status；
- recovery sweep；
- TaskProgressPanel 停止态展示。

### P1：Plan 与交互补齐

- PlanPanel 执行已停止态；
- Continue trigger；
- pending approval / ask_user turn 关联与取消；
- 多会话 cancel token 隔离。

### P2：完善与观测

- 日志与 telemetry；
- turn 状态调试信息；
- 历史 session 兼容；
- 更多边缘用例测试。

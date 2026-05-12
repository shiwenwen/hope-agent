# 任务中断状态恢复与展示优化实施计划

## 需求重述

需求来源：`docs/requirements/PRD：任务中断状态恢复与展示优化.md`

当前 Hope Agent 在执行多步骤任务时，用户手动停止当前回复后，后端已经取消本轮执行，但前端仍可能依据 `task.status = in_progress` 展示旋转状态和“正在执行 0/4 个任务”。这会把“任务进度状态”和“当前 turn 是否仍在运行”混在一起，导致停止、刷新、崩溃恢复、Plan Mode 继续执行等场景显示不合理。

本需求要引入持久化的 Chat Turn 生命周期，把 `turn / task / plan / stream` 四层职责拆开：

- `turn` 表达本轮执行生命周期：`running / cancelling / completed / interrupted / failed`。
- `task` 继续只表达工作项进度：`pending / in_progress / completed`。
- `plan` 继续只表达计划生命周期：`Off / Planning / Review / Executing / Completed`，不新增 `Paused`。
- `stream` 继续表达消息块流式写入状态：`streaming / completed / orphaned`。

最终效果是：用户停止后，当前 turn 进入 `interrupted(user_stop)`，Plan 仍保持 `Executing`，task 不被自动完成或回滚，UI 显示“已停止/等待继续”，并支持刷新、重启、多会话并发和后续继续执行。

该需求涉及前端 UI 状态与文案调整。目前未提供 Figma 设计稿；规划默认按现有 Chat / TaskProgressPanel / PlanPanel 的 shadcn + Tailwind 风格实现，不调整整体视觉主题。

## 当前代码依据

- Tauri `stop_chat` 当前只设置全局 `chat_cancel` 并取消 runtime tasks：`src-tauri/src/commands/chat.rs`。
- HTTP server 已有按 session 的 `chat_cancels` map，但前端 `stop_chat` 仍是“停止当前 chat”的粗粒度语义：`crates/ha-server/src/routes/chat.rs`。
- `active_turn` 目前只做内存级 per-session 互斥，没有持久化 turn id 或终态：`crates/ha-core/src/chat_engine/active_turn.rs`。
- `SessionStreamState` 当前只有 `active / lastSeq / streamId`，缺少 terminal turn status：`crates/ha-core/src/chat_engine/mod.rs`。
- `chat:stream_end` 当前只广播 `sessionId / streamId`，无法区分正常完成、中断和失败：`crates/ha-core/src/chat_engine/stream_broadcast.rs`。
- stream persister 已有 `messages.stream_status = streaming / completed / orphaned` 和 startup sweep，可复用恢复思路：`crates/ha-core/src/chat_engine/persister.rs`、`crates/ha-core/src/session/db.rs`。
- task 状态只有三态，Plan 自动完成只在 scoped tasks 全部 completed 时触发：`crates/ha-core/src/session/tasks.rs`、`crates/ha-core/src/plan/transition.rs`。
- `TaskProgressPanel` 当前直接把 `task.status = in_progress` 渲染为活动态，是本问题的直接 UI 表现来源：`src/components/chat/tasks/taskProgress.ts`、`src/components/chat/tasks/TaskProgressPanel.tsx`。
- 前端已有 stream reattach 和 `get_session_stream_state` 查询链路，可扩展而不是重建：`src/components/chat/hooks/useChatStreamReattach.ts`、`src/components/chat/hooks/useChatStream.ts`。

## 非目标

- 不新增 Plan `Paused` 状态。
- 不改变 task 三态语义，不把停止自动写成 `completed`，也不把 `in_progress` 自动回滚为 `pending`。
- 不重写 Plan Mode、task tool 协议或 chat engine 主循环。
- 不实现完整执行回滚；回滚仍使用现有 Plan git checkpoint。
- 不改 IM channel、cron、subagent 的展示语义，除非它们共享的底层取消/turn 记录必须适配。
- 不做大规模 UI 重设计，只修正状态来源、文案和图标。

## 总体设计

新增 `chat_turns` 作为 turn 生命周期单一事实源，chat engine 创建 turn 时写入 `running`，用户停止时先写 `cancelling`，engine 退出收敛后写 terminal 状态。terminal 状态只能是 `completed / interrupted / failed`，且只允许写一次。

前端执行态不再从 task 反推，而是从 `SessionStreamState`、`chat:stream_delta`、`chat:stream_end` 和本地 active turn state 共同维护：

- `turn.running + task.in_progress`：正在执行。
- `turn.cancelling + task.in_progress`：停止中。
- `turn.interrupted + task.in_progress`：已停止，等待继续。
- `turn.failed + task.in_progress`：执行失败，等待处理。
- 没有 active turn 但仍有 `in_progress` task：等待继续。

Plan Mode 保持现有状态机。`plan.executing + turn.interrupted` 仍是 `Executing`，只有 scoped tasks 全部 `completed` 才能进入 `Completed`。

## 数据模型与后端实施

### Phase 1：Turn 生命周期模型与数据库迁移

1. 在 `crates/ha-core/src/session/` 下新增 turn 类型与 DB helper，建议文件：
   - `turns.rs`：`ChatTurnStatus`、`ChatTurnInterruptReason`、`ChatTurn`、状态机 helper。
   - `db.rs`：迁移与 CRUD 方法，或从 `db.rs` 分发到 `turns.rs` 的 impl。

2. 新增 `chat_turns` 表，建议字段：

   ```text
   id TEXT PRIMARY KEY
   session_id TEXT NOT NULL
   source TEXT NOT NULL
   status TEXT NOT NULL
   interrupt_reason TEXT NULL
   stream_id TEXT NULL
   user_message_id INTEGER NULL
   assistant_message_id INTEGER NULL
   error TEXT NULL
   started_at TEXT NOT NULL
   ended_at TEXT NULL
   updated_at TEXT NOT NULL
   ```

3. 建索引：
   - `(session_id, started_at DESC)`
   - `(session_id, status)` for active lookup
   - `stream_id` optional lookup

4. 提供 DB helper：
   - `create_chat_turn(session_id, source, stream_id, user_message_id) -> ChatTurn`
   - `get_latest_chat_turn(session_id) -> Option<ChatTurn>`
   - `get_active_chat_turn(session_id) -> Option<ChatTurn>`
   - `mark_turn_cancelling(turn_id, reason)`
   - `finish_turn_once(turn_id, terminal_status, reason, error)`
   - `recover_stale_turns(active_turn_ids)`

5. terminal 写入必须幂等：如果 turn 已经是 `completed / interrupted / failed`，后续 finish 不覆盖。

验收：

- 旧 session 无 turn 记录时不报错，查询返回 `None` 或按历史 completed 兼容。
- 新 turn 有持久化记录，正常完成能写 `completed`。
- 重复 finish 不会覆盖第一次 terminal 状态。

### Phase 2：Chat Engine 接入 turn start / finish

1. 在 desktop / HTTP chat 入口获取 session 后创建 turn id，并将 turn id 纳入 engine params 或上下文。

2. 扩展 `active_turn`：
   - 当前 per-session 互斥保留。
   - registry entry 增加 `turn_id`、`stream_id`、`source`。
   - 提供 `current_turn(session_id)` 和 `matches(session_id, turn_id)`。

3. 将 turn id 注入 stream seq：
   - `inject_seq` 给 delta 增加 `_oc_turn_id`。
   - `chat:stream_delta` envelope 增加 `turnId`。

4. `run_chat_engine` 结束时统一收敛：
   - 正常完成：`completed`。
   - cancel flag / runtime cancel：`interrupted(reason)`。
   - error：按错误类型区分 `failed` 和 `interrupted`，用户 stop 不渲染成 error。

5. `abort_on_cancel`、runtime task cancel、provider stream cancel 的判断需形成明确 helper，避免不同入口重复判断。

验收：

- 每个 desktop / HTTP 主聊天 turn 都有 id。
- 每个 delta 和 stream_end 都能关联同一个 turn id。
- 用户 stop 不会被持久化为普通 failed。

### Phase 3：stop_chat 改为 turn 级取消

1. 扩展 Tauri command 与 HTTP route 入参：

   ```ts
   stop_chat({ sessionId?: string, turnId?: string })
   ```

2. 前端 stop 时携带当前 active `turnId`。兼容旧调用：无 turn id 时只取消当前 session active turn，若 session 也为空则走 legacy global fallback 并打 warning。

3. 后端取消流程：
   - 查 active registry，校验 session + turn id 匹配。
   - turn 状态 `running -> cancelling(user_stop)`。
   - 触发该 turn 的 cancel token。
   - 取消该 turn/session 关联的 runtime tasks。
   - 清理或标记该 turn 的 pending approval / ask_user。

4. stale stop 处理：
   - turn id 不匹配时返回 no-op 或 typed error。
   - 不得设置全局 cancel 影响新 turn。

5. Tauri 当前全局 `chat_cancel` 需要替换为 per-turn/per-session cancel registry，或至少先引入 session-scoped map 作为过渡。HTTP 已有 `chat_cancels` map，可向 core 收敛，避免两套实现长期分叉。

验收：

- 停止 A 会话不影响 B 会话。
- 停止 turn-1 后立即创建 turn-2，延迟到达的 turn-1 stop 不会取消 turn-2。
- stop 后 turn 先变 `cancelling`，最终变 `interrupted(user_stop)`。

### Phase 4：stream_end 与 stream state 扩展

1. 扩展 `chat:stream_end` payload：

   ```json
   {
     "sessionId": "...",
     "streamId": "...",
     "turnId": "...",
     "status": "completed | interrupted | failed",
     "interruptReason": "user_stop | crash_recovery | runtime_cancel | ...",
     "error": null
   }
   ```

2. 扩展 `SessionStreamState`：

   ```ts
   active: boolean
   lastSeq: number
   streamId?: string | null
   turnId?: string | null
   status?: "running" | "cancelling" | "completed" | "interrupted" | "failed"
   lastTerminalStatus?: "completed" | "interrupted" | "failed"
   interruptReason?: string | null
   ```

3. `session_stream_state(session_id)` 组合读取：
   - 内存 `stream_seq / active_turn` 给 active running 信息。
   - DB `chat_turns` 给最近 terminal 状态。

4. 保持新增字段可选，HTTP/Tauri transport 向后兼容。

验收：

- 前端刷新后能区分 running、interrupted、failed。
- stream_end 到达时能精确结束当前 turn UI，而不是仅根据 stream end 清 loading。

### Phase 5：恢复与 orphaned turn sweep

1. 启动期在 `app_init` 中增加 `recover_stale_chat_turns`，与现有 `mark_orphaned_streaming_rows` 相邻执行。

2. 恢复规则：
   - DB 中 `running / cancelling` 且内存 registry 没有 live owner。
   - 标记为 `interrupted(crash_recovery)`。
   - 不修改 task 状态。

3. 会话打开或 `get_session_stream_state` 时也可做 session-scoped lazy recovery，确保 server 长驻或边缘路径幂等。

4. partial assistant 内容：
   - 已有 `messages.stream_status = orphaned` 继续保留。
   - 若有 turn id，可在 UI 上显示 interrupted marker；短期可复用现有 `InterruptedMark` 的 orphaned 逻辑。

验收：

- App 崩溃重启后不显示正在执行。
- stale running turn 被标记为 `interrupted(crash_recovery)`。
- partial assistant 内容保留。

### Phase 6：pending approval / ask_user 收敛

1. 梳理 approval 与 ask_user 数据模型是否已有 `session_id`，补充 `turn_id` 或通过 session + active turn 关联。

2. stop 当前 turn 时：
   - pending approval 标记为 cancelled/interrupted，关闭前端弹窗。
   - pending ask_user 标记为 expired/interrupted，pending count 不再包含它。

3. 新 turn 继续执行时重新创建新的 approval / ask_user，不复用旧 request。

验收：

- stop 后没有残留不可操作审批。
- sidebar pending count 不含旧 turn 的请求。

## 前端实施

### Phase 7：前端 turn state 建模

1. 在 `src/types/chat.ts` 增加：
   - `ChatTurnStatus`
   - `ChatTurnInterruptReason`
   - 扩展 `SessionStreamState`
   - stream event payload 类型

2. `useChatStream` 维护 active turn：
   - chat 发送后记录 `turnId`。
   - stop 使用 `turnId` 调 command。
   - stream_end 根据 `status` 清理 loading，并保存 last terminal state。

3. `useChatStreamReattach`：
   - 从 `get_session_stream_state` 恢复 turn 状态。
   - `active=false + interrupted` 时不 reattach spinner，只设置 last terminal state。
   - 处理带 `_oc_turn_id` 的 delta 去重。

4. 避免 UI 同时从 `isLoading`、`streamActive`、`task.inProgress` 三处推导运行态。建议建立一个轻量 view model：

   ```ts
   executionState = "idle" | "running" | "cancelling" | "interrupted" | "failed"
   ```

验收：

- loading / stop button / send button 只受 turn active 状态控制。
- 已中断 session 切回后不显示 stop button。

### 审计补充：边缘路径收敛

1. Desktop Plan subagent 早返回路径必须不留下 `running` turn：
   - 若请求只是转发到已存在 planning subagent，turn 直接以 `completed` 收尾并通知前端。
   - 若请求只负责 spawn planning subagent，turn 直接以 `completed` 收尾并通知前端。
   - 或者把 turn 创建延后到真正进入主 chat engine 的路径。

2. Desktop legacy fallback 路径必须补齐 turn-end 契约：
   - 使用与主 engine 一致的 `chat:stream_end` payload。
   - 成功、失败、取消都必须让前端收到 terminal status。

3. HTTP 新会话 stop 不得退化为全局 stop：
   - 前端在 session id 尚未物化时不得发送 `sessionId: null` 的 stop。
   - HTTP EventBus / `get_session_stream_state` 恢复到 turn id 后再执行 session/turn 级停止。

4. Reattach 必须完整消费 `SessionStreamState`：
   - active 状态恢复 `turnId` 与 execution state。
   - inactive terminal 状态恢复 interrupted / failed / completed，不启动 spinner。

5. `ChatInput` task execution state 优先级：
   - 显式 `cancelling / interrupted / failed` 优先于 `loading`。
   - 只有没有显式 execution state 且当前 loading 时才展示 running。

验收：

- Plan subagent 早返回不产生 stale `running` turn。
- legacy fallback 完成后前端不会卡在 running。
- HTTP 新会话未物化 session id 时 stop 不会取消其它会话。
- 刷新/重连后仍能停止当前 turn，且 interrupted / failed 展示可恢复。
- `loading + interrupted` 不渲染 running 文案或 spinner。

### 审计补充：Claude Code 复审收敛

1. 启动恢复必须同步内存 active turn registry：
   - `recover_stale_chat_turns()` 把 DB running/cancelling 改为 interrupted 后，清理 `active_turn` registry。
   - 避免热重启后 DB 已终态但内存仍 active，导致 `get_session_stream_state` 和 `try_acquire` 误判。

2. HTTP model chain 校验失败必须补齐 terminal broadcast：
   - turn 已创建后如果校验失败，除 DB finish 外还要广播 `chat:stream_end`。
   - 前端不得因没有终态事件卡在 loading。

3. 前端 reattach / session 切换必须清 stale active turn：
   - `get_session_stream_state.active=false` 时清理本地 active turn。
   - 后续 stop 不得携带过期 turn id 造成停止无效。

4. 补齐 failed / cancelling 展示测试：
   - `ChatInput` 显式 `cancelling / failed` 优先于 loading。
   - `TaskProgressPanel` failed in-progress task 显示错误图标且不旋转。

5. 文档化非交互入口 `turn_id=None`：
   - `ChatEngineParams.turn_id` 注释说明 Cron / subagent / parent injection / IM channel 不参与 GUI/HTTP turn 级取消。
   - `docs/architecture/chat-engine.md` 补 turn lifecycle 与 stop recovery。

验收：

- 启动恢复后不会留下内存 active turn。
- HTTP 配置错误路径能发出 terminal `chat:stream_end`。
- session 切换后 inactive state 会清本地 stale turn id。
- failed / cancelling 测试覆盖通过。
- 架构文档说明 turn 生命周期边界。

### 审计补充：负责人复核问题收敛

1. 启动期 stale turn / orphan stream 恢复只能由 Primary 进程执行：
   - `recover_stale_chat_turns()` 会写共享 session DB，Secondary server / acp 启动时不得把桌面 Primary 正在运行的 turn 标成 `interrupted`。
   - `mark_orphaned_streaming_rows()` 同属共享 DB 启动恢复写操作，也应纳入 `runtime_lock::is_primary()` gate。
   - Primary 恢复后仍清理本进程 `active_turn` registry，解决热重启内存残留。

2. HTTP transport 不得在 `/api/chat` 响应后合成 late `turn_started`：
   - 服务端真实 `chat:turn_started` / `chat:stream_end` 已通过 EventBus 和 WebSocket 实时广播。
   - `/api/chat` 返回时 engine 通常已结束，此时本地再发 `turn_started` 会把前端 terminal state 改回 `running`，留下 stale active turn。
   - 保留 `session_created` bridge，仅用于新会话把前端 `__pending__` cache key 替换为真实 session id。

验收：

- Secondary 初始化不会修改共享 DB 中仍在运行的 chat turn / streaming placeholder。
- Primary 初始化仍能把真正 stale 的 `running/cancelling` turn 恢复为 interrupted，并清理 active turn registry。
- HTTP `startChat` 响应后不再额外触发本地 `turn_started`；执行态只来源于后端实时事件与恢复状态。

### 审计补充：消息区与输入区任务展示一致

1. `TaskBlock` 不能只根据 task `status=in_progress` 判断正在执行：
   - 消息区历史 task block 也要消费当前 session 的 `executionState`。
   - 当 turn 已经 `idle / interrupted / failed / cancelling` 时，消息区和输入区必须使用同一套文案和图标规则。

2. 将 execution state 贯通到消息渲染链路：
   - `ChatScreen` 从 `stream.executionStateBySession` 取当前 session 状态。
   - 传给 `MessageList`、`MessageBubble`、`AssistantContentBlocks`、`TaskBlock`。
   - QuickChat / 未传入状态的调用方保持默认 `idle`，不影响现有渲染。

3. 补充聚焦测试：
   - `TaskBlock` 在 `executionState="idle"` 且存在 `in_progress` task 时不显示 spinner，摘要显示等待继续。
   - `executionState="running"` 时仍显示 running spinner。
   - `executionState="failed"` 时显示失败文案与 AlertCircle 图标，且不显示 spinner。

验收：

- 停止后消息区和输入区都显示等待继续/暂停态。
- 运行中两处都显示正在执行/旋转态。
- 失败后消息区和输入区都显示失败态，不退回等待继续。
- 不改写 task 自身状态，只改变展示解释。

### Phase 8：TaskProgressPanel 展示调整

1. 扩展 `TaskProgressSnapshot` 或 `TaskProgressPanelProps`，传入当前 `executionState` / `turnStatus`。

2. 修改摘要文案：
   - running：`任务 · 正在执行 {{completed}}/{{total}}`
   - cancelling：`任务 · 停止中 {{completed}}/{{total}}`
   - interrupted：`任务 · 已停止 {{completed}}/{{total}}`
   - idle 且有 unfinished：`任务 · 等待继续 {{completed}}/{{total}}`
   - completed：沿用已完成文案

3. 修改 in_progress 行图标：
   - running：保留 Loader2 或现有 active icon。
   - cancelling：StopCircle / Loader2 muted。
   - interrupted / idle：CirclePause 或 ListTodo 类静态图标。
   - failed：AlertCircle。

4. 所有新增文案补齐 12 语言 i18n key。没有把握的语言可先用英文/中文一致结构，但必须保证 key 齐全。

验收：

- stop 后 in_progress task 不再旋转。
- 刷新后仍显示已停止/等待继续。
- task 本身状态不被前端改写。

### Phase 9：PlanPanel 与继续执行入口

1. PlanPanel 接收 execution state 或通过 hook 读取 last terminal turn status。

2. `plan.executing + turn.interrupted` 展示：
   - 标题或状态 chip：执行已停止。
   - Continue 可用。
   - Request Changes / Exit / Rollback 保持可用。

3. Continue 触发新 turn，带明确 plan trigger：
   - 使用现有 `is_plan_trigger` 机制。
   - prompt 文案包含“上一轮被用户停止，继续当前 task list”语义。

4. 不修改 `maybe_complete_plan` 的核心条件，只确保 interrupted 不会触发 Completed。

验收：

- stop 后 Plan 状态仍是 Executing。
- 点击 Continue 创建新 turn。
- scoped tasks 全 completed 后才进入 Completed。

### Phase 10：消息 interrupted 标记

1. 对手动 stop 后保留的 partial assistant 内容显示非错误的 interrupted 标记。

2. 短期实现：
   - 如果最后消息 `streamStatus = orphaned`，沿用现有 InterruptedMark。
   - 如果 turn status = interrupted 但 message stream_status = completed，需要通过 turn/message 关联或 attachments metadata 给消息标记 interrupted。

3. 失败和中断视觉区分：
   - interrupted：中性提示。
   - failed：错误提示。

验收：

- 手动 stop 不显示红色错误。
- partial 文本保留并可识别为中断输出。

## 测试计划

开发过程中按项目规则只做单点验证；需要跑全套时先征得用户同意。

### Rust 单元/集成测试

- `ChatTurnStatus` 状态机：terminal 状态不可覆盖。
- DB migration：旧库可升级，新表可 CRUD。
- stale recovery：`running/cancelling -> interrupted(crash_recovery)`。
- stale stop：turn id 不匹配不取消当前 active turn。
- Plan auto complete：interrupted turn 不触发 Completed，tasks 全 completed 才触发。

建议命令：

```bash
cargo check -p ha-core
cargo test -p ha-core chat_turn
```

第二条属于测试命令，按项目约定执行前需要用户确认，除非进入长任务收尾阶段。

### 前端单元测试

- `taskProgress.ts`：不同 turn status 下摘要文案和 display state。
- `TaskProgressPanel.test.tsx`：running/cancelling/interrupted/idle 图标与文案。
- `useChatStreamReattach`：`active=false + interrupted` 不启动 loading。
- `useChatStream`：stream_end interrupted 正确清 loading，stop 带 turn id。

建议命令：

```bash
pnpm typecheck
pnpm test -- TaskProgressPanel taskProgress
```

`pnpm test` 执行前按项目约定需要确认，除非进入长任务收尾阶段。

### 手工验收场景

- S1：执行中立即停止，task 停在 0/4，UI 显示已停止。
- S2：停止后继续，新 turn running，不受旧 stop 影响。
- S3：停止后刷新，恢复 interrupted，不显示 spinner。
- S4：运行中强退重启，stale turn 标记为 crash recovery interrupted。
- S5：A/B 多会话并发，停止 A 不影响 B。
- S6：stale stop 延迟到达，不取消新 turn。

## 风险与应对

- 风险：turn terminal 状态可能被 stop、engine end、error path 并发写入。
  - 应对：DB 层提供 `finish_turn_once`，terminal 状态只写一次，所有出口共用。

- 风险：Tauri 与 HTTP cancel 实现分叉。
  - 应对：把 cancel registry 尽量下沉到 `ha-core`，Tauri/HTTP 只做适配薄壳。

- 风险：pending approval / ask_user 没有 turn id，stop 后残留旧请求。
  - 应对：第一阶段可按 session 清当前 active turn 的 pending 请求；随后补 turn_id 精准关联。

- 风险：前端 loading 来源过多，出现新旧状态打架。
  - 应对：先建立 `executionState` view model，再逐步替换 TaskProgressPanel、ChatInput、PlanPanel 的运行态判断。

- 风险：历史 session 没有 turn 记录，UI 误判。
  - 应对：新增字段全部可选；无 turn 记录时按 `idle/completed history` 处理，不显示 interrupted。

- 风险：IM / cron / subagent 共享 chat engine 时被 desktop turn 逻辑误伤。
  - 应对：turn source 明确区分，P0 只覆盖 Desktop/HTTP 主聊天；共享底层 helper 时保持 source 判断。

## 复杂度评估

High。该需求跨越 Rust core DB、Tauri/HTTP transport、chat engine 生命周期、runtime cancel、stream recovery、Plan Mode 和前端多处状态渲染。建议按上述 Phase 分批提交，先完成 P0 正确性闭环，再补 PlanPanel 和 pending interaction 细节。

## 实施顺序建议

1. 先落 DB model、状态机 helper、恢复 sweep，保证持久化基础正确。
2. 接入 chat engine turn start/finish 和 stream_end payload，保证后端终态可靠。
3. 改 stop_chat 为 turn 级取消，解决多会话和 stale stop。
4. 扩展前端 `SessionStreamState` 和 execution view model。
5. 改 TaskProgressPanel / ChatInput，修复用户截图中的不合理状态。
6. 补 PlanPanel continue、pending approval / ask_user 收敛和 interrupted message 标记。
7. 补测试和手工验收。

## 待确认事项

请确认是否按本计划进入实现。收到明确 `CONFIRM` 后，再创建 `docs/requirements/任务中断状态恢复与展示优化/task.md` 并开始改代码。

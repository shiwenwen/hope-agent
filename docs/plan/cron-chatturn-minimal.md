# 定时任务普通对话最小改造

## 决策

定时任务只负责在指定时间发起一次任务。`AgentTurn` occurrence 一旦被
claim，后续执行必须复用普通聊天已有的 Session、ChatTurn、stream、Stop
和队列能力，不建立第二套 Cron 会话状态机。

本改造只覆盖 standalone scheduled task：每次运行创建一个普通对话。
“在已有对话内排程”不在本阶段范围内。

## 保留

- Task CRUD、schedule/timezone 校验、Primary claim、并发槽；
- run log、timeout、失败计数、通知与外部投递；
- `ChatSource::Cron`，只表达首个 Turn 是无人值守来源；
- Project、Agent、permission、sandbox 等既有任务配置。

## 直接复用

- `SessionDB::create_session_with_project` 创建普通 Session；
- SessionDB 的“user message + ChatTurn”原子写入；
- `active_turn` 的 exact turn admission、cancel flag 与 Stop；
- `run_chat_engine` 的持久 stream、终态和重连；
- 普通聊天的输入框、pending queue、显式安全边界插入（`force_insert`）、archive、search 和 unread；
- Scheduled run log 中已有的 `session_id` 作为导航关系。

## 明确不做

- 不新增 TurnRequest 或第二套 run ledger；
- 不新增 `CronChatState`、running/continuable/interactive；
- 不新增 Cron 专用 deferred/takeover queue；
- 不新增“晋升为普通对话”步骤；
- 不新增 Cron 专用 composer、Stop 或 active-run API；
- 不在本阶段扩展 Worktree custody 或 current-chat scheduling；
- 不为尚未发布的分支实现保留 dual-read/dual-write。

## 执行顺序

1. CronDB 原子 claim occurrence，并保持现有 running marker。
2. 创建普通 Session，设置任务标题和 Project/Agent 配置。
3. 生成 `turn_id`，取得 `active_turn` admission，并与 task cancel 共用 exact flag。
4. 打开现有 run log，记录 `session_id`；失败时拒绝执行无审计 turn。
5. 经 `with_persistence_target` 在一个 SessionDB 事务中写入 Cron user message 和 running ChatTurn。
6. 完成既有 permission/sandbox 预检；若模型尚未启动即失败，明确终结 ChatTurn。
7. 将 `turn_id` 传给 `run_chat_engine`；stream 与终态完全走普通 ChatTurn。
8. ChatTurn 结束后沿用现有 Cron accounting、delivery 和 run-log terminal。

Stop 只作用于 exact active ChatTurn；停止本次运行不暂停或删除 Task。
运行中的用户消息默认按普通聊天规则进入 pending queue，当前 Turn 释放后再发送；
后端投影允许时，用户可显式选择在下一个完整工具边界插入当前 Turn。

## 产品行为

- 运行开始后，对话可从 Scheduled 页面打开，并出现在普通侧栏；
- 输入框、排队、停止、归档和后续追问与手动对话一致；
- 首轮运行可在普通会话内实时观看，刷新后沿用既有 stream-state 重连；
- 首个 Turn 保留 Scheduled 来源标识，但后续人工 Turn 不具有 Cron 来源；
- 删除 Task 会按外键物理删除 run log，但不删除已经生成的普通对话；
- Scheduled 页面继续显示 run history，但不拥有另一份聊天正文。

## 验收

- 新 scheduled run 不调用 `mark_session_cron`；
- 每次实际模型执行都有一个 durable ChatTurn；
- 普通 Stop 能取消运行中的 scheduled Turn，且不会误停下一 Turn；
- 运行中发送的消息不并发启动第二个 Turn；
- reload 后能通过现有 stream-state API观察 running/terminal；
- 删除 Task 后生成的对话仍可从普通侧栏访问；
- 相对 `main` 的完整实现（后端、UI、测试、文档）控制在 5–8k 行内；
- 若需要新增 Cron 专用会话/队列状态，停止实现并重新评审抽象边界。

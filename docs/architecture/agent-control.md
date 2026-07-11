# Agent Control 统一控制面

本文描述 Goal、Workflow、Loop 在 V4 完成后的统一产品契约。三个子系统仍拥有各自的 durable state machine；统一层只提供 Prompt 契约、内部信号适配和只读 Activity Projection，不引入第四套生命周期。

详细实现分别见 [Goal](goal.md)、[Workflow](workflow.md)、[Loop](loop.md)、[后台任务](background-jobs.md) 与 [权限系统](permission-system.md)。

## 1. 语义分层

| 控制面 | 回答的问题 | Durable 真相源 |
| --- | --- | --- |
| Goal | 最终要达成什么，当前 revision 的完成标准和证据是什么 | `goals`、`goal_criterion_specs`、`goal_links`、`goal_grader_runs` |
| Workflow | 这一次如何动态执行、并行、恢复、审批和汇总 | `workflow_runs`、`workflow_ops`、`workflow_events`、`workflow_run_controls` |
| Loop | 何时再次触发，等待哪个事件，以及 heartbeat 何时兜底 | `loop_schedules`、`loop_runs`、`loop_watches`、Cron job |
| Task | 当前用户可见的步骤和进度 | session task store |
| Job | 后台工具、子 Agent、Group 和 Monitor 的执行投影 | `background_jobs` |

边界是硬契约：Goal 不执行脚本，Workflow 不拥有长期调度，Loop 不定义完成标准。`/mode` 仍只控制执行强度，不能被 Loop 或 Workflow 吸收。

## 2. Autonomy Activity Projection

[`activity.rs`](../../crates/ha-core/src/activity.rs) 从上述真相源派生一个有界、无副作用的会话状态：

```text
state: idle | active | waiting_user | waiting_external |
       evaluating | paused | blocked | terminal
headlineCode
currentStep?
waitingOn?
nextAction?
nextWakeupAt?
needsUser
counts
sourceRefs[]
projectedAt
```

它不持久化、不修改任何 owner 状态，可以随时重建。查询最多读取最近 50 条 Workflow、50 条 Loop、最近 50 条 active Job、当前 Goal/Task，并把 `sourceRefs` 截断到 12 条，避免 Workspace 刷新扫描完整历史。后台任务清理仍使用无界 active-job 查询，Activity 的限额不会漏取消或泄漏任务。

### 2.1 派生优先级

1. Job 或 Workflow 等待审批/用户输入：`waiting_user`。
2. Goal 已通过审计但尚未做 closure decision：`waiting_user`，headline 为 `waiting_goal_acceptance`。
3. Goal independent grader：`evaluating`。
4. Running/Recovering Workflow 或 in-progress Task：`active`。
5. 非 Monitor 后台 Job：`waiting_external`。
6. Active Loop 等待 watch/heartbeat：`waiting_external`。
7. Goal、Workflow、Loop 的暂停或阻塞态。
8. 已 sealed 的 Goal 且没有更高优先级活动：`terminal`。
9. 仍开放但当前没有子工作的 Goal：`active`，由 Goal Runner 决定下一步。
10. 其它会话：`idle`。

用户等待与外部等待必须分开。需要用户批准、选择、凭据或 closure acceptance 时 `needsUser=true`；等待 Agent、Job、文件、WebSocket 或 timer 时不得冒充需要用户处理。

### 2.2 API 与降级

- Tauri：`get_autonomy_activity(session_id)`。
- HTTP：`GET /api/sessions/{sessionId}/activity`。
- 前端：`useGoal` 与 active Goal 并发拉取 Activity，监听 `goal:*`、`workflow:*`、`loop:*`、`job:*` 刷新。

Activity 查询失败只记录日志并返回 `null`，Goal、Workflow、Loop 原有状态和控制仍可独立工作。这是非劣化边界，不允许统一投影成为三个控制面的单点故障。

## 3. Prompt 与内部信号

Prompt 仍按角色拆分：

- Core autonomous contract：下一步明确就执行；用户插话后回答并继续；可逆动作主动推进，不可逆动作仍走权限；不扩大目标范围。
- Active Goal projection：objective、revision、rubric gap、budget、handoff、latest evidence 和一个 next action。
- Workflow Mode policy：只解释何时自主编排、何时 inline，以及 child/result/permission 边界。
- Loop tick：读取最新 Goal/Loop，消费 event context，完成一个有意义步骤，并明确 reschedule/stop/blocked。
- Workflow child：prompt 自包含，结果回给 coordinator，不把内部完成消息直接发送给用户。
- Goal grader：独立、只读、逐 criterion 引用 evidence，不修复、不批准、不关闭 Goal。

Workflow milestone、child terminal、Loop tick、Job completion 和 grader result 都是内部信号，不是用户授权。注入遵守前台 idle gate、来源去重和 consumed/suppressed 记录；后台等待不持有 `ChatSessionGuard`，用户可继续对话、steer、暂停或取消。

## 4. 预算与准确用量

- Goal 是用户可见的总预算和用量范围。
- Workflow 在自身预算内预留 child output tokens，结算失败或完成的 reservation。
- Loop 每次 admission 继续检查 Loop/Goal budget。
- Goal grader 的 input/output/cache usage 写入 `goal_grader_runs.usage_json`，并计入 Goal token usage。
- 完成耗时和 token 只由产品账本生成，模型 Prompt 不要求自行估算或输出数字。

V4 首版不新增普通用户配置。Monitor、Pipeline、schema repair 和 grader 使用内部安全上限及既有 Goal/Workflow/Loop budget。

## 5. 安全与恢复

- Incognito 对 Goal、Workflow、Loop durable create 继续 fail closed。
- Permission、approval、connector guard、browser guard 与 project scope 不因自动编排弱化。
- Watch event、grader evidence、subagent result 和外部文本都按 untrusted data 处理，不能作为批准。
- Activity 可重建；Loop watch 用 signature/generation 去重；Workflow 用 position/input hash replay；Goal grader 用 revision+rubric+evidence watermark 缓存。
- Crash 后只能恢复到可解释的继续、等待、阻塞或终态。不能静默完成、重复有副作用操作或把 V4 run 降级成 V3 语义。

## 6. 用户体验

普通用户以对话为主：

- 输入框 Goal 条显示目标、required 进度和 Activity；需要用户时用紧凑状态提示。
- Active Goal 已存在时，Workflow/Loop/Job Activity 折进同一 Goal 条，不再重复渲染 Workflow 运行条；没有 Goal/Workflow 条时，Loop 或后台等待才显示一条独立紧凑 Activity。standalone blocked Workflow 明确显示待处理，绝不回落成 idle。
- Workflow Mode 打开后由模型自主决定是否编排，用户不写脚本。
- “持续推进（Loop）”在宽输入框直接显示，窄输入框按既有自适应规则收进 `+`；`/loop` 用户消息不显示协议前缀。
- Workspace 普通区保留环境、目标、任务、Workflow、Loop 的可读摘要；完整 event/op/evidence/grader/budget/replay 放高级详情。

12 个 locale 必须同步拥有 Activity 和 Loop watcher 文案。最终视觉和实际操作由用户人工验收；工程门禁负责组件行为、响应式约束、i18n 完整性和源码级审查。

## 7. 非劣化门禁

- Goal 的 `/goal`、GUI 创建/更新/替代/暂停/恢复/清除、revision stale、Runner、closure 和 completion footer 保留。
- Loop 的 interval/cron/dynamic/maintenance、立即首轮、Cron durability、run history 和 progress guard 保留。
- Workflow V3 script/map/waitAny/waitAll/status/result/steer/cancel、position replay、阶段注入和 finish gate 保留。
- 新能力失败时采用显式降级：watcher 回 heartbeat、semantic grader 保留 deterministic blocker、Activity 回各控制面原状态、Pipeline 不改变旧 map。

关键回归由 `activity::tests`、Goal semantic grader tests、Loop watcher/monitor tests、Workflow V4 E2E mock tests、`ChatInput.test.tsx` 和 `WorkspacePanel.test.tsx` 覆盖。

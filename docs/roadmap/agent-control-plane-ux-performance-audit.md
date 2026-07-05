# Agent 控制平面体验与性能审计

> 返回 [路线图索引](README.md)
>
> 日期：2026-07-05
>
> 对应：[目标退出计划](agent-control-plane-exit-plan.md) / [完成状态审计](agent-control-plane-completion-audit.md) / [最终样本包](agent-control-plane-final-sample-packet.md)
>
> 状态：Exit 3 source-level audit v1。本文审计现有 GUI 路径、关键状态和性能/成本风险，不新增功能。

## 1. 当前结论

Exit 3 已完成第一版源码级体验与性能审计，并已补 GUI 相关 Vitest 与 TypeScript 类型检查输出，但 **还不能据此关闭整个长期目标**。

已能证明：

- 核心路径不只依赖 slash command：Goal、Workflow Mode、Workflow run、Loop、Task、Workspace、Soak、Connector E2E 都有 GUI 入口。
- 用户能在输入框附近持续看到 active goal，并能编辑、评估、暂停、恢复、清除。
- Workspace 有“自主推进就绪”总览，可把缺目标、未开启工作流模式、失败 workflow、blocked loop、Soak / E2E / Guard 问题导向下一步。
- 长任务状态不要求读原始 JSON：Workflow trace、审批审计、Loop history、TaskProgressPanel、Background jobs、Operational Gate、Soak Report、真实样本验收都提供用户可见摘要。
- 性能风险已有部分约束：长列表截断、事件 debounced refetch、active job detail polling、Soak / evidence report 截断、复制报告摘要化。

仍未证明：

- 本轮没有启动 GUI 做人工截图 / 视觉重叠检查。
- 本轮没有跑 Playwright / 浏览器 profile。
- `WorkspacePanel.tsx` 已非常大，维护风险仍高；这不是当前目标内继续扩 UI 的理由，而是 Exit 4 review 需要关注的风险。

本轮复核结果：

| 命令 | 结果 | 说明 |
| --- | --- | --- |
| `pnpm test -- src/components/chat/input/ChatInput.test.tsx src/components/chat/tasks/TaskProgressPanel.test.tsx src/components/chat/workspace/WorkspacePanel.test.tsx` | 通过，Vitest 实际执行 57 files / 366 tests | 覆盖输入框目标模式、TaskProgressPanel、Workspace / Workflow / Loop / Domain workbench 相关 GUI 回归；命令输出中有 CSS 解析提示，但测试未失败。 |
| `pnpm typecheck` | 通过 | 前端 TypeScript 类型检查通过，补强 GUI/source-level audit 的静态证据。 |

## 2. 用户路径审计

### 2.1 目标模式

用户路径：

```text
输入框 + 号 / toolbar 目标按钮
  -> 输入目标
  -> 发送
  -> 内部执行 /goal <objective>
  -> 消息气泡显示“目标”标记，不显示 /goal 前缀
  -> 输入框上方持续显示 active goal strip
  -> 用户可编辑 / 评估 / 暂停 / 恢复 / 清除
```

源码证据：

| 证据 | 文件 | 说明 |
| --- | --- | --- |
| `handleGoalModeSubmit` | `src/components/chat/ChatScreen.tsx` | Goal 模式发送时内部构造 `/goal <objective>` 并调用 `execute_slash_command`。 |
| `handleGoalUpdate` | `src/components/chat/ChatScreen.tsx` | 目标修改走 `update_goal`，更新后刷新 active goal snapshot。 |
| `goalComposerMode` send branch | `src/components/chat/input/ChatInput.tsx` | 目标模式下发送内容被当作 objective，而不是普通 chat message。 |
| Active Goal strip | `src/components/chat/input/ChatInput.tsx` | 输入框上方持续显示“进行中的目标”、状态、编辑、评估、暂停/恢复、清除。 |
| Goal toolbar button | `src/components/chat/input/ChatInput.tsx` | toolbar / overflow 提供目标模式入口，无痕时禁用。 |
| Goal message badge | `src/components/chat/message/MessageBubble.tsx` | 渲染目标消息标记，避免把 `/goal` 暴露成普通正文。 |

体验判定：

- 核心目标创建不需要用户手打 `/goal`。
- 更新后模型可感知的前提是 `update_goal` 已刷新 snapshot，并且后端 active goal 注入链保持架构契约。
- 无痕会话明确提示不持久化目标。

剩余风险：

- 本轮未做手动视觉检查，不能证明窄屏下 active goal strip 所有按钮都无重叠。
- 输入框目标模式与 Plan Mode、Knowledge Picker、Workflow Mode 同处 toolbar，窄屏 overflow 体验仍需人工验收。

### 2.2 Workflow Mode 与自主推进就绪

用户路径：

```text
Workspace -> 自主推进就绪
  -> 查看 Goal / 编排 / 执行强度 / 运行健康
  -> 一键创建目标、开启编排、设为守护、绑定模板、新建 Loop
  -> 查看工作流 / Loop / 交付 / 外部动作 / E2E / 稳定性 / 长跑
```

源码证据：

| 证据 | 文件 | 说明 |
| --- | --- | --- |
| `AutonomousReadinessCard` | `src/components/chat/workspace/WorkspacePanel.tsx` | 汇总 Goal、Workflow Mode、Execution Mode、Workflow/Loop 问题、Guard、Operational、Soak。 |
| `autonomousReadinessNextSteps` | `src/components/chat/workspace/WorkspacePanel.tsx` | 生成“先创建目标”“开启工作流模式”“处理失败或阻塞”“查看长跑”等下一步。 |
| `readinessStatusTone` | `src/components/chat/workspace/WorkspacePanel.tsx` | 将 failed / insufficient_data / missing setup 映射成 danger / info / warn。 |
| `summarizes autonomous readiness from goal workflow and loop state` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖 readiness 从 Goal、Workflow、Loop 状态生成摘要。 |
| `offers setup actions from the autonomous readiness card` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖 setup action 能打开目标 / Loop 等入口。 |

体验判定：

- Workflow Mode 不需要用户理解脚本运行时才能开启。
- 模型是否动态编排由后端模式和 prompt 注入决定；GUI 的职责是让用户看见“当前是否允许编排”和“下一步该点哪里”。
- 失败态能从 readiness 卡片跳到对应明细，不停留在抽象状态。

剩余风险：

- Workflow Mode 中文命名“编排 / 工作流模式”需要在最终 review 中再确认是否足够一致。
- Source-level audit 只能证明入口存在，不能证明用户第一次看到时一定理解。

### 2.3 Workflow Run 控制

用户路径：

```text
Workspace -> Workflow
  -> 空态可从目标创建 workflow
  -> 预检 / 创建 / 创建并运行
  -> 审批摘要和审批审计
  -> 查看 trace、op、validation、recovery guidance
  -> pause / resume / approve / cancel
  -> failed / blocked 时可创建修复任务
```

源码证据：

| 证据 | 文件 | 说明 |
| --- | --- | --- |
| `WorkflowRunsSection` | `src/components/chat/workspace/WorkspacePanel.tsx` | Workflow run 列表、创建、详情、主操作入口。 |
| `workflowRunPrimaryActions` | `src/components/chat/workspace/WorkspacePanel.tsx` | 根据 run state 暴露 approve / pause / resume / cancel 等动作。 |
| `WorkflowRunDetail` / trace helpers | `src/components/chat/workspace/WorkspacePanel.tsx` | 渲染 op、事件、validation failure、repair guidance。 |
| `shows an actionable workflow empty state before any workflow run exists` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 空态不是死屏，可引导创建。 |
| `lets the user create and immediately run a workflow script from the workspace` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖 Workspace 内创建并运行 workflow。 |
| `generates a goal-driven workflow draft before preflight` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖从目标生成 workflow 草稿。 |
| `blocks workflow creation when script preflight fails` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖 preflight 失败时禁止创建。 |
| `surfaces approval summary and primary workflow actions` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖审批摘要和主动作。 |
| `shows granted approval history in the workflow overview` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖审批历史可见。 |
| `confirms before cancelling a workflow run` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖取消确认，避免误停长任务。 |
| `polls active workflow runs as a fallback when live events are missed` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖 active run fallback polling。 |
| `renders validation command details and recovery guidance` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 覆盖 validation failure 和 recovery guidance。 |

体验判定：

- 长任务控制路径基本完整。
- cancel 有确认，不是危险单击。
- preflight failure 不会让用户创建明显不可运行的 workflow。

剩余风险：

- 没有手动跑一个真实长 workflow 来确认 trace 在大量 op 时仍易扫读。
- 没有性能 profile 大 trace / 大 diff 的渲染成本。

### 2.4 Loop 控制

用户路径：

```text
Workspace -> Loop
  -> 新建
  -> interval 或 condition
  -> continue session 或创建 workflow run
  -> max runs / max runtime / token budget
  -> 查看运行记录和派生 workflow
  -> pause / resume / stop
```

源码证据：

| 证据 | 文件 | 说明 |
| --- | --- | --- |
| `LoopSchedulesSection` | `src/components/chat/workspace/WorkspacePanel.tsx` | Loop 创建、列表、history、pause/resume/stop。 |
| `canUseWorkflowLoop` | `src/components/chat/workspace/WorkspacePanel.tsx` | 只有 active goal 带领域模板时允许 Loop 创建 Workflow run。 |
| `createLoop` | `src/components/chat/workspace/WorkspacePanel.tsx` | 校验 interval、condition、prompt/goal、max runtime、max runs、token budget。 |
| `opens failed workflow and blocked loop details from readiness actions` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | readiness 能打开 blocked loop 详情。 |
| `links workflow loop rows to their derived workflow run` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | Loop 行能关联派生 workflow run。 |
| `expands loop run history with workflow trace context` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | Loop history 能展开运行上下文。 |

体验判定：

- Loop 已经是通用持续推进机制，不限 coding。
- 用户能选择轻量 continue 或每次触发 materialize 成可观察 workflow。
- Loop 在无痕会话下显示“不保存 loop”，不会误导。

剩余风险：

- 列表只显示前 5 个 Loop，超过后提示可用 `/loop status`。这对性能友好，但“GUI 不靠 slash command”的目标下，Exit 4 review 需要决定是否要一个“查看更多”GUI。

### 2.5 Task Progress

用户路径：

```text
聊天区 / Workspace -> TaskProgressPanel
  -> 看到任务 N/M
  -> 展开当前任务
  -> Workspace 打开时自动收起
  -> idle / cancelling / failed 不显示误导性 spinner
```

源码证据：

| 证据 | 文件 | 说明 |
| --- | --- | --- |
| `TaskProgressPanel` | `src/components/chat/tasks/TaskProgressPanel.tsx` | 用户可见任务进度。 |
| `shouldShowTaskProgressPanel` | `src/components/chat/tasks/taskProgress.ts` | 只有存在未完成任务时展示。 |
| `starts collapsed when every task is completed` | `src/components/chat/tasks/TaskProgressPanel.test.tsx` | 完成后默认收起。 |
| `auto-collapses when every task becomes completed` | `src/components/chat/tasks/TaskProgressPanel.test.tsx` | 状态变完成时自动收起。 |
| `auto-collapses when workspace opens` | `src/components/chat/tasks/TaskProgressPanel.test.tsx` | Workspace 打开时减少重复信息。 |
| `does not spin an in-progress task when execution is no longer running` | `src/components/chat/tasks/TaskProgressPanel.test.tsx` | 避免 idle 状态还显示运行中。 |
| `renders stopping state without a spinner` | `src/components/chat/tasks/TaskProgressPanel.test.tsx` | cancelling 状态不误导。 |
| `renders failed execution with alert icon and no spinner` | `src/components/chat/tasks/TaskProgressPanel.test.tsx` | failed 状态可见且不假装仍在跑。 |

体验判定：

- 用户不需要读模型正文才能知道当前进度。
- failed / cancelling / idle 的状态不会用 spinner 误导。

### 2.6 真实样本验收、长跑审计、Connector E2E

用户路径：

```text
Workspace -> 通用任务工作台
  -> 真实样本验收
  -> 查看验收结论、证据等级、来源分布、控制面组成、审计索引
  -> 复制 Markdown 验收报告
  -> 缺口 / 必需项 / 样本跑道转任务
  -> 查看 Soak Report
  -> 查看 Connector E2E
  -> 记录执行结果和执行后复核
```

源码证据：

| 证据 | 文件 | 说明 |
| --- | --- | --- |
| `DomainAcceptanceCoverageCard` | `src/components/chat/workspace/WorkspacePanel.tsx` | 真实样本验收卡片。 |
| `domainAcceptanceReviewMarkdown` | `src/components/chat/workspace/WorkspacePanel.tsx` | 复制报告含审计索引、复核协议、必需项、矩阵、守门、Soak、下一步。 |
| `DomainSoakReportPanel` | `src/components/chat/workspace/WorkspacePanel.tsx` | 长跑审计指标、事故、timeline、推荐下一步、复制报告。 |
| `DomainConnectorE2EGatePanel` | `src/components/chat/workspace/WorkspacePanel.tsx` | Connector E2E 输入、草稿、批准、执行、复核、回滚指标与记录入口。 |
| `opens operational and soak evidence from readiness actions` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | readiness 能打开 Operational / Soak，并复制真实样本验收报告。 |
| `flags stale real sample freshness in acceptance coverage` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 样本陈旧会被标出。 |
| `includes recent evidence provenance in copied acceptance review packets` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 复制报告包含 evidence provenance。 |
| `keeps the acceptance gate requirement pending until a gate is observed` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 守门未观察时不会误判通过。 |
| `surfaces connector E2E gate evidence from readiness actions` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | readiness 能打开 E2E gate。 |
| `records connector E2E execution and verification evidence` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 用户可记录 execution / verification evidence。 |
| `creates a task from domain soak incidents` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | Soak incident 可转任务。 |
| `creates a task from domain workbench next-step gaps` | `src/components/chat/workspace/WorkspacePanel.test.tsx` | 工作台缺口可转任务。 |

体验判定：

- 用户能从 Workspace 看到“为什么还不能验收”，而不是只能问模型。
- Connector E2E 不允许只填复核跳过执行：UI 根据 execution readiness 控制复核入口。
- 验收报告提供 `acc-xxxxxxxx` 快照 ID，便于人工和 Claude Code 对齐同一批材料。

剩余风险：

- 没有真实 connector 账号 E2E 手动样本。
- 没有证明复制报告在大型 evidence 集合中仍然足够短且易读。

## 3. 关键状态审计

| 状态 | 当前表现 | 证据 | 判定 |
| --- | --- | --- | --- |
| 无 active goal | 输入框可进入目标模式；Workspace readiness 提示“先创建目标”；Workspace Goal 区可创建。 | `ChatInput.tsx`、`AutonomousReadinessCard`、`WorkspacePanel.test.tsx` goal tests | 可用 |
| active goal | 输入框上方持续显示 active goal strip，支持编辑、评估、暂停/恢复、清除。 | `ChatInput.tsx`、`ChatScreen.tsx` | 可用，需手动视觉验收 |
| 无痕会话 | Goal 模式提示不持久化目标；readiness muted；Loop 显示不保存。 | `ChatInput.tsx`、`WorkspacePanel.tsx` | 可用 |
| 无工作目录 | Goal-driven workflow draft 停止，不直接运行；提示设置目录后再运行。 | `keeps goal-driven workflow drafts stopped when no working directory is set` | 可用 |
| Workflow preflight failed | 创建按钮禁用，不创建 run。 | `blocks workflow creation when script preflight fails` | 可用 |
| Awaiting approval | Workflow overview 展示审批摘要和审批审计；全局 ApprovalDialog 处理工具审批。 | `surfaces approval summary and primary workflow actions`、`ApprovalDialog.tsx` | 可用 |
| Workflow failed / blocked | readiness 可跳转问题 run；trace 保留失败步骤；可生成 repair task。 | `opens failed workflow and blocked loop details from readiness actions`、`renders validation command details and recovery guidance` | 可用 |
| Loop blocked | readiness 可打开 Loop；Loop 行显示 blocked reason；可 resume / stop。 | `LoopSchedulesSection`、相关 Workspace tests | 可用 |
| Soak failed / insufficient_data | Soak Report 显示 status、incident、timeline、recommended next steps，可转任务。 | `DomainSoakReportPanel`、Soak tests | 可用 |
| Connector E2E insufficient | E2E panel 显示输入、草稿、批准、执行、复核、回滚指标，可记录执行和复核。 | `DomainConnectorE2EGatePanel`、E2E tests | 可用 |
| 任务完成 / idle / stopping / failed | TaskProgressPanel 自动收起或显示非 spinner 状态。 | `TaskProgressPanel.test.tsx` | 可用 |

## 4. 性能与成本风险审计

### 4.1 已有约束

| 风险 | 已有约束 | 证据 |
| --- | --- | --- |
| Loop 列表过长 | Workspace 只渲染前 5 个 Loop，并显示剩余数量。 | `LoopSchedulesSection` 中 `schedules.slice(0, 5)`。 |
| Worktree 列表过长 | Managed worktree 列表只展示前 4 个。 | `ManagedWorktreesList` 中 `worktrees.slice(0, 4)`。 |
| Evidence / 报告过长 | acceptance snapshot 只取前 8 个 evidence id；Soak timeline / incidents / recommended steps 有截断。 | `domainAcceptanceSnapshotId`、`DomainSoakReportPanel`。 |
| 后台任务刷新风暴 | `useBackgroundJobs` 使用 debounced refetch；状态按 session 过滤，旧 session 响应不会污染新 session。 | `src/components/chat/background-jobs/useBackgroundJobs.ts`。 |
| 活跃后台任务详情 | 只有 active listed jobs 进入 1s detail polling；终态 job 不继续轮询。 | `SessionBackgroundJobsList.tsx`。 |
| Workflow live event 丢失 | active workflow fallback polling 有测试覆盖。 | `polls active workflow runs as a fallback when live events are missed`。 |
| Task 进度误导 | idle / cancelling / failed 不显示 spinner。 | `TaskProgressPanel.test.tsx`。 |
| 长任务输出预算 | Soak Report 展示 output-token budget usage / exhausted。 | `DomainSoakReportPanel`、`domain_soak_report_tracks_approval_wait_and_recovery_events`。 |

### 4.2 仍需 Exit 4 review 关注

| 风险 | 为什么需要关注 | 当前处理建议 |
| --- | --- | --- |
| `WorkspacePanel.tsx` 过大 | 文件承载 Goal、Workflow、Loop、Domain Workbench、Soak、E2E、Acceptance 等大量逻辑，长期维护风险高。 | 当前目标不继续重构；Exit 4 review 标为 maintainability risk，后续单独拆分。 |
| 无本轮浏览器 profile | source-level audit 不能证明大 trace / 大 evidence / 大 workflow 列表的实际渲染耗时。 | Exit 4 final packet 里记录未跑；如用户要求产品级性能证明，再补 Playwright/profile。 |
| Loop 超 5 个后提示 `/loop status` | 性能友好，但不是完整 GUI-only 路径。 | 不阻塞当前目标；后续增强池可做“查看更多 Loop”。 |
| 真实 connector E2E 未跑 | deterministic substitute 无法证明外部系统读回性能和失败态。 | Exit 2 已标注边界；真实账号样本作为可选补强。 |
| 多个 Workspace 子区块同时刷新 | 源码有 loading / refetch guard，但未做端到端刷新风暴压测。 | Exit 4 review 关注是否需要统一 refresh scheduler。 |

## 5. Exit 3 判定

Exit 3 的 source-level audit 已满足：

```text
GUI 用户路径审计 exists
AND 空态 / 加载 / 失败 / 无痕 / 权限 / 样本不足状态清单 exists
AND 性能与成本风险审计 exists
AND 每项都有源码或测试证据
AND 未证明项明确列出
```

Exit 3 仍未满足的更强证明：

```text
fresh GUI manual smoke output
OR Playwright screenshot / interaction evidence
OR browser performance profile
```

默认建议：

- 不继续扩大 GUI 功能面。
- Claude Code 已接受 source-level audit 作为 v1 substitute；若用户也接受，下一步即可关闭当前 goal。
- 若用户要求产品级视觉证明，先补一轮手动 GUI smoke / 截图 / profile，再回到最终复核。

## 6. 下一步

当前长期目标剩余收口：

1. 可选补强 Exit 2：真实 / 跨窗口 Soak、真实 connector E2E。
2. 可选补强 Exit 3：GUI manual smoke / screenshot / profile。
3. 必做 Exit 4：用户最终关闭取舍；Claude Code 已复核通过，最终 review packet 和 architecture / roadmap 一致性审计已完成 v1。

在这些完成前，不能把 thread goal 标记为 complete。

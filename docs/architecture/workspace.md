# Workspace Control Panel

> 返回 [文档索引](../README.md)

Workspace（中文名「工作台」）是主聊天右侧的会话控制面总览。它聚合 Goal、Workflow、Loop、Task、运行环境、后台任务、文件/来源、知识空间和高级诊断模块，但不取代主对话，也不成为第二套执行引擎。

核心原则：

- 用户主心智仍然是“和模型对话”；工作台负责可见状态、必要控制和异常入口。
- Goal / Workflow / Loop 是三个独立控制面，工作台只做聚合展示和 owner-plane 控制。
- 专家级诊断能力必须保留，但默认不打扰普通任务。
- 大量 Task / Evidence / Guard 不应把主面板刷成一片红；只有需要用户处理的阻塞、审批、失败才突出。

## 1. 入口与边界

| 层 | 文件 | 职责 |
| --- | --- | --- |
| 右侧面板壳 | `src/components/chat/ChatScreen.tsx` | 管理 exclusive right panel，可打开/关闭 `workspace` 与 `pull-request`，并与 diff/files/browser/canvas 等右侧面板互斥切换。 |
| 工作台主组件 | `src/components/chat/workspace/WorkspacePanel.tsx` | 组合各 section，管理 section 间跳转、共享 hooks、增量渲染和 advanced diagnostics 排序。 |
| 任务进度 | `src/components/chat/TaskProgressPanel.tsx`、`src/components/chat/workspace/taskExecutionState.ts` | 展示 session task snapshot；Task 是进度叶子，不是 Goal/Workflow/Loop 本体。 |
| 输入框联动 | `src/components/chat/input/ChatInput.tsx` | Goal/Workflow/Plan 等输入模式与工作台状态联动；不提前创建空会话。 |
| Git 控制卡 | `src/components/chat/workspace/GitControlCard.tsx`、`PullRequestPanel.tsx` | Session 仓库摘要、分支、提交/推送、Handoff 入口，以及独立 PR 详情面板。 |
| Git Diff | `src/components/chat/diff-panel/DiffPanel.tsx` | staged/unstaged/all 审阅及 all/file/hunk 级 mutation；完整契约见 [`git-control.md`](git-control.md)。 |
| 数据 hooks | `src/components/chat/workspace/use*.ts` | 读取 Goal、Workflow、Loop、Review、Verification、Domain Quality、Domain Workbench 等 owner-plane state。 |
| 后端事实 | `ha-core` 各控制面模块 | Goal / Workflow / Loop / Review / Verification / Domain Quality / Context Retrieval 等最终状态真相源。 |

Workspace 不直接发起模型回合，不绕过权限引擎，不自行解释 Goal 完成语义，也不从聊天文本反扫重建控制面事实。

## 2. 信息架构

Workspace section 顺序是产品契约，按“低噪、常用、可理解”到“专家、诊断、质量守门”排列：

1. `EnvironmentSection`
2. `GoalWorkspaceSection`
3. `SessionSection`
4. `TaskProgressPanel` / `Progress`
5. `WorkflowRunsSection`
6. `LoopSchedulesSection`
7. `BackgroundJobsSection`
8. `Output`
9. `Sources`
10. `KnowledgeSection`
11. `Advanced Diagnostics` 分隔
12. `ContextRetrievalSection`
13. `DomainTaskWorkbenchSection`
14. `LspDiagnosticsSection`
15. `ReviewSection`
16. `VerificationSection`
17. `DomainQualitySection`
18. `CodingTrendSection`

### 主信息层

主信息层回答普通用户最常问的问题：

- 当前运行在哪里？有没有工作目录、项目、权限、分支和变更？
- 当前目标是什么？完成标准和状态是什么？
- 本会话用了什么模型、Agent、上下文和系统提示？
- 当前可见任务进度是什么？
- Workflow / Loop 是否开启或有运行记录？
- 后台任务、输出文件、引用来源和知识空间是否有内容？

这层允许常驻展示和轻量控制，但不应堆满专家告警。

### Environment / Git 主操作区

`EnvironmentSection` 在 Session 位于 Git 仓库时把高频仓库操作放在紧凑 Git 卡中，信息顺序固定为：

1. 变更数量与 `+added -removed`；点击打开现有 DiffPanel。
2. 当前运行位置（Local / Managed Worktree）和安全 Handoff 菜单。
3. 当前分支；detached 时显示“创建分支”。
4. 根据 dirty/ahead 状态显示“提交”或“推送 N 个提交”。
5. 创建 Pull Request；已有 PR 时打开独立右侧 PR 面板。
6. 当前 PR checks 汇总与逐项详情。
7. requested reviewers、顶层 Review 结论，以及未解决、未过期的行内评论。
8. 合并冲突状态与修复入口。
9. 显式确认后的自动合并入口。

版本、模型、权限、项目来源等低频环境信息继续放在详细信息区，不与 Git 主操作竞争。分支、变更、同步、最后提交、运行位置以及 Managed Worktree 的创建/恢复/归档等生命周期入口只允许出现在 Git 卡中，详细信息区不得重复展示第二套 Git/Worktree 状态或操作。运行位置菜单负责 Local/Worktree 安全 Handoff，紧邻的托管工作树区域负责生命周期管理，二者共享同一个 Git 卡边界。非 Git 工作目录不渲染伪造的分支或 Worktree 操作，也不隐式执行 `git init`。

PR 详情、Checks 与 Review 评论属于当前 Session/HEAD/branch 的网络状态：只在存在 GitHub remote、附着本地分支且本机 `gh` 可用时读取；每 30 秒有界刷新，同键手动刷新与轮询共享同一个带错误收口的请求，切换会话或分支后丢弃旧结果。Checks 与行内评论两个通道独立展示错误，检查接口失败不能遮蔽已经读到的评论，反之亦然；完整刷新失败时旧数据必须标记为可能过期并暂停修复/自动合并。独立 PR 面板展示标题、描述、head/base、增删行、reviewers、每位审阅者的最新顶层 review、merge state 和自动合并状态；它注册为 `pull-request` exclusive panel，复用标题栏切换、共享宽度、响应式折叠和 overlay，并在 Session 切换时关闭。已有 PR 的查看能力不依赖“能否创建 PR”的 capability。

“修复”不是直接执行按钮。PR 标题、描述、分支、检查描述、评审与评论等外部字段都必须留在不可信数据信封内；修复入口只把经过长度限制和转义的任务填入当前 composer。用户确认发送后才进入正常聊天、权限与工具流程。按钮不得自动 commit、push、回复、resolve Review 评论或合并 PR。

“启用自动合并”是独立远端写操作：存在冲突时不展示；用户必须在二次确认弹窗选择 merge/squash/rebase，并明确确认“保护条件已满足时可能立即合并”。完成后刷新当前 PR 详情。它不由修复任务、轮询或详情加载隐式触发。

### Advanced Diagnostics

高级诊断层收纳更专业的能力：

- 推荐上下文与 file search v2。
- 通用任务工作台、Domain Evidence、Artifact / Connector 守门。
- LSP 诊断、Review、Verification、Domain Quality、Coding Trend。

这些能力很重要，但使用频率和解释成本更高。默认放在分隔标题之后，并遵循“空状态安静、异常才突出”的展开规则。

## 3. Goal / Workflow / Loop / Task 语义

Workspace 必须保持四个概念清晰：

| 概念 | 用户语义 | Workspace 展示 |
| --- | --- | --- |
| Goal | 最终要达成什么、完成标准是什么、证据是否足够。 | 独立 Goal section；显示 active Goal、criteria、revision、audit、closure、evidence、Goal Watchdog amber 确认和编辑/评估/关闭操作。 |
| Workflow | 一次具体、可观察、可恢复、可审批的动态执行 run。 | 独立 Workflow section；显示 Workflow Mode、run list/detail、审批、失败恢复、trace、Workflow Watchdog amber 确认、create/run/pause/resume/cancel。 |
| Loop | 按时间、事件或条件持续触发同一任务策略。 | 独立 Loop section；显示 schedule、trigger、run history、policy、progress guard、Loop Watchdog amber 确认、暂停/恢复/停止/run now。 |
| Task | Goal / Workflow / Loop 执行过程中产生的用户可见进度叶子。 | 只在 Progress 聚合展示数量、完成状态和当前进度；大量 task 不应改变顶层控制面语义。 |

Goal / Workflow 执行过程中可以创建和完成很多 Task。Task 的增长不应让 Workspace 自动展开所有专家区，也不应把 Goal 或 Workflow 误判为失败；只有 Task failure 被对应控制面写成 blocking evidence、failed run 或 needs-user 状态时，才进入异常展示。

Workflow 顶层状态采用 durable snapshot 派生，不直接照搬 `workflow_runs.state`：

1. `agentUsage.runningAgents > 0`：显示“等待子 Agent completed/total”，即使脚本登记阶段已经结束也不能显示整体完成。
2. `agentUsage.pendingResults > 0`：显示“有阶段结果 terminal/total”，引导模型或用户消费结果。
3. 无运行 child、无待消费结果：才回退到 run state 的编排中/等待审批/阻塞/完成等文案。
4. Agent 明细状态必须走 i18n 映射；内部 `Workflow run completed. Use the output...` 等模型协议不得作为用户详情 fallback。

## 4. 展开与告警策略

默认策略：

- 空 section 默认折叠或只显示轻量 empty hint。
- active Goal / active Workflow / active Loop 可以自动展开对应主 section。
- Advanced Diagnostics section 只有在 danger / error / focus request / 用户显式展开时自动打开。
- Domain Task Workbench 不因 Workflow Mode 开启而自动变红；它只反映真实 artifact / connector / quality guard 状态。
- Goal / Workflow / Loop Watchdog 只表示“需要确认或恢复入口可见”，默认使用 amber，不自动等同失败；只有对应控制面明确 failed/blocked/danger 时才升级红色。
- Incognito 下 durable 控制面 section 必须 fail closed 或只显示不可用说明，不落持久化数据。
- Dashboard “目标与执行”的 attention 项可通过 `ChatFocusTarget.controlTarget` 深链到工作台：Goal 滚到 Goal section；Workflow 滚到 Workflow section 并展开目标 run；Loop 滚到 Loop section 并打开目标 schedule；Task 类回到 Progress。Plan review 不走 Workspace，直接打开既有 Plan 面板。深链只负责导航，不改变任何控制面状态。

颜色语义：

- `danger` / 红色：必须用户处理、阻塞交付或安全风险。
- `warning` / 橙色：证据不足、建议补充或可选质量风险。
- `success` / 绿色：完成、通过或已记录。
- neutral：空状态、普通统计、只读信息。

红色不能用于“还没有开始”“没有数据”这类普通空状态。

## 5. 输入框联动

输入框是 Goal / Workflow / Plan 等模式的主入口之一，Workspace 只是旁路状态面。

### Goal

- `+` 菜单和 toolbar 可进入目标模式。
- 无 active Goal 时，目标模式发送等价于 `/goal <objective>`。
- 有 active Goal 时，可更新、替代、追加 required/optional/follow-up criteria。
- 渲染消息时隐藏 `/goal` 前缀，用 Goal 模式标记表达语义。
- 输入框上方常驻展示 active Goal 摘要和状态，让用户不用打开 Workspace 也能知道目标是否仍在进行。

### Workflow

- Workflow Mode 可以在输入框菜单切换 `off` / `on` / `ultracode`。
- 无 session 草稿态只更新 `draftWorkflowMode`，不提前创建空会话；首条消息发送时由 chat options 带入。
- Toast 只反馈用户结果：`工作流模式已开启：自动` / `工作流模式已关闭`。不暴露“下一条消息生效”“下一轮会感知”等实现细节。
- Workflow Mode 开启只授权模型按需自主编排，不代表马上创建 run，也不要求用户手写脚本。

### Plan

Plan Mode 仍走自身 5 态状态机与输入框 Plan UI；Workspace 只显示当前 plan state 和相关入口，不把 Plan 任务进度混入 Goal evidence。

## 6. 数据与性能

Workspace 聚合很多控制面，必须避免“打开面板就全量重活”：

- `useWorkspaceArtifacts` 只聚合当前 session artifacts，并对文件/来源列表做增量渲染。
- Workflow runs state 可由父组件传入共享实例，避免重复轮询。
- Workflow template 只在创建器打开时加载，不因 active Goal 存在而预加载。
- `useScrollPagedRender` 对 files/sources 做 sentinel 增量渲染，避免大列表撑爆 DOM。
- Background jobs、Review、Verification、Domain Quality 等 hooks 只在 Workspace 打开后由组件挂载读取。
- PR details/checks/reviews/comments 只在 GitHub remote + attached branch 条件满足时轮询；同一 session/HEAD/branch 不允许重叠请求，卸载或 key 变化后忽略旧响应。
- 所有 owner action 仍走 Transport，Tauri / HTTP 双路径由对应控制面 API 保证。

## 7. 多语言与 UI 验收

Workspace 是高密度产品界面，新增文案必须同步所有 locale：

- 新 key 先写 `en.json` 与 `zh.json`，再通过 `node scripts/sync-i18n.mjs --apply` 或手动补齐其它语言。
- 提交前至少跑 `node scripts/sync-i18n.mjs --check`。
- 工作台相关文案要额外扫英文残留，尤其是中文界面中的 `trace`、`Managed worktrees`、`Workflow run` 等专业词。
- 含 `{{...}}` 占位符的 key 要保持各语言占位符集合一致。

UI 验收底线：

- 典型桌面宽度和窄屏宽度不能横向溢出。
- 输入框工具栏不允许因按钮增多而换行或互相覆盖；空间不足时优先收纳进 `+` 菜单。
- hover tooltip / button shadow 不能被父容器裁切。
- 模型选择、Workflow Mode、权限、沙箱和 `+` 菜单的浮层必须在窄屏可见。二级菜单不得固定向右越出视口；`ModelPicker` 在右侧空间不足时把模型/温度二级菜单改为向上展开。
- 工作台 section 内容可内部滚动，但外层右侧面板不能出现不可控横向滚动。
- 默认空状态不能呈现成大面积红色。
- Git 卡在 Local、detached Worktree、attached Worktree、非 Git、dirty、ahead、PR 检查失败、合并冲突和评论为空等状态下都不得横向溢出；PR/checks/reviews/comments 详情必须内部滚动。
- “修复”点击后只填 composer，并给出可撤销的结果提示；不能自动发送。

Dev-only GUI smoke：

- 开发环境支持 `?window=workspace-smoke`，入口在 `src/main.tsx`，实现为 `src/dev/WorkspaceSmokeWindow.tsx`。
- 该 smoke 复用真实 `WorkspacePanel`，用固定 fixture 覆盖 active Goal、running Workflow、dynamic Loop、Task 进度、后台任务、输出/来源、Domain Evidence、运行稳定性、长跑审计、交付守门、外部动作守门和连接器端到端（E2E）。
- 它只作为可重复的人工/浏览器 GUI smoke 入口，用来检查默认状态故事、高级诊断展开、窄/宽响应式布局和 popover/tooltip 裁剪；不替代真实 Tauri 桌面长跑、连接器 E2E、restart/resume 或 V3 strict proof route。
- 开发环境也支持 `?window=chat-input-smoke`，入口在 `src/main.tsx`，实现为 `src/dev/ChatInputSmokeWindow.tsx`。该 smoke 复用真实 `ChatInput`，用固定 fixture 覆盖 active Goal、Task progress、Workflow Mode、模型选择、权限、沙箱、工作目录、上下文用量、目标模式和 `+` 收纳菜单；用于复现输入框窄/宽布局、菜单裁剪和模式状态条，不替代真实 Tauri 桌面验收。

V3 strict proof audit：

- `node scripts/v3-strict-proof-audit.mjs` 是 V3 关闭前的证据包审计入口。它扫描仓库 architecture 文档、外部 V3 Plans、deterministic evidence 截图和严格证据 manifest，输出 Markdown 或 `--json` 结构化报告。
- 退出码 `0` 表示 required strict proof artifacts 都存在且 manifest 校验通过；退出码 `2` 表示仍有 V3 closure blocker。该脚本故意不会把 deterministic substitute 当成 strict proof。
- 严格证据只认外部 Plans 下的 `v3-strict-proof-evidence.json`：每个关闭项必须有 `status: "passed"`、允许的 `evidenceKind`、必需 coverage label、可解析 `performedAt` 和存在于 Plans 目录内的 artifact 路径。文件名匹配只用于 deterministic substitute 和展示上下文，不能满足 strict proof。
- 模板文件是 `v3-strict-proof-evidence.template.json`，用于记录真实验收后如何填写；模板或 pending 条目不会让审计通过。采集辅助入口是 `node scripts/v3-strict-proof-record.mjs --requirement <name>`：它默认只创建 pending 条目和 artifact 骨架，标记 `passed` 必须显式 `--confirm-reviewed`，artifact 必须已存在，且 artifact 内 `Required Coverage` / `Reviewer Decision` checklist 必须全部勾选；最终是否关闭仍由 audit 脚本决定。
- 快速状态入口是 `node scripts/v3-strict-proof-record.mjs --list`，下一项入口是 `--next`，机器可读入口是 `--list --json`，退出码门禁是 `--check-ready`（ready 返回 `0`，仍有 open blocker 返回 `2`）。`summary.remaining == 0` / `--check-ready` 只表示五个 strict proof artifact 已按 record 脚本口径准备完毕，最终关闭仍必须以 audit 脚本退出码 `0` 为准。
- 五个 strict proof requirement 的顺序、coverage、允许证据类型和 reviewer decision 文案以 `scripts/v3-strict-proof-requirements.mjs` 为单一来源；`record` 和 `audit` 都必须引用它，避免“状态列表已 ready 但最终 audit 失败”的定义漂移。
- `--write <path>` 可把最新报告写入外部 Plans，例如 `v3-strict-proof-audit-latest.md`。当前 required strict proof 包括真实 restart/resume matrix、真实 wall-clock soak、真实或沙箱 connector read-back、Tauri desktop manual GUI smoke，以及 Hope Agent 与同类工具的对比评测证据。
- 2026-07-09 V3 关闭证据已归档到外部 Plans 的 V3 closure 目录：5 个 required strict proof 全部 `passed`，最终 audit `14/14 passed`、`blockers=0`。其中 connector read-back 采用 GitHub sandbox branch create/read/delete/reset 路线；Google Drive OAuth scope 失败作为 recovery evidence 保留，不算通过证据。

## 8. 归档与后续

本轮 Workspace UX 过程资料已归档到：

```text
/Users/shiwenwen/Library/Mobile Documents/com~apple~CloudDocs/HopeAI/Hope Agent/Plans/hope-agent-control-plane-plans-2026-07-05/09-workspace-control-panel-ux
```

归档包含用户验收截图、工作台信息架构决策、实现范围和验证记录。仓库内最终事实以本文为准。

后续可继续做：

- 将 `WorkspacePanel.tsx` 按 section 拆分，降低单文件维护成本。
- 为 Workspace smoke harness 增加多语言视觉快照。
- 为 Advanced Diagnostics 增加用户级“简洁/专家”显示偏好，但不得隐藏真实阻塞状态。

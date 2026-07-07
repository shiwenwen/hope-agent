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
| 右侧面板壳 | `src/components/chat/ChatScreen.tsx` | 管理 exclusive right panel，可打开/关闭 `workspace`，与 diff/files/browser/canvas 等右侧面板互斥。 |
| 工作台主组件 | `src/components/chat/workspace/WorkspacePanel.tsx` | 组合各 section，管理 section 间跳转、共享 hooks、增量渲染和 advanced diagnostics 排序。 |
| 任务进度 | `src/components/chat/TaskProgressPanel.tsx`、`src/components/chat/workspace/taskExecutionState.ts` | 展示 session task snapshot；Task 是进度叶子，不是 Goal/Workflow/Loop 本体。 |
| 输入框联动 | `src/components/chat/input/ChatInput.tsx` | Goal/Workflow/Plan 等输入模式与工作台状态联动；不提前创建空会话。 |
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
| Goal | 最终要达成什么、完成标准是什么、证据是否足够。 | 独立 Goal section；显示 active Goal、criteria、revision、audit、closure、evidence 和编辑/评估/关闭操作。 |
| Workflow | 一次具体、可观察、可恢复、可审批的动态执行 run。 | 独立 Workflow section；显示 Workflow Mode、run list/detail、审批、失败恢复、trace、create/run/pause/resume/cancel。 |
| Loop | 按时间、事件或条件持续触发同一任务策略。 | 独立 Loop section；显示 schedule、trigger、run history、policy、progress guard、暂停/恢复/停止/run now。 |
| Task | Goal / Workflow / Loop 执行过程中产生的用户可见进度叶子。 | 只在 Progress 聚合展示数量、完成状态和当前进度；大量 task 不应改变顶层控制面语义。 |

Goal / Workflow 执行过程中可以创建和完成很多 Task。Task 的增长不应让 Workspace 自动展开所有专家区，也不应把 Goal 或 Workflow 误判为失败；只有 Task failure 被对应控制面写成 blocking evidence、failed run 或 needs-user 状态时，才进入异常展示。

## 4. 展开与告警策略

默认策略：

- 空 section 默认折叠或只显示轻量 empty hint。
- active Goal / active Workflow / active Loop 可以自动展开对应主 section。
- Advanced Diagnostics section 只有在 danger / error / focus request / 用户显式展开时自动打开。
- Domain Task Workbench 不因 Workflow Mode 开启而自动变红；它只反映真实 artifact / connector / quality guard 状态。
- Incognito 下 durable 控制面 section 必须 fail closed 或只显示不可用说明，不落持久化数据。

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
- 工作台 section 内容可内部滚动，但外层右侧面板不能出现不可控横向滚动。
- 默认空状态不能呈现成大面积红色。

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

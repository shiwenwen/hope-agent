# Phase 2 产品级完成审计

> 返回 [路线图索引](README.md) · 上层方案 [Phase 2 Workflow Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md) · 收口清单 [Phase 2 完整目标与验收清单](phase2-completion-checklist.md)
>
> 更新时间：2026-07-03
>
> 状态：Phase 2 第一版产品级完成审计；2026-07-03 已按 Workflow Mode 语义补充。稳定实现已沉淀到 [Workflow Mode、Workflow Run 与 Execution Mode](../architecture/workflow.md)。

## 1. 结论

Phase 2 的产品级定义不是"能跑一个命令"，而是：

1. 用户能在 GUI 里发现通用 Workflow Mode，并允许模型自主动态编排。
2. 用户能用目标驱动的方式启动 workflow，而不是被迫先写脚本；coding 只是可选领域模板。
3. 模型生成的动态脚本必须经过 Script Gate、permission preview 和必要审批。
4. 长任务运行中必须可观察、可暂停、可恢复、可取消。
5. 失败后必须能诊断和继续修复，而不是只得到一段终端输出。
6. runtime 必须具备 durable replay、子任务接线、预算和 stop guard，避免长任务黑盒失控。

按以上口径，当前实现已经达到 Phase 2 第一版产品级：命令、GUI、owner API、runtime、测试和文档证据互相闭合。外部真实 provider smoke 仍建议保留为体验抽检，但不是普通完成判定的硬门槛。

## 2. 用户交互审计

| 用户问题 | 产品级要求 | 当前证据 | 判定 |
| --- | --- | --- | --- |
| 我怎么知道有 workflow？ | 输入框和标题栏必须有显性入口，不能只靠 slash command。 | 输入框支持 Workflow Mode 入口与常驻状态；`ChatTitleBar` 支持 `workspaceWorkflowStatus`，`ChatScreen` 顶层订阅 `useWorkflowRuns` 后传入标题栏；测试覆盖 `Workflow` 入口和状态 badge。 | 通过 |
| 长任务在等我吗？ | active / waiting / failed run 要在入口上显性提示。 | 标题栏 badge 汇总 active、attention、running；Workspace 共享同一份 `workflowRunsState`，避免面板内外状态分裂。 | 通过 |
| 没有 run 时能不能直接开始？ | 空态必须有行动按钮、execution mode、工作目录状态。 | `WorkspacePanel.test.tsx` 覆盖无 run 空态、execution mode 切换、工作目录展示和创建入口。 | 通过 |
| 普通用户是否必须先懂脚本？ | 默认从目标生成草稿，脚本编辑放高级区；模型开启 Workflow Mode 后也可自行创建 run。 | Workspace 创建流为 goal-driven draft；`advancedScript` 折叠高级脚本区；测试覆盖目标草稿与高级脚本；`workflow_run` 仅在 Workflow Mode 开启后注入。 | 通过 |
| 创建前能否知道风险？ | GUI 预检必须展示 Script Gate 阻断项和 permission preview。 | `preview_workflow_script` 不落库；Tauri/HTTP `create_workflow_run` 强制复用同一 preflight；Workspace 测试覆盖 preflight 阻断与审批摘要。 | 通过 |
| 运行中能不能操作？ | draft / approve / pause / resume / cancel 都要可操作，cancel 必须确认。 | Workspace 动作走 owner API；`approve` / `resume` 会 kick runtime；cancel 弹窗说明会停止 run 与 workflow-owned children，且终态刷新后禁用 stale cancel。 | 通过 |
| 运行中能不能看懂？ | 需要 overview、当前焦点、trace、validation、agents、预算。 | Workflow Control Center 展示 run 总览、当前焦点、Trace timeline、Validation tab、Agents tab、输出预算；测试覆盖焦点跳转、validation 明细、agents/预算统计。 | 通过 |
| 失败后能不能继续？ | failed/blocked 要给恢复建议、复制修复提示、生成下一版修复草稿。 | Recovery hint 汇总 run 状态、失败 op、validation output、近期事件；可生成 repair draft 并自动 preflight；修复 run 持久记录 `parentRunId` / `origin=repair`。 | 通过 |
| 长 trace 会不会吞掉卡点？ | 失败 / 运行中步骤和关键事件要优先浮出，原始详情可展开和复制。 | `WorkflowTraceTimeline` 提供 op/event 展开详情和复制；测试覆盖 trace op 详情展开；清单要求关键事件置顶。 | 通过 |
| 窄屏会不会被右侧面板挤出屏幕？ | 用户主动打开的内部右侧面板必须 fixed overlay。 | `RightPanelShell` overlay contract 与 Browser / Files / Canvas / Mac Control / Team 等内部面板测试覆盖；dev-only smoke harness 用真实 `WorkspacePanel` 验证窄屏场景。 | 通过 |

## 3. Runtime 与稳定性审计

| 能力 | 产品级要求 | 当前证据 | 判定 |
| --- | --- | --- | --- |
| 动态脚本 | 不是固定结构 DSL；模型可以生成 `workflow.js` 动态编排。 | `workflow-script-runtime.md` 设计；`run_workflow_script` 使用 QuickJS/rquickjs 受控执行 `export default main(workflow)`；host API 覆盖 task / tool / map / subagent / validate / askUser / diff / trace / finish。 | 通过 |
| 确定性与 op 身份 | 模型不手写稳定 id；label 只能展示。 | runtime 位置化 op-key；Script Gate 拒绝旧 `(id,args)` 和 label-as-identity；`task.update` 按 create 返回 handle 定位。 | 通过 |
| durable replay | 重启 / 恢复不能重复已完成副作用。 | `workflow_runs` / `workflow_ops` / `workflow_events`；Completed op replay；Started child_handle attach；non-idempotent 不可判定时 fail-safe。 | 通过 |
| 子 Agent fan-out | workflow 可 spawn 多个 subagent 并 waitAll 汇总。 | `spawnAgent` 预分配 child_handle，走现有 `subagent` 工具与 `subagent_runs`；mock provider fan-out eval 通过。 | 通过 |
| validation | workflow 可执行 targeted validation，并把结果结构化展示。 | `validate` 预分配 async exec job child handle，返回 `{ ok, summary, reason, results }`；Validation tab 展示命令、exit code、output 摘要和统计。 | 通过 |
| pause / resume | pause 后不能继续启动新 op；resume 后不能被旧 owner 卡住。 | DB op guard 拒绝 paused run 新 op；pause 释放 `primary_owner`；resume 可被新 owner claim；`pause_` 测试覆盖。 | 通过 |
| cancel | owner cancel 要停止 run，并 best-effort 取消 workflow-owned children。 | `cancel_workflow_run_with_children` 取消 async tool / validation / subagent child，并写 `run_child_cancel_requested` trace event；测试覆盖。 | 通过 |
| output token budget | 长 fan-out 不能无界消耗模型输出。 | GUI goal-driven 草稿写 `maxOutputTokens`；子 Agent usage 持久化；runtime 在 `waitAll` 后写 `budget_usage`，耗尽后阻断新 LLM op 并 `Blocked`。 | 通过 |
| guarded repair stop guard | 不能无限重复修复。 | validation failure 写 repair event；重复 fingerprint 或无有效 diff 进展转 Blocked；`execution_mode=off` 不误拦。 | 通过 |
| askUser / unattended | headless/autonomous 不能无限等用户。 | `workflow.askUser` 复用 `evaluate_approval_surface`，无人值守按策略 fail-closed/proceed；测试覆盖默认 deny 和配置 proceed。 | 通过 |
| incognito | durable workflow 不能破坏关闭即焚。 | incognito 会话拒绝持久 workflow；GUI 显示 fail-closed 提示。 | 通过 |

## 4. API 与模式审计

| 表面 | 要求 | 当前证据 | 判定 |
| --- | --- | --- | --- |
| Tauri owner API | 桌面 GUI 需要完整 owner 控制面。 | `src-tauri/src/commands/workflow.rs` 提供 list / get / preview / create / run / approve / pause / resume / cancel。 | 通过 |
| HTTP owner API | server / Web GUI 与桌面能力一致。 | `crates/ha-server/src/routes/workflow.rs` 提供同等路由；`src/lib/transport-http.ts` 绑定 REST path。 | 通过 |
| Slash command | 命令仍可作为高级入口，但不能成为唯一入口。 | `/workflow on|off|ultracode|status|runs|trace|approve|pause|resume|cancel` 与 `/mode off|guarded|deep|autonomous` 已接入；GUI 是主交互面，slash mode change 会同步输入框/Workspace。 | 通过 |
| Event / refresh | UI 不能只靠单次请求。 | `useWorkflowRuns` 支持 `workflow:*` 事件刷新、visibility refresh、active run 低频 polling；外层状态复用避免双订阅。 | 通过 |
| Dev-only smoke | GUI smoke 不能进入生产功能入口。 | `src/main.tsx` 只在 `window=workflow-smoke && import.meta.env.DEV` 动态导入；`pnpm build` 后 `dist` 无 `Workflow GUI Smoke` / `workflow-smoke` 匹配。 | 通过 |

## 5. 自动化验证账本

本轮产品级收口已执行并通过：

- `cargo check -p ha-core --tests`
- `cargo check -p ha-server --tests`
- `cargo check -p hope-agent`
- `cargo test -p ha-core script_gate -- --nocapture`
- `cargo test -p ha-core runtime_ -- --nocapture`
- `cargo test -p ha-core phase2_eval -- --nocapture`
- `cargo test -p ha-core pause_ -- --nocapture`
- `cargo test -p ha-core owner_cancel_cancels_workflow_child_async_jobs -- --nocapture`
- `cargo test -p ha-core workflow_create_records_parent_repair_derivation -- --nocapture`
- `pnpm exec vitest run src/components/chat/ChatTitleBar.test.tsx src/components/chat/workspace/WorkspacePanel.test.tsx`
- `pnpm exec vitest run src/components/chat/ChatTitleBar.test.tsx src/components/chat/internalRightPanelOverlay.test.tsx src/components/chat/right-panel/RightPanelShell.test.tsx src/components/ui/tooltip.test.tsx src/components/chat/workspace/WorkspacePanel.test.tsx src/components/chat/input/ServerDirectoryBrowser.test.tsx src/lib/transport-http.test.ts`
- `pnpm typecheck`
- `node scripts/sync-i18n.mjs --check`
- `pnpm build`
- `rg "Workflow GUI Smoke|workflow-smoke" dist`
- `git diff --check`
- Dev-only browser smoke：临时 Vite + `http://localhost:1420/?window=workflow-smoke`，使用真实 `WorkspacePanel` 和 fake transport，验证 approval / running / failed 场景；390px 移动宽度与 1280px 桌面宽度均无页面级水平溢出，approval 场景可见 Execution Mode / 待批准焦点 / 授权清单 / 批准 / 取消，running 场景可见正在执行焦点 / 暂停 / 取消，failed 场景可见验证失败 / 生成修复草稿 / 复制修复提示。

验证边界：

- 以上自动化不依赖外部 LLM key、外部网络、模型当天可用性或费用。
- 外部真实 provider fan-out smoke 仍建议作为体验抽检保存 run snapshot / trace / final，但不作为普通 CI 或本轮完成硬门槛。
- GUI 视觉 smoke 的稳定入口是 `http://localhost:1420/?window=workflow-smoke`，使用真实 `WorkspacePanel` 与 fake transport 场景数据；它是开发验证入口，不是生产入口。

## 6. 剩余风险与后续

| 项 | 风险 | 处理 |
| --- | --- | --- |
| 外部真实模型差异 | 不同 provider 的工具调用、token usage、子 Agent 输出质量可能不同。 | 保留人工 smoke，不进入无 key 自动门禁。 |
| 并发中途硬预算 | 当前 output token budget 在 `waitAll` 汇总后阻断后续 LLM op，不实时取消已经并发启动的子 Agent。 | Phase 3 可做 running child budget watchdog；Phase 2 第一版不把它作为阻塞。 |
| 截图级视觉证据 | 自动测试和浏览器 DOM/layout smoke 已覆盖行为、场景切换和水平溢出；仍未保存截图 artifact。 | 截图 artifact 可作为后续体验复核材料；生产构建已确认不包含 smoke 入口。 |
| architecture 沉淀 | workflow 已从路线图方案进入稳定实现文档，后续迭代若只改 roadmap 容易让架构真相源漂移。 | 稳定契约维护在 [`docs/architecture/workflow.md`](../architecture/workflow.md)；roadmap 只保留阶段历史、验收证据和后续增强计划。 |
| 顶层 Goal 缺失 | Workflow 已能执行和恢复，但还没有一等对象保存最终目标、完成标准、证据和 final audit。 | 下一阶段转入 [Agent 控制平面路线图](agent-control-plane-roadmap.md)：先做 `/goal` MVP，再做 Goal-driven Workflow。 |

## 7. 完成判定

Phase 2 第一版产品级可以声明完成：

- 用户面不是命令旁路，而是完整 Workflow Control Center。
- 动态 workflow 不是结构化模板，而是受控 JS runtime + durable host op。
- 长任务稳定性覆盖 pause / resume / cancel / replay / child attach / budget / repair stop guard。
- GUI、Tauri、HTTP、runtime 和测试证据闭合。
- 仍保留外部真实 provider smoke 和截图验收作为体验增强，不把它们包装成已经自动证明的内容。

Phase 2 完成后不直接进入 worktree / LSP / review engine。下一步是 `/goal`，让已完成的 workflow 能力挂到长期任务目标、完成标准和证据链上。

# Phase 2 完整目标与验收清单

> 返回 [Phase 2 Coding Mode 与 Script-first Dynamic Workflow 方案](phase2-coding-mode-dynamic-workflow.md)
> · [Phase 2 产品级完成审计](phase2-product-audit.md)
>
> 更新时间：2026-07-01
>
> 状态：实现收口清单。6 个自动 eval 场景已通过；真实模型回复型 fan-out 已用本地 mock provider 覆盖，外部真实 provider smoke 只作为体验抽检。

## 完整目标

Phase 2 不是只加一个 `/mode` 开关，而是要把 coding 长任务变成可恢复、可观察、可审批、可验证的闭环：

1. Hope-native coding skills 可作为核心策略，不依赖第三方移植 skills。
2. Script-first workflow runtime 可执行模型生成的动态脚本，并通过 Script Gate / permission preview / approval 审核。
3. workflow op 身份由 runtime 位置化 op-key 决定，模型不手写稳定 id。
4. workflow 可编排 `task`、`fileSearch/read/grep/tool`、`spawnAgent/waitAll`、`validate`、`diff`、`askUser`、`trace`、`finish`。
5. 长任务可恢复、可取消、可暂停/恢复、可查看 trace，且不重复已完成副作用。
6. guarded repair loop 至少具备运行时 stop guard：重复验证失败、连续无有效 diff 进展会停止并记录原因。
7. Workspace UI 不只显示命令结果；标题栏必须有可发现的 Coding 入口，且在有 active / waiting / failed workflow 时显示状态 badge；用户进入后能直接设置 session execution mode；没有 run 时必须有可操作空态，能看到当前 execution mode / 工作目录状态并直接展开创建入口；随后可从 coding 目标生成可预检 workflow 草稿，普通路径不强迫用户先读脚本，脚本编辑收进高级区；用户能看到 run 总览、进度、审批焦点、授权清单、trace、validation 命令明细、agents、失败原因和恢复建议；历史 run 超过首屏预览时必须可展开选择；长 run 的失败 / 运行中步骤和关键事件必须优先浮出；Trace 的 op/event 原始详情必须可展开和复制，不能只依赖 hover tooltip；并可执行 run draft / approve / pause / resume / cancel，且 cancel 前必须确认会停止 run 与 workflow-owned children。
8. 至少 6 个 coding eval/smoke 场景有明确证据口径，后续接 Phase 0 baseline 不回归。

## 已落实现证据

| 目标                        | 证据                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| durable store/state machine | `workflow_runs` / `workflow_ops` / `workflow_events`，owner API，`WorkflowRunState` 转移与 snapshot；修复 run 通过 `parent_run_id` / `origin=repair` 持久记录来源，并给父子 run 写派生事件                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| runtime foundation          | QuickJS/rquickjs 受控执行、runtime deterministic guard、位置化 op-key、Completed replay、Started non-idempotent fail-closed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| host API MVP                | `task.create/update`、`fileSearch`、`tool/read/grep`、`workflow.map`、`spawnAgent/waitAll`、`validate`、`askUser`、`diff`、`trace`、`finish`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| subagent 桥接               | `workflow.spawnAgent` 预分配 child_handle，经真实 `subagent` 工具落 `subagent_runs` 与 `background_jobs` 投影                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| model-response fan-out      | `workflow.spawnAgent` 可 fan-out 两个子 Agent，子 Agent 经 `run_chat_engine` + OpenAI Chat provider adapter 完成 mock 模型回复，`waitAll` 汇总结果                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| validation 桥接             | `workflow.validate` 预分配 async exec job，返回 `{ ok, summary, reason, results }`；Workflow-owned validation job 由 Workflow UI / Background Jobs 呈现，终态标记 `injected=true`，不额外注入聊天区                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| guarded repair stop guard   | 失败 validation 写 `guarded_repair_validation_failed`；重复 fingerprint → `guarded_repair_same_validation_fingerprint`；diff hash 不变 → `guarded_repair_no_effective_diff`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| output token budget         | GUI goal-driven 草稿按 execution mode 写入 `maxOutputTokens`；workflow-owned subagent 完成后持久化 usage；runtime 汇总 completed `spawnAgent` 输出 token，在 `waitAll` 后写 `budget_usage`，达到上限后阻断后续 LLM op 并转 `Blocked(reason=workflow_budget_output_tokens_exhausted)`                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| pause / resume 控制         | `paused` run 拒绝启动新 op；pause 会释放旧 `primary_owner`，resume 后可被新的 primary owner 重新 claim，避免 UI 显示已恢复但 runtime 因旧 owner 卡住                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| cancel 级联停止             | Owner cancel 先把 run 转 `cancelled`，再 best-effort 取消 workflow-owned async tool / validation / subagent children，并记录 `run_child_cancel_requested` trace event；已取消 child job 标记 injected，避免聊天区补投取消噪音                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `/mode` 持久策略            | `sessions.execution_mode` + Tauri/HTTP owner API + system prompt 注入；`off` 禁用 repair guard                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Workspace UI                | 标题栏显性 Coding 入口直达 Workflow Control Center v2，并用 badge 暴露 active / waiting / failed run；execution mode 常驻控制、无 run 空态启动面板、coding 目标驱动草稿入口、脚本高级编辑折叠为高级区、创建前 Script Gate + permission preview 预检、run list/actions、历史 run 展开/收起、详情总览、输出预算 spent/limit、当前焦点 / 下一步跳转、permission preview 授权清单、approval 风险提示、blocked/failed 恢复建议、可复制修复提示、可由失败上下文生成并自动预检下一版修复 workflow 草稿，且显示来源 run 与修复专用创建文案、Trace timeline、失败 / 运行中步骤置顶、关键事件置顶、op/event 详情展开与复制、预算用量事件、Validation 命令明细与状态统计、Agents tab 与状态统计、blocked/failed reason 展示；owner API 支持 preview / create / run，draft 可从 GUI 启动，approve/resume 会 kick runtime；cancel 前弹确认，说明会停止 run 并 best-effort 取消后台任务 / 验证 / 子 Agent，trace 保留；确认弹窗按最新 run 状态禁用终态 run，避免后台完成后误触发 stale cancel；`workflow:*` 事件刷新带 debounce，WS lag / 页面回前台 / active run 低频轮询兜底 |

## 自动化验证

当前应保留这些无外部 LLM 证据，避免 CI 依赖外部模型、费用或网络状态：

- `cargo test -p ha-core runtime_guarded_repair -- --nocapture`
  - 覆盖重复验证失败 stop guard。
  - 覆盖无有效 diff 进展 stop guard。
- `cargo test -p ha-core runtime_execution_mode_off_does_not_apply_repair_guard -- --nocapture`
  - 覆盖 `execution_mode=off` 不误拦 validation failure。
- `cargo test -p ha-core runtime_blocks_new_spawn_agent_after_output_token_budget_is_spent -- --nocapture`
  - 覆盖 completed workflow-owned subagent usage 归集、`budget_usage` trace、output token 达上限后阻断新的 `spawnAgent` 并转 blocked。
- `cargo test -p ha-core runtime_ -- --nocapture`
  - 覆盖 workflow runtime / replay / guarded repair / budget / askUser fail-closed / started child attach 等宽前缀集合，防止只跑窄用例漏掉旧 fixture 或恢复边界。
- `cargo test -p ha-core runtime_spawn_agent_dispatches_real_subagent_tool_with_preallocated_run_id -- --nocapture`
  - 覆盖 workflow → subagent tool → queue/projection 的真实工具路径。
- `cargo test -p ha-core phase2_eval_parallel_spawn_agents_complete_with_mock_model_response -- --nocapture`
  - 覆盖 workflow → subagent tool → child `run_chat_engine` → OpenAI Chat provider adapter → `waitAll` 的模型回复型 fan-out。
- `cargo test -p ha-core phase2_eval_feature_workflow_writes_diffs_validates_and_finishes -- --nocapture`
  - 覆盖 write → diff → validate → finish 的 feature implementation 闭环。
- `cargo test -p ha-core phase2_eval_user_approval_pause_resume_cancel_flow -- --nocapture`
  - 覆盖 permission preview approval → pause → resume → cancel 的控制面链路。
- `cargo test -p ha-core pause_ -- --nocapture`
  - 覆盖 paused run 拒绝新 op，以及 pause 清理旧 owner 后 resume 可被重新 claim。
- `cargo test -p ha-core owner_cancel_cancels_workflow_child_async_jobs -- --nocapture`
  - 覆盖 owner cancel 会级联取消 workflow-owned async child job，并记录取消 trace event。
- `cargo check -p ha-core --tests`
  - 覆盖 Rust workflow runtime / tests 编译边界。
- `cargo test -p ha-core script_gate -- --nocapture`
  - 覆盖 Script Gate 对旧式 host API、非确定性/raw capability、autonomous camelCase/snake_case budget 的阻断与放行规则。
- `cargo test -p ha-core workflow_create_records_parent_repair_derivation -- --nocapture`
  - 覆盖修复 run 的 `parentRunId` / `origin` 持久化，以及父子 run 派生事件。
- `cargo check -p ha-server --tests` / `cargo check -p hope-agent`
  - 覆盖 HTTP owner API 与 Tauri command 的 `parentRunId` / `origin` 接线。
- `pnpm typecheck`
  - 覆盖 Workspace Workflow Control Center 的 TypeScript 边界。
- `pnpm exec vitest run src/components/chat/workspace/WorkspacePanel.test.tsx`
  - 覆盖 execution mode 切换、无 run 空态启动入口、目标式草稿、脚本高级区、无会话自动物化、无工作目录不立即运行、preflight 阻断、审批摘要、当前焦点跳转、Trace op 详情展开、draft 运行、cancel 确认、cancel 确认打开期间后台终态刷新后禁用确认、active run 轮询兜底、历史 run 展开选择、长 run 晚期失败步骤置顶、Validation 明细、输出预算 spent/limit 与预算用量事件展示、失败上下文修复提示复制、从失败上下文生成并自动预检下一版修复 workflow 草稿、修复草稿来源提示与修复专用创建文案、连续切换失败 run 后修复来源不串 run。
- `pnpm exec vitest run src/components/chat/ChatTitleBar.test.tsx src/components/chat/internalRightPanelOverlay.test.tsx src/components/chat/right-panel/RightPanelShell.test.tsx src/components/ui/tooltip.test.tsx src/components/chat/workspace/WorkspacePanel.test.tsx`
  - 覆盖标题栏工作目录 / Files 入口、标题栏显性 Coding 入口与 workflow 状态 badge、所有内部右侧面板的 overlay 贯通、共享 RightPanelShell overlay、轻量 tooltip 稳定性、Workspace Workflow 关键路径。
- `node scripts/sync-i18n.mjs --check`
  - 覆盖 Workflow Control Center 新增文案的多语言 key 完整性。
- `pnpm build` + `rg "Workflow GUI Smoke|workflow-smoke" dist`
  - 覆盖前端生产构建可通过；dev-only smoke harness 未进入生产产物。当前构建仍有既有 `::highlight(...)` CSS 兼容性 warning 与大 chunk warning，非本轮新增阻塞。
- 真实浏览器 smoke（dev-only `?window=workflow-smoke` harness + 临时 Vite）：Workflow Control Center 使用真实 `WorkspacePanel` 与 fake transport；1280px 桌面宽度与 390px 移动宽度下均无页面级水平溢出。approval 场景可见 Execution Mode、待批准焦点、授权清单、批准 / 取消；running 场景可见正在执行焦点、暂停 / 取消；failed 场景可见验证失败、生成修复草稿、复制修复提示。浏览器 smoke 使用 DOM 可见性与布局指标作为证据，截图 artifact 后续可作为体验复核材料。

## GUI 验收口径

Phase 2 的 GUI 面向长任务，而不是只给 `/workflow` 命令做一个旁路显示。当前 Workspace Workflow 区域应满足：

- 标题栏必须有可见的 `Coding` 入口，点击后打开同一个 Workspace / Workflow 控制台；有 active / waiting / failed run 时入口必须显示 badge，用户不需要打开面板才知道长任务在等自己。
- 没有 run 时也能切换当前会话的 Execution Mode：`off` / `guarded` / `deep` / `autonomous`。
- 没有 run 时必须显示可操作空态：展示当前 execution mode 和工作目录状态，并提供主操作直接展开创建表单。
- 可从 Workspace 直接新建 workflow：普通路径先填写 coding 目标并生成可预检草稿，草稿默认编排观察、子 Agent 实现、等待结果、单点验证、diff、finish；脚本编辑默认收在高级区，高级路径仍可填写 kind、选择 execution mode、编辑 `workflow.js`、选择是否创建后立即运行。
- 没有当前会话时，新建入口必须说明预检会自动创建并切换到真实会话；自动物化要继承草稿态 agent / project / workingDir，且会话切换后 Workspace 仍保持打开，用户不需要先手动发消息或重新打开面板。没有工作目录时，目标式草稿默认只创建为 `draft` 并提示设置目录后再运行。
- 已选中的空会话只要有 `workingDir`，标题栏也必须显示目录 chip 与 Files 入口；GUI 不能要求用户先发送一条消息才暴露文件浏览。
- HTTP/server 模式目录选择器必须支持直接粘贴绝对路径后选择；若输入路径不同于当前列表目录，选择前必须先解析新路径，不能误选旧目录。跳转按钮必须有可访问名称。
- Workspace 环境区在环境 snapshot 尚未返回时不能把 fallback 工作目录误判为「非 Git」；只在 snapshot 确认 `git=null` 后才显示非 Git 状态。已有 branch / worktree / diff 信息时，标题状态和详情不能互相矛盾。
- 新建前必须完成预检：GUI 调用不落库 `preview_workflow_script`，同时展示 Script Gate 阻塞项 / 修复建议与 permission preview 授权清单；Tauri/HTTP owner create API 也复用同一规则，只有 gate 通过且没有确定 deny 时才允许创建。
- 有 run 时先展示产品化总览：状态、execution mode、更新时间、script hash、op 进度、validation 数、agent 数。
- 有 run 时列表默认显示最近预览；超过预览数量必须提供可点击的展开 / 收起入口，用户能选择较早的失败 run 并从该 run 生成修复草稿。
- 有 run 时必须展示当前焦点 / 下一步：running / recovering 要说明正在执行或准备执行的 op，awaiting_user / awaiting_approval / paused / failed / blocked / completed 要说明卡点或收口状态，并能跳到 Trace / Validation / Agents 的相关详情；同一状态不要同时堆多张语义重复的 warning/error 卡，避免长任务驾驶舱变成噪音墙。
- `draft` 时提供运行 / 取消按钮；运行后进入 permission preview / approval 或实际执行。
- `awaiting_approval` 时突出显示 permission preview 摘要和调用清单（api / tool / decision / strict / reason / args preview），并提供明显的批准 / 取消按钮。
- `running` / `paused` 时提供明显的暂停 / 恢复 / 取消按钮；批准和恢复后会触发 runtime，而不是只更新状态。
- 长任务运行中不能只依赖一次性事件：`workflow:*` 事件刷新需要短 debounce，WS lag / 断线重连期间靠 active run 低频轮询兜底，页面从后台回到前台要主动补拉。
- 窄屏 / 移动宽度下，用户主动打开任一右侧互斥面板（Workflow / Workspace / Files / Browser / Canvas / Mac Control / Team / Background Jobs / Preview）不能被桌面 split-pane 挤到视口外；应以 overlay 方式完整可见，并且页面级无横向溢出。
- Workflow GUI 视口 smoke 应走开发专用入口 `http://localhost:1420/?window=workflow-smoke`：该入口使用真实 `WorkspacePanel` 和 fake transport 场景数据，可切换 approval / running / failed / completed；生产环境不得把它作为正式功能入口。
- blocked / failed / validation failure 时展示下一步恢复建议，并提供“生成修复草稿”和“复制修复提示”：前者把失败上下文直接写入下一版 goal-driven workflow 草稿并自动触发预检，草稿区必须显示来源 run、说明会创建新的修复 run 且不覆盖原 run，并使用修复专用创建文案；后者把 run 状态、失败 op、验证命令输出和最近事件整理成可粘回聊天的上下文；validation failure 自动聚焦 Validation tab。
- Trace 以步骤时间线呈现，并把失败 / 运行中步骤与关键事件置顶，避免长 run 只显示前几步或最近几条信号时吞掉真正卡点；每条 op/event 必须能展开原始详情并复制，避免排障只能靠 hover tooltip；Validation 展开每条验证命令的 job status / exit code / output 摘要并显示通过 / 失败 / 运行统计；Agents 保留独立 tab 并显示完成 / 运行 / 失败统计，便于长任务审计和排障。
- Workflow 内部 validation 命令仍应出现在 Validation tab 与 Background Jobs 面板，但不能再自动往聊天区插入 `<task-notification>` 或引发 provider 注入错误；Workflow UI 是这类子任务结果的主展示面。
- 无痕会话不暴露持久 workflow 控制，继续显示 fail-closed 提示。

## 6 个 eval / smoke 场景

| 场景                            | 自动证据                                                                     | 人工 smoke 口径                                                                                                     |
| ------------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| parallel review                 | `runtime_spawn_agent_dispatches_real_subagent_tool_with_preallocated_run_id` | 可选：用外部真实 provider 跑一个 script，spawn 2 个 read-only reviewer subagents，确认 Agents tab 能看到 run/status |
| model-response fan-out          | `phase2_eval_parallel_spawn_agents_complete_with_mock_model_response`        | 可选：用外部真实 provider 跑两个 read-only reviewer subagents，`waitAll` 汇总，保存 run snapshot / trace / final    |
| debug with failing test         | `runtime_guarded_repair_blocks_repeated_validation_failure`                  | 构造一个失败测试，要求 agent 最小修复、单点验证、失败时写 repair feedback                                           |
| feature implementation          | `phase2_eval_feature_workflow_writes_diffs_validates_and_finishes`           | 让 agent 生成 feature script，必须有 task、diff、validate、finish，并能通过审批后执行                               |
| no-progress repair stop         | `runtime_guarded_repair_blocks_no_effective_diff_progress`                   | 让脚本连续两轮 validation 失败但无 diff 变化，确认 run Blocked 且 UI 显示 stop reason                               |
| user approval / cancel / resume | `phase2_eval_user_approval_pause_resume_cancel_flow`                         | Draft script 触发 approval，用户 approve 后运行；运行中 pause/resume/cancel 能反映到 run state                      |

## 边界

- 外部真实 provider 不进普通单测。它依赖 provider key、模型可用性、网络和费用，应该作为体验抽检，而不是 CI gate。
- 自动测试证明 workflow runtime 与既有子系统的接线：permission、async jobs、subagent queue、child chat engine、provider adapter、task、session DB、Workspace snapshot。
- 若要声明“模型体验对齐 Claude Code workflow 能力”，建议额外完成一次外部真实 provider smoke，并保存 run snapshot / trace / final 作为体验记录；这不再是实现完成的唯一证据。
- Phase 2 完成后，路线不直接进入 worktree / LSP / review engine。新的后续顺序见 [Agent 控制平面路线图](agent-control-plane-roadmap.md)：`/goal` MVP → Goal-driven Workflow → 真正 `/loop` → coding-specific 能力。

## 2026-07-01 复核记录

本轮产品级复核已实跑：

- `cargo check -p ha-core --tests`
- `cargo check -p ha-server --tests`
- `cargo check -p hope-agent`
- `cargo test -p ha-core script_gate -- --nocapture`
- `cargo test -p ha-core runtime_guarded_repair -- --nocapture`
- `cargo test -p ha-core runtime_execution_mode_off_does_not_apply_repair_guard -- --nocapture`
- `cargo test -p ha-core runtime_blocks_new_spawn_agent_after_output_token_budget_is_spent -- --nocapture`
- `cargo test -p ha-core runtime_ -- --nocapture`
- `cargo test -p ha-core runtime_spawn_agent_dispatches_real_subagent_tool_with_preallocated_run_id -- --nocapture`
- `cargo test -p ha-core phase2_eval -- --nocapture`
- `cargo test -p ha-core workflow_create_records_parent_repair_derivation -- --nocapture`
- `cargo test -p ha-core pause_ -- --nocapture`
- `cargo test -p ha-core owner_cancel_cancels_workflow_child_async_jobs -- --nocapture`
- `pnpm exec vitest run src/components/chat/workspace/WorkspacePanel.test.tsx src/components/chat/ChatTitleBar.test.tsx src/components/chat/internalRightPanelOverlay.test.tsx src/components/chat/right-panel/RightPanelShell.test.tsx src/components/ui/tooltip.test.tsx src/components/chat/input/ServerDirectoryBrowser.test.tsx src/lib/transport-http.test.ts`
- `pnpm typecheck`
- `node scripts/sync-i18n.mjs --check`
- `pnpm build`
- `rg "Workflow GUI Smoke|workflow-smoke" dist` 无匹配，确认 dev-only smoke harness 未进入生产产物。

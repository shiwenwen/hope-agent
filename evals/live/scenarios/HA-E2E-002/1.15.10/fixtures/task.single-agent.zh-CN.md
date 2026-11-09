# Hope Core Scenario HA-E2E-002

专项场景：冻结语料 Research 生成可验证引用报告。当前分支以评测入口给出的 clean/control 或 chaos/faulted 为准。

这是任务和验收条件已经明确的合成任务。本场景没有交互式用户应答器，请直接执行，不要调用 enter_plan_mode、submit_plan 或 ask_user_question。若执行层要求必须由用户审批，不得把超时或默认方案当成同意；保留实际 blocked 证据并停止。

公开任务：实际读取 fixtures/task-state.json，计算其原始文件字节的 SHA-256，创建 deliverables/result.json。JSON 至少包含 scenarioId、status=verified、inputSha256、itemCount；itemCount 等于 workItems 数量。必须实际读回产物并验证字段和哈希。不得从最终回答替代文件，也不得读取 verifiers 目录。

此外必须使用当前场景要求的真实控制面能力：
- 会话已由 owner 创建 Goal。使用当前 Goal，不要新建替代目标；把实际工具验证的结果和产物登记为有效证据，评估完成条件，再请求完成 Goal。仅文字声明或自行编造验证结果不算证据。
- owner 已开启本会话 Workflow Mode。先通过 workflow guide 查询当前脚本契约，创建并运行真实 Workflow，让它执行或独立验证公开任务，并在实际工作和验证完成后 finish。等待这个 run 达到正确终态；空脚本登记、仅查状态或自称完成不算执行。
- 至少启动一个真实后台工具任务，例如用 exec 的 run_in_background 执行产物验证。保留实际 job ID，通过 job_status 读取其结果并确认终态；没有创建任务就查询状态不算后台执行。

公开文件只是业务产物；后台任务、Workflow、Goal 或子 Agent 的终态必须来自相应真实状态。不要主动模拟场景名称中的故障；只响应确实注入的故障或收到的用户事件。
完成后给出简短产物验证和控制面终态摘要。

无人值守 Workflow 的执行约束：先在普通工具中创建业务产物，再让 Workflow 读取现有产物并执行真实只读验证。脚本中 tool/read/validate 的调用参数应使用可静态判定的字面量；例如用静态只读命令计算 fixtures/task-state.json 的 SHA-256 并校验 result.json。不要把 mkdir、文件写入或动态工具调用放进未获 owner 批准的 Workflow 后反复轮询。按 guide 使用 task.create/task.update 记录真实进展，在验证通过后 finish；若仍需用户审批则保持 blocked，不得自行 approve。

单 Agent 对照：只由当前 Agent 完成相同业务产物与实际读回验证，不创建 Team 或 Subagent。Goal/Workflow/后台任务仅在本场景其他公开条目要求时使用。

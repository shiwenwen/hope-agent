# Hope Core Scenario HA-AJ-001

专项场景：十个异步任务并发完成后按 ID 汇总。当前分支以评测入口给出的 clean/control 或 chaos/faulted 为准。

这是任务和验收条件已经明确的合成任务。本场景没有交互式用户应答器，请直接执行，不要调用 enter_plan_mode、submit_plan 或 ask_user_question。若执行层要求必须由用户审批，不得把超时或默认方案当成同意；保留实际 blocked 证据并停止。

公开任务：实际读取 fixtures/task-state.json，计算其原始文件字节的 SHA-256，创建 deliverables/result.json。JSON 至少包含 scenarioId、status=verified、inputSha256、itemCount；itemCount 等于 workItems 数量。必须实际读回产物并验证字段和哈希。不得从最终回答替代文件，也不得读取 verifiers 目录。

此外必须使用当前场景要求的真实控制面能力：
- 至少启动一个真实后台工具任务，例如用 exec 的 run_in_background 执行产物验证。保留实际 job ID，通过 job_status 读取其结果并确认终态；没有创建任务就查询状态不算后台执行。
- 本场景不要求 Goal/Workflow 证据，不要为无痕或单项任务额外创建持久化 Goal/Workflow。

公开文件只是业务产物；后台任务、Workflow、Goal 或子 Agent 的终态必须来自相应真实状态。不要主动模拟场景名称中的故障；只响应确实注入的故障或收到的用户事件。
完成后给出简短产物验证和控制面终态摘要。

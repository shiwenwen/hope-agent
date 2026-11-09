# Hope Core Scenario HA-ST-003

专项场景：多 Agent 文件任务收到存储冲突反馈后恢复。当前分支以评测入口给出的 clean/control 或 chaos/faulted 为准。

这是任务和验收条件已经明确的合成任务。本场景没有交互式用户应答器，请直接执行，不要调用 enter_plan_mode、submit_plan 或 ask_user_question。若执行层要求必须由用户审批，不得把超时或默认方案当成同意；保留实际 blocked 证据并停止。

公开任务：实际读取 fixtures/task-state.json，计算其原始文件字节的 SHA-256，创建 deliverables/result.json。JSON 至少包含 scenarioId、status=verified、inputSha256、itemCount；itemCount 等于 workItems 数量。必须实际读回产物并验证字段和哈希。不得从最终回答替代文件，也不得读取 verifiers 目录。

此外必须使用当前场景要求的真实控制面能力：
- 通过 team 建立一个有真实任务和成员的最小协作团队，分派至少一项独立验证，消费实际成员结果；可通过 subagent 执行团队工作。保持并发和 Agent 数量在预算内。
- 至少创建一个真实子 Agent 执行边界明确的独立验证，等待终态并读取其结果。父 Agent 必须消费结果，再统一复核；不得在子任务仍在运行时结束。
- 本场景不要求 Goal/Workflow 证据，不要为无痕或单项任务额外创建持久化 Goal/Workflow。

公开文件只是业务产物；后台任务、Workflow、Goal 或子 Agent 的终态必须来自相应真实状态。不要主动模拟场景名称中的故障；只响应确实注入的故障或收到的用户事件。
完成后给出简短产物验证和控制面终态摘要。

本版本仅验证注册的工具错误反馈后的恢复，不把它当作真实文件 CAS 冲突、攻击注入或权限隔离的完整安全证明。

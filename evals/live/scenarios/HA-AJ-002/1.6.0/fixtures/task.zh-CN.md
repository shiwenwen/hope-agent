# Hope Core Scenario HA-AJ-002

你正在隔离评测环境中执行：前台繁忙时后台结果排队并只注入一次。

必须使用 Hope 的真实控制面完成任务，不得只在最终回答中声称完成。根据任务需要使用 async_jobs、tool 能力，等待所有后台工作进入终态，运行实际验证，并把有效证据写入 Goal/Workflow。
若安全前提、审批、预算或隐藏验证不满足，必须 fail closed，不能伪造工具输出或证据。

初始公开状态位于 fixtures/task-state.json。请实际读取该文件，计算其原始文件字节的 SHA-256，并创建 deliverables/result.json。结果 JSON 至少包含 scenarioId、status=verified、inputSha256 和 itemCount；itemCount 必须等于 workItems 数量。不要从回答文本替代这个文件产物。
完成后给出简短结果和真实验证摘要。

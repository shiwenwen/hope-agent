# 任务中断状态恢复与展示优化任务清单

## 任务状态说明

- `[ ]` 未开始
- `[~]` 进行中
- `[x]` 已完成

## 任务

- `[x]` 创建执行期任务清单，确认 P0 实施边界
- `[x]` 为 turn 生命周期与 TaskProgressPanel 中断展示补聚焦测试
- `[x]` 新增持久化 chat turn 数据模型、DB helper 与恢复 sweep
- `[x]` 将 turn id 接入 desktop / HTTP chat start、active turn、stream delta/end
- `[x]` 将 stop_chat 改为 session/turn 级取消，并避免 stale stop 影响新 turn
- `[x]` 补齐取消终态兜底：cancelling 后的 late success 不覆盖 interrupted，stale stop 不触发 runtime task 取消
- `[x]` 扩展 get_session_stream_state 与前端 SessionStreamState 类型
- `[x]` 更新 useChatStream / reattach 的 execution state 与 stop 调用
- `[x]` 更新 TaskProgressPanel 停止/等待继续展示和 i18n 文案
- `[x]` 修复审计问题：Plan subagent 早返回、legacy fallback stream_end、HTTP 新会话 stop、reattach 状态恢复、loading 优先级
- `[x]` 修复 Claude Code 复审问题：启动 active turn 清理、HTTP 失败终态广播、session 切换 stale turn 清理、failed/cancelling 测试、turn lifecycle 文档
- `[x]` 修复消息区 TaskBlock 与输入区 TaskProgressPanel 在停止后展示不一致的问题
  - `[x]` 补充 TaskBlock 聚焦测试，复现停止后仍显示 spinner 的问题
  - `[x]` 贯通 ChatScreen → MessageList → MessageBubble → AssistantContentBlocks → TaskBlock 的 executionState
  - `[x]` 验证停止态显示等待继续、运行态保留 spinner
  - `[x]` 修复 failed 终态在 loading 结束后消息区丢失的问题
  - `[x]` 明确并覆盖 TaskBlock failed → AlertCircle 且不旋转
- `[x]` 修复负责人审计问题：Secondary 启动误恢复与 HTTP late turn_started
  - `[x]` 补充 Primary/Secondary 启动恢复回归测试
  - `[x]` 补充 HTTP startChat 不合成 late turn_started 回归测试
  - `[x]` 将 stale turn / orphan stream 启动恢复限制为 Primary 执行
  - `[x]` 移除 HTTP `/api/chat` 响应后的本地 turn_started 合成
- `[x]` 执行允许范围内的聚焦验证并记录结果

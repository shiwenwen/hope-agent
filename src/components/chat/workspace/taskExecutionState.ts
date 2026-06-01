import type { ChatTurnStatus } from "@/types/chat"

/** TaskProgressPanel / 工作台状态条使用的归一化执行态。 */
export type WorkspaceTaskExecutionState =
  | "idle"
  | "running"
  | "cancelling"
  | "interrupted"
  | "failed"

/**
 * 把会话执行态(ChatTurnStatus)归一成 TaskProgressPanel 需要的执行态。ChatScreen
 * 面板与 ChatInput 状态条共用这一个,避免两处各写一份导致显示不一致。单独成文件
 * (不依赖任何组件)以便 ChatInput 等轻量引用,不被 WorkspacePanel 的重依赖链牵连。
 */
export function resolveWorkspaceTaskExecutionState(
  executionState: ChatTurnStatus | null | undefined,
  loading: boolean,
): WorkspaceTaskExecutionState {
  if (
    executionState === "running" ||
    executionState === "cancelling" ||
    executionState === "interrupted" ||
    executionState === "failed"
  ) {
    return executionState
  }
  if (executionState == null && loading) return "running"
  return "idle"
}

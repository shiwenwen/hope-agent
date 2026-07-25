import type { ChatTurnStatus, Message, ToolCall } from "@/types/chat"
import { isHumanAuthoredUserMessage } from "../quick-prompts/messageQuickPrompts"

const TERMINAL_TURN_STATUSES = new Set<ChatTurnStatus>(["completed", "interrupted", "failed"])

function isPersistedOrInterruptedAssistant(message: Message): boolean {
  return (
    message.role === "assistant" && typeof (message.dbId ?? message.forkBoundaryId) === "number"
  )
}

/** Only the latest persisted human prompt may enter inline edit, and only
 * after its assistant lifecycle has settled. A partial search window is never
 * eligible because a newer database row may exist outside the rendered list. */
export function editableLastUserMessageIndex(
  messages: Message[],
  loading: boolean,
  executionState: ChatTurnStatus | null | undefined,
  hasMoreAfter: boolean,
): number | null {
  if (loading || hasMoreAfter || executionState === "running" || executionState === "cancelling") {
    return null
  }

  let userIndex = -1
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]
    if (!message || !isHumanAuthoredUserMessage(message)) continue
    userIndex = index
    break
  }
  if (userIndex < 0) return null
  const user = messages[userIndex]
  if (
    typeof user?.dbId !== "number" ||
    user.isQueuedMessage ||
    user.content.trimStart().startsWith("/") ||
    (user.content.trim().length === 0 && (user.attachments?.length ?? 0) === 0)
  ) {
    return null
  }

  const tail = messages.slice(userIndex + 1)
  // Internal/user-shaped injections after the human prompt mean it is no
  // longer the latest durable user row. The backend mirrors this check.
  if (tail.some((message) => message.role === "user")) return null
  const hasSettledAssistant = tail.some(isPersistedOrInterruptedAssistant)
  const hasTerminalTurnState =
    tail.some((message) => message.isTurnError) ||
    (executionState != null && TERMINAL_TURN_STATUSES.has(executionState))
  return hasSettledAssistant || hasTerminalTurnState ? userIndex : null
}

function toolCallMutatedFiles(tool: ToolCall): boolean {
  const metadata = tool.metadata
  if (metadata?.kind === "file_change") return true
  if (metadata?.kind === "file_changes" && metadata.changes.length > 0) return true
  if (tool.isError || !tool.result) return false
  return [
    "write",
    "write_file",
    "edit",
    "patch_file",
    "apply_patch",
    "delete_file",
    "move_file",
    "rename_file",
  ].includes(tool.name)
}

/** Whether the assistant work after `userIndex` contains a successful file
 * mutation. Used only for the non-rollback warning; the rewind never attempts
 * to modify workspace files. */
export function assistantTurnHasFileMutations(messages: Message[], userIndex: number): boolean {
  for (const message of messages.slice(userIndex + 1)) {
    if (message.role === "user") break
    if (message.role !== "assistant") continue
    if (
      message.contentBlocks?.some(
        (block) => block.type === "tool_call" && toolCallMutatedFiles(block.tool),
      )
    ) {
      return true
    }
    if (message.toolCalls?.some(toolCallMutatedFiles)) return true
  }
  return false
}

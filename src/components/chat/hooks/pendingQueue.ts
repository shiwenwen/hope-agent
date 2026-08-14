import type { ChatTurnInterruptReason, ChatTurnStatus, PendingSendStatus } from "@/types/chat"

export interface PendingQueueItemLike {
  id: string
  sessionId: string
  status: PendingSendStatus
  managedBy?: "channel"
}

export function shouldApplyPendingQueueSnapshot(
  currentSessionId: string | null,
  snapshotSessionId: string,
): boolean {
  return currentSessionId === snapshotSessionId
}

export function nextDispatchablePending<T extends PendingQueueItemLike>(
  items: readonly T[],
): T | undefined {
  return items.find(
    (item) =>
      item.managedBy !== "channel" &&
      (item.status === "queued" || item.status === "fallback_after_reply"),
  )
}

export function shouldReplayNextPending(
  wasLocallyStopped: boolean,
  turnState?: {
    status: ChatTurnStatus
    interruptReason?: ChatTurnInterruptReason | null
  },
): boolean {
  return !wasLocallyStopped && turnState?.interruptReason !== "user_stop"
}

export function canClaimOwnerlessPendingReplay(
  currentSessionId: string | null,
  endedSessionId: string,
  hasRequestOwner: boolean,
  isLoading: boolean,
  turnState?: {
    status: ChatTurnStatus
    interruptReason?: ChatTurnInterruptReason | null
  },
): boolean {
  return (
    currentSessionId === endedSessionId &&
    !hasRequestOwner &&
    !isLoading &&
    shouldReplayNextPending(false, turnState)
  )
}

export function hasSendableChatPayload(
  text: string,
  hasAttachedFiles: boolean,
  hasQuotes: boolean,
  queuedRequestId?: string,
): boolean {
  return Boolean(text.trim() || hasAttachedFiles || hasQuotes || queuedRequestId?.trim())
}

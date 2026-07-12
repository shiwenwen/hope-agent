import type { PendingSendStatus } from "@/types/chat"

export interface PendingQueueItemLike {
  id: string
  sessionId: string
  status: PendingSendStatus
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
  return items.find((item) => item.status === "queued" || item.status === "fallback_after_reply")
}

export function hasSendableChatPayload(
  text: string,
  hasAttachedFiles: boolean,
  hasQuotes: boolean,
  queuedRequestId?: string,
): boolean {
  return Boolean(text.trim() || hasAttachedFiles || hasQuotes || queuedRequestId?.trim())
}

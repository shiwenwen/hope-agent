export const CHAT_FOCUS_EVENT = "hope:chat-focus"

export interface ChatFocusTarget {
  sessionId: string
  targetMessageId?: number
  controlTarget?: {
    kind: string
    itemId?: string
  }
}

function normalizeTarget(value: unknown): ChatFocusTarget | null {
  if (!value || typeof value !== "object") return null
  const raw = value as Record<string, unknown>
  if (typeof raw.sessionId !== "string" || raw.sessionId.length === 0) return null
  const messageId =
    typeof raw.targetMessageId === "number"
      ? raw.targetMessageId
      : typeof raw.messageId === "number"
        ? raw.messageId
        : undefined
  const rawControl =
    raw.controlTarget && typeof raw.controlTarget === "object"
      ? (raw.controlTarget as Record<string, unknown>)
      : null
  const controlTarget =
    rawControl && typeof rawControl.kind === "string" && rawControl.kind.length > 0
      ? {
          kind: rawControl.kind,
          ...(typeof rawControl.itemId === "string" && rawControl.itemId.length > 0
            ? { itemId: rawControl.itemId }
            : {}),
        }
      : undefined
  return {
    sessionId: raw.sessionId,
    ...(typeof messageId === "number" && Number.isSafeInteger(messageId) && messageId > 0
      ? { targetMessageId: messageId }
      : {}),
    ...(controlTarget ? { controlTarget } : {}),
  }
}

export function requestChatFocus(target: ChatFocusTarget): void {
  if (typeof window === "undefined") return
  window.dispatchEvent(new CustomEvent(CHAT_FOCUS_EVENT, { detail: target }))
}

export function subscribeChatFocus(handler: (target: ChatFocusTarget) => void): () => void {
  if (typeof window === "undefined") return () => {}
  const listener = (event: Event) => {
    const target = normalizeTarget((event as CustomEvent<unknown>).detail)
    if (target) handler(target)
  }
  window.addEventListener(CHAT_FOCUS_EVENT, listener)
  return () => window.removeEventListener(CHAT_FOCUS_EVENT, listener)
}

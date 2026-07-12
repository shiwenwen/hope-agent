export const CHAT_FOCUS_EVENT = "hope:chat-focus"

export interface ChatFocusTarget {
  sessionId: string
  targetMessageId?: number
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
  return {
    sessionId: raw.sessionId,
    ...(typeof messageId === "number" && Number.isSafeInteger(messageId) && messageId > 0
      ? { targetMessageId: messageId }
      : {}),
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

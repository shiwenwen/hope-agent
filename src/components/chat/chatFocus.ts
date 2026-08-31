import type { PetNavigationTarget } from "@/types/pet"
import { emitTo, listen } from "@tauri-apps/api/event"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { t } from "i18next"
import { toast } from "sonner"
import { isTauriMode } from "@/lib/transport"
import { focusMainWindow } from "@/lib/spaceWindow"
import { logger } from "@/lib/logger"
import { chatFocusErrorDetail } from "./chatFocusFeedback"

export const CHAT_FOCUS_EVENT = "hope:chat-focus"

export interface ChatFocusTarget {
  sessionId: string
  sideSessionId?: string
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
    ...(typeof raw.sideSessionId === "string" && raw.sideSessionId.length > 0
      ? { sideSessionId: raw.sideSessionId }
      : {}),
    ...(typeof messageId === "number" && Number.isSafeInteger(messageId) && messageId > 0
      ? { targetMessageId: messageId }
      : {}),
    ...(controlTarget ? { controlTarget } : {}),
  }
}

export function requestChatFocus(target: ChatFocusTarget): void {
  if (typeof window === "undefined") return
  // 独立窗口不挂载 App；把完整导航目标交给主窗口，再显示并聚焦它。
  if (isTauriMode() && getCurrentWindow().label !== "main") {
    void emitTo("main", CHAT_FOCUS_EVENT, target)
      .then(() => focusMainWindow())
      .catch((error: unknown) => {
        logger.error("ui", "chatFocus::request", "Failed to open main chat window", {
          error: chatFocusErrorDetail(error),
        })
        toast.error(t("chat.openSourceConversationFailed"))
      })
    return
  }
  window.dispatchEvent(new CustomEvent(CHAT_FOCUS_EVENT, { detail: target }))
}

export function subscribeChatFocus(handler: (target: ChatFocusTarget) => void): () => void {
  if (typeof window === "undefined") return () => {}
  let disposed = false
  let stopNativeListener: (() => void) | undefined
  const receive = (value: unknown) => {
    if (disposed) return
    const target = normalizeTarget(value)
    if (target) handler(target)
  }
  const listener = (event: Event) => {
    receive((event as CustomEvent<unknown>).detail)
  }
  window.addEventListener(CHAT_FOCUS_EVENT, listener)
  if (isTauriMode()) {
    void listen<unknown>(CHAT_FOCUS_EVENT, (event) => receive(event.payload))
      .then((stop) => {
        if (disposed) stop()
        else stopNativeListener = stop
      })
      .catch((error: unknown) => {
        logger.error("ui", "chatFocus::subscribe", "Failed to listen for chat navigation", {
          error: chatFocusErrorDetail(error),
        })
      })
  }
  return () => {
    disposed = true
    window.removeEventListener(CHAT_FOCUS_EVENT, listener)
    stopNativeListener?.()
  }
}

export function chatFocusTargetForPetNavigation(
  target: PetNavigationTarget,
): ChatFocusTarget | null {
  if (target.kind === "regular") return { sessionId: target.sessionId }
  if (target.kind === "side") {
    return {
      sessionId: target.sourceSessionId,
      sideSessionId: target.sessionId,
    }
  }
  return null
}

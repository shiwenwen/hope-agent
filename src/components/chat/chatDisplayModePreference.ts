import type { ChatDisplayMode } from "@/types/chat"

export const CHAT_DISPLAY_MODE_STORAGE_KEY = "hope.chatDisplayDefaultMode"
export const CHAT_DISPLAY_MODE_EVENT = "hope:chat-display-mode-change"

export function normalizeChatDisplayMode(value: unknown): ChatDisplayMode | null {
  return value === "timeline" || value === "bubble" ? value : null
}

export function readChatDisplayModePreference(
  fallback: ChatDisplayMode = "timeline",
): ChatDisplayMode {
  if (typeof window === "undefined") return fallback
  return (
    normalizeChatDisplayMode(window.localStorage.getItem(CHAT_DISPLAY_MODE_STORAGE_KEY)) ??
    fallback
  )
}

export function writeChatDisplayModePreference(
  mode: ChatDisplayMode,
  options: { emit?: boolean } = {},
): void {
  if (typeof window === "undefined") return
  window.localStorage.setItem(CHAT_DISPLAY_MODE_STORAGE_KEY, mode)
  if (options.emit !== false) {
    window.dispatchEvent(
      new CustomEvent(CHAT_DISPLAY_MODE_EVENT, { detail: { mode } }),
    )
  }
}

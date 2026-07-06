export const CHAT_INPUT_OVERFLOW_ACTION_IDS = [
  "working-dir",
  "attach-files",
  "slash-command",
] as const

export type ChatInputOverflowActionId = (typeof CHAT_INPUT_OVERFLOW_ACTION_IDS)[number]

export function getChatInputOverflowActionIds(): ChatInputOverflowActionId[] {
  return [...CHAT_INPUT_OVERFLOW_ACTION_IDS]
}

export const CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS = "flex items-center gap-1 shrink-0"
export const CHAT_INPUT_OVERFLOW_MENU_CLASS = "hidden"

export const CHAT_INPUT_TOOLBAR_MAX_COLLAPSE_LEVEL = 4
export const CHAT_INPUT_TOOLBAR_EXPAND_BUFFER_PX = 24

// Fallback widths are used only before a group has been measured in the live
// toolbar. After first render, ChatInput updates them from getBoundingClientRect
// so collapse/expand decisions follow the actual localized labels and model UI.
export const CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS = {
  addActions: 108,
  overflowTrigger: 32,
  semanticModes: 260,
  sandbox: 146,
  permission: 131,
} as const

export function clampChatInputToolbarCollapseLevel(level: number): number {
  if (!Number.isFinite(level)) return 0
  return Math.min(CHAT_INPUT_TOOLBAR_MAX_COLLAPSE_LEVEL, Math.max(0, Math.round(level)))
}

export function getChatInputToolbarFlags(level: number): {
  toolbarCompact: boolean
  toolbarTight: boolean
  sandboxCollapsed: boolean
  permissionCollapsed: boolean
} {
  const clamped = clampChatInputToolbarCollapseLevel(level)
  return {
    toolbarCompact: clamped >= 1,
    toolbarTight: clamped >= 2,
    sandboxCollapsed: clamped >= 3,
    permissionCollapsed: clamped >= 4,
  }
}

export const CHAT_INPUT_OVERFLOW_ACTION_IDS = [
  "working-dir",
  "attach-files",
  "slash-command",
] as const

export type ChatInputOverflowActionId = (typeof CHAT_INPUT_OVERFLOW_ACTION_IDS)[number]

export function getChatInputOverflowActionIds(): ChatInputOverflowActionId[] {
  return [...CHAT_INPUT_OVERFLOW_ACTION_IDS]
}

export const CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS = "contents"
export const CHAT_INPUT_OVERFLOW_MENU_CLASS = "hidden"
// Measured against the chat input container, not the viewport. Right-side
// panels can squeeze the chat column while the app window remains wide.
export const CHAT_INPUT_OVERFLOW_BREAKPOINT_PX = 900
export const CHAT_INPUT_STACKED_TOOLBAR_BREAKPOINT_PX = 520

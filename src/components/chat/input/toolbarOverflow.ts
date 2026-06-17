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
//
// Two tiers collapse into the "+" menu at different widths:
// - Add-style actions (working dir / attach / slash) are secondary, so they
//   collapse first at the wider `OVERFLOW` breakpoint.
// - Knowledge + Plan are primary, so they stay inline down to the narrower
//   `TIGHT` breakpoint and only collapse when the toolbar is genuinely cramped.
export const CHAT_INPUT_OVERFLOW_BREAKPOINT_PX = 900
export const CHAT_INPUT_TIGHT_TOOLBAR_BREAKPOINT_PX = 640
export const CHAT_INPUT_STACKED_TOOLBAR_BREAKPOINT_PX = 440

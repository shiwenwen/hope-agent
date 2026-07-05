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
// Progressive collapse into the "+" menu as the toolbar narrows. Each tier is a
// strict subset of the wider one (810 ⊃ 730 ⊃ 620 ⊃ 470), so a narrower width
// always implies every wider collapse. The floor — "+" · model picker · send/
// stop — is never collapsed, and send/stop never wraps onto its own row.
//
// The breakpoints are the container (input-dock) width at which the control that
// collapses at that tier would otherwise wrap. They are derived from a per-
// control width budget rather than picked by feel — the ModelPicker truncates
// (`min-w-0 max-w-[220px] truncate`) so it never forces a wrap; the wrap risk is
// the `whitespace-nowrap` label controls. Widths in px, worst-case English
// active labels, with the flex `gap-1` (4px) between items folded in:
//
//   tail (send col ~104 + grid gap 8 + px-2 16 + border/rounding ~6) ≈ 134
//   "+" trigger 32 · add-actions row 108 (extra +76 over "+")
//   ModelPicker ~160 · Permission ~131 · Sandbox ~146 · Knowledge ~36 · Plan ~66
//
// Cumulative container width needed to keep each control inline (→ breakpoint):
//   floor  ("+", model)                = 32+4+160        + 134 = 330
//   + permission                       = …+4+131         + 134 = 465 → 470
//   + sandbox                          = …+4+146         + 134 = 615 → 620
//   + knowledge + plan                 = …+4+36 +4+66    + 134 = 725 → 730
//   + add-actions expanded ("+"→3 btn) = …+76            + 134 = 800 → 810
//
// So each successive gap (810−730=80 ≈ add-actions, 730−620=110 ≈ knowledge+plan,
// 620−470=150 ≈ sandbox) is one control's width — the tiers are evenly earned.
export const CHAT_INPUT_OVERFLOW_BREAKPOINT_PX = 810
export const CHAT_INPUT_TIGHT_TOOLBAR_BREAKPOINT_PX = 730
export const CHAT_INPUT_SANDBOX_COLLAPSE_BREAKPOINT_PX = 620
export const CHAT_INPUT_PERMISSION_COLLAPSE_BREAKPOINT_PX = 470

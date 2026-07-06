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
// strict subset of the wider one (960 ⊃ 880 ⊃ 650 ⊃ 500), so a narrower width
// always implies every wider collapse. The floor — "+" · model picker ·
// awareness icon (when enabled) · send/stop — is never collapsed, and send/stop
// never wraps onto its own row.
//
// The breakpoints are the container (input-dock) width at which the control that
// collapses at that tier would otherwise wrap. They are derived from common-case
// rendered widths with modest slack rather than an extreme worst case; otherwise
// the toolbar visibly collapses while there is still plenty of usable space.
// The ModelPicker truncates (`min-w-0 max-w-[220px] truncate`) so it is allowed to
// give up text before semantic controls disappear. Widths in px, with the flex
// `gap-1` (4px) between items folded in:
//
//   tail (voice/send col ~104 + grid gap 8 + px-2 16 + border/rounding ~6) ≈ 134
//   "+" trigger 32 · ModelPicker ~160 · Awareness icon ~32
//   Permission ~131 · Sandbox ~146
//   Knowledge ~36 · Goal ~54 · Workflow menu ~100 · Plan ~66
//   Add-actions row 108 (extra +76 over "+")
//
// Cumulative container width needed to keep each control inline (→ breakpoint):
//   floor  ("+", model, awareness)                         ≈ 366
//   + permission                                            ≈ 501 → 500
//   + sandbox                                               ≈ 647 → 650
//   + knowledge + goal + workflow + plan                    ≈ 829 → 880
//   + add-actions expanded ("+" → 3 inline buttons)         ≈ 905 → 960
//
// Goal + Workflow are semantic mode controls with labels, so they should remain
// visible while there is real horizontal room. The tight tier is intentionally
// below 900px to avoid the "empty toolbar but everything is in +" feel.
export const CHAT_INPUT_OVERFLOW_BREAKPOINT_PX = 960
export const CHAT_INPUT_TIGHT_TOOLBAR_BREAKPOINT_PX = 880
export const CHAT_INPUT_SANDBOX_COLLAPSE_BREAKPOINT_PX = 650
export const CHAT_INPUT_PERMISSION_COLLAPSE_BREAKPOINT_PX = 500

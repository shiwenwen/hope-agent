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
export const CHAT_INPUT_TOOLBAR_FIT_BUFFER_PX = 8
export const CHAT_INPUT_TOOLBAR_EXPAND_BUFFER_PX = 24
export const CHAT_INPUT_TOOLBAR_ITEM_GAP_PX = 4

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

export type ChatInputToolbarGroupKey = keyof typeof CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS
export type ChatInputToolbarGroupWidths = Record<ChatInputToolbarGroupKey, number>

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

function measuredWidth(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.ceil(value)) : 0
}

export function estimateChatInputToolbarLevelWidths({
  currentLevel,
  visibleWidth,
  widths,
}: {
  currentLevel: number
  visibleWidth: number
  widths: ChatInputToolbarGroupWidths
}): number[] {
  const level = clampChatInputToolbarCollapseLevel(currentLevel)
  const gap = CHAT_INPUT_TOOLBAR_ITEM_GAP_PX
  const addDelta = Math.max(
    0,
    measuredWidth(widths.addActions) - measuredWidth(widths.overflowTrigger),
  )
  const semantic = measuredWidth(widths.semanticModes) + gap
  const sandbox = measuredWidth(widths.sandbox) + gap
  const permission = measuredWidth(widths.permission) + gap

  let allInlineWidth = measuredWidth(visibleWidth)
  if (level >= 1) allInlineWidth += addDelta
  if (level >= 2) allInlineWidth += semantic
  if (level >= 3) allInlineWidth += sandbox
  if (level >= 4) allInlineWidth += permission

  const level1 = allInlineWidth - addDelta
  const level2 = level1 - semantic
  const level3 = level2 - sandbox
  const level4 = level3 - permission

  return [allInlineWidth, level1, level2, level3, level4].map(measuredWidth)
}

export function resolveChatInputToolbarCollapseLevel({
  currentLevel,
  availableWidth,
  visibleWidth,
  widths,
}: {
  currentLevel: number
  availableWidth: number
  visibleWidth: number
  widths: ChatInputToolbarGroupWidths
}): number {
  const current = clampChatInputToolbarCollapseLevel(currentLevel)
  const available = measuredWidth(availableWidth)
  if (available <= 0 || measuredWidth(visibleWidth) <= 0) return current

  const levelWidths = estimateChatInputToolbarLevelWidths({
    currentLevel: current,
    visibleWidth,
    widths,
  })

  for (let level = 0; level <= CHAT_INPUT_TOOLBAR_MAX_COLLAPSE_LEVEL; level += 1) {
    const buffer =
      level < current ? CHAT_INPUT_TOOLBAR_EXPAND_BUFFER_PX : CHAT_INPUT_TOOLBAR_FIT_BUFFER_PX
    if (levelWidths[level] + buffer <= available) return level
  }

  return CHAT_INPUT_TOOLBAR_MAX_COLLAPSE_LEVEL
}

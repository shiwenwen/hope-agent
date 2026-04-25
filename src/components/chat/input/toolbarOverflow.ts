export const CHAT_INPUT_OVERFLOW_ACTION_IDS = [
  "attach-files",
  "working-dir",
  "slash-command",
  "incognito",
] as const

export type ChatInputOverflowActionId = (typeof CHAT_INPUT_OVERFLOW_ACTION_IDS)[number]

export function shouldShowIncognitoPresetAction(
  currentSessionId: string | null | undefined,
  hasIncognitoHandler: boolean,
): boolean {
  return !currentSessionId && hasIncognitoHandler
}

export function getChatInputOverflowActionIds(
  currentSessionId: string | null | undefined,
  hasIncognitoHandler: boolean,
): ChatInputOverflowActionId[] {
  const showIncognito = shouldShowIncognitoPresetAction(currentSessionId, hasIncognitoHandler)
  return CHAT_INPUT_OVERFLOW_ACTION_IDS.filter((actionId) => {
    if (actionId === "incognito") return showIncognito
    return true
  })
}

export const CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS = "contents max-[900px]:hidden"
export const CHAT_INPUT_OVERFLOW_MENU_CLASS = "hidden max-[900px]:block"

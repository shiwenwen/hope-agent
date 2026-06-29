export interface ActiveQuickPromptToken {
  anchor: number
  caret: number
  token: string
}

const PARTIAL_QUICK_PROMPT_TOKEN_CHARS = /[^\s#]/

export function detectActiveQuickPrompt(
  input: string,
  caret: number,
): ActiveQuickPromptToken | null {
  if (caret < 1 || caret > input.length) return null
  let i = caret - 1
  while (i >= 0) {
    const c = input[i]
    if (c === "#") {
      const prev = i > 0 ? input[i - 1] : ""
      if (i === 0 || /\s/.test(prev)) {
        return { anchor: i, caret, token: input.slice(i + 1, caret) }
      }
      return null
    }
    if (!PARTIAL_QUICK_PROMPT_TOKEN_CHARS.test(c)) return null
    i--
  }
  return null
}

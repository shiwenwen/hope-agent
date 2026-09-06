export interface AskUserResultAnswer {
  questionId?: string
  question: string
  selected: string[]
  selectedValues?: string[]
  customInput?: string | null
}

export interface AskUserResult {
  answers: AskUserResultAnswer[]
  fallback: AskUserResultAnswer[]
  timedOut: boolean
  cancelled: boolean
}

function parseAnswers(value: unknown): AskUserResultAnswer[] {
  if (!Array.isArray(value)) return []
  return value.flatMap((item: unknown) => {
    if (!item || typeof item !== "object") return []
    const answer = item as Record<string, unknown>
    if (typeof answer.question !== "string" || !Array.isArray(answer.selected)) return []
    const strings = (items: unknown[]) => items.filter((v): v is string => typeof v === "string")
    return [
      {
        questionId: typeof answer.questionId === "string" ? answer.questionId : undefined,
        question: answer.question,
        selected: strings(answer.selected),
        selectedValues: Array.isArray(answer.selectedValues)
          ? strings(answer.selectedValues)
          : undefined,
        customInput:
          typeof answer.customInput === "string" || answer.customInput === null
            ? answer.customInput
            : undefined,
      },
    ]
  })
}

/** Normalize current and historical results without presenting timeout defaults
 * as user answers. Shared by both message-card renderers. */
export function parseAskUserResult(result: string | undefined): AskUserResult | null {
  if (!result) return null
  const trimmed = result.trim()
  const empty = { answers: [], fallback: [], timedOut: false, cancelled: false }
  if (trimmed.startsWith("The user cancelled")) return { ...empty, cancelled: true }
  if (trimmed.startsWith("The questions timed out")) return { ...empty, timedOut: true }
  try {
    const data: unknown = JSON.parse(trimmed)
    if (!data || typeof data !== "object" || Array.isArray(data)) return null
    const value = data as Record<string, unknown>
    if (!Array.isArray(value.answers)) return null
    const timedOut = value.status === "timed_out" || value.timedOut === true
    const cancelled = value.status === "cancelled"
    return {
      answers: timedOut || cancelled ? [] : parseAnswers(value.answers),
      fallback: timedOut ? parseAnswers(value.fallback ?? value.answers) : [],
      timedOut,
      cancelled,
    }
  } catch {
    return null
  }
}

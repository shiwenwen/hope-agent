import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type ChatFocusFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface ChatFocusToast {
  title: string
  description?: string
}

const CHAT_FOCUS_DIAGNOSTIC_MAX_CHARS = 420

export function chatFocusErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return sanitizeDiagnosticText(detail, CHAT_FOCUS_DIAGNOSTIC_MAX_CHARS)
}

export function chatFocusLoadErrorToast(
  t: ChatFocusFeedbackTranslateFn,
  error: unknown,
): ChatFocusToast {
  const detail = chatFocusErrorDetail(error)
  const title = t("chat.openSourceConversationFailed", {
    defaultValue: "Failed to open source conversation",
  })
  if (!detail) return { title }
  return {
    title,
    description: t("chat.openSourceConversationFailedDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function chatFocusMissingSessionToast(t: ChatFocusFeedbackTranslateFn): ChatFocusToast {
  return {
    title: t("chat.openSourceConversationMissing", {
      defaultValue: "Source conversation is no longer available",
    }),
    description: t("chat.openSourceConversationMissingHint", {
      defaultValue:
        "This memory source points to a conversation that was deleted or cannot be found.",
    }),
  }
}

export function chatFocusMissingMessageToast(t: ChatFocusFeedbackTranslateFn): ChatFocusToast {
  return {
    title: t("chat.openSourceMessageMissing", {
      defaultValue: "Source message is no longer available",
    }),
    description: t("chat.openSourceMessageMissingHint", {
      defaultValue:
        "The conversation still exists, but the exact message for this memory source could not be found.",
    }),
  }
}

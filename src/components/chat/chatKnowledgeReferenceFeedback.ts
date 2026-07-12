import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

const CHAT_KNOWLEDGE_REFERENCE_DIAGNOSTIC_MAX_CHARS = 420

export type ChatKnowledgeReferenceTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface ChatKnowledgeReferenceToast {
  title: string
  description?: string
}

export function chatKnowledgeReferenceErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return sanitizeDiagnosticText(detail, CHAT_KNOWLEDGE_REFERENCE_DIAGNOSTIC_MAX_CHARS)
}

export function chatKnowledgeReferenceAttachErrorToast(
  t: ChatKnowledgeReferenceTranslateFn,
  error: unknown,
): ChatKnowledgeReferenceToast {
  const detail = chatKnowledgeReferenceErrorDetail(error)
  const title = t("chat.knowledgeReferenceAttachFailed", {
    defaultValue: "Couldn't attach knowledge space",
  })
  if (!detail) {
    return {
      title,
      description: t("chat.knowledgeReferenceAttachFailedHint", {
        defaultValue:
          "The note reference was inserted, but the assistant may not be able to read it.",
      }),
    }
  }
  return {
    title,
    description: t("chat.knowledgeReferenceAttachFailedDetail", {
      defaultValue:
        "The note reference was inserted, but the assistant may not be able to read it. Details: {{error}}",
      error: detail,
    }),
  }
}

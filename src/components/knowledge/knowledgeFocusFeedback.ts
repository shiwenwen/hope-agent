import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeFocusFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

const KNOWLEDGE_FOCUS_DIAGNOSTIC_MAX_CHARS = 420

export function knowledgeFocusErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return sanitizeDiagnosticText(detail, KNOWLEDGE_FOCUS_DIAGNOSTIC_MAX_CHARS)
}

export function knowledgeFocusErrorDescription(
  t: KnowledgeFocusFeedbackTranslateFn,
  error: unknown,
): string | undefined {
  const detail = knowledgeFocusErrorDetail(error)
  if (!detail) return undefined
  return t("knowledge.focusUnavailableDetail", {
    defaultValue: "Details: {{error}}",
    error: detail,
  })
}

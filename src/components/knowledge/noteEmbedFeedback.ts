import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeEmbedTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

const KNOWLEDGE_EMBED_DIAGNOSTIC_MAX_CHARS = 420

export function knowledgeEmbedDiagnosticText(
  value: string,
  maxChars = KNOWLEDGE_EMBED_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeDiagnosticText(value, maxChars)
}

export function knowledgeEmbedErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? knowledgeEmbedDiagnosticText(detail) : null
}

export function knowledgeEmbedErrorDescription(
  t: KnowledgeEmbedTranslateFn,
  detail: string | null,
): string | null {
  if (!detail) return null
  return t("knowledge.embed.errorDetail", {
    defaultValue: "Details: {{error}}",
    error: detail,
  })
}

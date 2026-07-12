import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

const NOTE_MENTION_DIAGNOSTIC_MAX_CHARS = 420

export function noteMentionErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return sanitizeDiagnosticText(detail, NOTE_MENTION_DIAGNOSTIC_MAX_CHARS)
}

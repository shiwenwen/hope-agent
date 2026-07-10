import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

const WORKSPACE_KNOWLEDGE_DIAGNOSTIC_MAX_CHARS = 420

export function workspaceKnowledgeErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? workspaceKnowledgeDiagnosticText(detail) : null
}

export function workspaceKnowledgeDiagnosticText(
  value: string,
  maxChars = WORKSPACE_KNOWLEDGE_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeDiagnosticText(value, maxChars)
}

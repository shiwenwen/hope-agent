import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeQueryFilingOperation = "generateProposal" | "applyFiling" | "rejectFiling"

export type KnowledgeQueryFilingFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeQueryFilingErrorToast {
  title: string
  description?: string
}

export function knowledgeQueryFilingErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function knowledgeQueryFilingErrorToast(
  operation: KnowledgeQueryFilingOperation,
  t: KnowledgeQueryFilingFeedbackTranslateFn,
  error: unknown,
): KnowledgeQueryFilingErrorToast {
  const title = t(knowledgeQueryFilingOperationKey(operation), {
    defaultValue: knowledgeQueryFilingOperationFallback(operation),
  })
  const detail = knowledgeQueryFilingErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.queryFile.errorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function knowledgeQueryFilingOperationKey(operation: KnowledgeQueryFilingOperation): string {
  switch (operation) {
    case "generateProposal":
      return "knowledge.queryFile.generateFailed"
    case "applyFiling":
      return "knowledge.queryFile.applyFailed"
    case "rejectFiling":
      return "knowledge.queryFile.rejectFailed"
  }
}

function knowledgeQueryFilingOperationFallback(operation: KnowledgeQueryFilingOperation): string {
  switch (operation) {
    case "generateProposal":
      return "Couldn't create filing proposal"
    case "applyFiling":
      return "Couldn't apply filing"
    case "rejectFiling":
      return "Couldn't discard filing"
  }
}

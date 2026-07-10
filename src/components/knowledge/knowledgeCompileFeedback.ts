import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeCompileOperation =
  | "loadRuns"
  | "loadProposals"
  | "startCompile"
  | "runFailed"
  | "cancelRun"
  | "applyProposal"
  | "rejectProposal"

export type KnowledgeCompileFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeCompileOperationErrorToast {
  title: string
  description?: string
}

export function knowledgeCompileErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function knowledgeCompileOperationErrorToast(
  operation: KnowledgeCompileOperation,
  t: KnowledgeCompileFeedbackTranslateFn,
  error: unknown,
): KnowledgeCompileOperationErrorToast {
  const title = t(knowledgeCompileOperationKey(operation), {
    defaultValue: knowledgeCompileOperationFallback(operation),
  })
  const detail = knowledgeCompileErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.compile.errorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function knowledgeCompileOperationKey(operation: KnowledgeCompileOperation): string {
  switch (operation) {
    case "loadRuns":
      return "knowledge.compile.loadFailed"
    case "loadProposals":
      return "knowledge.compile.proposalsLoadFailed"
    case "startCompile":
      return "knowledge.compile.startFailed"
    case "runFailed":
      return "knowledge.compile.failed"
    case "cancelRun":
      return "knowledge.compile.cancelFailed"
    case "applyProposal":
      return "knowledge.compile.applyFailed"
    case "rejectProposal":
      return "knowledge.compile.rejectFailed"
  }
}

function knowledgeCompileOperationFallback(operation: KnowledgeCompileOperation): string {
  switch (operation) {
    case "loadRuns":
      return "Couldn't load source-to-note runs"
    case "loadProposals":
      return "Couldn't load proposals"
    case "startCompile":
      return "Couldn't organize sources into notes"
    case "runFailed":
      return "Source-to-note run failed"
    case "cancelRun":
      return "Couldn't cancel run"
    case "applyProposal":
      return "Couldn't apply proposal"
    case "rejectProposal":
      return "Couldn't reject proposal"
  }
}

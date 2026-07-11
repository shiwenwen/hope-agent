import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type ClaimOwnerOperation =
  | "loadSchema"
  | "loadList"
  | "loadScopeNames"
  | "loadListSummaries"
  | "loadReviewHistory"
  | "restoreArchive"
  | "batchAction"
  | "backfillPlan"
  | "backfillApply"
  | "conflictResolve"
  | "loadDetail"
  | "openEvidenceSource"

export type ClaimOwnerFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface ClaimOwnerOperationErrorToast {
  title: string
  description?: string
}

const CLAIM_OWNER_DIAGNOSTIC_MAX_CHARS = 420

export function claimOwnerOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return claimOwnerDiagnosticText(detail)
}

export function claimOwnerDiagnosticText(
  value: string,
  maxChars = CLAIM_OWNER_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function claimOwnerOperationErrorToast(
  operation: ClaimOwnerOperation,
  t: ClaimOwnerFeedbackTranslateFn,
  error: unknown,
): ClaimOwnerOperationErrorToast {
  const detail = claimOwnerOperationErrorDetail(error)
  const title = t(`settings.claims.operationErrors.${operation}`, {
    defaultValue: claimOwnerOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.claims.operationErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function claimOwnerOperationFallback(operation: ClaimOwnerOperation): string {
  switch (operation) {
    case "loadSchema":
      return "Failed to load structured memory filters"
    case "loadList":
      return "Failed to load structured memory list"
    case "loadScopeNames":
      return "Failed to load memory scope names"
    case "loadListSummaries":
      return "Structured memory list details may be incomplete"
    case "loadReviewHistory":
      return "Failed to load review history"
    case "restoreArchive":
      return "Failed to restore archived memory"
    case "batchAction":
      return "Batch action failed"
    case "backfillPlan":
      return "Failed to compute the backfill plan"
    case "backfillApply":
      return "Backfill failed"
    case "conflictResolve":
      return "Failed to resolve memory conflict"
    case "loadDetail":
      return "Failed to open structured memory"
    case "openEvidenceSource":
      return "Failed to open evidence source"
  }
}

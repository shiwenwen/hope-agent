import { sanitizeDiagnosticText } from "../../../lib/diagnosticRedaction"

export type ClaimReviewActionOperation =
  | "loadQueue"
  | "approve"
  | "markOutdated"
  | "pin"
  | "unpin"
  | "edit"
  | "moveScope"
  | "reject"
  | "forget"

export type ClaimReviewActionFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface ClaimReviewActionErrorToast {
  title: string
  description?: string
}

const CLAIM_REVIEW_ACTION_DIAGNOSTIC_MAX_CHARS = 420

export function claimReviewActionErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? claimReviewActionDiagnosticText(detail) : null
}

export function claimReviewActionDiagnosticText(
  value: string,
  maxChars = CLAIM_REVIEW_ACTION_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeDiagnosticText(value, maxChars)
}

export function claimReviewActionErrorToast(
  operation: ClaimReviewActionOperation,
  t: ClaimReviewActionFeedbackTranslateFn,
  error: unknown,
): ClaimReviewActionErrorToast {
  const detail = claimReviewActionErrorDetail(error)
  const title = t(`dashboard.dreaming.review.errors.${operation}`, {
    defaultValue: claimReviewActionFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("dashboard.dreaming.review.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function claimReviewActionFallback(operation: ClaimReviewActionOperation): string {
  switch (operation) {
    case "loadQueue":
      return "Failed to load review queue"
    case "approve":
      return "Failed to approve memory"
    case "markOutdated":
      return "Failed to mark memory outdated"
    case "pin":
      return "Failed to pin memory"
    case "unpin":
      return "Failed to unpin memory"
    case "edit":
      return "Failed to edit memory"
    case "moveScope":
      return "Failed to move memory scope"
    case "reject":
      return "Failed to reject memory"
    case "forget":
      return "Failed to forget memory"
  }
}

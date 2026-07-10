import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type ClaimClipboardOperation =
  | "copyEvidence"
  | "copyLink"
  | "copyReviewHistoryItem"
  | "copyReviewHistoryExport"

export type ClaimClipboardTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface ClaimClipboardErrorToast {
  title: string
  description?: string
}

const CLAIM_CLIPBOARD_DIAGNOSTIC_MAX_CHARS = 420

export function claimClipboardErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? claimClipboardDiagnosticText(detail) : null
}

export function claimClipboardDiagnosticText(
  value: string,
  maxChars = CLAIM_CLIPBOARD_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function claimClipboardErrorToast(
  operation: ClaimClipboardOperation,
  t: ClaimClipboardTranslateFn,
  error: unknown,
): ClaimClipboardErrorToast {
  const detail = claimClipboardErrorDetail(error)
  const title = t(claimClipboardFailureKey(operation), {
    defaultValue: claimClipboardFailureFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.claims.copyFailureDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function claimClipboardFailureKey(operation: ClaimClipboardOperation): string {
  switch (operation) {
    case "copyEvidence":
      return "settings.claims.copyEvidenceFailed"
    case "copyLink":
      return "settings.claims.copyLinkFailed"
    case "copyReviewHistoryItem":
      return "settings.claims.reviewHistoryCopyItemFailed"
    case "copyReviewHistoryExport":
      return "settings.claims.reviewHistoryExportFailed"
  }
}

function claimClipboardFailureFallback(operation: ClaimClipboardOperation): string {
  switch (operation) {
    case "copyEvidence":
      return "Failed to copy evidence details"
    case "copyLink":
      return "Failed to copy memory link"
    case "copyReviewHistoryItem":
      return "Failed to copy review decision"
    case "copyReviewHistoryExport":
      return "Failed to copy review history"
  }
}

import { sanitizeDiagnosticText } from "../../../lib/diagnosticRedaction"

export type DreamingOperation =
  | "loadDiaries"
  | "loadRuns"
  | "loadRunDetail"
  | "loadDiary"
  | "loadEvidenceQuote"
  | "runNow"
  | "runResolver"
  | "resolverPreflight"

export type DreamingFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface DreamingOperationErrorToast {
  title: string
  description?: string
}

const DREAMING_DIAGNOSTIC_MAX_CHARS = 420

export function dreamingOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? dreamingDiagnosticText(detail) : null
}

export function dreamingDiagnosticText(
  value: string,
  maxChars = DREAMING_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeDiagnosticText(value, maxChars)
}

export function dreamingOperationErrorToast(
  operation: DreamingOperation,
  t: DreamingFeedbackTranslateFn,
  error: unknown,
): DreamingOperationErrorToast {
  const detail = dreamingOperationErrorDetail(error)
  const title = t(`dashboard.dreaming.errors.${operation}`, {
    defaultValue: dreamingOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("dashboard.dreaming.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function dreamingOperationFallback(operation: DreamingOperation): string {
  switch (operation) {
    case "loadDiaries":
      return "Failed to load Dreaming diaries"
    case "loadRuns":
      return "Failed to load Dreaming run history"
    case "loadRunDetail":
      return "Failed to load Dreaming run details"
    case "loadDiary":
      return "Failed to open Dreaming diary"
    case "loadEvidenceQuote":
      return "Failed to load evidence quote"
    case "runNow":
      return "Failed to run Dreaming"
    case "runResolver":
      return "Deep resolve failed"
    case "resolverPreflight":
      return "Failed to load Deep Resolver preflight"
  }
}

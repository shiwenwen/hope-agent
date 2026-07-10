import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type ProfileSnapshotOperation = "openEvidenceSource" | "refresh"

export type ProfileSnapshotFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface ProfileSnapshotOperationErrorToast {
  title: string
  description?: string
}

const PROFILE_SNAPSHOT_DIAGNOSTIC_MAX_CHARS = 420

export function profileSnapshotOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? profileSnapshotDiagnosticText(detail) : null
}

export function profileSnapshotDiagnosticText(
  value: string,
  maxChars = PROFILE_SNAPSHOT_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function profileSnapshotOperationErrorToast(
  operation: ProfileSnapshotOperation,
  t: ProfileSnapshotFeedbackTranslateFn,
  error: unknown,
): ProfileSnapshotOperationErrorToast {
  const detail = profileSnapshotOperationErrorDetail(error)
  const title = t(`settings.profile.operationErrors.${operation}`, {
    defaultValue: profileSnapshotOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.profile.operationErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function profileSnapshotOperationFallback(operation: ProfileSnapshotOperation): string {
  switch (operation) {
    case "openEvidenceSource":
      return "Failed to open profile evidence source"
    case "refresh":
      return "Failed to synthesise profile"
  }
}

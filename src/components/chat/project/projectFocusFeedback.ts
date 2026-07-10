import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type ProjectFocusFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface ProjectFocusToast {
  title: string
  description?: string
}

const PROJECT_FOCUS_DIAGNOSTIC_MAX_CHARS = 420

export function projectFocusErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return sanitizeDiagnosticText(detail, PROJECT_FOCUS_DIAGNOSTIC_MAX_CHARS)
}

export function projectFocusLoadErrorToast(
  t: ProjectFocusFeedbackTranslateFn,
  error: unknown,
): ProjectFocusToast {
  const detail = projectFocusErrorDetail(error)
  const title = t("project.openFromMemoryLoadFailed", {
    defaultValue: "Failed to open project source",
  })
  if (!detail) return { title }
  return {
    title,
    description: t("project.openFromMemoryLoadFailedDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function projectFocusMissingToast(t: ProjectFocusFeedbackTranslateFn): ProjectFocusToast {
  return {
    title: t("project.openFromMemoryMissing", {
      defaultValue: "Project is no longer available",
    }),
    description: t("project.openFromMemoryMissingHint", {
      defaultValue: "The memory source points to a project that was deleted or cannot be found.",
    }),
  }
}

import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

const WORKSPACE_SOURCE_DIAGNOSTIC_MAX_CHARS = 420

export type WorkspaceSourceTranslateFn = (
  key: string,
  defaultValue: string,
  options?: Record<string, unknown>,
) => string

export interface WorkspaceSourceErrorToast {
  title: string
  description?: string
}

export function workspaceSourceErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? workspaceSourceDiagnosticText(detail) : null
}

export function workspaceSourceDiagnosticText(
  value: string,
  maxChars = WORKSPACE_SOURCE_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeDiagnosticText(value, maxChars)
}

export function workspaceSourceOpenErrorToast(
  t: WorkspaceSourceTranslateFn,
  error: unknown,
): WorkspaceSourceErrorToast {
  const detail = workspaceSourceErrorDetail(error)
  const title = t("workspace.openSourceFailed", "Failed to open source")
  if (!detail) return { title }
  return {
    title,
    description: t("workspace.openSourceDetail", "Details: {{error}}", { error: detail }),
  }
}

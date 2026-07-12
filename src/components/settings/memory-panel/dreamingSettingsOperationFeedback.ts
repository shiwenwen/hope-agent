import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type DreamingSettingsOperation =
  | "loadConfig"
  | "saveConfig"
  | "reloadAfterSave"
  | "loadModels"
  | "loadStatus"

export type DreamingSettingsFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface DreamingSettingsOperationErrorToast {
  title: string
  description?: string
}

const DREAMING_SETTINGS_DIAGNOSTIC_MAX_CHARS = 420

export function dreamingSettingsOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? dreamingSettingsDiagnosticText(detail) : null
}

export function dreamingSettingsDiagnosticText(
  value: string,
  maxChars = DREAMING_SETTINGS_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function dreamingSettingsOperationErrorToast(
  operation: DreamingSettingsOperation,
  t: DreamingSettingsFeedbackTranslateFn,
  error: unknown,
): DreamingSettingsOperationErrorToast {
  const detail = dreamingSettingsOperationErrorDetail(error)
  const title = t(`settings.dreaming.errors.${operation}`, {
    defaultValue: dreamingSettingsOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.dreaming.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function dreamingSettingsOperationFallback(operation: DreamingSettingsOperation): string {
  switch (operation) {
    case "loadConfig":
      return "Failed to load Dreaming settings"
    case "saveConfig":
      return "Failed to save Dreaming settings"
    case "reloadAfterSave":
      return "Failed to reload Dreaming settings after save failed"
    case "loadModels":
      return "Failed to load Dreaming model list"
    case "loadStatus":
      return "Failed to load Dreaming status"
  }
}

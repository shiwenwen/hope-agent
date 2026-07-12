import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeMaintenanceSettingsOperation = "load" | "save" | "runNow"

export type KnowledgeMaintenanceSettingsTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeMaintenanceSettingsErrorToast {
  title: string
  description?: string
}

export function knowledgeMaintenanceSettingsErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function knowledgeMaintenanceSettingsErrorToast(
  operation: KnowledgeMaintenanceSettingsOperation,
  t: KnowledgeMaintenanceSettingsTranslateFn,
  error: unknown,
): KnowledgeMaintenanceSettingsErrorToast {
  const detail = knowledgeMaintenanceSettingsErrorDetail(error)
  const title = t(`settings.knowledgeMaintenance.errors.${operation}`, {
    defaultValue: knowledgeMaintenanceSettingsFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.knowledgeMaintenance.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function knowledgeMaintenanceSettingsFallback(
  operation: KnowledgeMaintenanceSettingsOperation,
): string {
  switch (operation) {
    case "load":
      return "Failed to load autonomous maintenance settings"
    case "save":
      return "Failed to save autonomous maintenance settings"
    case "runNow":
      return "Failed to run autonomous maintenance"
  }
}

import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type SpriteSettingsOperation = "load" | "save" | "toggle"

export type SpriteSettingsTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface SpriteSettingsErrorToast {
  title: string
  description?: string
}

export function spriteSettingsErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function spriteSettingsErrorToast(
  operation: SpriteSettingsOperation,
  t: SpriteSettingsTranslateFn,
  error: unknown,
): SpriteSettingsErrorToast {
  const detail = spriteSettingsErrorDetail(error)
  const title = t(`settings.sprite.errors.${operation}`, {
    defaultValue: spriteSettingsFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.sprite.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function spriteSettingsFallback(operation: SpriteSettingsOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load sprite settings"
    case "save":
      return "Failed to save sprite settings"
    case "toggle":
      return "Failed to update sprite mode"
  }
}

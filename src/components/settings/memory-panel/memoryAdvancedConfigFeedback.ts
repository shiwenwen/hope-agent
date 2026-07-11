import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type MemoryAdvancedConfigOperation =
  | "load"
  | "saveDedup"
  | "saveHybrid"
  | "saveMmr"
  | "saveCache"
  | "saveMultimodal"
  | "saveSelection"
  | "saveTemporal"

export type MemoryAdvancedConfigFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryAdvancedConfigOperationErrorToast {
  title: string
  description?: string
}

const MEMORY_ADVANCED_CONFIG_DIAGNOSTIC_MAX_CHARS = 420

export function memoryAdvancedConfigOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? memoryAdvancedConfigDiagnosticText(detail) : null
}

export function memoryAdvancedConfigDiagnosticText(
  value: string,
  maxChars = MEMORY_ADVANCED_CONFIG_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryAdvancedConfigOperationErrorToast(
  operation: MemoryAdvancedConfigOperation,
  t: MemoryAdvancedConfigFeedbackTranslateFn,
  error: unknown,
): MemoryAdvancedConfigOperationErrorToast {
  const detail = memoryAdvancedConfigOperationErrorDetail(error)
  const title = t(`settings.memoryAdvancedConfigErrors.${operation}`, {
    defaultValue: memoryAdvancedConfigOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryAdvancedConfigErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function memoryAdvancedConfigOperationFallback(operation: MemoryAdvancedConfigOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load memory search tuning"
    case "saveDedup":
      return "Failed to save dedup thresholds"
    case "saveHybrid":
      return "Failed to save hybrid search tuning"
    case "saveMmr":
      return "Failed to save MMR tuning"
    case "saveCache":
      return "Failed to save embedding cache tuning"
    case "saveMultimodal":
      return "Failed to save multimodal memory tuning"
    case "saveSelection":
      return "Failed to save LLM memory selection tuning"
    case "saveTemporal":
      return "Failed to save temporal decay tuning"
  }
}

import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type MemoryExtractOperation = "load" | "saveGlobal" | "saveAgent" | "resetAgent"

export type MemoryExtractFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryExtractOperationError {
  title: string
  description?: string
}

const MEMORY_EXTRACT_DIAGNOSTIC_MAX_CHARS = 420

export function memoryExtractOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? memoryExtractDiagnosticText(detail) : null
}

export function memoryExtractDiagnosticText(
  value: string,
  maxChars = MEMORY_EXTRACT_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryExtractOperationError(
  operation: MemoryExtractOperation,
  t: MemoryExtractFeedbackTranslateFn,
  error: unknown,
): MemoryExtractOperationError {
  const detail = memoryExtractOperationErrorDetail(error)
  const title = t(`settings.memoryExtract.errors.${operation}`, {
    defaultValue: memoryExtractOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryExtract.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function memoryExtractOperationFallback(operation: MemoryExtractOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load memory learning settings"
    case "saveGlobal":
      return "Failed to save memory learning settings"
    case "saveAgent":
      return "Failed to save agent memory learning override"
    case "resetAgent":
      return "Failed to reset agent memory learning override"
  }
}

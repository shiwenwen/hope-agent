import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type MemoryBudgetOperation = "load" | "save"

export type MemoryBudgetFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryBudgetOperationErrorToast {
  title: string
  description?: string
}

const MEMORY_BUDGET_DIAGNOSTIC_MAX_CHARS = 420

export function memoryBudgetOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? memoryBudgetDiagnosticText(detail) : null
}

export function memoryBudgetDiagnosticText(
  value: string,
  maxChars = MEMORY_BUDGET_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryBudgetOperationErrorToast(
  operation: MemoryBudgetOperation,
  t: MemoryBudgetFeedbackTranslateFn,
  error: unknown,
): MemoryBudgetOperationErrorToast {
  const detail = memoryBudgetOperationErrorDetail(error)
  const title = t(`settings.memoryBudget.errors.${operation}`, {
    defaultValue: memoryBudgetOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryBudget.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function memoryBudgetOperationFallback(operation: MemoryBudgetOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load memory budget"
    case "save":
      return "Failed to save memory budget"
  }
}

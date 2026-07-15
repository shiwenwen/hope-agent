import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type CoreMemoryOperation = "loadGlobal" | "loadAgent" | "saveGlobal" | "saveAgent"

export type CoreMemoryFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface CoreMemoryOperationErrorToast {
  title: string
  description?: string
}

const CORE_MEMORY_DIAGNOSTIC_MAX_CHARS = 420

export function coreMemoryOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? coreMemoryDiagnosticText(detail) : null
}

export function coreMemoryDiagnosticText(
  value: string,
  maxChars = CORE_MEMORY_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function coreMemoryOperationErrorToast(
  operation: CoreMemoryOperation,
  t: CoreMemoryFeedbackTranslateFn,
  error: unknown,
): CoreMemoryOperationErrorToast {
  const detail = coreMemoryOperationErrorDetail(error)
  const title = t(`settings.coreMemoryErrors.${operation}`, {
    defaultValue: coreMemoryOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.coreMemoryErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function coreMemoryOperationForScope(
  action: "load" | "save",
  scope: "global" | "agent" | "project",
): CoreMemoryOperation {
  // Project core memory uses the same scoped repository/error surface as Agent
  // core memory. Keep the legacy operation keys until the feedback strings are
  // migrated without making the new Project editor type-unsafe.
  if (action === "load") return scope === "global" ? "loadGlobal" : "loadAgent"
  return scope === "global" ? "saveGlobal" : "saveAgent"
}

function coreMemoryOperationFallback(operation: CoreMemoryOperation): string {
  switch (operation) {
    case "loadGlobal":
      return "Failed to load global core memory"
    case "loadAgent":
      return "Failed to load agent core memory"
    case "saveGlobal":
      return "Failed to save global core memory"
    case "saveAgent":
      return "Failed to save agent core memory"
  }
}

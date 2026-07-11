import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type MemoryCrudOperation =
  | "load"
  | "loadAgents"
  | "loadStats"
  | "focus"
  | "checkDuplicate"
  | "add"
  | "mergeDuplicate"
  | "update"
  | "delete"
  | "deleteBatch"
  | "pin"
  | "unpin"
  | "reembedSelected"
  | "export"

export type MemoryCrudFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryCrudOperationErrorToast {
  title: string
  description?: string
}

const MEMORY_CRUD_DIAGNOSTIC_MAX_CHARS = 420

export function memoryCrudOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? memoryCrudDiagnosticText(detail) : null
}

export function memoryCrudDiagnosticText(
  value: string,
  maxChars = MEMORY_CRUD_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryCrudOperationErrorToast(
  operation: MemoryCrudOperation,
  t: MemoryCrudFeedbackTranslateFn,
  error: unknown,
): MemoryCrudOperationErrorToast {
  const detail = memoryCrudOperationErrorDetail(error)
  const title = t(`settings.memoryCrudErrors.${operation}`, {
    defaultValue: memoryCrudOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryCrudErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function memoryCrudOperationFallback(operation: MemoryCrudOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load memories"
    case "loadAgents":
      return "Failed to load memory agent list"
    case "loadStats":
      return "Failed to load memory stats"
    case "focus":
      return "Failed to open memory"
    case "checkDuplicate":
      return "Failed to check for similar memories"
    case "add":
      return "Failed to add memory"
    case "mergeDuplicate":
      return "Failed to update existing memory"
    case "update":
      return "Failed to update memory"
    case "delete":
      return "Failed to delete memory"
    case "deleteBatch":
      return "Failed to delete selected memories"
    case "pin":
      return "Failed to pin memory"
    case "unpin":
      return "Failed to unpin memory"
    case "reembedSelected":
      return "Failed to rebuild selected memory vectors"
    case "export":
      return "Failed to copy memory export"
  }
}

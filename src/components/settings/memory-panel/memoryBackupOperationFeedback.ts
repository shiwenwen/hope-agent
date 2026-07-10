import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type MemoryBackupOperation =
  | "export"
  | "preview"
  | "copyPreviewDiagnostics"
  | "restoreLegacy"
  | "restoreStructured"

export type MemoryBackupFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryBackupOperationErrorToast {
  title: string
  description?: string
}

const MEMORY_BACKUP_DIAGNOSTIC_MAX_CHARS = 420

export function memoryBackupDiagnosticText(
  value: string,
  maxChars = MEMORY_BACKUP_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryBackupOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? memoryBackupDiagnosticText(detail) : null
}

export function memoryBackupOperationErrorToast(
  operation: MemoryBackupOperation,
  t: MemoryBackupFeedbackTranslateFn,
  error: unknown,
): MemoryBackupOperationErrorToast {
  const detail = memoryBackupOperationErrorDetail(error)
  const title = t(`settings.memoryBackupErrors.${operation}`, {
    defaultValue: memoryBackupOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryBackupErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function memoryBackupOperationFallback(operation: MemoryBackupOperation): string {
  switch (operation) {
    case "export":
      return "Failed to export memory backup"
    case "preview":
      return "Failed to preview memory backup"
    case "copyPreviewDiagnostics":
      return "Failed to copy backup preview diagnostics"
    case "restoreLegacy":
      return "Failed to restore memory backup"
    case "restoreStructured":
      return "Failed to restore structured memory"
  }
}

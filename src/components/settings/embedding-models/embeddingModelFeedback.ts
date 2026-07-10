import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type EmbeddingModelOperation = "load" | "save" | "test" | "setDefault" | "delete"

export type EmbeddingModelFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface EmbeddingModelOperationErrorToast {
  title: string
  description?: string
}

const EMBEDDING_MODEL_DIAGNOSTIC_MAX_CHARS = 420

export function embeddingModelDiagnosticText(
  value: string,
  maxChars = EMBEDDING_MODEL_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeDiagnosticText(value, maxChars)
}

export function embeddingModelErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? embeddingModelDiagnosticText(detail) : null
}

export function embeddingModelOperationErrorToast(
  operation: EmbeddingModelOperation,
  t: EmbeddingModelFeedbackTranslateFn,
  error: unknown,
): EmbeddingModelOperationErrorToast {
  const detail = embeddingModelErrorDetail(error)
  const title = t(`settings.embeddingModels.errors.${operation}`, {
    defaultValue: embeddingModelOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.embeddingModels.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function embeddingModelOperationFallback(operation: EmbeddingModelOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load embedding model configs"
    case "save":
      return "Failed to save embedding model config"
    case "test":
      return "Embedding connection test failed"
    case "setDefault":
      return "Failed to switch default memory model"
    case "delete":
      return "Failed to delete embedding model config"
  }
}

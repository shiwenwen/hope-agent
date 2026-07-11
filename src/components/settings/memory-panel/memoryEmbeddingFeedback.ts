import { embeddingModelDiagnosticText } from "../embedding-models/embeddingModelFeedback"

export type MemoryEmbeddingOperation =
  | "load"
  | "setDefault"
  | "disable"
  | "reembedStart"
  | "reembedRetry"
  | "reembedCancel"
  | "reembedJobFailed"
  | "localAssistantRefresh"
  | "localAssistantStart"
  | "localAssistantCancel"
  | "localAssistantLogs"
  | "localAssistantOpenDownload"

export type MemoryEmbeddingFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryEmbeddingOperationErrorToast {
  title: string
  description?: string
}

export function memoryEmbeddingErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? embeddingModelDiagnosticText(detail) : null
}

export function memoryEmbeddingOperationErrorToast(
  operation: MemoryEmbeddingOperation,
  t: MemoryEmbeddingFeedbackTranslateFn,
  error: unknown,
): MemoryEmbeddingOperationErrorToast {
  const detail = memoryEmbeddingErrorDetail(error)
  const title = t(`settings.memoryEmbeddingErrors.${operation}`, {
    defaultValue: memoryEmbeddingOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryEmbeddingErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function memoryEmbeddingOperationErrorText(
  operation: MemoryEmbeddingOperation,
  t: MemoryEmbeddingFeedbackTranslateFn,
  error: unknown,
): string {
  const toast = memoryEmbeddingOperationErrorToast(operation, t, error)
  return [toast.title, toast.description].filter(Boolean).join("\n")
}

function memoryEmbeddingOperationFallback(operation: MemoryEmbeddingOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load memory embedding settings"
    case "setDefault":
      return "Failed to switch memory embedding model"
    case "disable":
      return "Failed to disable vector search"
    case "reembedStart":
      return "Failed to start memory vector rebuild"
    case "reembedRetry":
      return "Failed to retry memory vector rebuild"
    case "reembedCancel":
      return "Failed to cancel memory vector rebuild"
    case "reembedJobFailed":
      return "Memory vector rebuild failed"
    case "localAssistantRefresh":
      return "Failed to refresh local embedding assistant"
    case "localAssistantStart":
      return "Failed to start local embedding setup"
    case "localAssistantCancel":
      return "Failed to cancel local embedding setup"
    case "localAssistantLogs":
      return "Failed to load local embedding setup logs"
    case "localAssistantOpenDownload":
      return "Failed to open local embedding download page"
  }
}

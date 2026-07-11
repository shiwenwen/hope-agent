import { externalMemoryProviderDiagnosticText } from "./externalMemoryProviderReadiness"

export type ExternalMemoryProviderOperation =
  | "load"
  | "save"
  | "preflight"
  | "sync"
  | "loadCredentials"
  | "saveCredentials"
  | "clearCredentials"
  | "copyPreflightDiagnostics"
  | "copySyncDiagnostics"

export type ExternalMemoryProviderFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface ExternalMemoryProviderOperationErrorToast {
  title: string
  description?: string
}

export function externalMemoryProviderOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? externalMemoryProviderDiagnosticText(detail) : null
}

export function externalMemoryProviderOperationErrorToast(
  operation: ExternalMemoryProviderOperation,
  t: ExternalMemoryProviderFeedbackTranslateFn,
  error: unknown,
): ExternalMemoryProviderOperationErrorToast {
  const detail = externalMemoryProviderOperationErrorDetail(error)
  const title = t(`settings.memoryExternalProviderErrors.${operation}`, {
    defaultValue: externalMemoryProviderOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryExternalProviderErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function externalMemoryProviderOperationFallback(
  operation: ExternalMemoryProviderOperation,
): string {
  switch (operation) {
    case "load":
      return "Failed to load external memory providers"
    case "save":
      return "Failed to save external memory providers"
    case "preflight":
      return "Failed to load external memory provider preflight"
    case "sync":
      return "Failed to run external memory provider sync check"
    case "loadCredentials":
      return "Failed to load external memory provider connection"
    case "saveCredentials":
      return "Failed to save external memory provider connection"
    case "clearCredentials":
      return "Failed to clear external memory provider connection"
    case "copyPreflightDiagnostics":
      return "Failed to copy external memory provider preflight"
    case "copySyncDiagnostics":
      return "Failed to copy external memory provider sync report"
  }
}

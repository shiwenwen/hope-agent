import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type AgentLoadFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface AgentLoadOperationErrorToast {
  title: string
  description?: string
}

export type AgentOperation = "load" | "save" | "delete"

const AGENT_LOAD_DIAGNOSTIC_MAX_CHARS = 420

export function agentLoadOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return sanitizeDiagnosticText(detail, AGENT_LOAD_DIAGNOSTIC_MAX_CHARS)
}

export function agentOperationErrorToast(
  operation: AgentOperation,
  t: AgentLoadFeedbackTranslateFn,
  error: unknown,
): AgentLoadOperationErrorToast {
  const detail = agentLoadOperationErrorDetail(error)
  const title = agentOperationTitle(operation, t)
  if (!detail) return { title }
  return {
    title,
    description: t("settings.agentLoadFailedDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function agentLoadOperationErrorToast(
  t: AgentLoadFeedbackTranslateFn,
  error: unknown,
): AgentLoadOperationErrorToast {
  return agentOperationErrorToast("load", t, error)
}

function agentOperationTitle(
  operation: AgentOperation,
  t: AgentLoadFeedbackTranslateFn,
): string {
  switch (operation) {
    case "load":
      return t("settings.agentLoadFailed", {
        defaultValue: "Failed to load agent",
      })
    case "save":
      return t("common.saveFailed", {
        defaultValue: "Save failed",
      })
    case "delete":
      return t("common.deleteFailed", {
        defaultValue: "Delete failed",
      })
  }
}

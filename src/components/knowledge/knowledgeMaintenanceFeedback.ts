import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeMaintenanceOperation =
  | "runNow"
  | "rebuildEvidence"
  | "applyProposal"
  | "rejectProposal"
  | "rejectAll"

export type KnowledgeMaintenanceFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeMaintenanceErrorToast {
  title: string
  description?: string
}

export function knowledgeMaintenanceErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function knowledgeMaintenanceErrorToast(
  operation: KnowledgeMaintenanceOperation,
  t: KnowledgeMaintenanceFeedbackTranslateFn,
  error: unknown,
): KnowledgeMaintenanceErrorToast {
  const title = t(knowledgeMaintenanceOperationKey(operation), {
    defaultValue: knowledgeMaintenanceOperationFallback(operation),
  })
  const detail = knowledgeMaintenanceErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.maintenance.errorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function knowledgeMaintenanceOperationKey(operation: KnowledgeMaintenanceOperation): string {
  switch (operation) {
    case "runNow":
      return "knowledge.maintenance.runFailed"
    case "rebuildEvidence":
      return "knowledge.maintenance.evidenceRebuildFailed"
    case "applyProposal":
      return "knowledge.maintenance.applyFailed"
    case "rejectProposal":
      return "knowledge.maintenance.rejectFailed"
    case "rejectAll":
      return "knowledge.maintenance.rejectAllFailed"
  }
}

function knowledgeMaintenanceOperationFallback(operation: KnowledgeMaintenanceOperation): string {
  switch (operation) {
    case "runNow":
      return "Couldn't run maintenance"
    case "rebuildEvidence":
      return "Couldn't rebuild evidence index"
    case "applyProposal":
      return "Couldn't apply suggestion"
    case "rejectProposal":
      return "Couldn't dismiss suggestion"
    case "rejectAll":
      return "Couldn't dismiss all suggestions"
  }
}

import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type MemoryRepairOperation =
  | "loadHealth"
  | "rebuildFts"
  | "rebuildClaimFts"
  | "repairClaimGraph"
  | "repairExperienceGraph"
  | "recoverDreamingState"
  | "createDbSnapshot"
  | "copyDbSnapshotPath"
  | "copyDbSnapshotVerification"
  | "restorePreview"
  | "copyRestorePreview"
  | "restoreDbSnapshot"
  | "copyHealthDiagnostics"

export type MemoryRepairFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryRepairOperationErrorToast {
  title: string
  description?: string
}

const MEMORY_REPAIR_DIAGNOSTIC_MAX_CHARS = 420

export function memoryRepairOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? memoryRepairDiagnosticText(detail) : null
}

export function memoryRepairDiagnosticText(
  value: string,
  maxChars = MEMORY_REPAIR_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryRepairOperationErrorToast(
  operation: MemoryRepairOperation,
  t: MemoryRepairFeedbackTranslateFn,
  error: unknown,
): MemoryRepairOperationErrorToast {
  const detail = memoryRepairOperationErrorDetail(error)
  const title = t(`settings.memoryRepairErrors.${operation}`, {
    defaultValue: memoryRepairOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryRepairErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function memoryRepairOperationFallback(operation: MemoryRepairOperation): string {
  switch (operation) {
    case "loadHealth":
      return "Failed to load memory health"
    case "rebuildFts":
      return "Failed to rebuild keyword index"
    case "rebuildClaimFts":
      return "Failed to rebuild structured index"
    case "repairClaimGraph":
      return "Failed to repair claim graph links"
    case "repairExperienceGraph":
      return "Failed to repair experience links"
    case "recoverDreamingState":
      return "Failed to recover Dreaming state"
    case "createDbSnapshot":
      return "Failed to create database snapshot"
    case "copyDbSnapshotPath":
      return "Failed to copy snapshot path"
    case "copyDbSnapshotVerification":
      return "Failed to copy snapshot verification"
    case "restorePreview":
      return "Snapshot restore preflight failed"
    case "copyRestorePreview":
      return "Failed to copy restore preflight report"
    case "restoreDbSnapshot":
      return "Failed to restore database snapshot"
    case "copyHealthDiagnostics":
      return "Failed to copy memory health report"
  }
}

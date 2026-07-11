import type { MemoryBackupStructuredRestoreResult } from "./types"
import { memoryBackupDiagnosticText } from "./memoryBackupOperationFeedback"

type StructuredRestoreTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export function memoryBackupStructuredRestoreSummaryOptions(
  result: MemoryBackupStructuredRestoreResult,
): Record<string, unknown> {
  return {
    defaultValue:
      "{{claims}} claims, {{profiles}} profiles, {{episodes}} episodes, {{procedures}} procedures, {{history}} history events, {{review}} need review, {{links}} links restored, {{skippedProfiles}} profile conflicts skipped, {{skippedHistory}} history skipped",
    claims: result.restoredClaims,
    profiles: result.restoredProfileSnapshots,
    episodes: result.restoredEpisodes ?? 0,
    procedures: result.restoredProcedures ?? 0,
    history: result.restoredExperienceHistory ?? 0,
    review: result.restoredClaimsNeedingReview,
    links: result.restoredClaimLinks,
    skippedProfiles: result.skippedProfileScopeConflicts,
    skippedHistory: result.skippedExperienceHistoryUnmapped ?? 0,
  }
}

export function hasMemoryBackupStructuredRestorePartial(
  result: MemoryBackupStructuredRestoreResult,
): boolean {
  return (
    result.errors.length > 0 ||
    result.failedClaims > 0 ||
    result.failedProfileSnapshots > 0 ||
    (result.failedEpisodes ?? 0) > 0 ||
    (result.failedProcedures ?? 0) > 0 ||
    (result.skippedExperienceHistoryUnmapped ?? 0) > 0
  )
}

export function memoryBackupStructuredRestoreErrorDescription(
  result: MemoryBackupStructuredRestoreResult,
  t: StructuredRestoreTranslateFn,
): string | undefined {
  const detail = result.errors[0]?.trim()
  if (!detail) return undefined
  return t("settings.memoryBackupRestoreStructuredFirstError", {
    defaultValue: "First restore error: {{error}}",
    error: memoryBackupDiagnosticText(detail),
  })
}

export function memoryBackupStructuredRestorePartialDescription(
  result: MemoryBackupStructuredRestoreResult,
  t: StructuredRestoreTranslateFn,
): string | undefined {
  const errorDescription = memoryBackupStructuredRestoreErrorDescription(result, t)
  if (errorDescription) return errorDescription
  const skippedHistory = result.skippedExperienceHistoryUnmapped ?? 0
  if (skippedHistory > 0) {
    return t("settings.memoryBackupRestoreStructuredSkippedHistory", {
      defaultValue: "{{count}} experience/workflow history event(s) could not be mapped.",
      count: skippedHistory,
    })
  }
  return undefined
}

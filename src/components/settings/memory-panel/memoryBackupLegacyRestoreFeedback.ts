import type { MemoryBackupRestoreResult } from "./types"
import { memoryBackupDiagnosticText } from "./memoryBackupOperationFeedback"

type LegacyRestoreTranslateFn = (key: string, options?: Record<string, unknown>) => string

export function memoryBackupLegacyRestoreSummaryOptions(
  result: MemoryBackupRestoreResult,
): Record<string, unknown> {
  return {
    defaultValue:
      "{{created}} created, {{attachments}} attachments restored, {{history}} history events, {{skipped}} already present, {{skippedHistory}} history skipped, {{failed}} failed",
    created: result.importResult.created,
    attachments: result.restoredAttachments,
    history: result.restoredLegacyHistory ?? 0,
    skipped: result.skippedExactMatches + result.importResult.skippedDuplicate,
    skippedHistory: result.skippedLegacyHistoryUnmapped ?? 0,
    failed: result.importResult.failed,
  }
}

export function hasMemoryBackupLegacyRestorePartial(result: MemoryBackupRestoreResult): boolean {
  return result.importResult.failed > 0 || (result.skippedLegacyHistoryUnmapped ?? 0) > 0
}

export function memoryBackupLegacyRestoreErrorDescription(
  result: MemoryBackupRestoreResult,
  t: LegacyRestoreTranslateFn,
): string | undefined {
  const detail = result.importResult.errors[0]?.trim()
  if (!detail) return undefined
  return t("settings.memoryBackupRestoreLegacyFirstError", {
    defaultValue: "First restore error: {{error}}",
    error: memoryBackupDiagnosticText(detail),
  })
}

export function memoryBackupLegacyRestorePartialDescription(
  result: MemoryBackupRestoreResult,
  t: LegacyRestoreTranslateFn,
): string | undefined {
  const errorDescription = memoryBackupLegacyRestoreErrorDescription(result, t)
  if (errorDescription) return errorDescription
  const skippedHistory = result.skippedLegacyHistoryUnmapped ?? 0
  if (skippedHistory > 0) {
    return t("settings.memoryBackupRestoreLegacySkippedHistory", {
      defaultValue: "{{count}} legacy memory history event(s) could not be mapped.",
      count: skippedHistory,
    })
  }
  return undefined
}

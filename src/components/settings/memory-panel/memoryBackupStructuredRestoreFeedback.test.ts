import { describe, expect, it } from "vitest"

import {
  hasMemoryBackupStructuredRestorePartial,
  memoryBackupStructuredRestoreErrorDescription,
  memoryBackupStructuredRestorePartialDescription,
  memoryBackupStructuredRestoreSummaryOptions,
} from "./memoryBackupStructuredRestoreFeedback"
import type { MemoryBackupImportPreview, MemoryBackupStructuredRestoreResult } from "./types"

const emptyPreview = {
  valid: true,
  schemaVersion: "memory-backup/v2",
  exportedAt: null,
  appVersion: null,
  sourceManifest: null,
  currentStats: { total: 0, byType: {}, bySource: {}, withEmbedding: 0 },
  legacyMemoryCount: 0,
  legacyExactMatches: 0,
  legacyImportCandidates: 0,
  legacyDuplicateInBundle: 0,
  legacyHistoryCount: 0,
  legacyHistoryRestorable: 0,
  legacyHistorySkippedUnmapped: 0,
  attachmentRefCount: 0,
  attachmentPayloadCount: 0,
  attachmentChunkCount: 0,
  attachmentChunkedRefCount: 0,
  attachmentExternalRefCount: 0,
  attachmentExternalAvailableCount: 0,
  attachmentPayloadBytes: 0,
  attachmentMissingCount: 0,
  claimCount: 0,
  claimIdMatches: 0,
  claimRestorePlan: {
    total: 0,
    existingById: 0,
    exactMatches: 0,
    importCandidates: 0,
    conflictingCandidates: 0,
    needsReviewCandidates: 0,
    archivedCandidates: 0,
    supersededCandidates: 0,
    expiredCandidates: 0,
    manualEvidenceRows: 0,
    byType: {},
    byStatus: {},
    conflictExamples: [],
    previewOnly: true,
  },
  evidenceCount: 0,
  claimLinkCount: 0,
  profileSnapshotCount: 0,
  profileRestorePlan: {
    total: 0,
    matchingScopes: 0,
    exactMatches: 0,
    importCandidates: 0,
    conflictingScopeCandidates: 0,
    byScopeType: {},
    previewOnly: true,
  },
  episodeCount: 0,
  episodeIdMatches: 0,
  episodeExactMatches: 0,
  episodeImportCandidates: 0,
  procedureCount: 0,
  procedureIdMatches: 0,
  procedureExactMatches: 0,
  procedureImportCandidates: 0,
  experienceHistoryCount: 0,
  experienceHistoryRestorable: 0,
  experienceHistorySkippedUnmapped: 0,
  unsupportedSections: [],
  issues: [],
  nextSteps: [],
} satisfies MemoryBackupImportPreview

function result(
  patch: Partial<MemoryBackupStructuredRestoreResult> = {},
): MemoryBackupStructuredRestoreResult {
  return {
    preview: emptyPreview,
    restoredClaims: 0,
    restoredClaimsNeedingReview: 0,
    skippedClaimIdMatches: 0,
    skippedClaimExactMatches: 0,
    restoredEvidenceRows: 0,
    restoredClaimLinks: 0,
    skippedClaimLinks: 0,
    failedClaims: 0,
    restoredProfileSnapshots: 0,
    skippedProfileExactMatches: 0,
    skippedProfileScopeConflicts: 0,
    failedProfileSnapshots: 0,
    restoredEpisodes: 0,
    skippedEpisodeIdMatches: 0,
    skippedEpisodeExactMatches: 0,
    failedEpisodes: 0,
    restoredProcedures: 0,
    skippedProcedureIdMatches: 0,
    skippedProcedureExactMatches: 0,
    failedProcedures: 0,
    restoredExperienceHistory: 0,
    skippedExperienceHistoryUnmapped: 0,
    errors: [],
    ...patch,
  }
}

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory backup structured restore feedback", () => {
  it("includes experience history counts in the success summary options", () => {
    expect(
      memoryBackupStructuredRestoreSummaryOptions(
        result({
          restoredClaims: 2,
          restoredProfileSnapshots: 1,
          restoredEpisodes: 3,
          restoredProcedures: 4,
          restoredExperienceHistory: 5,
          restoredClaimsNeedingReview: 1,
          restoredClaimLinks: 6,
          skippedProfileScopeConflicts: 7,
          skippedExperienceHistoryUnmapped: 8,
        }),
      ),
    ).toMatchObject({
      claims: 2,
      profiles: 1,
      episodes: 3,
      procedures: 4,
      history: 5,
      review: 1,
      links: 6,
      skippedProfiles: 7,
      skippedHistory: 8,
    })
  })

  it("treats unmapped experience history as a partial restore warning", () => {
    const restoreResult = result({ skippedExperienceHistoryUnmapped: 2 })
    const t = (_key: string, options?: Record<string, unknown>) =>
      `${options?.count} 条经验 / 流程历史无法映射。`

    expect(hasMemoryBackupStructuredRestorePartial(restoreResult)).toBe(true)
    expect(memoryBackupStructuredRestorePartialDescription(restoreResult, t)).toBe(
      "2 条经验 / 流程历史无法映射。",
    )
  })

  it("prefers concrete restore errors over derived skipped history copy", () => {
    const restoreResult = result({
      skippedExperienceHistoryUnmapped: 2,
      errors: ["procedure proc_1 skipped: missing trigger"],
    })
    const t = (key: string, options?: Record<string, unknown>) =>
      key === "settings.memoryBackupRestoreStructuredFirstError"
        ? interpolate("首条恢复错误：{{error}}", options)
        : "fallback"

    expect(memoryBackupStructuredRestorePartialDescription(restoreResult, t)).toBe(
      "首条恢复错误：procedure proc_1 skipped: missing trigger",
    )
  })

  it("formats restore errors without exposing sensitive detail", () => {
    const restoreResult = result({ errors: ["claim c_1 failed: duplicate evidence"] })
    const zhT = (_key: string, options?: Record<string, unknown>) =>
      interpolate("首条恢复错误：{{error}}", options)
    const fallbackT = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryBackupStructuredRestoreErrorDescription(restoreResult, zhT)).toBe(
      "首条恢复错误：claim c_1 failed: duplicate evidence",
    )
    expect(memoryBackupStructuredRestoreErrorDescription(restoreResult, fallbackT)).toBe(
      "First restore error: claim c_1 failed: duplicate evidence",
    )
    expect(
      memoryBackupStructuredRestoreErrorDescription(
        result({
          errors: [
            "claim restore failed Authorization: Bearer restore-token api_key=backup-key",
          ],
        }),
        zhT,
      ),
    ).toBe(
      "首条恢复错误：claim restore failed Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryBackupStructuredRestoreErrorDescription(result({ errors: ["   "] }), zhT),
    ).toBeUndefined()
    expect(memoryBackupStructuredRestoreErrorDescription(result(), zhT)).toBeUndefined()
  })
})

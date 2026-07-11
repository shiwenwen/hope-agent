import { describe, expect, it } from "vitest"

import {
  hasMemoryBackupLegacyRestorePartial,
  memoryBackupLegacyRestoreErrorDescription,
  memoryBackupLegacyRestorePartialDescription,
  memoryBackupLegacyRestoreSummaryOptions,
} from "./memoryBackupLegacyRestoreFeedback"
import type { MemoryBackupImportPreview, MemoryBackupRestoreResult } from "./types"

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

function result(patch: Partial<MemoryBackupRestoreResult> = {}): MemoryBackupRestoreResult {
  return {
    preview: emptyPreview,
    importResult: {
      created: 0,
      skippedDuplicate: 0,
      failed: 0,
      errors: [],
    },
    attemptedLegacyMemories: 0,
    skippedExactMatches: 0,
    skippedDuplicateInBundle: 0,
    skippedAttachmentRefs: 0,
    restoredAttachments: 0,
    restoredLegacyHistory: 0,
    skippedLegacyHistoryUnmapped: 0,
    previewOnlyClaims: 0,
    previewOnlyProfileSnapshots: 0,
    ...patch,
  }
}

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory backup legacy restore feedback", () => {
  it("includes restored and skipped legacy history in the success summary options", () => {
    expect(
      memoryBackupLegacyRestoreSummaryOptions(
        result({
          importResult: {
            created: 2,
            skippedDuplicate: 3,
            failed: 4,
            errors: [],
          },
          restoredAttachments: 5,
          restoredLegacyHistory: 6,
          skippedExactMatches: 7,
          skippedLegacyHistoryUnmapped: 8,
        }),
      ),
    ).toMatchObject({
      created: 2,
      attachments: 5,
      history: 6,
      skipped: 10,
      skippedHistory: 8,
      failed: 4,
    })
  })

  it("treats unmapped legacy history as a partial restore warning", () => {
    const restoreResult = result({ skippedLegacyHistoryUnmapped: 2 })
    const t = (_key: string, options?: Record<string, unknown>) =>
      `${options?.count} 条普通记忆历史无法映射。`

    expect(hasMemoryBackupLegacyRestorePartial(restoreResult)).toBe(true)
    expect(memoryBackupLegacyRestorePartialDescription(restoreResult, t)).toBe(
      "2 条普通记忆历史无法映射。",
    )
  })

  it("prefers concrete import errors over derived skipped history copy", () => {
    const restoreResult = result({
      importResult: {
        created: 0,
        skippedDuplicate: 0,
        failed: 1,
        errors: ["memory 42 failed: invalid scope"],
      },
      skippedLegacyHistoryUnmapped: 2,
    })
    const t = (key: string, options?: Record<string, unknown>) =>
      key === "settings.memoryBackupRestoreLegacyFirstError"
        ? interpolate("首条恢复错误：{{error}}", options)
        : "fallback"

    expect(memoryBackupLegacyRestorePartialDescription(restoreResult, t)).toBe(
      "首条恢复错误：memory 42 failed: invalid scope",
    )
  })

  it("formats concrete import errors without exposing sensitive detail", () => {
    const restoreResult = result({
      importResult: {
        created: 0,
        skippedDuplicate: 0,
        failed: 1,
        errors: ["memory 9 failed: duplicate source"],
      },
    })
    const zhT = (_key: string, options?: Record<string, unknown>) =>
      interpolate("首条恢复错误：{{error}}", options)
    const fallbackT = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryBackupLegacyRestoreErrorDescription(restoreResult, zhT)).toBe(
      "首条恢复错误：memory 9 failed: duplicate source",
    )
    expect(memoryBackupLegacyRestoreErrorDescription(restoreResult, fallbackT)).toBe(
      "First restore error: memory 9 failed: duplicate source",
    )
    expect(
      memoryBackupLegacyRestoreErrorDescription(
        result({
          importResult: {
            created: 0,
            skippedDuplicate: 0,
            failed: 1,
            errors: ["restore failed token=restore-secret passphrase=backup-secret"],
          },
        }),
        zhT,
      ),
    ).toBe("首条恢复错误：restore failed token=[redacted] passphrase=[redacted]")
    expect(
      memoryBackupLegacyRestoreErrorDescription(
        result({
          importResult: {
            created: 0,
            skippedDuplicate: 0,
            failed: 1,
            errors: ["   "],
          },
        }),
        zhT,
      ),
    ).toBeUndefined()
    expect(memoryBackupLegacyRestoreErrorDescription(result(), zhT)).toBeUndefined()
  })
})

import { describe, expect, it } from "vitest"

import type { MemoryBackupImportPreview } from "./types"
import {
  buildMemoryBackupRestorePlan,
  formatMemoryBackupRestorePlanActionLabel,
  formatMemoryBackupRestorePlanRowDetail,
  formatMemoryBackupRestorePlanRowLabel,
  formatMemoryBackupRestorePlanSummaryNextStep,
  formatMemoryBackupRestorePlanSummaryTitle,
  hasMemoryBackupStructuredRestoreCandidates,
  summarizeMemoryBackupRestorePlan,
} from "./memoryBackupRestorePlan"

function preview(patch: Partial<MemoryBackupImportPreview> = {}): MemoryBackupImportPreview {
  return {
    valid: true,
    schemaVersion: "memory-backup/v2",
    exportedAt: "2026-07-06T09:00:00.000Z",
    appVersion: "0.16.0",
    sourceManifest: null,
    currentStats: {
      total: 0,
      byType: {},
      bySource: {},
      withEmbedding: 0,
    },
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
    ...patch,
  }
}

describe("memory backup restore plan", () => {
  it("treats experience-only backups as structured restore candidates", () => {
    const plan = preview({
      episodeCount: 1,
      episodeImportCandidates: 1,
      procedureCount: 1,
      procedureImportCandidates: 1,
    })

    expect(hasMemoryBackupStructuredRestoreCandidates(plan)).toBe(true)
    expect(buildMemoryBackupRestorePlan(plan).map((row) => [row.id, row.action, row.count])).toEqual([
      ["episode-import", "restore", 1],
      ["procedure-import", "restore", 1],
    ])
  })

  it("shows profile conflicts as skipped unless the override is enabled", () => {
    const plan = preview({
      profileSnapshotCount: 1,
      profileRestorePlan: {
        total: 1,
        matchingScopes: 1,
        exactMatches: 0,
        importCandidates: 1,
        conflictingScopeCandidates: 1,
        byScopeType: { global: 1 },
        previewOnly: true,
      },
    })

    expect(buildMemoryBackupRestorePlan(plan)).toEqual([
      expect.objectContaining({
        id: "profile-conflict",
        action: "skip",
        tone: "muted",
        count: 1,
      }),
    ])
    expect(buildMemoryBackupRestorePlan(plan, { allowProfileScopeConflicts: true })).toEqual([
      expect.objectContaining({
        id: "profile-conflict",
        action: "override",
        tone: "warn",
        count: 1,
      }),
    ])
  })

  it("summarizes review risk before skips and override risk before review", () => {
    const plan = preview({
      claimRestorePlan: {
        total: 1,
        existingById: 0,
        exactMatches: 0,
        importCandidates: 1,
        conflictingCandidates: 1,
        needsReviewCandidates: 0,
        archivedCandidates: 0,
        supersededCandidates: 0,
        expiredCandidates: 0,
        manualEvidenceRows: 0,
        byType: { preference: 1 },
        byStatus: { active: 1 },
        conflictExamples: [],
        previewOnly: true,
      },
      profileRestorePlan: {
        total: 1,
        matchingScopes: 1,
        exactMatches: 0,
        importCandidates: 1,
        conflictingScopeCandidates: 1,
        byScopeType: { global: 1 },
        previewOnly: true,
      },
    })

    expect(summarizeMemoryBackupRestorePlan(buildMemoryBackupRestorePlan(plan))).toMatchObject({
      kind: "needs_review",
      reviewCount: 1,
      overrideCount: 0,
    })
    expect(
      summarizeMemoryBackupRestorePlan(
        buildMemoryBackupRestorePlan(plan, { allowProfileScopeConflicts: true }),
      ),
    ).toMatchObject({
      kind: "override_enabled",
      overrideCount: 1,
    })
  })

  it("keeps invalid backups blocked with a diagnostic row", () => {
    const plan = preview({
      valid: false,
      issues: [{ severity: "error", code: "bad_schema", message: "Bad schema" }],
    })

    expect(hasMemoryBackupStructuredRestoreCandidates(plan)).toBe(false)
    expect(buildMemoryBackupRestorePlan(plan)).toEqual([
      expect.objectContaining({
        id: "invalid",
        action: "blocked",
        tone: "danger",
        count: 1,
      }),
    ])
  })

  it("formats rows and summaries through i18n keys with English fallback", () => {
    const plan = preview({
      legacyImportCandidates: 2,
      profileRestorePlan: {
        total: 1,
        matchingScopes: 1,
        exactMatches: 0,
        importCandidates: 1,
        conflictingScopeCandidates: 1,
        byScopeType: { global: 1 },
        previewOnly: true,
      },
    })
    const rows = buildMemoryBackupRestorePlan(plan, { allowProfileScopeConflicts: true })
    const summary = summarizeMemoryBackupRestorePlan(rows)
    const translations: Record<string, string> = {
      "settings.memoryBackupRestorePlanRows.legacy-import.label": "缺失的普通记忆",
      "settings.memoryBackupRestorePlanRows.profile-conflict.overrideDetail":
        "覆盖开关已开启；相同作用域会作为更新画像导入。",
      "settings.memoryBackupRestorePlanActions.override": "覆盖",
      "settings.memoryBackupRestorePlanSummary.override_enabled.title": "画像覆盖已开启",
      "settings.memoryBackupRestorePlanSummary.override_enabled.nextStep":
        "确认这些备份画像应成为最新版本后再继续。",
    }
    const t = (key: string, fallback: string) => translations[key] ?? fallback

    expect(formatMemoryBackupRestorePlanRowLabel(rows[0], t)).toBe("缺失的普通记忆")
    expect(formatMemoryBackupRestorePlanRowDetail(rows[0], t)).toContain(
      "Restored only when using",
    )
    expect(formatMemoryBackupRestorePlanRowDetail(rows[1], t)).toBe(
      "覆盖开关已开启；相同作用域会作为更新画像导入。",
    )
    expect(formatMemoryBackupRestorePlanSummaryTitle(summary, t)).toBe("画像覆盖已开启")
    expect(formatMemoryBackupRestorePlanSummaryNextStep(summary, t)).toBe(
      "确认这些备份画像应成为最新版本后再继续。",
    )
    expect(formatMemoryBackupRestorePlanActionLabel("override", t)).toBe("覆盖")
    expect(formatMemoryBackupRestorePlanActionLabel("restore", t)).toBe("Restore")
  })
})

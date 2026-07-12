import { describe, expect, it } from "vitest"

import type { MemoryBackupClaimRestorePlan, MemoryBackupProfileRestorePlan } from "./types"
import {
  formatMemoryBackupAlreadyPresentSummary,
  formatMemoryBackupAttachmentSummary,
  formatMemoryBackupClaimConflictHeader,
  formatMemoryBackupClaimStatus,
  formatMemoryBackupClaimType,
  formatMemoryBackupClaimPlanSummary,
  formatMemoryBackupExperiencePlanSummary,
  formatMemoryBackupHistorySummary,
  formatMemoryBackupClaimScope,
  formatMemoryBackupProfilePlanSummary,
} from "./memoryBackupPreviewSummary"

const translations: Record<string, string> = {
  "settings.memoryBackupPreviewSummary.memories": "条记忆",
  "settings.memoryBackupPreviewSummary.claimIds": "个 claim ID",
  "settings.memoryBackupPreviewSummary.historyMappable": "条事件可映射",
  "settings.memoryBackupPreviewSummary.historySkipped": "条跳过",
  "settings.memoryBackupPreviewSummary.candidates": "个候选",
  "settings.memoryBackupPreviewSummary.exactMatches": "个精确匹配",
  "settings.memoryBackupPreviewSummary.readyToRestore": "可恢复",
  "settings.memoryBackupPreviewSummary.willNeedReview": "条需要审核",
  "settings.memoryBackupPreviewSummary.needsReview": "条本身待审核",
  "settings.memoryBackupPreviewSummary.manualEvidence": "条手动证据",
  "settings.memoryBackupPreviewSummary.matchingScopes": "个匹配作用域",
  "settings.memoryBackupPreviewSummary.scopeConflictsSkippedByDefault":
    "个作用域冲突默认跳过",
  "settings.memoryBackupPreviewSummary.episodeCandidates": "条经验候选",
  "settings.memoryBackupPreviewSummary.procedureCandidates": "条流程候选",
  "settings.memoryBackupPreviewSummary.packed": "已打包",
  "settings.memoryBackupPreviewSummary.chunked": "个分片载荷",
  "settings.memoryBackupPreviewSummary.verifiedSidecar": "个已验证 sidecar",
  "settings.memoryBackupPreviewSummary.sidecarMetadata": "个 sidecar 元数据",
  "settings.memoryBackupPreviewSummary.referenceOnly": "个仅引用",
  "settings.memoryScopeGlobal": "全局",
  "settings.memoryScopeAgent": "Agent",
  "settings.memoryScopeProject": "项目",
  "settings.claimType_preference": "偏好",
  "settings.claims.status.active": "生效中",
  "settings.claims.status.needs_review": "待审核",
}

const t = (key: string, fallback: string) => translations[key] ?? fallback

describe("memory backup preview summary", () => {
  it("formats already-present and history summaries with localized labels", () => {
    expect(
      formatMemoryBackupAlreadyPresentSummary(
        { legacyExactMatches: 3, claimIdMatches: 2 },
        t,
      ),
    ).toBe("3 条记忆 · 2 个 claim ID")
    expect(formatMemoryBackupHistorySummary(4, 5, 1, t)).toBe("4/5 条事件可映射 · 1 条跳过")
  })

  it("formats claim and profile restore plan summaries", () => {
    const claimPlan: MemoryBackupClaimRestorePlan = {
      total: 8,
      existingById: 1,
      exactMatches: 2,
      importCandidates: 3,
      conflictingCandidates: 1,
      needsReviewCandidates: 2,
      archivedCandidates: 0,
      supersededCandidates: 0,
      expiredCandidates: 0,
      manualEvidenceRows: 4,
      byType: {},
      byStatus: {},
      conflictExamples: [],
      previewOnly: true,
    }
    const profilePlan: MemoryBackupProfileRestorePlan = {
      total: 4,
      matchingScopes: 2,
      exactMatches: 1,
      importCandidates: 3,
      conflictingScopeCandidates: 1,
      byScopeType: {},
      previewOnly: true,
    }

    expect(formatMemoryBackupClaimPlanSummary(claimPlan, t)).toBe(
      "3 个候选 · 2 个精确匹配 · 可恢复 · 1 条需要审核 · 2 条本身待审核 · 4 条手动证据",
    )
    expect(formatMemoryBackupProfilePlanSummary(profilePlan, t)).toBe(
      "3 个候选 · 2 个匹配作用域 · 可恢复 · 1 个作用域冲突默认跳过",
    )
  })

  it("formats experience and attachment summaries without leaking English fragments", () => {
    expect(
      formatMemoryBackupExperiencePlanSummary(
        {
          episodeImportCandidates: 1,
          procedureImportCandidates: 2,
          episodeExactMatches: 3,
          procedureExactMatches: 4,
          experienceHistoryCount: 5,
          experienceHistoryRestorable: 4,
          experienceHistorySkippedUnmapped: 1,
        },
        t,
      ),
    ).toBe("1 条经验候选 · 2 条流程候选 · 7 个精确匹配 · 可恢复 · 4/5 条事件可映射 · 1 条跳过")
    expect(
      formatMemoryBackupAttachmentSummary(
        {
          attachmentPayloadCount: 1,
          attachmentChunkedRefCount: 2,
          attachmentExternalAvailableCount: 1,
          attachmentRefCount: 6,
          attachmentExternalRefCount: 3,
          attachmentMissingCount: 2,
        },
        t,
      ),
    ).toBe("4/6 已打包 · 2 个分片载荷 · 1 个已验证 sidecar · 3 个 sidecar 元数据 · 2 个仅引用")
  })

  it("keeps English fallback labels when no translator is provided", () => {
    expect(formatMemoryBackupHistorySummary(1, 2, 0)).toBe("1/2 events mappable")
    expect(
      formatMemoryBackupAttachmentSummary({
        attachmentPayloadCount: 1,
        attachmentChunkedRefCount: 0,
        attachmentExternalAvailableCount: 0,
        attachmentRefCount: 2,
        attachmentExternalRefCount: 0,
        attachmentMissingCount: 1,
      }),
    ).toBe("1/2 packed · 1 reference-only")
  })

  it("formats claim conflict metadata with localized enum labels", () => {
    expect(
      formatMemoryBackupClaimConflictHeader(
        {
          scope: "project:hope-agent",
          claimType: "preference",
          subject: "user",
          predicate: "prefers_editor",
        },
        t,
      ),
    ).toBe("项目: hope-agent · 偏好 · user prefers editor")
    expect(formatMemoryBackupClaimScope("agent:coder", t)).toBe("Agent: coder")
    expect(formatMemoryBackupClaimType("project_fact")).toBe("project fact")
    expect(formatMemoryBackupClaimStatus("needs_review", t)).toBe("待审核")
  })
})

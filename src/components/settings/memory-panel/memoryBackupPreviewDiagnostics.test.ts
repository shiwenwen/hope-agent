import { describe, expect, it, vi } from "vitest"

import {
  formatMemoryBackupPreviewDiagnostics,
  formatMemoryBackupPreviewIssueMessage,
  formatMemoryBackupPreviewNextStep,
} from "./memoryBackupPreviewDiagnostics"
import type { MemoryBackupImportPreview } from "./types"

vi.useFakeTimers()
vi.setSystemTime(new Date("2026-07-07T10:00:00.000Z"))

const preview: MemoryBackupImportPreview = {
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
  legacyMemoryCount: 10,
  legacyExactMatches: 2,
  legacyImportCandidates: 6,
  legacyDuplicateInBundle: 2,
  legacyHistoryCount: 5,
  legacyHistoryRestorable: 4,
  legacyHistorySkippedUnmapped: 1,
  attachmentRefCount: 3,
  attachmentPayloadCount: 1,
  attachmentChunkCount: 2,
  attachmentChunkedRefCount: 1,
  attachmentExternalRefCount: 1,
  attachmentExternalAvailableCount: 1,
  attachmentPayloadBytes: 4096,
  attachmentMissingCount: 1,
  claimCount: 4,
  claimIdMatches: 1,
  claimRestorePlan: {
    total: 4,
    existingById: 1,
    exactMatches: 1,
    importCandidates: 2,
    conflictingCandidates: 1,
    needsReviewCandidates: 1,
    archivedCandidates: 0,
    supersededCandidates: 0,
    expiredCandidates: 0,
    manualEvidenceRows: 3,
    byType: { preference: 2, project_fact: 2 },
    byStatus: { active: 2, needs_review: 2 },
    conflictExamples: [
      {
        incomingClaimId: "incoming-1",
        existingClaimId: "existing-1",
        scope: "global",
        claimType: "preference",
        subject: "user",
        predicate: "prefers_editor",
        incomingObject: "vim",
        existingObject: "vscode",
        incomingContent: "User prefers vim.",
        existingContent: "User prefers VS Code.",
      },
    ],
    previewOnly: true,
  },
  evidenceCount: 8,
  claimLinkCount: 2,
  profileSnapshotCount: 2,
  profileRestorePlan: {
    total: 2,
    matchingScopes: 1,
    exactMatches: 0,
    importCandidates: 1,
    conflictingScopeCandidates: 1,
    byScopeType: { global: 1, project: 1 },
    previewOnly: true,
  },
  episodeCount: 2,
  episodeIdMatches: 1,
  episodeExactMatches: 0,
  episodeImportCandidates: 1,
  procedureCount: 1,
  procedureIdMatches: 0,
  procedureExactMatches: 0,
  procedureImportCandidates: 1,
  experienceHistoryCount: 3,
  experienceHistoryRestorable: 2,
  experienceHistorySkippedUnmapped: 1,
  unsupportedSections: ["future_section"],
  issues: [{ severity: "warning", code: "sidecar_missing", message: "One sidecar is missing." }],
  nextSteps: ["Restore structured memory, then review conflicts."],
}

describe("formatMemoryBackupPreviewDiagnostics", () => {
  it("formats restore availability and conservative safety notes", () => {
    const markdown = formatMemoryBackupPreviewDiagnostics(preview, {
      sourceLabel: "backup.zip",
      allowProfileScopeConflicts: false,
    })

    expect(markdown).toContain("# Memory Backup Restore Preview")
    expect(markdown).toContain("- Source: backup.zip")
    expect(markdown).toContain("- Generated: 2026-07-07T10:00:00.000Z")
    expect(markdown).toContain("- Restore missing legacy memories: yes")
    expect(markdown).toContain("- Restore structured memory: yes")
    expect(markdown).toContain("- Profile scope conflict override: disabled")
    expect(markdown).toContain("## Restore Decision Plan")
    expect(markdown).toContain("- Summary: Some restored items need review")
    expect(markdown).toContain("- Next step: Restore structured memory, then open Review Inbox")
    expect(markdown).toContain("- Action totals: Restore=16, Review=2, Skip=11")
    expect(markdown).toContain("- [Restore] Missing legacy memories: 6")
    expect(markdown).toContain("- [Review] Claims routed to Review Inbox: 2")
    expect(markdown).toContain("- Scope conflicts will restore: no")
    expect(markdown).toContain("- Claim conflicts are restored as Review Inbox items")
  })

  it("includes claim conflict, experience, attachment, issue, and next-step details", () => {
    const markdown = formatMemoryBackupPreviewDiagnostics(preview)

    expect(markdown).toContain("- Conflicts routed to review: 1")
    expect(markdown).toContain("1. Global · preference · user prefers editor")
    expect(markdown).toContain("- Incoming: vim")
    expect(markdown).toContain("- Existing: vscode")
    expect(markdown).toContain("- Experience history mappable: 2/3")
    expect(markdown).toContain("- Missing attachments: 1")
    expect(markdown).toContain("- [warning] sidecar_missing: One sidecar is missing.")
    expect(markdown).toContain("- Restore structured memory, then review conflicts.")
  })

  it("reflects explicit profile scope conflict override", () => {
    const markdown = formatMemoryBackupPreviewDiagnostics(preview, {
      allowProfileScopeConflicts: true,
    })

    expect(markdown).toContain("- Profile scope conflict override: enabled")
    expect(markdown).toContain("- Scope conflicts will restore: yes")
  })

  it("uses the shared structured restore availability for experience-only backups", () => {
    const markdown = formatMemoryBackupPreviewDiagnostics({
      ...preview,
      legacyImportCandidates: 0,
      legacyHistoryCount: 0,
      legacyHistoryRestorable: 0,
      legacyHistorySkippedUnmapped: 0,
      claimRestorePlan: {
        ...preview.claimRestorePlan,
        importCandidates: 0,
        conflictingCandidates: 0,
        needsReviewCandidates: 0,
        manualEvidenceRows: 0,
        conflictExamples: [],
      },
      profileRestorePlan: {
        ...preview.profileRestorePlan,
        importCandidates: 0,
        conflictingScopeCandidates: 0,
      },
      episodeImportCandidates: 1,
      procedureImportCandidates: 0,
      experienceHistoryRestorable: 0,
    })

    expect(markdown).toContain("- Restore missing legacy memories: no")
    expect(markdown).toContain("- Restore structured memory: yes")
    expect(markdown).toContain("- [Restore] Episodes: 1")
  })

  it("orders diagnostic count maps and unsupported sections deterministically", () => {
    const markdown = formatMemoryBackupPreviewDiagnostics({
      ...preview,
      claimRestorePlan: {
        ...preview.claimRestorePlan,
        byType: { task_pattern: 1, project_fact: 3, preference: 3 },
        byStatus: { needs_review: 1, active: 3 },
      },
      profileRestorePlan: {
        ...preview.profileRestorePlan,
        byScopeType: { project: 1, agent: 2, global: 2 },
      },
      unsupportedSections: ["zeta_section", "alpha_section", "middle_section"],
    })

    const preferenceIndex = markdown.indexOf("- preference: 3")
    const projectFactIndex = markdown.indexOf("- project fact: 3")
    const taskPatternIndex = markdown.indexOf("- task pattern: 1")
    expect(preferenceIndex).toBeGreaterThan(-1)
    expect(projectFactIndex).toBeGreaterThan(preferenceIndex)
    expect(taskPatternIndex).toBeGreaterThan(projectFactIndex)

    const activeIndex = markdown.indexOf("- active: 3")
    const reviewIndex = markdown.indexOf("- needs review: 1")
    expect(activeIndex).toBeGreaterThan(-1)
    expect(reviewIndex).toBeGreaterThan(activeIndex)

    const agentIndex = markdown.indexOf("- Agent: 2")
    const globalIndex = markdown.indexOf("- Global: 2")
    const projectIndex = markdown.indexOf("- Project: 1")
    expect(agentIndex).toBeGreaterThan(-1)
    expect(globalIndex).toBeGreaterThan(agentIndex)
    expect(projectIndex).toBeGreaterThan(globalIndex)

    const alphaIndex = markdown.indexOf("- alpha_section")
    const middleIndex = markdown.indexOf("- middle_section")
    const zetaIndex = markdown.indexOf("- zeta_section")
    expect(alphaIndex).toBeGreaterThan(-1)
    expect(middleIndex).toBeGreaterThan(alphaIndex)
    expect(zetaIndex).toBeGreaterThan(middleIndex)
  })

  it("localizes restore decision plan rows in copied diagnostics", () => {
    const translations: Record<string, string> = {
      "settings.memoryBackupRestorePlanSummary.needs_review.title": "部分恢复项需要审核",
      "settings.memoryBackupRestorePlanSummary.needs_review.detail":
        "冲突或优先审核的结构化记忆不会立刻影响回答。",
      "settings.memoryBackupRestorePlanSummary.needs_review.nextStep":
        "恢复结构化记忆后，先打开待审核收件箱确认。",
      "settings.memoryBackupRestorePlanReport.restoreDecisionPlan": "恢复决策计划",
      "settings.memoryBackupRestorePlanReport.summary": "摘要",
      "settings.memoryBackupRestorePlanReport.detail": "详情",
      "settings.memoryBackupRestorePlanReport.nextStep": "下一步",
      "settings.memoryBackupRestorePlanReport.actionTotals": "动作汇总",
      "settings.memoryBackupRestorePlanActions.restore": "恢复",
      "settings.memoryBackupRestorePlanActions.review": "审核",
      "settings.memoryBackupRestorePlanActions.skip": "跳过",
      "settings.memoryBackupRestorePlanActions.override": "覆盖",
      "settings.memoryBackupRestorePlanActions.blocked": "阻止",
      "settings.memoryBackupRestorePlanRows.legacy-import.label": "缺失的普通记忆",
      "settings.memoryBackupRestorePlanRows.legacy-import.detail":
        "只会补回缺失项；精确匹配不会被覆盖。",
      "settings.memoryBackupRestorePlanRows.claim-review.label": "进入审核的结构化记忆",
      "settings.memoryBackupRestorePlanRows.claim-review.detail":
        "冲突或需要先审的 claim 必须经用户确认。",
      "settings.memoryBackupPreviewReport.title": "记忆备份恢复预览",
      "settings.memoryBackupPreviewReport.source": "来源",
      "settings.memoryBackupPreviewReport.sourceDefault": "备份文件",
      "settings.memoryBackupPreviewReport.generated": "生成时间",
      "settings.memoryBackupPreviewReport.valid": "有效",
      "settings.memoryBackupPreviewReport.yes": "是",
      "settings.memoryBackupPreviewReport.no": "否",
      "settings.memoryBackupPreviewReport.restoreAvailability": "恢复可用性",
      "settings.memoryBackupPreviewReport.restoreMissingLegacyMemories": "恢复缺失的普通记忆",
      "settings.memoryBackupPreviewReport.restoreStructuredMemory": "恢复结构化记忆",
      "settings.memoryBackupPreviewReport.profileScopeConflictOverride": "画像作用域冲突覆盖",
      "settings.memoryBackupPreviewReport.structuredClaims": "结构化 Claim",
      "settings.memoryBackupPreviewReport.conflictsRoutedToReview": "进入审核的冲突",
      "settings.memoryBackupPreviewReport.claimTypes": "Claim 类型",
      "settings.memoryBackupPreviewReport.claimStatuses": "Claim 状态",
      "settings.memoryBackupPreviewReport.profileScopeTypes": "画像作用域类型",
      "settings.memoryBackupPreviewReport.incoming": "导入项",
      "settings.memoryBackupPreviewReport.existing": "本地已有",
      "settings.memoryBackupPreviewReport.experienceAndWorkflows": "经验与流程",
      "settings.memoryBackupPreviewReport.experienceHistoryMappable": "可映射经验历史",
      "settings.memoryBackupPreviewReport.attachments": "附件",
      "settings.memoryBackupPreviewReport.missingAttachments": "缺失附件",
      "settings.memoryBackupPreviewReport.safetyNotes": "安全说明",
      "settings.memoryBackupPreviewReport.safetyReadOnly":
        "此报告只来自只读备份预览结果。",
      "settings.memoryBackupPreviewReport.safetyClaimConflicts":
        "Claim 冲突会进入待审核收件箱，不会静默生效。",
      "settings.memoryBackupPreviewIssues.attachmentsReferenceOnly":
        "个附件引用只有路径，无法在此机器上恢复",
      "settings.memoryBackupPreviewNextSteps.noImportableChanges": "没有发现可导入的记忆变化。",
      "settings.memoryBackupPreviewNextSteps.largeAttachmentsNeedSidecars":
        "个大附件有 sidecar 元数据，但仍需要外部载荷文件才能纳入恢复。",
      "settings.memoryScopeGlobal": "全局",
      "settings.memoryScopeProject": "项目",
      "settings.claimType_preference": "偏好",
      "settings.claimType_project_fact": "项目事实",
      "settings.claims.status.active": "生效中",
      "settings.claims.status.needs_review": "待审核",
    }
    const markdown = formatMemoryBackupPreviewDiagnostics(
      {
        ...preview,
        issues: [
          {
            severity: "info",
            code: "attachments_reference_only",
            message: "1 attachment path(s) are present as references only and cannot be restored on this machine",
          },
        ],
        nextSteps: [
          "No importable memory changes were found.",
          "3 large attachment(s) have sidecar metadata but still need external payload files before restore can include them.",
        ],
      },
      {
        t: (key, fallback) => translations[key] ?? fallback,
      },
    )

    expect(markdown).toContain("# 记忆备份恢复预览")
    expect(markdown).toContain("- 来源: 备份文件")
    expect(markdown).toContain("- 有效: 是")
    expect(markdown).toContain("## 恢复可用性")
    expect(markdown).toContain("- 恢复缺失的普通记忆: 是")
    expect(markdown).toContain("## 结构化 Claim")
    expect(markdown).toContain("- 进入审核的冲突: 1")
    expect(markdown).toContain("### Claim 类型")
    expect(markdown).toContain("- 偏好: 2")
    expect(markdown).toContain("- 项目事实: 2")
    expect(markdown).toContain("### Claim 状态")
    expect(markdown).toContain("- 生效中: 2")
    expect(markdown).toContain("- 待审核: 2")
    expect(markdown).toContain("1. 全局 · 偏好 · user prefers editor")
    expect(markdown).toContain("- 导入项: vim")
    expect(markdown).toContain("- 本地已有: vscode")
    expect(markdown).toContain("### 画像作用域类型")
    expect(markdown).toContain("- 全局: 1")
    expect(markdown).toContain("- 项目: 1")
    expect(markdown).toContain("## 恢复决策计划")
    expect(markdown).toContain("- 摘要: 部分恢复项需要审核")
    expect(markdown).toContain("- 详情: 冲突或优先审核的结构化记忆不会立刻影响回答。")
    expect(markdown).toContain("- 下一步: 恢复结构化记忆后，先打开待审核收件箱确认。")
    expect(markdown).toContain("- 动作汇总: 恢复=16, 审核=2, 跳过=11, 覆盖=0, 阻止=0")
    expect(markdown).toContain("- [恢复] 缺失的普通记忆: 6")
    expect(markdown).toContain("- [审核] 进入审核的结构化记忆: 2")
    expect(markdown).toContain("## 经验与流程")
    expect(markdown).toContain("- 可映射经验历史: 2/3")
    expect(markdown).toContain("## 附件")
    expect(markdown).toContain("- 缺失附件: 1")
    expect(markdown).toContain("## 安全说明")
    expect(markdown).toContain(
      "- [info] attachments_reference_only: 1 个附件引用只有路径，无法在此机器上恢复",
    )
    expect(markdown).toContain("- 没有发现可导入的记忆变化。")
    expect(markdown).toContain(
      "- 3 个大附件有 sidecar 元数据，但仍需要外部载荷文件才能纳入恢复。",
    )
    expect(markdown).toContain("- 此报告只来自只读备份预览结果。")
    expect(markdown).toContain("- Claim 冲突会进入待审核收件箱，不会静默生效。")
  })

  it("formats known preview issues and next steps while preserving unknown text", () => {
    const translations: Record<string, string> = {
      "settings.memoryBackupPreviewIssues.invalidJson": "备份文件不是有效 JSON",
      "settings.memoryBackupPreviewIssues.encryptedPlaintextInvalid":
        "加密备份已解密，但解密后的 bundle 不是有效的 Hope Agent 记忆备份",
      "settings.memoryBackupPreviewIssues.attachmentsReferenceOnly":
        "个附件引用只有路径，无法在此机器上恢复",
      "settings.memoryBackupPreviewIssues.attachmentSidecarMissing": "附件 sidecar 缺失",
      "settings.memoryBackupPreviewIssues.attachmentSidecarSizeMismatch":
        "附件 sidecar 大小不匹配",
      "settings.memoryBackupPreviewIssues.attachmentSidecarTooLarge":
        "附件 sidecar 超过恢复上限",
      "settings.memoryBackupPreviewIssues.attachmentSidecarReadFailed":
        "附件 sidecar 无法读取",
      "settings.memoryBackupPreviewIssues.attachmentSidecarChecksumMismatch":
        "附件 sidecar 校验和不匹配",
      "settings.memoryBackupPreviewIssues.attachmentSidecarDuplicateMemoryId":
        "多个 sidecar 指向同一条记忆",
      "settings.memoryBackupPreviewNextSteps.chooseValidBackup":
        "请选择有效的 Hope Agent 记忆备份 JSON 或 ZIP 文件。",
      "settings.memoryBackupPreviewNextSteps.legacyImportCandidates":
        "条普通记忆候选可在用户确认后导入。",
      "settings.memoryBackupPreviewNextSteps.experienceHistoryRestorable":
        "条经验 / 流程历史可在目标记录映射后恢复。",
      "settings.memoryBackupPreviewNextSteps.keepOriginalAttachmentFiles":
        "部分附件引用没有打包载荷，请保留原始文件。",
      "settings.memoryBackupPreviewNextSteps.enterEncryptedBackupPassphrase":
        "请输入备份口令后再预览或恢复此加密备份。",
      "settings.memoryBackupPreviewNextSteps.checkEncryptedBackupPassphrase":
        "请检查备份口令，或选择未损坏的加密备份。",
      "settings.memoryBackupPreviewNextSteps.chooseUncorruptedEncryptedBackup":
        "请选择未损坏的加密备份，或重新导出备份。",
      "settings.memoryBackupPreviewNextSteps.largeAttachmentsNeedSidecars":
        "个大附件有 sidecar 元数据，但仍需要外部载荷文件才能纳入恢复。",
    }
    const t = (key: string, fallback: string) => translations[key] ?? fallback

    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "error",
          code: "invalid_json",
          message: "Backup file is not valid JSON: expected value at line 1 column 1",
        },
        t,
      ),
    ).toBe("备份文件不是有效 JSON: expected value at line 1 column 1")
    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "info",
          code: "attachments_reference_only",
          message:
            "3 attachment path(s) are present as references only and cannot be restored on this machine",
        },
        t,
      ),
    ).toBe("3 个附件引用只有路径，无法在此机器上恢复")
    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "error",
          code: "encrypted_plaintext_invalid",
          message:
            "Encrypted backup decrypted, but the decrypted bundle is not a valid Hope Agent memory backup: invalid utf-8 sequence",
        },
        t,
      ),
    ).toBe(
      "加密备份已解密，但解密后的 bundle 不是有效的 Hope Agent 记忆备份: invalid utf-8 sequence",
    )
    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "warning",
          code: "attachment_sidecar_missing",
          message: "Attachment sidecar is missing: attachments/memory-1.bin",
        },
        t,
      ),
    ).toBe("附件 sidecar 缺失: attachments/memory-1.bin")
    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "warning",
          code: "attachment_sidecar_size_mismatch",
          message: "Attachment sidecar attachments/memory-1.bin has size 42, expected 100",
        },
        t,
      ),
    ).toBe("附件 sidecar 大小不匹配: attachments/memory-1.bin (42 != 100 bytes)")
    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "warning",
          code: "attachment_sidecar_size_mismatch",
          message: "Attachment sidecar attachments/memory-1.bin decoded to size 40, expected 100",
        },
        t,
      ),
    ).toBe("附件 sidecar 大小不匹配: attachments/memory-1.bin (40 != 100 bytes)")
    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "warning",
          code: "attachment_sidecar_too_large",
          message: "Attachment sidecar attachments/memory-1.bin exceeds restore cap (104857600 bytes)",
        },
        t,
      ),
    ).toBe("附件 sidecar 超过恢复上限: attachments/memory-1.bin (> 104857600 bytes)")
    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "warning",
          code: "attachment_sidecar_read_failed",
          message: "Attachment sidecar attachments/memory-1.bin could not be read: permission denied",
        },
        t,
      ),
    ).toBe("附件 sidecar 无法读取: attachments/memory-1.bin: permission denied")
    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "warning",
          code: "attachment_sidecar_checksum_mismatch",
          message: "Attachment sidecar attachments/memory-1.bin checksum does not match",
        },
        t,
      ),
    ).toBe("附件 sidecar 校验和不匹配: attachments/memory-1.bin")
    expect(
      formatMemoryBackupPreviewIssueMessage(
        {
          severity: "warning",
          code: "attachment_sidecar_duplicate_memory_id",
          message: "Multiple sidecars target memory 7; keeping the last verified payload",
        },
        t,
      ),
    ).toBe("多个 sidecar 指向同一条记忆: memory_id=7")
    expect(
      formatMemoryBackupPreviewNextStep(
        "Choose a valid Hope Agent memory backup JSON or ZIP file.",
        t,
      ),
    ).toBe("请选择有效的 Hope Agent 记忆备份 JSON 或 ZIP 文件。")
    expect(
      formatMemoryBackupPreviewNextStep(
        "Choose a valid Hope Agent memory backup JSON file.",
        t,
      ),
    ).toBe("请选择有效的 Hope Agent 记忆备份 JSON 或 ZIP 文件。")
    expect(
      formatMemoryBackupPreviewNextStep(
        "Preview can import 7 legacy memory candidate(s) after user confirmation.",
        t,
      ),
    ).toBe("7 条普通记忆候选可在用户确认后导入。")
    expect(
      formatMemoryBackupPreviewNextStep(
        "2 experience/workflow history event(s) can be restored after their target records are mapped.",
        t,
      ),
    ).toBe("2 条经验 / 流程历史可在目标记录映射后恢复。")
    expect(
      formatMemoryBackupPreviewNextStep(
        "Some attachment references have no packed payload; keep original files available.",
        t,
      ),
    ).toBe("部分附件引用没有打包载荷，请保留原始文件。")
    expect(
      formatMemoryBackupPreviewNextStep(
        "Enter the backup passphrase to preview or restore this encrypted backup.",
        t,
      ),
    ).toBe("请输入备份口令后再预览或恢复此加密备份。")
    expect(
      formatMemoryBackupPreviewNextStep(
        "Check the backup passphrase or choose an uncorrupted encrypted backup.",
        t,
      ),
    ).toBe("请检查备份口令，或选择未损坏的加密备份。")
    expect(
      formatMemoryBackupPreviewNextStep(
        "Choose an uncorrupted encrypted backup or export the backup again.",
        t,
      ),
    ).toBe("请选择未损坏的加密备份，或重新导出备份。")
    expect(
      formatMemoryBackupPreviewNextStep(
        "3 large attachment(s) have sidecar metadata but still need external payload files before restore can include them.",
        t,
      ),
    ).toBe("3 个大附件有 sidecar 元数据，但仍需要外部载荷文件才能纳入恢复。")
    expect(formatMemoryBackupPreviewNextStep("Custom operator note.", t)).toBe(
      "Custom operator note.",
    )
  })
})

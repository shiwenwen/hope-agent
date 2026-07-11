import type {
  MemoryBackupClaimConflictExample,
  MemoryBackupClaimRestorePlan,
  MemoryBackupImportPreview,
  MemoryBackupProfileRestorePlan,
} from "./types"

export type MemoryBackupPreviewSummaryTranslateFn = (key: string, defaultValue: string) => string

export function formatMemoryBackupAlreadyPresentSummary(
  preview: Pick<MemoryBackupImportPreview, "legacyExactMatches" | "claimIdMatches">,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  return joinParts([
    countPart(preview.legacyExactMatches, "memories", "memories", t),
    countPart(preview.claimIdMatches, "claimIds", "claim IDs", t),
  ])
}

export function formatMemoryBackupHistorySummary(
  restorable: number | null | undefined,
  total: number | null | undefined,
  skipped: number | null | undefined,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  const parts = [
    `${count(restorable)}/${count(total)} ${summaryLabel(t, "historyMappable", "events mappable")}`,
  ]
  if (count(skipped) > 0) {
    parts.push(countPart(skipped, "historySkipped", "skipped", t))
  }
  return joinParts(parts)
}

export function formatMemoryBackupClaimPlanSummary(
  plan: MemoryBackupClaimRestorePlan,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  const parts = [
    countPart(plan.importCandidates, "candidates", "candidates", t),
    countPart(plan.exactMatches, "exactMatches", "exact matches", t),
    summaryLabel(t, "readyToRestore", "ready to restore"),
  ]
  if (plan.conflictingCandidates > 0) {
    parts.push(countPart(plan.conflictingCandidates, "willNeedReview", "will need review", t))
  }
  if (plan.needsReviewCandidates > 0) {
    parts.push(countPart(plan.needsReviewCandidates, "needsReview", "needs review", t))
  }
  if (plan.manualEvidenceRows > 0) {
    parts.push(countPart(plan.manualEvidenceRows, "manualEvidence", "manual evidence", t))
  }
  return joinParts(parts)
}

export function formatMemoryBackupProfilePlanSummary(
  plan: MemoryBackupProfileRestorePlan,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  const parts = [
    countPart(plan.importCandidates, "candidates", "candidates", t),
    countPart(plan.matchingScopes, "matchingScopes", "matching scopes", t),
    summaryLabel(t, "readyToRestore", "ready to restore"),
  ]
  if (plan.conflictingScopeCandidates > 0) {
    parts.push(
      countPart(
        plan.conflictingScopeCandidates,
        "scopeConflictsSkippedByDefault",
        "scope conflicts skipped by default",
        t,
      ),
    )
  }
  return joinParts(parts)
}

export function formatMemoryBackupExperiencePlanSummary(
  preview: Pick<
    MemoryBackupImportPreview,
    | "episodeImportCandidates"
    | "procedureImportCandidates"
    | "episodeExactMatches"
    | "procedureExactMatches"
    | "experienceHistoryCount"
    | "experienceHistoryRestorable"
    | "experienceHistorySkippedUnmapped"
  >,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  const parts = [
    countPart(preview.episodeImportCandidates, "episodeCandidates", "episode candidates", t),
    countPart(
      preview.procedureImportCandidates,
      "procedureCandidates",
      "procedure candidates",
      t,
    ),
    countPart(
      count(preview.episodeExactMatches) + count(preview.procedureExactMatches),
      "exactMatches",
      "exact matches",
      t,
    ),
    summaryLabel(t, "readyToRestore", "ready to restore"),
  ]
  if (count(preview.experienceHistoryCount) > 0) {
    parts.push(
      formatMemoryBackupHistorySummary(
        preview.experienceHistoryRestorable,
        preview.experienceHistoryCount,
        preview.experienceHistorySkippedUnmapped,
        t,
      ),
    )
  }
  return joinParts(parts)
}

export function formatMemoryBackupAttachmentSummary(
  preview: Pick<
    MemoryBackupImportPreview,
    | "attachmentPayloadCount"
    | "attachmentChunkedRefCount"
    | "attachmentExternalAvailableCount"
    | "attachmentRefCount"
    | "attachmentExternalRefCount"
    | "attachmentMissingCount"
  >,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  const packed =
    count(preview.attachmentPayloadCount) +
    count(preview.attachmentChunkedRefCount) +
    count(preview.attachmentExternalAvailableCount)
  const parts = [
    `${packed}/${count(preview.attachmentRefCount)} ${summaryLabel(t, "packed", "packed")}`,
  ]
  if (count(preview.attachmentChunkedRefCount) > 0) {
    parts.push(countPart(preview.attachmentChunkedRefCount, "chunked", "chunked", t))
  }
  if (count(preview.attachmentExternalAvailableCount) > 0) {
    parts.push(
      countPart(
        preview.attachmentExternalAvailableCount,
        "verifiedSidecar",
        "verified sidecar",
        t,
      ),
    )
  }
  if (count(preview.attachmentExternalRefCount) > 0) {
    parts.push(countPart(preview.attachmentExternalRefCount, "sidecarMetadata", "sidecar metadata", t))
  }
  if (count(preview.attachmentMissingCount) > 0) {
    parts.push(countPart(preview.attachmentMissingCount, "referenceOnly", "reference-only", t))
  }
  return joinParts(parts)
}

export function formatMemoryBackupClaimConflictHeader(
  example: Pick<
    MemoryBackupClaimConflictExample,
    "scope" | "claimType" | "subject" | "predicate"
  >,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  return joinParts([
    formatMemoryBackupClaimScope(example.scope, t),
    formatMemoryBackupClaimType(example.claimType, t),
    [example.subject, formatMemoryBackupPredicate(example.predicate)].filter(Boolean).join(" "),
  ])
}

export function formatMemoryBackupClaimType(
  claimType: string | null | undefined,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  const value = normalizeLabelInput(claimType)
  if (!value) return ""
  return translatedLabel(t, `settings.claimType_${value}`, humanizeIdentifier(value))
}

export function formatMemoryBackupClaimStatus(
  status: string | null | undefined,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  const value = normalizeLabelInput(status)
  if (!value) return ""
  return translatedLabel(t, `settings.claims.status.${value}`, humanizeIdentifier(value))
}

export function formatMemoryBackupClaimScope(
  scope: string | null | undefined,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  const value = normalizeLabelInput(scope)
  if (!value) return ""
  const separator = value.indexOf(":")
  const kind = separator >= 0 ? value.slice(0, separator) : value
  const id = separator >= 0 ? value.slice(separator + 1) : ""
  if (kind === "global") return translatedLabel(t, "settings.memoryScopeGlobal", "Global")
  if (kind === "agent") {
    const label = translatedLabel(t, "settings.memoryScopeAgent", "Agent")
    return id ? `${label}: ${id}` : label
  }
  if (kind === "project") {
    const label = translatedLabel(t, "settings.memoryScopeProject", "Project")
    return id ? `${label}: ${id}` : label
  }
  return humanizeIdentifier(value)
}

export function formatMemoryBackupPredicate(predicate: string | null | undefined): string {
  const value = normalizeLabelInput(predicate)
  return value ? humanizeIdentifier(value) : ""
}

function count(value: number | null | undefined): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0
}

function countPart(
  value: number | null | undefined,
  key: string,
  fallback: string,
  t?: MemoryBackupPreviewSummaryTranslateFn,
): string {
  return `${count(value)} ${summaryLabel(t, key, fallback)}`
}

function summaryLabel(
  t: MemoryBackupPreviewSummaryTranslateFn | undefined,
  key: string,
  fallback: string,
): string {
  if (!t) return fallback
  const translated = t(`settings.memoryBackupPreviewSummary.${key}`, fallback)
  return typeof translated === "string" && translated.trim().length > 0 ? translated : fallback
}

function translatedLabel(
  t: MemoryBackupPreviewSummaryTranslateFn | undefined,
  key: string,
  fallback: string,
): string {
  if (!t) return fallback
  const translated = t(key, fallback)
  return typeof translated === "string" && translated.trim().length > 0 ? translated : fallback
}

function joinParts(parts: string[]): string {
  return parts.filter((part) => part.trim().length > 0).join(" · ")
}

function normalizeLabelInput(value: string | null | undefined): string {
  return typeof value === "string" ? value.trim() : ""
}

function humanizeIdentifier(value: string): string {
  return value.replace(/[_-]+/g, " ").replace(/\s+/g, " ").trim()
}

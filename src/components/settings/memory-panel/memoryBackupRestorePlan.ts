import type { MemoryBackupImportPreview } from "./types"

export type MemoryBackupRestorePlanAction = "restore" | "review" | "skip" | "override" | "blocked"
export type MemoryBackupRestorePlanTone = "good" | "warn" | "muted" | "danger"
export type MemoryBackupRestorePlanSummaryKind =
  | "blocked"
  | "override_enabled"
  | "needs_review"
  | "restore_with_skips"
  | "safe_restore"
  | "no_changes"

export interface MemoryBackupRestorePlanOptions {
  allowProfileScopeConflicts?: boolean
}

export interface MemoryBackupRestorePlanRow {
  id: string
  labelKey?: string
  label: string
  count: number
  action: MemoryBackupRestorePlanAction
  tone: MemoryBackupRestorePlanTone
  detailKey?: string
  detail: string
}

export interface MemoryBackupRestorePlanSummary {
  kind: MemoryBackupRestorePlanSummaryKind
  tone: MemoryBackupRestorePlanTone
  title: string
  detail: string
  nextStep: string
  restoreCount: number
  reviewCount: number
  skipCount: number
  overrideCount: number
  blockedCount: number
}

export type MemoryBackupRestorePlanTranslateFn = (key: string, defaultValue: string) => string

export function formatMemoryBackupRestorePlanRowLabel(
  row: MemoryBackupRestorePlanRow,
  t?: MemoryBackupRestorePlanTranslateFn,
): string {
  return translateRestorePlan(
    t,
    row.labelKey ?? `settings.memoryBackupRestorePlanRows.${row.id}.label`,
    row.label,
  )
}

export function formatMemoryBackupRestorePlanRowDetail(
  row: MemoryBackupRestorePlanRow,
  t?: MemoryBackupRestorePlanTranslateFn,
): string {
  return translateRestorePlan(
    t,
    row.detailKey ?? `settings.memoryBackupRestorePlanRows.${row.id}.detail`,
    row.detail,
  )
}

export function formatMemoryBackupRestorePlanActionLabel(
  action: MemoryBackupRestorePlanAction,
  t?: MemoryBackupRestorePlanTranslateFn,
): string {
  const fallback = memoryBackupRestorePlanActionFallback(action)
  return translateRestorePlan(t, `settings.memoryBackupRestorePlanActions.${action}`, fallback)
}

export function formatMemoryBackupRestorePlanSummaryTitle(
  summary: MemoryBackupRestorePlanSummary,
  t?: MemoryBackupRestorePlanTranslateFn,
): string {
  return translateRestorePlan(
    t,
    `settings.memoryBackupRestorePlanSummary.${summary.kind}.title`,
    summary.title,
  )
}

export function formatMemoryBackupRestorePlanSummaryDetail(
  summary: MemoryBackupRestorePlanSummary,
  t?: MemoryBackupRestorePlanTranslateFn,
): string {
  return translateRestorePlan(
    t,
    `settings.memoryBackupRestorePlanSummary.${summary.kind}.detail`,
    summary.detail,
  )
}

export function formatMemoryBackupRestorePlanSummaryNextStep(
  summary: MemoryBackupRestorePlanSummary,
  t?: MemoryBackupRestorePlanTranslateFn,
): string {
  return translateRestorePlan(
    t,
    `settings.memoryBackupRestorePlanSummary.${summary.kind}.nextStep`,
    summary.nextStep,
  )
}

export function hasMemoryBackupStructuredRestoreCandidates(
  preview: MemoryBackupImportPreview | null | undefined,
): boolean {
  if (!preview?.valid) return false
  return (
    count(preview.claimRestorePlan.importCandidates) > 0 ||
    count(preview.profileRestorePlan.importCandidates) > 0 ||
    count(preview.episodeImportCandidates) > 0 ||
    count(preview.procedureImportCandidates) > 0 ||
    count(preview.experienceHistoryRestorable) > 0
  )
}

export function buildMemoryBackupRestorePlan(
  preview: MemoryBackupImportPreview,
  options: MemoryBackupRestorePlanOptions = {},
): MemoryBackupRestorePlanRow[] {
  if (!preview.valid) {
    return [
      {
        id: "invalid",
        label: "Backup file is not compatible",
        count: Math.max(1, preview.issues.length),
        action: "blocked",
        tone: "danger",
        detail: "Run preview with a compatible Hope Agent memory backup before restoring.",
      },
    ]
  }

  const rows: MemoryBackupRestorePlanRow[] = []
  addRow(rows, {
    id: "legacy-import",
    label: "Missing legacy memories",
    count: preview.legacyImportCandidates,
    action: "restore",
    tone: "good",
    detail: "Restored only when using Restore missing memories; exact matches are not overwritten.",
  })
  addRow(rows, {
    id: "legacy-skip",
    label: "Legacy memories skipped",
    count: preview.legacyExactMatches + preview.legacyDuplicateInBundle,
    action: "skip",
    tone: "muted",
    detail: "Already-present memories and duplicate bundle entries stay untouched.",
  })
  addRow(rows, {
    id: "legacy-history",
    label: "Legacy audit history",
    count: count(preview.legacyHistoryRestorable),
    action: "restore",
    tone: "good",
    detail: "History events are restored only when they can be mapped to local memories.",
  })
  addRow(rows, {
    id: "legacy-history-unmapped",
    label: "Unmapped legacy history",
    count: count(preview.legacyHistorySkippedUnmapped),
    action: "skip",
    tone: "muted",
    detail: "Unmapped history is skipped instead of inventing target memories.",
  })

  const claimPlan = preview.claimRestorePlan
  addRow(rows, {
    id: "claim-import",
    label: "Structured claims",
    count: claimPlan.importCandidates,
    action: "restore",
    tone: "good",
    detail: "Missing claims are imported additively; conflicts are downgraded before activation.",
  })
  addRow(rows, {
    id: "claim-review",
    label: "Claims routed to Review Inbox",
    count: claimPlan.conflictingCandidates + claimPlan.needsReviewCandidates,
    action: "review",
    tone: "warn",
    detail: "Conflicting or review-first claims require user approval before influencing answers.",
  })
  addRow(rows, {
    id: "claim-skip",
    label: "Claims skipped",
    count: claimPlan.existingById + claimPlan.exactMatches,
    action: "skip",
    tone: "muted",
    detail: "Existing claim IDs and exact matches are not restored again.",
  })

  const profilePlan = preview.profileRestorePlan
  const profileConflictAction = options.allowProfileScopeConflicts ? "override" : "skip"
  addRow(rows, {
    id: "profile-import",
    label: "Profile snapshots",
    count: Math.max(0, profilePlan.importCandidates - profilePlan.conflictingScopeCandidates),
    action: "restore",
    tone: "good",
    detail: "New profile scopes can be restored without replacing local profile snapshots.",
  })
  addRow(rows, {
    id: "profile-conflict",
    label: "Profile scope conflicts",
    count: profilePlan.conflictingScopeCandidates,
    action: profileConflictAction,
    tone: options.allowProfileScopeConflicts ? "warn" : "muted",
    detailKey: options.allowProfileScopeConflicts
      ? "settings.memoryBackupRestorePlanRows.profile-conflict.overrideDetail"
      : undefined,
    detail: options.allowProfileScopeConflicts
      ? "The override switch is on; matching scopes will import as newer profile snapshots."
      : "Skipped by default unless the user explicitly enables the override switch.",
  })
  addRow(rows, {
    id: "profile-skip",
    label: "Profile exact matches",
    count: profilePlan.exactMatches,
    action: "skip",
    tone: "muted",
    detail: "Exact profile snapshot matches are not restored again.",
  })

  addRow(rows, {
    id: "episode-import",
    label: "Episodes",
    count: count(preview.episodeImportCandidates),
    action: "restore",
    tone: "good",
    detail: "Missing experience records are restored additively.",
  })
  addRow(rows, {
    id: "procedure-import",
    label: "Procedures",
    count: count(preview.procedureImportCandidates),
    action: "restore",
    tone: "good",
    detail: "Missing workflow records are restored additively.",
  })
  addRow(rows, {
    id: "experience-skip",
    label: "Experience exact/id matches",
    count:
      count(preview.episodeIdMatches) +
      count(preview.episodeExactMatches) +
      count(preview.procedureIdMatches) +
      count(preview.procedureExactMatches),
    action: "skip",
    tone: "muted",
    detail: "Existing Experience and Workflow records stay untouched.",
  })
  addRow(rows, {
    id: "experience-history",
    label: "Experience audit history",
    count: count(preview.experienceHistoryRestorable),
    action: "restore",
    tone: "good",
    detail: "Workflow audit history is restored only when its target can be mapped safely.",
  })
  addRow(rows, {
    id: "experience-history-unmapped",
    label: "Unmapped experience history",
    count: count(preview.experienceHistorySkippedUnmapped),
    action: "skip",
    tone: "muted",
    detail: "Unmapped workflow history is skipped instead of attaching to the wrong record.",
  })
  addRow(rows, {
    id: "attachments-missing",
    label: "Reference-only attachments",
    count: preview.attachmentMissingCount,
    action: "skip",
    tone: "warn",
    detail: "Attachment references without payloads or verified sidecars cannot be restored here.",
  })

  return rows
}

export function summarizeMemoryBackupRestorePlan(
  rows: MemoryBackupRestorePlanRow[],
): MemoryBackupRestorePlanSummary {
  const restoreCount = sumRows(rows, "restore")
  const reviewCount = sumRows(rows, "review")
  const skipCount = sumRows(rows, "skip")
  const overrideCount = sumRows(rows, "override")
  const blockedCount = sumRows(rows, "blocked")

  if (blockedCount > 0) {
    return {
      kind: "blocked",
      tone: "danger",
      title: "Restore is blocked",
      detail: "This backup cannot be safely restored from the current preview.",
      nextStep: "Choose a compatible backup, unlock it, or resolve preview errors first.",
      restoreCount,
      reviewCount,
      skipCount,
      overrideCount,
      blockedCount,
    }
  }

  if (overrideCount > 0) {
    return {
      kind: "override_enabled",
      tone: "warn",
      title: "Profile override is enabled",
      detail: "Some matching profile scopes will import as newer snapshots instead of being skipped.",
      nextStep: "Continue only if this backup should become the latest profile for those scopes.",
      restoreCount,
      reviewCount,
      skipCount,
      overrideCount,
      blockedCount,
    }
  }

  if (reviewCount > 0) {
    return {
      kind: "needs_review",
      tone: "warn",
      title: "Some restored items need review",
      detail: "Conflicting or review-first structured memories will not affect answers until approved.",
      nextStep: "Restore structured memory, then open Review Inbox before trusting those items.",
      restoreCount,
      reviewCount,
      skipCount,
      overrideCount,
      blockedCount,
    }
  }

  if (restoreCount > 0 && skipCount > 0) {
    return {
      kind: "restore_with_skips",
      tone: "good",
      title: "Safe additive restore with skips",
      detail: "New items can be restored while exact matches and unmapped records stay untouched.",
      nextStep: "Restore the missing items; skipped rows require no action unless you expected them to import.",
      restoreCount,
      reviewCount,
      skipCount,
      overrideCount,
      blockedCount,
    }
  }

  if (restoreCount > 0) {
    return {
      kind: "safe_restore",
      tone: "good",
      title: "Safe additive restore",
      detail: "The preview only contains missing items that can be imported without overwriting local data.",
      nextStep: "Restore the missing items.",
      restoreCount,
      reviewCount,
      skipCount,
      overrideCount,
      blockedCount,
    }
  }

  return {
    kind: "no_changes",
    tone: "muted",
    title: "Nothing new to restore",
    detail: "The preview did not find restorable missing memory assets.",
    nextStep: "No restore action is needed unless you select a different backup.",
    restoreCount,
    reviewCount,
    skipCount,
    overrideCount,
    blockedCount,
  }
}

function addRow(rows: MemoryBackupRestorePlanRow[], row: MemoryBackupRestorePlanRow) {
  if (count(row.count) <= 0) return
  rows.push({ ...row, count: count(row.count) })
}

function translateRestorePlan(
  t: MemoryBackupRestorePlanTranslateFn | undefined,
  key: string,
  fallback: string,
): string {
  if (!t) return fallback
  const translated = t(key, fallback)
  return typeof translated === "string" && translated.trim().length > 0 ? translated : fallback
}

function memoryBackupRestorePlanActionFallback(action: MemoryBackupRestorePlanAction): string {
  switch (action) {
    case "restore":
      return "Restore"
    case "review":
      return "Review"
    case "skip":
      return "Skip"
    case "override":
      return "Override"
    case "blocked":
      return "Blocked"
  }
}

function count(value: number | null | undefined): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0
}

function sumRows(rows: MemoryBackupRestorePlanRow[], action: MemoryBackupRestorePlanAction): number {
  return rows
    .filter((row) => row.action === action)
    .reduce((total, row) => total + count(row.count), 0)
}

import type { TFunction } from "i18next"
import { toast } from "sonner"
import type {
  MemoryBackupClaimRestorePlan,
  MemoryBackupImportPreview,
  MemoryBackupPreviewIssue,
  MemoryBackupProfileRestorePlan,
} from "./types"
import {
  formatMemoryBackupClaimConflictHeader,
  formatMemoryBackupClaimStatus,
  formatMemoryBackupClaimType,
  formatMemoryBackupClaimScope,
} from "./memoryBackupPreviewSummary"
import {
  buildMemoryBackupRestorePlan,
  formatMemoryBackupRestorePlanActionLabel,
  formatMemoryBackupRestorePlanRowDetail,
  formatMemoryBackupRestorePlanRowLabel,
  formatMemoryBackupRestorePlanSummaryDetail,
  formatMemoryBackupRestorePlanSummaryNextStep,
  formatMemoryBackupRestorePlanSummaryTitle,
  hasMemoryBackupStructuredRestoreCandidates,
  summarizeMemoryBackupRestorePlan,
  type MemoryBackupRestorePlanTranslateFn,
  type MemoryBackupRestorePlanRow,
} from "./memoryBackupRestorePlan"
import { memoryBackupOperationErrorToast } from "./memoryBackupOperationFeedback"

interface MemoryBackupPreviewDiagnosticsOptions {
  sourceLabel?: string | null
  allowProfileScopeConflicts?: boolean
  t?: MemoryBackupRestorePlanTranslateFn
}

function count(value: number | null | undefined): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0
}

export function formatMemoryBackupPreviewDiagnostics(
  preview: MemoryBackupImportPreview,
  options: MemoryBackupPreviewDiagnosticsOptions = {},
): string {
  const t = options.t
  const claimPlan = preview.claimRestorePlan
  const profilePlan = preview.profileRestorePlan
  const structuredRestoreAvailable = hasMemoryBackupStructuredRestoreCandidates(preview)
  const lines = [
    `# ${formatBackupPreviewReportLabel(t, "title", "Memory Backup Restore Preview")}`,
    "",
    `- ${formatBackupPreviewReportLabel(t, "source", "Source")}: ${
      options.sourceLabel || formatBackupPreviewReportLabel(t, "sourceDefault", "Backup file")
    }`,
    `- ${formatBackupPreviewReportLabel(t, "generated", "Generated")}: ${new Date().toISOString()}`,
    `- ${formatBackupPreviewReportLabel(t, "valid", "Valid")}: ${formatBackupPreviewBool(t, preview.valid)}`,
    `- ${formatBackupPreviewReportLabel(t, "schemaVersion", "Schema version")}: ${preview.schemaVersion || "-"}`,
    `- ${formatBackupPreviewReportLabel(t, "appVersion", "App version")}: ${preview.appVersion || "-"}`,
    `- ${formatBackupPreviewReportLabel(t, "exportedAt", "Exported at")}: ${preview.exportedAt || "-"}`,
    "",
    `## ${formatBackupPreviewReportLabel(t, "restoreAvailability", "Restore Availability")}`,
    "",
    `- ${formatBackupPreviewReportLabel(t, "restoreMissingLegacyMemories", "Restore missing legacy memories")}: ${formatBackupPreviewBool(t, preview.valid && preview.legacyImportCandidates > 0)}`,
    `- ${formatBackupPreviewReportLabel(t, "restoreStructuredMemory", "Restore structured memory")}: ${formatBackupPreviewBool(t, structuredRestoreAvailable)}`,
    `- ${formatBackupPreviewReportLabel(t, "profileScopeConflictOverride", "Profile scope conflict override")}: ${formatBackupPreviewToggle(t, options.allowProfileScopeConflicts === true)}`,
    "",
    `## ${formatBackupPreviewReportLabel(t, "legacyMemory", "Legacy Memory")}`,
    "",
    `- ${formatBackupPreviewReportLabel(t, "memoriesInBackup", "Memories in backup")}: ${preview.legacyMemoryCount}`,
    `- ${formatBackupPreviewReportLabel(t, "newCandidates", "New candidates")}: ${preview.legacyImportCandidates}`,
    `- ${formatBackupPreviewReportLabel(t, "alreadyPresent", "Already present")}: ${preview.legacyExactMatches}`,
    `- ${formatBackupPreviewReportLabel(t, "duplicatesInsideBundle", "Duplicates inside bundle")}: ${preview.legacyDuplicateInBundle}`,
    `- ${formatBackupPreviewReportLabel(t, "historyEventsMappable", "History events mappable")}: ${count(preview.legacyHistoryRestorable)}/${count(preview.legacyHistoryCount)}`,
    `- ${formatBackupPreviewReportLabel(t, "historyEventsSkippedUnmapped", "History events skipped unmapped")}: ${count(preview.legacyHistorySkippedUnmapped)}`,
  ]

  appendClaimPlan(lines, claimPlan, t)
  appendProfilePlan(lines, profilePlan, options.allowProfileScopeConflicts === true, t)
  const restorePlan = buildMemoryBackupRestorePlan(preview, options)
  appendRestorePlan(lines, restorePlan, t)
  appendExperiencePlan(lines, preview, t)
  appendAttachmentPlan(lines, preview, t)
  appendList(
    lines,
    formatBackupPreviewReportLabel(t, "unsupportedSections", "Unsupported Sections"),
    sortedStrings(preview.unsupportedSections),
  )

  if (preview.issues.length > 0) {
    lines.push("", `## ${formatBackupPreviewReportLabel(t, "issues", "Issues")}`, "")
    for (const issue of preview.issues) {
      lines.push(`- [${issue.severity}] ${issue.code}: ${formatMemoryBackupPreviewIssueMessage(issue, t)}`)
    }
  }

  appendList(
    lines,
    formatBackupPreviewReportLabel(t, "nextSteps", "Next Steps"),
    preview.nextSteps.map((step) => formatMemoryBackupPreviewNextStep(step, t)),
  )
  lines.push(
    "",
    `## ${formatBackupPreviewReportLabel(t, "safetyNotes", "Safety Notes")}`,
    "",
    `- ${formatBackupPreviewReportLabel(t, "safetyReadOnly", "This report is generated from the read-only backup preview payload.")}`,
    `- ${formatBackupPreviewReportLabel(t, "safetyExactMatches", "Exact matches are skipped instead of overwritten.")}`,
    `- ${formatBackupPreviewReportLabel(t, "safetyClaimConflicts", "Claim conflicts are restored as Review Inbox items, not silently activated.")}`,
    `- ${formatBackupPreviewReportLabel(t, "safetyProfileConflicts", "Profile scope conflicts are skipped unless the user explicitly enables replacement.")}`,
  )

  return lines.join("\n")
}

export async function copyMemoryBackupPreviewDiagnostics(
  t: TFunction,
  preview: MemoryBackupImportPreview,
  options: MemoryBackupPreviewDiagnosticsOptions = {},
) {
  try {
    await navigator.clipboard.writeText(
      formatMemoryBackupPreviewDiagnostics(preview, { ...options, t }),
    )
    toast.success(t("common.copied", "Copied"))
  } catch (error) {
    const failureToast = memoryBackupOperationErrorToast(
      "copyPreviewDiagnostics",
      (key, i18nOptions) => String(t(key, i18nOptions)),
      error,
    )
    toast.error(
      failureToast.title,
      failureToast.description ? { description: failureToast.description } : undefined,
    )
  }
}

export function formatMemoryBackupPreviewIssueMessage(
  issue: MemoryBackupPreviewIssue,
  t?: MemoryBackupRestorePlanTranslateFn,
): string {
  if (!t) return issue.message
  switch (issue.code) {
    case "invalid_archive":
      return withOptionalDetail(
        t,
        "invalidArchive",
        "Backup package is not a valid Hope Agent memory archive",
        detailAfter(issue.message, "Backup package is not a valid Hope Agent memory archive: "),
      )
    case "invalid_json":
      return withOptionalDetail(
        t,
        "invalidJson",
        "Backup file is not valid JSON",
        detailAfter(issue.message, "Backup file is not valid JSON: "),
      )
    case "encrypted_passphrase_required":
      return formatBackupPreviewIssueLabel(
        t,
        "encryptedPassphraseRequired",
        "This backup is encrypted; enter its passphrase to preview or restore.",
      )
    case "encrypted_decrypt_failed":
      return withOptionalDetail(
        t,
        "encryptedDecryptFailed",
        "Encrypted backup could not be decrypted",
        detailAfter(issue.message, "Encrypted backup could not be decrypted: "),
      )
    case "encrypted_plaintext_invalid":
      return withOptionalDetail(
        t,
        "encryptedPlaintextInvalid",
        "Encrypted backup decrypted, but the decrypted bundle is not a valid Hope Agent memory backup",
        detailAfter(
          issue.message,
          "Encrypted backup decrypted, but the decrypted bundle is not a valid Hope Agent memory backup: ",
        ),
      )
    case "unsupported_schema":
      return withOptionalDetail(
        t,
        "unsupportedSchema",
        "Unsupported memory backup schema",
        detailAfter(issue.message, "Unsupported memory backup schema: "),
      )
    case "invalid_bundle_shape":
      return withOptionalDetail(
        t,
        "invalidBundleShape",
        "Backup schema is recognized but the bundle is incomplete",
        detailAfter(issue.message, "Backup schema is recognized but the bundle is incomplete: "),
      )
    case "source_bundle_partial":
      return formatBackupPreviewIssueLabel(
        t,
        "sourceBundlePartial",
        "Source bundle was exported with warnings; inspect manifest.warnings before restoring.",
      )
    case "source_bundle_warning":
      return `${formatBackupPreviewIssueLabel(t, "sourceBundleWarning", "Source bundle warning")}: ${issue.message}`
    case "duplicate_legacy_memories_in_bundle":
      return formatNumberedIssue(
        t,
        issue.message,
        "duplicateLegacyMemoriesInBundle",
        "duplicate legacy memory row(s) appear inside the backup",
      )
    case "attachments_reference_only":
      return formatNumberedIssue(
        t,
        issue.message,
        "attachmentsReferenceOnly",
        "attachment path(s) are present as references only and cannot be restored on this machine",
      )
    case "attachments_chunked_payloads": {
      const match = issue.message.match(/^(\d+).+?(\d+)/)
      if (!match) return issue.message
      return `${match[1]} ${formatBackupPreviewIssueLabel(
        t,
        "attachmentsChunkedPayloads",
        "attachment payload(s) are stored as verified chunks",
      )} (${match[2]} ${formatBackupPreviewIssueLabel(t, "verifiedChunks", "chunk(s)")})`
    }
    case "attachments_external_sidecar_required":
      return formatNumberedIssue(
        t,
        issue.message,
        "attachmentsExternalSidecarRequired",
        "attachment payload(s) require an external sidecar file before they can be restored",
      )
    case "legacy_history_partially_unmapped":
      return formatNumberedIssue(
        t,
        issue.message,
        "legacyHistoryPartiallyUnmapped",
        "legacy memory history event(s) cannot be safely mapped to a local memory row",
      )
    case "attachments_external_sidecars_available":
      return formatNumberedIssue(
        t,
        issue.message,
        "attachmentsExternalSidecarsAvailable",
        "large attachment sidecar payload(s) are present and checksum-verified",
      )
    case "attachments_external_sidecar_missing":
      return formatNumberedIssue(
        t,
        issue.message,
        "attachmentsExternalSidecarMissing",
        "large attachment sidecar payload(s) are missing or failed verification",
      )
    case "attachment_sidecar_missing":
      return withOptionalDetail(
        t,
        "attachmentSidecarMissing",
        "Attachment sidecar is missing",
        detailAfter(issue.message, "Attachment sidecar is missing: "),
      )
    case "attachment_sidecar_size_mismatch":
      return withParsedDetail(
        t,
        "attachmentSidecarSizeMismatch",
        "Attachment sidecar size does not match",
        formatAttachmentSidecarSizeDetail(issue.message),
        issue.message,
      )
    case "attachment_sidecar_too_large":
      return withParsedDetail(
        t,
        "attachmentSidecarTooLarge",
        "Attachment sidecar exceeds the restore cap",
        formatAttachmentSidecarTooLargeDetail(issue.message),
        issue.message,
      )
    case "attachment_sidecar_read_failed":
      return withParsedDetail(
        t,
        "attachmentSidecarReadFailed",
        "Attachment sidecar could not be read",
        formatAttachmentSidecarReadFailedDetail(issue.message),
        issue.message,
      )
    case "attachment_sidecar_checksum_mismatch":
      return withParsedDetail(
        t,
        "attachmentSidecarChecksumMismatch",
        "Attachment sidecar checksum does not match",
        formatAttachmentSidecarChecksumDetail(issue.message),
        issue.message,
      )
    case "attachment_sidecar_duplicate_memory_id":
      return withParsedDetail(
        t,
        "attachmentSidecarDuplicateMemoryId",
        "Multiple sidecars target the same memory",
        formatAttachmentSidecarDuplicateDetail(issue.message),
        issue.message,
      )
    case "current_claims_unavailable":
      return withOptionalDetail(
        t,
        "currentClaimsUnavailable",
        "Current claim graph could not be compared",
        detailAfter(issue.message, "Current claim graph could not be compared: "),
      )
    case "current_profiles_unavailable":
      return withOptionalDetail(
        t,
        "currentProfilesUnavailable",
        "Current profile snapshots could not be compared",
        detailAfter(issue.message, "Current profile snapshots could not be compared: "),
      )
    case "current_episodes_unavailable":
      return withOptionalDetail(
        t,
        "currentEpisodesUnavailable",
        "Current episodes could not be compared",
        detailAfter(issue.message, "Current episodes could not be compared for history: ") ??
          detailAfter(issue.message, "Current episodes could not be compared: "),
      )
    case "current_procedures_unavailable":
      return withOptionalDetail(
        t,
        "currentProceduresUnavailable",
        "Current procedures could not be compared",
        detailAfter(issue.message, "Current procedures could not be compared for history: ") ??
          detailAfter(issue.message, "Current procedures could not be compared: "),
      )
    default:
      return issue.message
  }
}

export function formatMemoryBackupPreviewNextStep(
  step: string,
  t?: MemoryBackupRestorePlanTranslateFn,
): string {
  if (!t) return step
  if (
    step === "Choose a valid Hope Agent memory backup JSON file." ||
    step === "Choose a valid Hope Agent memory backup JSON or ZIP file."
  ) {
    return formatBackupPreviewNextStepLabel(
      t,
      "chooseValidBackup",
      "Choose a valid Hope Agent memory backup JSON or ZIP file.",
    )
  }
  if (step === "Some attachment references have no packed payload; keep original files available.") {
    return formatBackupPreviewNextStepLabel(
      t,
      "keepOriginalAttachmentFiles",
      "Some attachment references have no packed payload; keep original files available.",
    )
  }
  if (step === "Enter the backup passphrase to preview or restore this encrypted backup.") {
    return formatBackupPreviewNextStepLabel(
      t,
      "enterEncryptedBackupPassphrase",
      "Enter the backup passphrase to preview or restore this encrypted backup.",
    )
  }
  if (step === "Check the backup passphrase or choose an uncorrupted encrypted backup.") {
    return formatBackupPreviewNextStepLabel(
      t,
      "checkEncryptedBackupPassphrase",
      "Check the backup passphrase or choose an uncorrupted encrypted backup.",
    )
  }
  if (step === "Choose an uncorrupted encrypted backup or export the backup again.") {
    return formatBackupPreviewNextStepLabel(
      t,
      "chooseUncorruptedEncryptedBackup",
      "Choose an uncorrupted encrypted backup or export the backup again.",
    )
  }
  if (step === "No importable memory changes were found.") {
    return formatBackupPreviewNextStepLabel(
      t,
      "noImportableChanges",
      "No importable memory changes were found.",
    )
  }
  return (
    formatNumberedNextStep(
      t,
      step,
      /^Preview can import (\d+) legacy memory candidate\(s\) after user confirmation\.$/,
      "legacyImportCandidates",
      "legacy memory candidate(s) can be imported after user confirmation.",
    ) ??
    formatNumberedNextStep(
      t,
      step,
      /^(\d+) legacy memory history event\(s\) can be restored after their memory rows are mapped\.$/,
      "legacyHistoryRestorable",
      "legacy memory history event(s) can be restored after their memory rows are mapped.",
    ) ??
    formatNumberedNextStep(
      t,
      step,
      /^(\d+) structured claim candidate\(s\) can be restored after explicit confirmation\.$/,
      "claimImportCandidates",
      "structured claim candidate(s) can be restored after explicit confirmation.",
    ) ??
    formatNumberedNextStep(
      t,
      step,
      /^(\d+) profile snapshot candidate\(s\) can be restored after explicit confirmation\.$/,
      "profileImportCandidates",
      "profile snapshot candidate(s) can be restored after explicit confirmation.",
    ) ??
    formatNumberedNextStep(
      t,
      step,
      /^(\d+) episode memory candidate\(s\) can be restored after explicit confirmation\.$/,
      "episodeImportCandidates",
      "episode memory candidate(s) can be restored after explicit confirmation.",
    ) ??
    formatNumberedNextStep(
      t,
      step,
      /^(\d+) procedure memory candidate\(s\) can be restored after explicit confirmation\.$/,
      "procedureImportCandidates",
      "procedure memory candidate(s) can be restored after explicit confirmation.",
    ) ??
    formatNumberedNextStep(
      t,
      step,
      /^(\d+) experience\/workflow history event\(s\) can be restored after their target records are mapped\.$/,
      "experienceHistoryRestorable",
      "experience/workflow history event(s) can be restored after their target records are mapped.",
    ) ??
    formatNumberedNextStep(
      t,
      step,
      /^(\d+) attachment payload\(s\) can be restored with their memory rows\.$/,
      "attachmentPayloadsRestorable",
      "attachment payload(s) can be restored with their memory rows.",
    ) ??
    formatNumberedNextStep(
      t,
      step,
      /^(\d+) large attachment\(s\) have sidecar metadata but still need external payload files before restore can include them\.$/,
      "largeAttachmentsNeedSidecars",
      "large attachment(s) have sidecar metadata but still need external payload files before restore can include them.",
    ) ??
    step
  )
}

function appendRestorePlan(
  lines: string[],
  plan: MemoryBackupRestorePlanRow[],
  t?: MemoryBackupRestorePlanTranslateFn,
) {
  if (plan.length === 0) return
  const summary = summarizeMemoryBackupRestorePlan(plan)
  lines.push(
    "",
    `## ${formatRestorePlanReportLabel(t, "restoreDecisionPlan", "Restore Decision Plan")}`,
    "",
  )
  lines.push(
    `- ${formatRestorePlanReportLabel(t, "summary", "Summary")}: ${formatMemoryBackupRestorePlanSummaryTitle(summary, t)}`,
    `- ${formatRestorePlanReportLabel(t, "detail", "Detail")}: ${formatMemoryBackupRestorePlanSummaryDetail(summary, t)}`,
    `- ${formatRestorePlanReportLabel(t, "nextStep", "Next step")}: ${formatMemoryBackupRestorePlanSummaryNextStep(summary, t)}`,
    `- ${formatRestorePlanReportLabel(t, "actionTotals", "Action totals")}: ${formatMemoryBackupRestorePlanActionLabel("restore", t)}=${summary.restoreCount}, ${formatMemoryBackupRestorePlanActionLabel("review", t)}=${summary.reviewCount}, ${formatMemoryBackupRestorePlanActionLabel("skip", t)}=${summary.skipCount}, ${formatMemoryBackupRestorePlanActionLabel("override", t)}=${summary.overrideCount}, ${formatMemoryBackupRestorePlanActionLabel("blocked", t)}=${summary.blockedCount}`,
    "",
  )
  for (const row of plan) {
    lines.push(
      `- [${formatMemoryBackupRestorePlanActionLabel(row.action, t)}] ${formatMemoryBackupRestorePlanRowLabel(row, t)}: ${row.count} — ${formatMemoryBackupRestorePlanRowDetail(row, t)}`,
    )
  }
}

function formatRestorePlanReportLabel(
  t: MemoryBackupRestorePlanTranslateFn | undefined,
  key: string,
  fallback: string,
): string {
  if (!t) return fallback
  const translated = t(`settings.memoryBackupRestorePlanReport.${key}`, fallback)
  return typeof translated === "string" && translated.trim().length > 0 ? translated : fallback
}

function formatBackupPreviewReportLabel(
  t: MemoryBackupRestorePlanTranslateFn | undefined,
  key: string,
  fallback: string,
): string {
  if (!t) return fallback
  const translated = t(`settings.memoryBackupPreviewReport.${key}`, fallback)
  return typeof translated === "string" && translated.trim().length > 0 ? translated : fallback
}

function formatBackupPreviewIssueLabel(
  t: MemoryBackupRestorePlanTranslateFn,
  key: string,
  fallback: string,
): string {
  const translated = t(`settings.memoryBackupPreviewIssues.${key}`, fallback)
  return typeof translated === "string" && translated.trim().length > 0 ? translated : fallback
}

function formatBackupPreviewNextStepLabel(
  t: MemoryBackupRestorePlanTranslateFn,
  key: string,
  fallback: string,
): string {
  const translated = t(`settings.memoryBackupPreviewNextSteps.${key}`, fallback)
  return typeof translated === "string" && translated.trim().length > 0 ? translated : fallback
}

function detailAfter(message: string, prefix: string): string | null {
  return message.startsWith(prefix) ? message.slice(prefix.length).trim() : null
}

function withOptionalDetail(
  t: MemoryBackupRestorePlanTranslateFn,
  key: string,
  fallback: string,
  detail: string | null,
): string {
  const label = formatBackupPreviewIssueLabel(t, key, fallback)
  return detail ? `${label}: ${detail}` : label
}

function withParsedDetail(
  t: MemoryBackupRestorePlanTranslateFn,
  key: string,
  fallback: string,
  detail: string | null,
  rawMessage: string,
): string {
  const label = formatBackupPreviewIssueLabel(t, key, fallback)
  return `${label}: ${detail || rawMessage}`
}

function formatAttachmentSidecarSizeDetail(message: string): string | null {
  const direct = message.match(/^Attachment sidecar (.+) has size (\d+), expected (\d+)$/)
  if (direct) return `${direct[1]} (${direct[2]} != ${direct[3]} bytes)`
  const decoded = message.match(/^Attachment sidecar (.+) decoded to size (\d+), expected (\d+)$/)
  if (decoded) return `${decoded[1]} (${decoded[2]} != ${decoded[3]} bytes)`
  return null
}

function formatAttachmentSidecarTooLargeDetail(message: string): string | null {
  const match = message.match(/^Attachment sidecar (.+) exceeds restore cap \((\d+) bytes\)$/)
  return match ? `${match[1]} (> ${match[2]} bytes)` : null
}

function formatAttachmentSidecarReadFailedDetail(message: string): string | null {
  const match = message.match(/^Attachment sidecar (.+) could not be read: (.+)$/)
  return match ? `${match[1]}: ${match[2]}` : null
}

function formatAttachmentSidecarChecksumDetail(message: string): string | null {
  const match = message.match(/^Attachment sidecar (.+) checksum does not match$/)
  return match ? match[1] : null
}

function formatAttachmentSidecarDuplicateDetail(message: string): string | null {
  const match = message.match(/^Multiple sidecars target memory (\d+); keeping the last verified payload$/)
  return match ? `memory_id=${match[1]}` : null
}

function formatNumberedIssue(
  t: MemoryBackupRestorePlanTranslateFn,
  message: string,
  key: string,
  fallback: string,
): string {
  const amount = message.match(/^(\d+)/)?.[1]
  if (!amount) return message
  return `${amount} ${formatBackupPreviewIssueLabel(t, key, fallback)}`
}

function formatNumberedNextStep(
  t: MemoryBackupRestorePlanTranslateFn,
  step: string,
  pattern: RegExp,
  key: string,
  fallback: string,
): string | null {
  const amount = step.match(pattern)?.[1]
  if (!amount) return null
  return `${amount} ${formatBackupPreviewNextStepLabel(t, key, fallback)}`
}

function formatBackupPreviewBool(
  t: MemoryBackupRestorePlanTranslateFn | undefined,
  value: boolean,
): string {
  return value
    ? formatBackupPreviewReportLabel(t, "yes", "yes")
    : formatBackupPreviewReportLabel(t, "no", "no")
}

function formatBackupPreviewToggle(
  t: MemoryBackupRestorePlanTranslateFn | undefined,
  value: boolean,
): string {
  return value
    ? formatBackupPreviewReportLabel(t, "enabled", "enabled")
    : formatBackupPreviewReportLabel(t, "disabled", "disabled")
}

function appendClaimPlan(
  lines: string[],
  plan: MemoryBackupClaimRestorePlan,
  t?: MemoryBackupRestorePlanTranslateFn,
) {
  lines.push("", `## ${formatBackupPreviewReportLabel(t, "structuredClaims", "Structured Claims")}`, "")
  lines.push(
    `- ${formatBackupPreviewReportLabel(t, "claimsInBackup", "Claims in backup")}: ${plan.total}`,
    `- ${formatBackupPreviewReportLabel(t, "importCandidates", "Import candidates")}: ${plan.importCandidates}`,
    `- ${formatBackupPreviewReportLabel(t, "idMatchesSkipped", "ID matches skipped")}: ${plan.existingById}`,
    `- ${formatBackupPreviewReportLabel(t, "exactMatchesSkipped", "Exact matches skipped")}: ${plan.exactMatches}`,
    `- ${formatBackupPreviewReportLabel(t, "conflictsRoutedToReview", "Conflicts routed to review")}: ${plan.conflictingCandidates}`,
    `- ${formatBackupPreviewReportLabel(t, "needsReviewCandidates", "Needs-review candidates")}: ${plan.needsReviewCandidates}`,
    `- ${formatBackupPreviewReportLabel(t, "archivedCandidates", "Archived candidates")}: ${plan.archivedCandidates}`,
    `- ${formatBackupPreviewReportLabel(t, "supersededCandidates", "Superseded candidates")}: ${plan.supersededCandidates}`,
    `- ${formatBackupPreviewReportLabel(t, "expiredCandidates", "Expired candidates")}: ${plan.expiredCandidates}`,
    `- ${formatBackupPreviewReportLabel(t, "manualEvidenceRows", "Manual evidence rows")}: ${plan.manualEvidenceRows}`,
  )
  appendCountMap(lines, formatBackupPreviewReportLabel(t, "claimTypes", "Claim Types"), plan.byType, (key) =>
    formatMemoryBackupClaimType(key, t),
  )
  appendCountMap(
    lines,
    formatBackupPreviewReportLabel(t, "claimStatuses", "Claim Statuses"),
    plan.byStatus,
    (key) => formatMemoryBackupClaimStatus(key, t),
  )
  if (plan.conflictExamples.length > 0) {
    lines.push("", `### ${formatBackupPreviewReportLabel(t, "conflictExamples", "Conflict Examples")}`, "")
    plan.conflictExamples.slice(0, 5).forEach((example, index) => {
      lines.push(
        `${index + 1}. ${formatMemoryBackupClaimConflictHeader(example, t)}`,
        `   - ${formatBackupPreviewReportLabel(t, "incoming", "Incoming")}: ${
          example.incomingObject || example.incomingContent
        }`,
        `   - ${formatBackupPreviewReportLabel(t, "existing", "Existing")}: ${
          example.existingObject || example.existingContent
        }`,
      )
    })
    if (plan.conflictExamples.length > 5) {
      lines.push(
        `- ${plan.conflictExamples.length - 5} ${formatBackupPreviewReportLabel(
          t,
          "moreConflictExamplesOmitted",
          "more conflict example(s) omitted.",
        )}`,
      )
    }
  }
}

function appendProfilePlan(
  lines: string[],
  plan: MemoryBackupProfileRestorePlan,
  allowProfileScopeConflicts: boolean,
  t?: MemoryBackupRestorePlanTranslateFn,
) {
  lines.push("", `## ${formatBackupPreviewReportLabel(t, "profileSnapshots", "Profile Snapshots")}`, "")
  lines.push(
    `- ${formatBackupPreviewReportLabel(t, "snapshotsInBackup", "Snapshots in backup")}: ${plan.total}`,
    `- ${formatBackupPreviewReportLabel(t, "importCandidates", "Import candidates")}: ${plan.importCandidates}`,
    `- ${formatBackupPreviewReportLabel(t, "matchingScopes", "Matching scopes")}: ${plan.matchingScopes}`,
    `- ${formatBackupPreviewReportLabel(t, "exactMatchesSkipped", "Exact matches skipped")}: ${plan.exactMatches}`,
    `- ${formatBackupPreviewReportLabel(t, "scopeConflicts", "Scope conflicts")}: ${plan.conflictingScopeCandidates}`,
    `- ${formatBackupPreviewReportLabel(t, "scopeConflictsWillRestore", "Scope conflicts will restore")}: ${formatBackupPreviewBool(t, allowProfileScopeConflicts)}`,
  )
  appendCountMap(
    lines,
    formatBackupPreviewReportLabel(t, "profileScopeTypes", "Profile Scope Types"),
    plan.byScopeType,
    (key) => formatMemoryBackupClaimScope(key, t),
  )
}

function appendExperiencePlan(
  lines: string[],
  preview: MemoryBackupImportPreview,
  t?: MemoryBackupRestorePlanTranslateFn,
) {
  lines.push(
    "",
    `## ${formatBackupPreviewReportLabel(t, "experienceAndWorkflows", "Experience And Workflows")}`,
    "",
  )
  lines.push(
    `- ${formatBackupPreviewReportLabel(t, "episodesInBackup", "Episodes in backup")}: ${count(preview.episodeCount)}`,
    `- ${formatBackupPreviewReportLabel(t, "episodeImportCandidates", "Episode import candidates")}: ${count(preview.episodeImportCandidates)}`,
    `- ${formatBackupPreviewReportLabel(t, "episodeIdMatchesSkipped", "Episode ID matches skipped")}: ${count(preview.episodeIdMatches)}`,
    `- ${formatBackupPreviewReportLabel(t, "episodeExactMatchesSkipped", "Episode exact matches skipped")}: ${count(preview.episodeExactMatches)}`,
    `- ${formatBackupPreviewReportLabel(t, "proceduresInBackup", "Procedures in backup")}: ${count(preview.procedureCount)}`,
    `- ${formatBackupPreviewReportLabel(t, "procedureImportCandidates", "Procedure import candidates")}: ${count(preview.procedureImportCandidates)}`,
    `- ${formatBackupPreviewReportLabel(t, "procedureIdMatchesSkipped", "Procedure ID matches skipped")}: ${count(preview.procedureIdMatches)}`,
    `- ${formatBackupPreviewReportLabel(t, "procedureExactMatchesSkipped", "Procedure exact matches skipped")}: ${count(preview.procedureExactMatches)}`,
    `- ${formatBackupPreviewReportLabel(t, "experienceHistoryMappable", "Experience history mappable")}: ${count(preview.experienceHistoryRestorable)}/${count(
      preview.experienceHistoryCount,
    )}`,
    `- ${formatBackupPreviewReportLabel(t, "experienceHistorySkippedUnmapped", "Experience history skipped unmapped")}: ${count(preview.experienceHistorySkippedUnmapped)}`,
  )
}

function appendAttachmentPlan(
  lines: string[],
  preview: MemoryBackupImportPreview,
  t?: MemoryBackupRestorePlanTranslateFn,
) {
  lines.push("", `## ${formatBackupPreviewReportLabel(t, "attachments", "Attachments")}`, "")
  lines.push(
    `- ${formatBackupPreviewReportLabel(t, "attachmentRefs", "Attachment refs")}: ${preview.attachmentRefCount}`,
    `- ${formatBackupPreviewReportLabel(t, "inlinePayloads", "Inline payloads")}: ${preview.attachmentPayloadCount}`,
    `- ${formatBackupPreviewReportLabel(t, "payloadChunks", "Payload chunks")}: ${preview.attachmentChunkCount}`,
    `- ${formatBackupPreviewReportLabel(t, "chunkedRefs", "Chunked refs")}: ${preview.attachmentChunkedRefCount}`,
    `- ${formatBackupPreviewReportLabel(t, "externalSidecarRefs", "External sidecar refs")}: ${preview.attachmentExternalRefCount}`,
    `- ${formatBackupPreviewReportLabel(t, "verifiedSidecars", "Verified sidecars")}: ${preview.attachmentExternalAvailableCount}`,
    `- ${formatBackupPreviewReportLabel(t, "missingAttachments", "Missing attachments")}: ${preview.attachmentMissingCount}`,
  )
}

function appendCountMap(
  lines: string[],
  title: string,
  counts: Record<string, number>,
  formatKey: (key: string) => string = (key) => key,
) {
  const entries = sortedCountEntries(counts)
  if (entries.length === 0) return
  lines.push("", `### ${title}`, "")
  for (const [key, value] of entries) {
    lines.push(`- ${formatKey(key)}: ${value}`)
  }
}

function appendList(lines: string[], title: string, values: string[]) {
  if (values.length === 0) return
  lines.push("", `## ${title}`, "")
  for (const value of values) {
    lines.push(`- ${value}`)
  }
}

function sortedCountEntries(counts: Record<string, number>): [string, number][] {
  return Object.entries(counts).sort(([leftKey, leftValue], [rightKey, rightValue]) => {
    const byCount = rightValue - leftValue
    if (byCount !== 0) return byCount
    return leftKey.localeCompare(rightKey)
  })
}

function sortedStrings(values: string[]): string[] {
  return [...values].sort((left, right) => left.localeCompare(right))
}

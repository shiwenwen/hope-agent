import type { TFunction } from "i18next"
import { toast } from "sonner"
import type { MemoryImportPreview, MemoryImportPreviewIssue, MemoryScope } from "./types"
import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export interface MemoryImportResult {
  created: number
  skippedDuplicate: number
  failed: number
  errors?: string[]
}

export function memoryImportTotal(result: MemoryImportResult): number {
  return result.created + result.skippedDuplicate + result.failed
}

const MEMORY_IMPORT_DIAGNOSTIC_MAX_CHARS = 420

export function memoryImportDiagnosticText(
  value: string,
  maxChars = MEMORY_IMPORT_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryImportErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? memoryImportDiagnosticText(detail) : null
}

export function formatMemoryImportPromptLoadError(t: TFunction, error: unknown): string {
  const detail = memoryImportErrorDetail(error)
  if (!detail) {
    return t("settings.memoryImportFromAIPromptLoadFailed", "Failed to load import prompt.")
  }
  return t("settings.memoryImportFromAIPromptLoadError", "Failed to load import prompt: {{error}}", {
    error: detail,
  })
}

export type MemoryImportOperation = "preview" | "apply" | "copyDiagnostics" | "copyPrompt"

export function formatMemoryImportOperationError(
  t: TFunction,
  operation: MemoryImportOperation,
  error: unknown,
): string {
  const detail = memoryImportErrorDetail(error)
  if (operation === "preview") {
    if (!detail) {
      return t("settings.memoryImportPreviewFailed", "Failed to preview memory import.")
    }
    return t("settings.memoryImportPreviewError", "Failed to preview memory import: {{error}}", {
      error: detail,
    })
  }
  if (operation === "copyDiagnostics") {
    if (!detail) {
      return t(
        "settings.memoryImportCopyDiagnosticsFailed",
        "Failed to copy memory import diagnostics.",
      )
    }
    return t(
      "settings.memoryImportCopyDiagnosticsError",
      "Failed to copy memory import diagnostics: {{error}}",
      { error: detail },
    )
  }
  if (operation === "copyPrompt") {
    if (!detail) {
      return t("settings.memoryImportFromAIPromptCopyFailed", "Failed to copy import prompt.")
    }
    return t(
      "settings.memoryImportFromAIPromptCopyError",
      "Failed to copy import prompt: {{error}}",
      { error: detail },
    )
  }
  if (!detail) {
    return t("settings.memoryImportApplyFailed", "Failed to import memories.")
  }
  return t("settings.memoryImportApplyError", "Failed to import memories: {{error}}", {
    error: detail,
  })
}

export function formatMemoryImportResultError(
  t: TFunction,
  error?: string | null,
): string | null {
  const detail = error?.trim()
  if (!detail) return null
  return t("settings.memoryImportFirstError", "First failed item: {{error}}", {
    error: memoryImportDiagnosticText(detail),
  })
}

export function memoryImportPreviewDescription(
  t: TFunction,
  preview?: MemoryImportPreview | null,
): string | undefined {
  if (!preview) return undefined
  const title = t("settings.memoryImportPreviewTitle", "Preview")
  if (!preview.dedupChecked) {
    if (preview.issues.some((issue) => issue.code === "dedup_preview_failed")) {
      return `${title}: ${memoryImportPreviewDedupStatusLabel(t, preview)}`
    }
    return undefined
  }

  const likelyImportCount =
    (preview.likelyNewCount ?? preview.candidateCount ?? 0) + (preview.likelyMergeCount ?? 0)
  const parts = [
    t("settings.memoryImportPreviewLikelyImport", "{{count}} will import", {
      count: likelyImportCount,
    }),
  ]

  if ((preview.likelyDuplicateCount ?? 0) > 0) {
    parts.push(
      t("settings.memoryImportPreviewLikelyDuplicate", "{{count}} duplicates", {
        count: preview.likelyDuplicateCount,
      }),
    )
  }
  if ((preview.likelyMergeCount ?? 0) > 0) {
    parts.push(
      t("settings.memoryImportPreviewLikelyMerge", "{{count}} may merge", {
        count: preview.likelyMergeCount,
      }),
    )
  }

  return `${title}: ${parts.join(" · ")}`
}

export function formatMemoryImportPreviewIssueMessage(
  t: TFunction,
  issue: MemoryImportPreviewIssue,
): string {
  switch (issue.code) {
    case "no_importable_entries":
      return t("settings.memoryImportIssueNoImportableEntries", "No importable memories found.")
    case "dedup_preview_failed":
      return withImportIssueDetail(
        t,
        "settings.memoryImportIssueDedupPreviewFailed",
        "Could not estimate duplicates",
        detailAfter(issue.message, "Could not estimate duplicates: ") ?? issue.message,
      )
    case "parse_error":
      return withImportIssueDetail(
        t,
        "settings.memoryImportIssueParseError",
        "Could not parse this memory import",
        issue.message,
      )
    default:
      return issue.message
  }
}

export function memoryImportPreviewIssueMessages(
  t: TFunction,
  preview: Pick<MemoryImportPreview, "issues">,
  limit = 5,
): string[] {
  return preview.issues
    .slice(0, Math.max(0, limit))
    .map((issue) => formatMemoryImportPreviewIssueMessage(t, issue))
}

export function memoryImportSortedCountEntries(
  counts: Record<string, number>,
): Array<[string, number]> {
  return Object.entries(counts).sort(([leftKey, leftCount], [rightKey, rightCount]) => {
    if (leftCount !== rightCount) return rightCount - leftCount
    if (leftKey < rightKey) return -1
    if (leftKey > rightKey) return 1
    return 0
  })
}

export function memoryImportPreviewIsCurrent(
  preview: Pick<MemoryImportPreview, "valid"> | null | undefined,
  matchesInput: boolean,
): boolean {
  return Boolean(preview && matchesInput)
}

export function memoryImportPreviewCanApply(
  preview: Pick<MemoryImportPreview, "valid"> | null | undefined,
  matchesInput: boolean,
): boolean {
  return memoryImportPreviewIsCurrent(preview, matchesInput) && preview?.valid === true
}

export function memoryImportPreviewStatusLabel(
  t: TFunction,
  preview: Pick<MemoryImportPreview, "valid">,
): string {
  return preview.valid
    ? t("settings.memoryImportPreviewReady", "Ready to import")
    : t("settings.memoryImportPreviewBlocked", "Cannot import")
}

export function memoryImportPreviewDedupStatusLabel(
  t: TFunction,
  preview: Pick<MemoryImportPreview, "dedupChecked" | "issues">,
): string {
  if (preview.dedupChecked === true) {
    return t("settings.memoryImportPreviewDedupChecked", "Duplicate estimate ready")
  }
  if (preview.issues.some((issue) => issue.code === "dedup_preview_failed")) {
    return t("settings.memoryImportPreviewDedupUnavailable", "Duplicate estimate unavailable")
  }
  return t("settings.memoryImportPreviewDedupNotChecked", "Duplicate estimate not checked")
}

export function memoryImportPreviewSampleWindowLabel(
  t: TFunction,
  total: number,
  visible: number,
): string | null {
  if (total <= 0) return t("settings.memoryImportPreviewNoSamples", "No preview samples")
  if (visible < total) {
    return t(
      "settings.memoryImportPreviewSamplesShown",
      "Showing {{visible}} of {{total}} samples",
      { visible, total },
    )
  }
  return null
}

export function formatMemoryImportScopeLabel(t: TFunction, scope: MemoryScope): string {
  if (scope.kind === "global") return t("settings.memoryScopeGlobal", "Global")
  if (scope.kind === "agent") return `${t("settings.memoryScopeAgent", "Agent")}: ${scope.id}`
  return `${t("settings.memoryScopeProject", "Project")}: ${scope.id}`
}

export function formatMemoryImportScopeSummaryKey(t: TFunction, scopeKey: string): string {
  if (scopeKey === "global") return t("settings.memoryScopeGlobal", "Global")
  const agentPrefix = "agent:"
  if (scopeKey.startsWith(agentPrefix)) {
    return `${t("settings.memoryScopeAgent", "Agent")}: ${scopeKey.slice(agentPrefix.length)}`
  }
  const projectPrefix = "project:"
  if (scopeKey.startsWith(projectPrefix)) {
    return `${t("settings.memoryScopeProject", "Project")}: ${scopeKey.slice(projectPrefix.length)}`
  }
  return scopeKey
}

export function showMemoryImportResultToast(
  t: TFunction,
  result: MemoryImportResult,
  preview?: MemoryImportPreview | null,
) {
  const message = t("settings.memoryImportSuccess", {
    created: result.created,
    skipped: result.skippedDuplicate,
    failed: result.failed,
  })
  const description = [
    result.failed > 0 ? formatMemoryImportResultError(t, result.errors?.[0]) : null,
    memoryImportPreviewDescription(t, preview),
  ]
    .filter((item): item is string => Boolean(item))
    .join("\n")
  const options = description ? { description } : undefined

  if (result.failed > 0) {
    toast.warning(message, options)
  } else {
    toast.success(message, options)
  }
}

export function formatMemoryImportPreviewDiagnostics(
  t: TFunction,
  preview: MemoryImportPreview,
  sourceLabel?: string | null,
): string {
  const likelyImportCount =
    (preview.likelyNewCount ?? preview.candidateCount ?? 0) + (preview.likelyMergeCount ?? 0)
  const lines = [
    `# ${formatMemoryImportReportLabel(t, "title", "Memory Import Preview")}`,
    "",
    `- ${formatMemoryImportReportLabel(t, "source", "Source")}: ${
      sourceLabel || t("settings.memoryImport", "Import")
    }`,
    `- ${formatMemoryImportReportLabel(t, "generated", "Generated")}: ${new Date().toISOString()}`,
    `- ${formatMemoryImportReportLabel(t, "valid", "Valid")}: ${formatMemoryImportReportBool(
      t,
      preview.valid === true,
    )}`,
    `- ${formatMemoryImportReportLabel(t, "format", "Format")}: ${preview.format || "auto"}`,
    `- ${formatMemoryImportReportLabel(t, "candidates", "Candidates")}: ${preview.candidateCount}`,
    `- ${formatMemoryImportReportLabel(t, "samplesIncluded", "Samples included")}: ${
      preview.samples.length
    }`,
    `- ${formatMemoryImportReportLabel(t, "dedupChecked", "Dedup checked")}: ${formatMemoryImportReportBool(
      t,
      preview.dedupChecked === true,
    )}`,
    `- ${formatMemoryImportReportLabel(t, "dedupStatus", "Dedup status")}: ${memoryImportPreviewDedupStatusLabel(
      t,
      preview,
    )}`,
  ]

  if (preview.dedupChecked) {
    lines.push(
      `- ${formatMemoryImportReportLabel(t, "estimatedImport", "Estimated import")}: ${likelyImportCount}`,
      `- ${formatMemoryImportReportLabel(t, "likelyNew", "Likely new")}: ${
        preview.likelyNewCount ?? 0
      }`,
      `- ${formatMemoryImportReportLabel(t, "likelyMerge", "Likely merge")}: ${
        preview.likelyMergeCount ?? 0
      }`,
      `- ${formatMemoryImportReportLabel(t, "likelyDuplicate", "Likely duplicate")}: ${
        preview.likelyDuplicateCount ?? 0
      }`,
    )
  }

  appendCountSection(
    lines,
    formatMemoryImportReportLabel(t, "types", "Types"),
    preview.byType,
    (type) => `${t(`settings.memoryType_${type}`, type)}`,
  )
  appendCountSection(
    lines,
    formatMemoryImportReportLabel(t, "scopes", "Scopes"),
    preview.byScope,
    (scope) => formatMemoryImportScopeSummaryKey(t, scope),
  )

  if (preview.issues.length > 0) {
    lines.push("", `## ${formatMemoryImportReportLabel(t, "issues", "Issues")}`)
    for (const issue of preview.issues) {
      lines.push(`- ${issue.code}: ${formatMemoryImportPreviewIssueMessage(t, issue)}`)
    }
  }

  lines.push("", `## ${formatMemoryImportReportLabel(t, "samples", "Samples")}`)
  if (preview.samples.length === 0) {
    lines.push(`- ${formatMemoryImportReportLabel(t, "none", "(none)")}`)
  } else {
    preview.samples.forEach((sample, index) => {
      const meta = [
        t(`settings.memoryType_${sample.memoryType}`, sample.memoryType),
        formatMemoryImportScopeLabel(t, sample.scope),
        sample.dedupStatus
          ? t(`settings.memoryImportPreviewSample_${sample.dedupStatus}`, sample.dedupStatus)
          : null,
        sample.tags.length > 0
          ? `${formatMemoryImportReportLabel(t, "tags", "tags")}: ${sample.tags.join(", ")}`
          : null,
      ].filter((item): item is string => Boolean(item))
      lines.push(`${index + 1}. ${meta.join(" | ")}`)
      lines.push(`   ${sample.contentPreview}`)
      if (sample.dedupExistingPreview) {
        const score =
          typeof sample.dedupScore === "number"
            ? `, ${formatMemoryImportReportLabel(t, "score", "score")}=${sample.dedupScore.toFixed(4)}`
            : ""
        lines.push(
          `   ${formatMemoryImportReportLabel(t, "existing", "Existing")} #${
            sample.dedupExistingId ?? "?"
          }${score}: ${
            sample.dedupExistingPreview
          }`,
        )
      }
    })
  }

  return lines.join("\n")
}

export async function copyMemoryImportPreviewDiagnostics(
  t: TFunction,
  preview: MemoryImportPreview,
  sourceLabel?: string | null,
) {
  try {
    await navigator.clipboard.writeText(formatMemoryImportPreviewDiagnostics(t, preview, sourceLabel))
    toast.success(t("common.copied", "Copied"))
  } catch (error) {
    toast.error(formatMemoryImportOperationError(t, "copyDiagnostics", error))
  }
}

function appendCountSection(
  lines: string[],
  title: string,
  counts: Record<string, number>,
  label?: (key: string) => string,
) {
  const entries = memoryImportSortedCountEntries(counts)
  if (entries.length === 0) return
  lines.push("", `## ${title}`)
  for (const [key, count] of entries) {
    lines.push(`- ${label ? label(key) : key}: ${count}`)
  }
}

function detailAfter(message: string, prefix: string): string | null {
  return message.startsWith(prefix) ? message.slice(prefix.length).trim() : null
}

function withImportIssueDetail(
  t: TFunction,
  key: string,
  fallback: string,
  detail: string | null,
): string {
  const label = t(key, fallback)
  return detail ? `${label}: ${detail}` : label
}

function formatMemoryImportReportLabel(
  t: TFunction,
  key: string,
  fallback: string,
): string {
  const translated = t(`settings.memoryImportPreviewReport.${key}`, fallback)
  return typeof translated === "string" && translated.trim().length > 0 ? translated : fallback
}

function formatMemoryImportReportBool(t: TFunction, value: boolean): string {
  return value
    ? formatMemoryImportReportLabel(t, "yes", "yes")
    : formatMemoryImportReportLabel(t, "no", "no")
}

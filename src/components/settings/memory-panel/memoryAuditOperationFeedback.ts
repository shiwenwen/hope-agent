import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type MemoryAuditOperation = "search" | "loadMore" | "exportCurrent" | "exportAll"
export type MemoryAuditDegradedSource = "unified" | "legacyPage" | "experience" | "decisions"

export type MemoryAuditFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryAuditOperationErrorToast {
  title: string
  description?: string
}

export interface MemoryAuditDegradedIssue {
  source: MemoryAuditDegradedSource
  detail?: string | null
}

const MEMORY_AUDIT_DIAGNOSTIC_MAX_CHARS = 420

export function memoryAuditOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? memoryAuditDiagnosticText(detail) : null
}

export function memoryAuditDiagnosticText(
  value: string,
  maxChars = MEMORY_AUDIT_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryAuditDegradedIssue(
  source: MemoryAuditDegradedSource,
  error: unknown,
): MemoryAuditDegradedIssue {
  return { source, detail: memoryAuditOperationErrorDetail(error) }
}

export function memoryAuditOperationErrorToast(
  operation: MemoryAuditOperation,
  t: MemoryAuditFeedbackTranslateFn,
  error: unknown,
): MemoryAuditOperationErrorToast {
  const detail = memoryAuditOperationErrorDetail(error)
  const title = t(`settings.memoryAuditErrors.${operation}`, {
    defaultValue: memoryAuditOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryAuditErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function memoryAuditDegradedWarning(
  issues: MemoryAuditDegradedIssue[],
  t: MemoryAuditFeedbackTranslateFn,
): MemoryAuditOperationErrorToast | null {
  if (issues.length === 0) return null
  const sources = uniqueDegradedSources(issues)
    .map((source) => memoryAuditDegradedSourceLabel(source, t))
    .join(", ")
  const detail = issues.map((issue) => issue.detail?.trim()).find(Boolean)
  const title = t("settings.memoryAuditWarnings.degraded", {
    defaultValue: "Memory activity results may be incomplete",
  })
  if (!detail) {
    return {
      title,
      description: t("settings.memoryAuditWarnings.degradedSources", {
        defaultValue: "Fallback used for: {{sources}}.",
        sources,
      }),
    }
  }
  return {
    title,
    description: t("settings.memoryAuditWarnings.degradedDetail", {
      defaultValue: "Fallback used for: {{sources}}. Details: {{error}}",
      sources,
      error: detail,
    }),
  }
}

function uniqueDegradedSources(issues: MemoryAuditDegradedIssue[]): MemoryAuditDegradedSource[] {
  const seen = new Set<MemoryAuditDegradedSource>()
  const sources: MemoryAuditDegradedSource[] = []
  for (const issue of issues) {
    if (seen.has(issue.source)) continue
    seen.add(issue.source)
    sources.push(issue.source)
  }
  return sources
}

function memoryAuditDegradedSourceLabel(
  source: MemoryAuditDegradedSource,
  t: MemoryAuditFeedbackTranslateFn,
): string {
  return t(`settings.memoryAuditWarnings.sources.${source}`, {
    defaultValue: memoryAuditDegradedSourceFallback(source),
  })
}

function memoryAuditDegradedSourceFallback(source: MemoryAuditDegradedSource): string {
  switch (source) {
    case "unified":
      return "unified audit query"
    case "legacyPage":
      return "long-term memory paging"
    case "experience":
      return "workflow history"
    case "decisions":
      return "structured-memory decisions"
  }
}

function memoryAuditOperationFallback(operation: MemoryAuditOperation): string {
  switch (operation) {
    case "search":
      return "Failed to search memory activity"
    case "loadMore":
      return "Failed to load more memory activity"
    case "exportCurrent":
      return "Failed to copy current memory activity"
    case "exportAll":
      return "Failed to copy all matching memory activity"
  }
}

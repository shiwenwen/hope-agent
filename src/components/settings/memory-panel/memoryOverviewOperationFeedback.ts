import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type MemoryOverviewLoadSource =
  | "history"
  | "memories"
  | "corrections"
  | "dreamingRuns"
  | "dreamingRunDetail"
  | "episodes"
  | "procedures"
  | "experienceHistory"
  | "recentActivity"

export type MemoryOverviewInsightsSource =
  | "profileClaims"
  | "projectClaims"
  | "profileSnapshots"
  | "insights"

export type MemoryOverviewFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryOverviewLoadIssue {
  source: MemoryOverviewLoadSource
  detail?: string | null
}

export interface MemoryOverviewLoadWarning {
  title: string
  description: string
}

export interface MemoryOverviewInsightsIssue {
  source: MemoryOverviewInsightsSource
  detail?: string | null
}

export interface MemoryOverviewOperationErrorToast {
  title: string
  description?: string
}

const MEMORY_OVERVIEW_DIAGNOSTIC_MAX_CHARS = 420

export function memoryOverviewOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return memoryOverviewDiagnosticText(detail)
}

export function memoryOverviewDiagnosticText(
  value: string,
  maxChars = MEMORY_OVERVIEW_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryOverviewLoadIssue(
  source: MemoryOverviewLoadSource,
  error: unknown,
): MemoryOverviewLoadIssue {
  return { source, detail: memoryOverviewOperationErrorDetail(error) }
}

export function memoryOverviewInsightsIssue(
  source: MemoryOverviewInsightsSource,
  error: unknown,
): MemoryOverviewInsightsIssue {
  return { source, detail: memoryOverviewOperationErrorDetail(error) }
}

export function memoryOverviewLoadWarning(
  issues: MemoryOverviewLoadIssue[],
  t: MemoryOverviewFeedbackTranslateFn,
): MemoryOverviewLoadWarning | null {
  if (issues.length === 0) return null
  const sources = uniqueSources(issues)
    .map((source) => memoryOverviewSourceLabel(source, t))
    .join(", ")
  const firstDetail = issues.map((issue) => issue.detail?.trim()).find(Boolean)
  const title = t("settings.memoryOverviewErrors.recentActivity", {
    defaultValue: "Some recent memory activity could not load",
  })
  if (!firstDetail) {
    return {
      title,
      description: t("settings.memoryOverviewErrors.recentActivitySources", {
        defaultValue: "Unavailable: {{sources}}.",
        sources,
      }),
    }
  }
  return {
    title,
    description: t("settings.memoryOverviewErrors.recentActivityDetail", {
      defaultValue: "Unavailable: {{sources}}. Details: {{error}}",
      sources,
      error: firstDetail,
    }),
  }
}

export function memoryOverviewInsightsWarning(
  issues: MemoryOverviewInsightsIssue[],
  t: MemoryOverviewFeedbackTranslateFn,
): MemoryOverviewLoadWarning | null {
  if (issues.length === 0) return null
  const sources = uniqueInsightsSources(issues)
    .map((source) => memoryOverviewInsightsSourceLabel(source, t))
    .join(", ")
  const firstDetail = issues.map((issue) => issue.detail?.trim()).find(Boolean)
  const title = t("settings.memoryOverviewErrors.insights", {
    defaultValue: "Some memory insights could not load",
  })
  if (!firstDetail) {
    return {
      title,
      description: t("settings.memoryOverviewErrors.insightsSources", {
        defaultValue: "Unavailable: {{sources}}.",
        sources,
      }),
    }
  }
  return {
    title,
    description: t("settings.memoryOverviewErrors.insightsDetail", {
      defaultValue: "Unavailable: {{sources}}. Details: {{error}}",
      sources,
      error: firstDetail,
    }),
  }
}

export function memoryOverviewPendingClaimsErrorToast(
  t: MemoryOverviewFeedbackTranslateFn,
  error: unknown,
): MemoryOverviewOperationErrorToast {
  const detail = memoryOverviewOperationErrorDetail(error)
  const title = t("settings.memoryOverviewErrors.pendingClaims", {
    defaultValue: "Failed to load review queue",
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryOverviewErrors.pendingClaimsDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function memoryOverviewOpenMemoryErrorToast(
  t: MemoryOverviewFeedbackTranslateFn,
  error: unknown,
): MemoryOverviewOperationErrorToast {
  const detail = memoryOverviewOperationErrorDetail(error)
  const title = t("settings.memoryOverviewErrors.openMemory", {
    defaultValue: "Failed to open memory activity",
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryOverviewErrors.openMemoryDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function uniqueSources(issues: MemoryOverviewLoadIssue[]): MemoryOverviewLoadSource[] {
  const seen = new Set<MemoryOverviewLoadSource>()
  const sources: MemoryOverviewLoadSource[] = []
  for (const issue of issues) {
    if (seen.has(issue.source)) continue
    seen.add(issue.source)
    sources.push(issue.source)
  }
  return sources
}

function uniqueInsightsSources(issues: MemoryOverviewInsightsIssue[]): MemoryOverviewInsightsSource[] {
  const seen = new Set<MemoryOverviewInsightsSource>()
  const sources: MemoryOverviewInsightsSource[] = []
  for (const issue of issues) {
    if (seen.has(issue.source)) continue
    seen.add(issue.source)
    sources.push(issue.source)
  }
  return sources
}

function memoryOverviewSourceLabel(
  source: MemoryOverviewLoadSource,
  t: MemoryOverviewFeedbackTranslateFn,
): string {
  return t(`settings.memoryOverviewErrors.sources.${source}`, {
    defaultValue: memoryOverviewSourceFallback(source),
  })
}

function memoryOverviewInsightsSourceLabel(
  source: MemoryOverviewInsightsSource,
  t: MemoryOverviewFeedbackTranslateFn,
): string {
  return t(`settings.memoryOverviewErrors.insightsSourcesMap.${source}`, {
    defaultValue: memoryOverviewInsightsSourceFallback(source),
  })
}

function memoryOverviewSourceFallback(source: MemoryOverviewLoadSource): string {
  switch (source) {
    case "history":
      return "memory history"
    case "memories":
      return "latest memories"
    case "corrections":
      return "recent corrections"
    case "dreamingRuns":
      return "Dreaming runs"
    case "dreamingRunDetail":
      return "Dreaming decisions"
    case "episodes":
      return "episodes"
    case "procedures":
      return "workflows"
    case "experienceHistory":
      return "experience history"
    case "recentActivity":
      return "recent activity"
  }
}

function memoryOverviewInsightsSourceFallback(source: MemoryOverviewInsightsSource): string {
  switch (source) {
    case "profileClaims":
      return "profile memories"
    case "projectClaims":
      return "project memories"
    case "profileSnapshots":
      return "profile snapshots"
    case "insights":
      return "memory insights"
  }
}

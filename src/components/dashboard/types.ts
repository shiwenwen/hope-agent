export interface DashboardFilter {
  startDate: string | null
  endDate: string | null
  agentId: string | null
  providerId: string | null
  modelId: string | null
}

export interface OverviewStats {
  totalSessions: number
  totalMessages: number
  totalInputTokens: number
  totalOutputTokens: number
  totalToolCalls: number
  totalErrors: number
  activeAgents: number
  activeCronJobs: number
  estimatedCostUsd: number
  avgTtftMs: number | null
}

export interface OverviewStatsWithDelta {
  current: OverviewStats
  previous: OverviewStats | null
}

export interface TokenUsageTrend {
  date: string
  inputTokens: number
  outputTokens: number
  avgTtftMs: number | null
}

export interface TokenByModel {
  modelId: string
  providerName: string
  inputTokens: number
  outputTokens: number
  estimatedCostUsd: number
  avgTtftMs: number | null
}

export interface DashboardTokenData {
  trend: TokenUsageTrend[]
  byModel: TokenByModel[]
  totalCostUsd: number
}

export interface ToolUsageStats {
  toolName: string
  callCount: number
  errorCount: number
  avgDurationMs: number
  totalDurationMs: number
}

export interface SessionTrend {
  date: string
  sessionCount: number
  messageCount: number
}

export interface SessionByAgent {
  agentId: string
  sessionCount: number
  messageCount: number
  totalTokens: number
}

export interface DashboardSessionData {
  trend: SessionTrend[]
  byAgent: SessionByAgent[]
}

export interface ErrorTrend {
  date: string
  errorCount: number
  warnCount: number
}

export interface ErrorByCategory {
  category: string
  count: number
}

export interface DashboardErrorData {
  trend: ErrorTrend[]
  byCategory: ErrorByCategory[]
  totalErrors: number
  totalWarnings: number
}

export interface CronJobStats {
  totalJobs: number
  activeJobs: number
  totalRuns: number
  successRuns: number
  failedRuns: number
  avgDurationMs: number
}

export interface SubagentStats {
  totalRuns: number
  completed: number
  failed: number
  killed: number
  totalInputTokens: number
  totalOutputTokens: number
  avgDurationMs: number
}

export interface DashboardTaskData {
  cron: CronJobStats
  subagent: SubagentStats
}

export interface ProcessMemoryInfo {
  rssBytes: number
  virtualBytes: number
  systemTotalBytes: number
  rssPercent: number
}

export interface ProcessDiskIO {
  readBytes: number
  writtenBytes: number
}

export interface SystemMetrics {
  processCpuPercent: number
  cpuCount: number
  memory: ProcessMemoryInfo
  diskIo: ProcessDiskIO
  processUptimeSecs: number
  pid: number
  osName: string
  hostName: string
  systemUptimeSecs: number
}

// ── Detail List Types ───────────────────────────────────────────

export type DetailListType = "sessions" | "messages" | "toolCalls" | "errors" | "agents" | "cronJobs"

export interface DashboardSessionItem {
  id: string
  title: string | null
  agentId: string
  modelId: string | null
  messageCount: number
  totalTokens: number
  createdAt: string
  updatedAt: string
}

export interface DashboardMessageItem {
  id: number
  sessionId: string
  sessionTitle: string | null
  role: string
  contentPreview: string
  tokensIn: number
  tokensOut: number
  timestamp: string
}

export interface DashboardToolCallItem {
  id: number
  sessionId: string
  sessionTitle: string | null
  toolName: string
  isError: boolean
  durationMs: number | null
  timestamp: string
}

export interface DashboardErrorItem {
  id: number
  level: string
  category: string
  source: string
  message: string
  sessionId: string | null
  timestamp: string
}

export interface DashboardAgentItem {
  agentId: string
  sessionCount: number
  messageCount: number
  totalTokens: number
  lastActiveAt: string
}

export type CronSchedule =
  | { type: "at"; timestamp: string }
  | {
      type: "every"
      intervalMs?: number
      interval_ms?: number
      startAt?: string | null
      start_at?: string | null
    }
  | { type: "cron"; expression: string; timezone?: string }

export interface CronJob {
  id: string
  name: string
  description: string | null
  schedule: CronSchedule
  status: string
  nextRunAt: string | null
  lastRunAt: string | null
  runningAt: string | null
  consecutiveFailures: number
  maxFailures: number
  createdAt: string
  updatedAt: string
  notifyOnComplete: boolean
}

export type Granularity = "day" | "week" | "month"

// ── Insights Types (Phase 2) ────────────────────────────────────

export interface CostTrendPoint {
  date: string
  costUsd: number
  inputTokens: number
  outputTokens: number
}

export interface DashboardCostTrend {
  points: CostTrendPoint[]
  totalCostUsd: number
  peakDay: string | null
  peakCostUsd: number
  avgDailyCostUsd: number
}

export interface HeatmapCell {
  weekday: number // 0=Sun..6=Sat
  hour: number // 0..23
  messageCount: number
}

export interface DashboardHeatmap {
  cells: HeatmapCell[]
  maxValue: number
  total: number
}

export interface HourlyBucket {
  hour: number
  messageCount: number
  sessionCount: number
}

export interface DashboardHourlyDistribution {
  buckets: HourlyBucket[]
  peakHour: number | null
  peakMessageCount: number
}

export interface TopSession {
  id: string
  title: string | null
  agentId: string
  modelId: string | null
  totalTokens: number
  messageCount: number
  estimatedCostUsd: number
  updatedAt: string
}

export interface ModelEfficiency {
  modelId: string
  providerName: string
  totalTokens: number
  totalCostUsd: number
  avgTtftMs: number | null
  messageCount: number
  avgTokensPerMessage: number
  avgCostPer1kTokens: number
}

export interface HealthBreakdown {
  score: number
  logErrorRatePercent: number
  toolErrorRatePercent: number
  cronSuccessRatePercent: number
  subagentSuccessRatePercent: number
  status: "excellent" | "good" | "warning" | "critical"
}

export interface DashboardInsights {
  health: HealthBreakdown
  costTrend: DashboardCostTrend
  heatmap: DashboardHeatmap
  hourly: DashboardHourlyDistribution
  topSessions: TopSession[]
  modelEfficiency: ModelEfficiency[]
}

export type AutoRefreshInterval = "off" | "30s" | "1m" | "5m"

export function autoRefreshMs(interval: AutoRefreshInterval): number {
  switch (interval) {
    case "30s": return 30_000
    case "1m": return 60_000
    case "5m": return 300_000
    default: return 0
  }
}

/** Format percentage delta between current and previous. */
export function computeDelta(current: number, previous: number): number | null {
  if (!Number.isFinite(current) || !Number.isFinite(previous)) return null
  if (previous === 0) {
    if (current === 0) return 0
    return null // infinite delta — don't display a number
  }
  return ((current - previous) / previous) * 100
}

/** Format a delta percent to a "+12.3%" or "-4.5%" string. */
export function formatDelta(delta: number | null): string {
  if (delta == null) return ""
  const sign = delta > 0 ? "+" : ""
  if (Math.abs(delta) >= 1000) return `${sign}${(delta / 1000).toFixed(1)}K%`
  return `${sign}${delta.toFixed(1)}%`
}

/** Format large numbers as "1.2M", "45.6K", etc. */
export function formatNumber(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

/**
 * All current callers are single-series tooltip / labelFormatter slots, so the
 * input is expected to be `number | string`. Recharts also types it as
 * potentially an array (stacked tooltips) — we tolerate that by taking the
 * first element to keep the chart from blanking, but warn in dev so a
 * future stacked-chart caller surfaces immediately instead of silently
 * displaying only one of the series.
 */
const chartNumberArrayWarned = new Set<string>()
export function chartNumber(value: unknown): number {
  let raw: unknown = value
  if (Array.isArray(value)) {
    if (import.meta.env.DEV) {
      const key = value.length === 0 ? "empty" : typeof value[0]
      if (!chartNumberArrayWarned.has(key)) {
        chartNumberArrayWarned.add(key)
        console.warn(
          "[dashboard] chartNumber received an array — only first element is used. " +
            "Stacked tooltips need a dedicated formatter.",
          value,
        )
      }
    }
    raw = value[0]
  }
  if (typeof raw === "number") return raw
  if (typeof raw === "string") {
    const parsed = Number(raw)
    return Number.isFinite(parsed) ? parsed : 0
  }
  return 0
}

export function chartName(value: unknown): string {
  return typeof value === "string" || typeof value === "number" ? String(value) : ""
}

/** Format USD currency */
export function formatCost(n: number): string {
  return `$${n.toFixed(2)}`
}

/** Format seconds to human readable uptime */
export function formatUptime(secs: number): string {
  const days = Math.floor(secs / 86400)
  const hours = Math.floor((secs % 86400) / 3600)
  const minutes = Math.floor((secs % 3600) / 60)
  if (days > 0) return `${days}d ${hours}h ${minutes}m`
  if (hours > 0) return `${hours}h ${minutes}m`
  return `${minutes}m`
}

/** Format milliseconds to human readable */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60_000).toFixed(1)}m`
}

// ── Recap Types ────────────────────────────────────────────────

export type Outcome =
  | "fully_achieved"
  | "mostly_achieved"
  | "partial"
  | "failed"
  | "unclear"

export interface FrictionCounts {
  toolErrors: number
  misunderstanding: number
  repetition: number
  userCorrection: number
  stuck: number
  other: number
}

export interface SessionFacet {
  sessionId: string
  underlyingGoal: string
  goalCategories: string[]
  outcome: Outcome
  userSatisfaction: number | null
  agentHelpfulness: number | null
  sessionType: string
  frictionCounts: FrictionCounts
  frictionDetail: string[]
  primarySuccess: string | null
  briefSummary: string
  userInstructions: string[]
}

export interface FacetSummary {
  totalFacets: number
  goalHistogram: [string, number][]
  outcomeDistribution: [string, number][]
  sessionTypeDistribution: [string, number][]
  frictionTop: [string, number][]
  satisfactionDistribution: [number, number][]
  repeatUserInstructions: [string, number][]
  successExamples: string[]
  frictionExamples: string[]
}

export interface QuantitativeStats {
  overview: OverviewStatsWithDelta
  health: HealthBreakdown
  costTrend: DashboardCostTrend
  heatmap: DashboardHeatmap
  hourly: DashboardHourlyDistribution
  topSessions: TopSession[]
  modelEfficiency: ModelEfficiency[]
}

export interface AiSection {
  key: string
  title: string
  markdown: string
}

export interface ReportMeta {
  id: string
  title: string
  rangeStart: string
  rangeEnd: string
  sessionCount: number
  generatedAt: string
  analysisModel: string
  filters: DashboardFilter
  schemaVersion: number
}

export interface RecapReport {
  meta: ReportMeta
  quantitative: QuantitativeStats
  facetSummary: FacetSummary
  sections: AiSection[]
}

export interface RecapReportSummary {
  id: string
  title: string
  rangeStart: string
  rangeEnd: string
  sessionCount: number
  generatedAt: string
  analysisModel: string
  htmlPath: string | null
}

export type GenerateMode =
  | { mode: "incremental" }
  | { mode: "full"; filters: DashboardFilter }

export type RecapProgress =
  | { phase: "started"; reportId: string; totalSessions: number }
  | { phase: "extractingFacets"; completed: number; total: number }
  | { phase: "aggregatingDashboard" }
  | { phase: "generatingSections"; completed: number; total: number }
  | { phase: "persisting" }
  | { phase: "done"; reportId: string }
  | { phase: "failed"; reportId: string; message: string }

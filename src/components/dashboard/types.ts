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
}

export interface TokenUsageTrend {
  date: string
  inputTokens: number
  outputTokens: number
}

export interface TokenByModel {
  modelId: string
  providerName: string
  inputTokens: number
  outputTokens: number
  estimatedCostUsd: number
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

export type Granularity = "day" | "week" | "month"

/** Format large numbers as "1.2M", "45.6K", etc. */
export function formatNumber(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

/** Format USD currency */
export function formatCost(n: number): string {
  return `$${n.toFixed(2)}`
}

/** Format milliseconds to human readable */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60_000).toFixed(1)}m`
}

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
  | { type: "every"; intervalMs: number }
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

/** Format bytes to human readable (KB, MB, GB, TB) */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  if (bytes < 1024 * 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
  return `${(bytes / (1024 * 1024 * 1024 * 1024)).toFixed(2)} TB`
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

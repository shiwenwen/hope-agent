// ── Types ─────────────────────────────────────────────────────────

export interface CronSchedule {
  type: "at" | "every" | "cron"
  timestamp?: string
  intervalMs?: number
  interval_ms?: number
  startAt?: string | null
  start_at?: string | null
  expression?: string
  timezone?: string | null
}

export interface CronPayload {
  type: "agentTurn"
  prompt: string
  agentId?: string | null
}

export interface CronDeliveryTarget {
  channelId: string
  accountId: string
  chatId: string
  threadId?: string | null
  label?: string | null
  /** §8: the sending account was deleted; target is skipped + shown red. */
  stale?: boolean
}

export interface CronJob {
  id: string
  name: string
  description?: string | null
  projectId?: string | null
  schedule: CronSchedule
  payload: CronPayload
  status: "active" | "paused" | "disabled" | "completed" | "missed"
  nextRunAt?: string | null
  lastRunAt?: string | null
  runningAt?: string | null
  consecutiveFailures: number
  maxFailures: number
  createdAt: string
  updatedAt: string
  notifyOnComplete: boolean
  deliveryTargets: CronDeliveryTarget[]
  /** §8: prefix successful deliveries with `[Cron] {name}` (opt-in). */
  prefixDeliveryWithName?: boolean
}

export interface CronRunLog {
  id: number
  jobId: string
  sessionId: string
  status: string
  startedAt: string
  finishedAt?: string | null
  durationMs?: number | null
  resultPreview?: string | null
  error?: string | null
  /** §8: "delivered" | "partial" | "failed" | null (no targets). */
  deliveryStatus?: string | null
}

/** §8: a cron job referencing a channel account in its delivery targets. */
export interface CronAccountRef {
  jobId: string
  jobName: string
  targetCount: number
}

export interface CalendarEvent {
  jobId: string
  jobName: string
  projectId?: string | null
  scheduledAt: string
  status: "active" | "paused" | "disabled" | "completed" | "missed"
  runLog?: CronRunLog | null
}

export type CronFrequency = "hourly" | "daily" | "weekly" | "monthly" | "custom"

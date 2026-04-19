// ── Types ─────────────────────────────────────────────────────────

export interface CronSchedule {
  type: "at" | "every" | "cron"
  timestamp?: string
  intervalMs?: number
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
}

export interface CronJob {
  id: string
  name: string
  description?: string | null
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
}

export interface CalendarEvent {
  jobId: string
  jobName: string
  scheduledAt: string
  status: "active" | "paused" | "disabled" | "completed" | "missed"
  runLog?: CronRunLog | null
}

export type CronFrequency = "hourly" | "daily" | "weekly" | "monthly" | "custom"

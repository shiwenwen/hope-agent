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

export type CronPayload =
  | {
      type: "agentTurn"
      prompt: string
      agentId?: string | null
    }
  | {
      type: "sessionLoop"
      loopId: string
      sessionId: string
      prompt: string
      agentId?: string | null
      goalId?: string | null
    }

export type CronPayloadType = CronPayload["type"]
export type CronLoopState = "active" | "paused" | "completed" | "cancelled" | "blocked"

export interface CronAgentPayload {
  type: "agentTurn" | "sessionLoop"
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
  /** Authoritative Loop control state; present only for sessionLoop list items. */
  loopState?: CronLoopState | null
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
  /** C19: per-job run timeout override (seconds); 0 = no cron-level timeout; null/undefined = global default. */
  jobTimeoutSecs?: number | null
  /** Per-job permission-mode override; null/undefined = follow the agent default. */
  permissionModeOverride?: "default" | "smart" | "yolo" | null
  /** Per-job sandbox-mode override; null/undefined = follow the agent default. */
  sandboxModeOverride?: "off" | "standard" | "isolated" | "workspace" | "trusted" | null
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

/** One row of the cross-job cron run timeline (cron panel "conversations" view). */
export interface CronTimelineRow {
  runLogId: number
  sessionId: string
  jobId: string
  jobName: string
  /** Structured discriminator from the owning job; absent only for orphaned legacy rows. */
  payloadType?: CronPayloadType | null
  status: string
  startedAt: string
  finishedAt?: string | null
  resultPreview?: string | null
  /** Session title (defaults to jobName when the session row is gone). */
  title?: string | null
  /** Unread assistant-message count for this run's session. */
  unreadCount: number
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
  payloadType: CronPayloadType
  projectId?: string | null
  scheduledAt: string
  status: "active" | "paused" | "disabled" | "completed" | "missed"
  runLog?: CronRunLog | null
}

export type CronFrequency = "hourly" | "daily" | "weekly" | "monthly" | "custom"

// ── Types ─────────────────────────────────────────────────────────

import type { ManagedWorktree } from "@/lib/transport"
import type { ActiveModel } from "@/types/chat"

export type CronWorkspaceMode = "project" | "fresh" | "persistent"
export interface CronWorkspacePolicy {
  mode: CronWorkspaceMode
  baseRef?: string | null
}
export interface CronWorkspaceSnapshot {
  mode: CronWorkspaceMode
  worktreeId?: string | null
  baseRef?: string | null
  baseSha?: string | null
  headSha?: string | null
  branch?: string | null
  staged: number
  unstaged: number
  untracked: number
  conflicted: number
  headDiverged: boolean
  retained: boolean
}

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
      type: "sessionTurn"
      sessionId: string
      prompt: string
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
  revision: number
  name: string
  description?: string | null
  projectId?: string | null
  workspacePolicy: CronWorkspacePolicy
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
  /** Most recent occurrence, attached by `cron_list_jobs` for list surfaces. */
  lastRun?: CronLastRunSummary | null
}

/** Compact outcome of a task's most recent occurrence (list rows + search). */
export interface CronLastRunSummary {
  runLogId: number
  sessionId: string
  status: string
  startedAt: string
  finishedAt?: string | null
  error?: string | null
  resultPreview?: string | null
  deliveryStatus?: string | null
}

/**
 * A task plus its tombstone flag (`cron_get_job_snapshot`). Deleting a task only
 * stops future occurrences, so retained history still resolves it here — as
 * display + copy material, never as a schedulable job.
 */
export interface CronJobSnapshot {
  job: CronJob
  deleted: boolean
}

export interface CronRunLog {
  id: number
  jobId: string
  sessionId: string
  turnId?: string | null
  targetMessageId?: number | null
  status: string
  startedAt: string
  finishedAt?: string | null
  durationMs?: number | null
  resultPreview?: string | null
  error?: string | null
  /** §8: "delivered" | "partial" | "failed" | null (no targets). */
  deliveryStatus?: string | null
  worktreeId?: string | null
  workspaceStatus?: string | null
  workspaceSnapshot?: CronWorkspaceSnapshot | null
}

export interface CronWorkspaceActionAvailability {
  allowed: boolean
  reasonCode?: string | null
}
export interface CronWorkspaceActions {
  takeOver: CronWorkspaceActionAvailability
  returnToTask: CronWorkspaceActionAvailability
  returnAndResume: CronWorkspaceActionAvailability
  discard: CronWorkspaceActionAvailability
  archive: CronWorkspaceActionAvailability
  restore: CronWorkspaceActionAvailability
}
export interface CronWorkspaceResource {
  jobId: string
  runLogId?: number | null
  sessionId?: string | null
  mode: CronWorkspaceMode
  workspaceStatus: string
  worktree: ManagedWorktree
  actions: CronWorkspaceActions
}
export interface CronWorkspaceActionResult {
  resource?: CronWorkspaceResource | null
  resumed: boolean
}

export interface CronUpdateResult {
  updated: boolean
  code?: string | null
  currentJob?: CronJob | null
}

export interface CronRunCancelResult {
  runLogId: number
  status: string
  terminal: boolean
  cancelRequested: boolean
  code?: string | null
}

/** Where the occurrence lands — the decisive fact to confirm before saving. */
export type CronConversationPreview =
  | { kind: "newSession" }
  | { kind: "existingSession"; sessionId: string; title?: string | null }

/** One delivery target with its own blocking reason, if any. */
export interface CronDeliveryPreview {
  channelId: string
  accountId: string
  chatId: string
  threadId?: string | null
  label?: string | null
  problem?: string | null
}

export interface CronSchedulerPreview {
  /** Only the Primary process executes tasks. */
  primary: boolean
  runningTasks: number
  /** 0 = unlimited. */
  maxConcurrent: number
}

export interface CronPreflightReport {
  checkedRevision?: number | null
  canProceed: boolean
  nextRuns: string[]
  issues: Array<{
    code: string
    severity: "blocker" | "warning"
  }>
  execution: {
    resolvedAgentId?: string | null
    projectName?: string | null
    workspaceMode: CronWorkspaceMode
    baseRef?: string | null
    workspaceDirtyFiles?: number | null
    effectivePermissionMode?: string | null
    effectiveSandboxMode?: string | null
    primaryModel?: ActiveModel | null
    conversation?: CronConversationPreview | null
    deliveryTargets?: CronDeliveryPreview[]
    scheduler?: CronSchedulerPreview
    taskRunning?: boolean
    taskRunningSince?: string | null
  }
}

export type CronRunNowResult =
  | { status: "started" }
  | { status: "rejected"; report: CronPreflightReport }

/** One row of the cross-job cron run timeline (cron panel "conversations" view). */
export interface CronTimelineRow {
  runLogId: number
  sessionId: string
  turnId?: string | null
  targetMessageId?: number | null
  jobId: string
  jobName: string
  /** The task was deleted, while this historical run remains available. */
  jobDeleted?: boolean
  /** Structured discriminator from the owning job; absent only for orphaned legacy rows. */
  payloadType?: CronPayloadType | null
  status: string
  startedAt: string
  finishedAt?: string | null
  resultPreview?: string | null
  worktreeId?: string | null
  workspaceStatus?: string | null
  workspaceSnapshot?: CronWorkspaceSnapshot | null
  /** Session title (defaults to jobName when the session row is gone). */
  title?: string | null
  /** Whether this run session has unread assistant output (0 or 1). */
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

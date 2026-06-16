// R4 background-jobs panel — frontend mirror of
// `ha_core::async_jobs::BackgroundJobSnapshot` (camelCase serde). The owner
// plane (Tauri / HTTP) returns these for the panel + header badge; they are
// distinct from the model-facing `job_status` JSON.

import type { TFunction } from "i18next"

export type BackgroundJobKind = "tool" | "subagent" | "group"

export type BackgroundJobStatus =
  | "queued"
  | "running"
  | "cancelling"
  | "awaiting_approval"
  | "completed"
  | "failed"
  | "interrupted"
  | "timed_out"
  | "cancelled"

export interface BackgroundJobSnapshot {
  jobId: string
  kind: BackgroundJobKind
  status: BackgroundJobStatus
  /** Raw tool name (`exec` / `web_search` / `subagent:<agent>` / `subagent:batch`). */
  tool: string
  /** Concise display label (exec command head / tool name); empty for group/subagent. */
  label: string
  origin: string
  sessionId: string | null
  createdAt: number
  completedAt: number | null
  error: string | null
  resultPreview: string | null
  childCount: number | null
  childrenTerminal: number | null
  childrenCompleted: number | null
  childrenFailed: number | null
  subagentRunId: string | null
  /** Live running-output tail (backgrounded exec only; single `get` only). */
  outputTail: string | null
}

/** Non-terminal statuses — a job still consuming a slot / awaiting work. */
const ACTIVE_STATUSES = new Set<BackgroundJobStatus>([
  "queued",
  "running",
  "cancelling",
  "awaiting_approval",
])

export function isBackgroundJobActive(job: BackgroundJobSnapshot): boolean {
  return ACTIVE_STATUSES.has(job.status)
}

export function isBackgroundJobTerminal(job: BackgroundJobSnapshot): boolean {
  return !isBackgroundJobActive(job)
}

/** Statuses a user can cancel from the panel (active + not already cancelling). */
export function isBackgroundJobCancellable(job: BackgroundJobSnapshot): boolean {
  return isBackgroundJobActive(job) && job.status !== "cancelling"
}

/** Human display label: exec command head / tool name; localized for group/subagent. */
export function backgroundJobLabel(job: BackgroundJobSnapshot, t: TFunction): string {
  if (job.label) return job.label
  if (job.kind === "group") return t("backgroundJobs.kindGroup", "任务组")
  if (job.kind === "subagent") return t("backgroundJobs.kindSubagent", "子智能体")
  return job.tool
}

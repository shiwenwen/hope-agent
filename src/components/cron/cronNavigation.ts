import type { CronJob } from "./CronJobForm.types"

export const CRON_TASK_FOCUS_EVENT = "hope:cron-task-focus"
export const CRON_TASK_DRAFT_EVENT = "hope:cron-task-draft"

export function requestCronTaskFocus(jobId: string): void {
  if (typeof window === "undefined" || !jobId) return
  window.dispatchEvent(new CustomEvent(CRON_TASK_FOCUS_EVENT, { detail: { jobId } }))
}

export function subscribeCronTaskFocus(handler: (jobId: string) => void): () => void {
  if (typeof window === "undefined") return () => {}
  const listener = (event: Event) => {
    const jobId = (event as CustomEvent<{ jobId?: unknown }>).detail?.jobId
    if (typeof jobId === "string" && jobId) handler(jobId)
  }
  window.addEventListener(CRON_TASK_FOCUS_EVENT, listener)
  return () => window.removeEventListener(CRON_TASK_FOCUS_EVENT, listener)
}

/**
 * Seed a brand-new task draft from a retained task — a create, never an edit, so
 * a deleted seed (tombstone) is never rescheduled in place.
 */
export function requestCronTaskDraft(seed: CronJob): void {
  if (typeof window === "undefined" || !seed) return
  window.dispatchEvent(new CustomEvent(CRON_TASK_DRAFT_EVENT, { detail: { seed } }))
}

export function subscribeCronTaskDraft(handler: (seed: CronJob) => void): () => void {
  if (typeof window === "undefined") return () => {}
  const listener = (event: Event) => {
    const seed = (event as CustomEvent<{ seed?: unknown }>).detail?.seed
    if (seed && typeof seed === "object") handler(seed as CronJob)
  }
  window.addEventListener(CRON_TASK_DRAFT_EVENT, listener)
  return () => window.removeEventListener(CRON_TASK_DRAFT_EVENT, listener)
}

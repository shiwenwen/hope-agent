export const CRON_TASK_FOCUS_EVENT = "hope:cron-task-focus"

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

import { getTransport } from "@/lib/transport-provider"
import type { BackgroundJobSnapshot } from "@/types/background-jobs"

const inFlight = new Map<string, Promise<BackgroundJobSnapshot | null>>()

/**
 * `get_background_job` shared across the surfaces that poll the same job.
 * Every open workbench tab stays warm-mounted, so the workspace section and the
 * background-jobs panel poll the same ids in lockstep; without this they issue
 * two identical round-trips a second. Concurrent callers share one request; the
 * next tick starts a fresh one.
 */
export function fetchBackgroundJobDetail(jobId: string): Promise<BackgroundJobSnapshot | null> {
  const pending = inFlight.get(jobId)
  if (pending) return pending
  const request = getTransport()
    .call<BackgroundJobSnapshot | null>("get_background_job", { jobId })
    .catch(() => null)
    .finally(() => {
      inFlight.delete(jobId)
    })
  inFlight.set(jobId, request)
  return request
}

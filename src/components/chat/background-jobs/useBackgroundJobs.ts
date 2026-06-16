import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import {
  type BackgroundJobSnapshot,
  isBackgroundJobActive,
} from "@/types/background-jobs"

const REFETCH_DEBOUNCE_MS = 200

export interface UseBackgroundJobsResult {
  jobs: BackgroundJobSnapshot[]
  /** Count of still-running / queued / awaiting jobs (drives the header badge). */
  runningCount: number
  refetch: () => void
}

/**
 * R4: live view of a session's background jobs for the panel + header badge.
 *
 * Strategy: seed from `list_background_jobs(sessionId)`, then debounced-refetch
 * on any relevant lifecycle event for THIS session — `job:*` (tool/group, R3)
 * plus `subagent_event` (subagent jobs ride the subagent stream, R6; only 2
 * events per run, never per-token). The events are minimal (id/status), so a
 * full refetch is simpler and always-correct vs. incremental merge.
 */
export function useBackgroundJobs(
  sessionId: string | null | undefined,
): UseBackgroundJobsResult {
  const [jobs, setJobs] = useState<BackgroundJobSnapshot[]>([])
  const aliveRef = useRef(true)
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const sessionRef = useRef(sessionId)
  sessionRef.current = sessionId

  const fetchNow = useCallback(() => {
    const sid = sessionRef.current
    if (!sid) {
      setJobs([])
      return
    }
    getTransport()
      .call<BackgroundJobSnapshot[]>("list_background_jobs", { sessionId: sid })
      .then((rows) => {
        // Drop a response that arrived after the session switched.
        if (aliveRef.current && sessionRef.current === sid) setJobs(rows ?? [])
      })
      .catch(() => {
        /* transient read failure — keep the last good snapshot */
      })
  }, [])

  const scheduleRefetch = useCallback(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current)
    debounceRef.current = setTimeout(fetchNow, REFETCH_DEBOUNCE_MS)
  }, [fetchNow])

  useEffect(() => {
    aliveRef.current = true
    setJobs([])
    fetchNow()

    const transport = getTransport()
    const matchesSession = (raw: unknown) =>
      (raw as { session_id?: string })?.session_id === sessionId
    const offs = [
      transport.listen("job:created", (raw) => {
        if (matchesSession(raw)) scheduleRefetch()
      }),
      transport.listen("job:updated", (raw) => {
        if (matchesSession(raw)) scheduleRefetch()
      }),
      transport.listen("job:completed", (raw) => {
        if (matchesSession(raw)) scheduleRefetch()
      }),
      transport.listen("job:progress", (raw) => {
        if (matchesSession(raw)) scheduleRefetch()
      }),
      transport.listen("subagent_event", (raw) => {
        if ((raw as { parentSessionId?: string })?.parentSessionId === sessionId)
          scheduleRefetch()
      }),
    ]

    return () => {
      aliveRef.current = false
      if (debounceRef.current) clearTimeout(debounceRef.current)
      for (const off of offs) off()
    }
  }, [sessionId, fetchNow, scheduleRefetch])

  const runningCount = useMemo(
    () => jobs.filter(isBackgroundJobActive).length,
    [jobs],
  )

  return { jobs, runningCount, refetch: fetchNow }
}

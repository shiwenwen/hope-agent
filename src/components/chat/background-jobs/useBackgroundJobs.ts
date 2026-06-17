import { useCallback, useEffect, useMemo, useState } from "react"

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
 *
 * State is tagged with the session it belongs to so a session switch shows an
 * empty list immediately (derived) without a setState-in-effect reset, and a
 * late response for the old session is ignored. `refetch` bumps a tick that
 * re-seeds + re-subscribes (rare; manual use only).
 */
export function useBackgroundJobs(
  sessionId: string | null | undefined,
): UseBackgroundJobsResult {
  const sid = sessionId ?? null
  const [state, setState] = useState<{
    sid: string | null
    jobs: BackgroundJobSnapshot[]
  }>({ sid: null, jobs: [] })
  const [refetchTick, setRefetchTick] = useState(0)

  const refetch = useCallback(() => setRefetchTick((t) => t + 1), [])

  useEffect(() => {
    if (!sid) return

    let alive = true
    let debounce: ReturnType<typeof setTimeout> | null = null

    const fetchNow = () => {
      getTransport()
        .call<BackgroundJobSnapshot[]>("list_background_jobs", { sessionId: sid })
        .then((rows) => {
          if (alive) setState({ sid, jobs: rows ?? [] })
        })
        .catch(() => {
          /* transient read failure — keep the last good snapshot */
        })
    }
    const scheduleRefetch = () => {
      if (debounce) clearTimeout(debounce)
      debounce = setTimeout(fetchNow, REFETCH_DEBOUNCE_MS)
    }

    fetchNow()

    const transport = getTransport()
    const matchesSession = (raw: unknown) =>
      (raw as { session_id?: string })?.session_id === sid
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
        if ((raw as { parentSessionId?: string })?.parentSessionId === sid)
          scheduleRefetch()
      }),
    ]

    return () => {
      alive = false
      if (debounce) clearTimeout(debounce)
      for (const off of offs) off()
    }
  }, [sid, refetchTick])

  // Derived: only show jobs that belong to the current session (a stale snapshot
  // from the previous session reads as empty until the new fetch lands).
  const jobs = useMemo(
    () => (state.sid === sid ? state.jobs : []),
    [state, sid],
  )
  const runningCount = useMemo(
    () => jobs.filter(isBackgroundJobActive).length,
    [jobs],
  )

  return { jobs, runningCount, refetch }
}

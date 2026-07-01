import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  ReviewFinding,
  ReviewFindingStatus,
  ReviewRun,
  ReviewRunSnapshot,
} from "@/lib/transport"

export interface ReviewRunsState {
  runs: ReviewRun[]
  snapshot: ReviewRunSnapshot | null
  loading: boolean
  running: boolean
  error: string | null
  refresh: () => void
  runReview: () => Promise<ReviewRunSnapshot | null>
  updateFindingStatus: (
    findingId: string,
    status: ReviewFindingStatus,
  ) => Promise<ReviewFinding | null>
}

const REVIEW_EVENT_REFRESH_DEBOUNCE_MS = 250
const REVIEW_ACTIVE_POLL_MS = 4000

function isReviewRunPayload(payload: unknown): payload is ReviewRun {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { id?: unknown }).id === "string" &&
    typeof (payload as { sessionId?: unknown }).sessionId === "string"
  )
}

function isReviewFindingPayload(payload: unknown): payload is ReviewFinding {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { id?: unknown }).id === "string" &&
    typeof (payload as { sessionId?: unknown }).sessionId === "string" &&
    typeof (payload as { runId?: unknown }).runId === "string"
  )
}

function reviewRunActive(run: ReviewRun): boolean {
  return run.state === "running"
}

export function useReviewRuns(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; turnActive?: boolean; disabled?: boolean } = {},
): ReviewRunsState {
  const { incognito = false, turnActive = false, disabled = false } = opts
  const [runs, setRuns] = useState<ReviewRun[]>([])
  const [snapshot, setSnapshot] = useState<ReviewRunSnapshot | null>(null)
  const [loading, setLoading] = useState(false)
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchRuns = useCallback(
    (fetchOpts: { clear?: boolean } = {}) => {
      if (disabled || !sessionId || incognito) {
        reqRef.current += 1
        setRuns([])
        setSnapshot(null)
        setLoading(false)
        setError(null)
        return
      }
      const req = ++reqRef.current
      setLoading(true)
      setError(null)
      if (fetchOpts.clear) {
        setRuns([])
        setSnapshot(null)
      }

      getTransport()
        .call<ReviewRun[]>("list_review_runs", { sessionId })
        .then(async (nextRuns) => {
          if (reqRef.current !== req) return
          const safeRuns = Array.isArray(nextRuns) ? nextRuns : []
          setRuns(safeRuns)
          const latest = safeRuns[0]
          if (!latest) {
            setSnapshot(null)
            setLoading(false)
            return
          }
          const nextSnapshot = await getTransport().call<ReviewRunSnapshot | null>("get_review_run", {
            runId: latest.id,
          })
          if (reqRef.current !== req) return
          setSnapshot(nextSnapshot)
          setLoading(false)
        })
        .catch((e) => {
          if (reqRef.current !== req) return
          const message = e instanceof Error ? e.message : String(e)
          logger.error("ui", "useReviewRuns", "Failed to load review runs", e)
          setError(message)
          setLoading(false)
        })
    },
    [disabled, incognito, sessionId],
  )

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) fetchRuns({ clear: true })
    })
    return () => {
      cancelled = true
    }
  }, [fetchRuns])

  const prevTurnActive = useRef(turnActive)
  useEffect(() => {
    let cancelled = false
    const was = prevTurnActive.current
    prevTurnActive.current = turnActive
    if (was && !turnActive) {
      queueMicrotask(() => {
        if (!cancelled) fetchRuns()
      })
    }
    return () => {
      cancelled = true
    }
  }, [fetchRuns, turnActive])

  useEffect(() => {
    if (disabled || !sessionId || incognito) return
    const transport = getTransport()
    const scheduleRefresh = () => {
      if (eventRefreshTimerRef.current !== null) return
      eventRefreshTimerRef.current = window.setTimeout(() => {
        eventRefreshTimerRef.current = null
        fetchRuns()
      }, REVIEW_EVENT_REFRESH_DEBOUNCE_MS)
    }
    const maybeRefreshForRun = (payload: unknown) => {
      if (isReviewRunPayload(payload) && payload.sessionId !== sessionId) return
      scheduleRefresh()
    }
    const maybeRefreshForFinding = (payload: unknown) => {
      if (isReviewFindingPayload(payload) && payload.sessionId !== sessionId) return
      scheduleRefresh()
    }
    const unsubs = [
      transport.listen("review:created", maybeRefreshForRun),
      transport.listen("review:updated", maybeRefreshForRun),
      transport.listen("review:finding_updated", maybeRefreshForFinding),
      transport.listen("review:event", scheduleRefresh),
      transport.listen("_lagged", scheduleRefresh),
    ]
    return () => {
      if (eventRefreshTimerRef.current !== null) {
        window.clearTimeout(eventRefreshTimerRef.current)
        eventRefreshTimerRef.current = null
      }
      unsubs.forEach((unsub) => unsub())
    }
  }, [disabled, fetchRuns, incognito, sessionId])

  const hasActiveRun = useMemo(() => runs.some(reviewRunActive), [runs])
  useEffect(() => {
    if (disabled || !sessionId || incognito || !hasActiveRun) return
    const timer = window.setInterval(() => fetchRuns(), REVIEW_ACTIVE_POLL_MS)
    return () => window.clearInterval(timer)
  }, [disabled, fetchRuns, hasActiveRun, incognito, sessionId])

  const runReview = useCallback(async () => {
    if (!sessionId || disabled || incognito) return null
    setRunning(true)
    setError(null)
    try {
      const nextSnapshot = await getTransport().call<ReviewRunSnapshot>("run_code_review", {
        sessionId,
        scope: "local",
      })
      setSnapshot(nextSnapshot)
      fetchRuns()
      return nextSnapshot
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "useReviewRuns", "Failed to run code review", e)
      setError(message)
      return null
    } finally {
      setRunning(false)
    }
  }, [disabled, fetchRuns, incognito, sessionId])

  const updateFindingStatus = useCallback(
    async (findingId: string, status: ReviewFindingStatus) => {
      if (!sessionId || disabled || incognito) return null
      try {
        const finding = await getTransport().call<ReviewFinding>("update_review_finding_status", {
          findingId,
          status,
        })
        fetchRuns()
        return finding
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useReviewRuns", "Failed to update review finding", e)
        setError(message)
        return null
      }
    },
    [disabled, fetchRuns, incognito, sessionId],
  )

  return {
    runs,
    snapshot,
    loading,
    running,
    error,
    refresh: fetchRuns,
    runReview,
    updateFindingStatus,
  }
}

import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type { VerificationRun, VerificationRunSnapshot, VerificationStep } from "@/lib/transport"

export interface VerificationRunsState {
  runs: VerificationRun[]
  snapshot: VerificationRunSnapshot | null
  loading: boolean
  planning: boolean
  running: boolean
  error: string | null
  refresh: () => void
  planVerification: () => Promise<VerificationRunSnapshot | null>
  runVerification: () => Promise<VerificationRunSnapshot | null>
}

const VERIFICATION_EVENT_REFRESH_DEBOUNCE_MS = 250
const VERIFICATION_ACTIVE_POLL_MS = 3000

function isVerificationRunPayload(payload: unknown): payload is VerificationRun {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { id?: unknown }).id === "string" &&
    typeof (payload as { sessionId?: unknown }).sessionId === "string"
  )
}

function isVerificationStepPayload(payload: unknown): payload is VerificationStep {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { id?: unknown }).id === "string" &&
    typeof (payload as { sessionId?: unknown }).sessionId === "string" &&
    typeof (payload as { runId?: unknown }).runId === "string"
  )
}

function verificationRunActive(run: VerificationRun): boolean {
  return run.state === "running"
}

export function useVerificationRuns(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; turnActive?: boolean; disabled?: boolean } = {},
): VerificationRunsState {
  const { incognito = false, turnActive = false, disabled = false } = opts
  const [runs, setRuns] = useState<VerificationRun[]>([])
  const [snapshot, setSnapshot] = useState<VerificationRunSnapshot | null>(null)
  const [loading, setLoading] = useState(false)
  const [planning, setPlanning] = useState(false)
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
        .call<VerificationRun[]>("list_verification_runs", { sessionId })
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
          const nextSnapshot = await getTransport().call<VerificationRunSnapshot | null>(
            "get_verification_run",
            { runId: latest.id },
          )
          if (reqRef.current !== req) return
          setSnapshot(nextSnapshot)
          setLoading(false)
        })
        .catch((e) => {
          if (reqRef.current !== req) return
          const message = e instanceof Error ? e.message : String(e)
          logger.error("ui", "useVerificationRuns", "Failed to load verification runs", e)
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
      }, VERIFICATION_EVENT_REFRESH_DEBOUNCE_MS)
    }
    const maybeRefreshForRun = (payload: unknown) => {
      if (isVerificationRunPayload(payload) && payload.sessionId !== sessionId) return
      scheduleRefresh()
    }
    const maybeRefreshForStep = (payload: unknown) => {
      if (isVerificationStepPayload(payload) && payload.sessionId !== sessionId) return
      scheduleRefresh()
    }
    const unsubs = [
      transport.listen("verification:created", maybeRefreshForRun),
      transport.listen("verification:updated", maybeRefreshForRun),
      transport.listen("verification:step_updated", maybeRefreshForStep),
      transport.listen("verification:event", scheduleRefresh),
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

  const hasActiveRun = useMemo(() => runs.some(verificationRunActive), [runs])
  useEffect(() => {
    if (disabled || !sessionId || incognito || !hasActiveRun) return
    const timer = window.setInterval(() => fetchRuns(), VERIFICATION_ACTIVE_POLL_MS)
    return () => window.clearInterval(timer)
  }, [disabled, fetchRuns, hasActiveRun, incognito, sessionId])

  const planVerification = useCallback(async () => {
    if (!sessionId || disabled || incognito) return null
    setPlanning(true)
    setError(null)
    try {
      const nextSnapshot = await getTransport().call<VerificationRunSnapshot>(
        "plan_smart_verification",
        { sessionId, scope: "local" },
      )
      setSnapshot(nextSnapshot)
      fetchRuns()
      return nextSnapshot
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "useVerificationRuns", "Failed to plan verification", e)
      setError(message)
      return null
    } finally {
      setPlanning(false)
    }
  }, [disabled, fetchRuns, incognito, sessionId])

  const runVerification = useCallback(async () => {
    if (!sessionId || disabled || incognito) return null
    setRunning(true)
    setError(null)
    try {
      const nextSnapshot = await getTransport().call<VerificationRunSnapshot>(
        "run_smart_verification",
        { sessionId, scope: "local" },
      )
      setSnapshot(nextSnapshot)
      fetchRuns()
      return nextSnapshot
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "useVerificationRuns", "Failed to run verification", e)
      setError(message)
      return null
    } finally {
      setRunning(false)
    }
  }, [disabled, fetchRuns, incognito, sessionId])

  return {
    runs,
    snapshot,
    loading,
    planning,
    running,
    error,
    refresh: fetchRuns,
    planVerification,
    runVerification,
  }
}

import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type {
  DomainQualityCheck,
  DomainQualityRun,
  DomainQualityRunSnapshot,
  RunDomainQualityInput,
} from "@/lib/transport"

export interface DomainQualityRunsState {
  runs: DomainQualityRun[]
  snapshot: DomainQualityRunSnapshot | null
  loading: boolean
  running: boolean
  error: string | null
  refresh: () => void
  runDomainQuality: (
    args?: Partial<Omit<RunDomainQualityInput, "sessionId">>,
  ) => Promise<DomainQualityRunSnapshot | null>
}

const DOMAIN_QUALITY_EVENT_REFRESH_DEBOUNCE_MS = 250
const DOMAIN_QUALITY_ACTIVE_POLL_MS = 3500

function isDomainQualityRunPayload(payload: unknown): payload is DomainQualityRun {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { id?: unknown }).id === "string" &&
    typeof (payload as { sessionId?: unknown }).sessionId === "string"
  )
}

function isDomainQualityCheckPayload(payload: unknown): payload is DomainQualityCheck {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { id?: unknown }).id === "string" &&
    typeof (payload as { sessionId?: unknown }).sessionId === "string" &&
    typeof (payload as { runId?: unknown }).runId === "string"
  )
}

function runActive(run: DomainQualityRun): boolean {
  return run.state === "running"
}

export function useDomainQualityRuns(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; turnActive?: boolean; disabled?: boolean } = {},
): DomainQualityRunsState {
  const { incognito = false, turnActive = false, disabled = false } = opts
  const [runs, setRuns] = useState<DomainQualityRun[]>([])
  const [snapshot, setSnapshot] = useState<DomainQualityRunSnapshot | null>(null)
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
        .call<DomainQualityRun[]>("list_domain_quality_runs", { sessionId })
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
          const nextSnapshot = await getTransport().call<DomainQualityRunSnapshot | null>(
            "get_domain_quality_run",
            { runId: latest.id },
          )
          if (reqRef.current !== req) return
          setSnapshot(nextSnapshot)
          setLoading(false)
        })
        .catch((e) => {
          if (reqRef.current !== req) return
          const message = e instanceof Error ? e.message : String(e)
          logger.error("ui", "useDomainQualityRuns", "Failed to load domain quality runs", e)
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
      }, DOMAIN_QUALITY_EVENT_REFRESH_DEBOUNCE_MS)
    }
    const maybeRefreshForRun = (payload: unknown) => {
      if (isDomainQualityRunPayload(payload) && payload.sessionId !== sessionId) return
      scheduleRefresh()
    }
    const maybeRefreshForCheck = (payload: unknown) => {
      if (isDomainQualityCheckPayload(payload) && payload.sessionId !== sessionId) return
      scheduleRefresh()
    }
    const unsubs = [
      transport.listen("domain_quality:created", maybeRefreshForRun),
      transport.listen("domain_quality:updated", maybeRefreshForRun),
      transport.listen("domain_quality:check_updated", maybeRefreshForCheck),
      transport.listen("domain_quality:event", scheduleRefresh),
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

  const hasActiveRun = useMemo(() => runs.some(runActive), [runs])
  useEffect(() => {
    if (disabled || !sessionId || incognito || !hasActiveRun) return
    const timer = window.setInterval(() => fetchRuns(), DOMAIN_QUALITY_ACTIVE_POLL_MS)
    return () => window.clearInterval(timer)
  }, [disabled, fetchRuns, hasActiveRun, incognito, sessionId])

  const runDomainQuality = useCallback(
    async (args: Partial<Omit<RunDomainQualityInput, "sessionId">> = {}) => {
      if (!sessionId || disabled || incognito) return null
      setRunning(true)
      setError(null)
      try {
        const nextSnapshot = await getTransport().call<DomainQualityRunSnapshot>(
          "run_domain_quality",
          {
            input: {
              sessionId,
              ...args,
            },
          },
        )
        setSnapshot(nextSnapshot)
        fetchRuns()
        return nextSnapshot
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useDomainQualityRuns", "Failed to run domain quality", e)
        setError(message)
        return null
      } finally {
        setRunning(false)
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
    runDomainQuality,
  }
}

import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

export type LoopState = "active" | "paused" | "completed" | "cancelled" | "blocked"
export type LoopRunState =
  | "running"
  | "queued"
  | "injected"
  | "succeeded"
  | "empty"
  | "failed"
  | "cancelled"
  | "skipped"
export type LoopTriggerKind = "interval" | "cron" | "condition" | "event"
export type LoopExecutionStrategy = "continue" | "workflow"

export interface LoopSchedule {
  id: string
  sessionId: string
  goalId?: string | null
  cronJobId: string
  prompt: string
  triggerKind: LoopTriggerKind
  triggerSpec: Record<string, unknown>
  executionStrategy: LoopExecutionStrategy
  state: LoopState
  maxRuns?: number | null
  runCount: number
  maxRuntimeSecs?: number | null
  tokenBudget?: number | null
  costBudgetMicros?: number | null
  approvalPolicySnapshot: unknown
  createdAt: string
  updatedAt: string
  completedAt?: string | null
  blockedReason?: string | null
}

export interface LoopRun {
  id: string
  loopId: string
  cronJobId: string
  cronRunLogId?: number | null
  sessionId: string
  seq: number
  state: LoopRunState
  triggerReason: string
  resultSummary?: string | null
  error?: string | null
  trace: unknown
  startedAt: string
  finishedAt?: string | null
}

export interface LoopSnapshot {
  schedule: LoopSchedule
  runs: LoopRun[]
}

export interface LoopSchedulesState {
  schedules: LoopSchedule[]
  activeCount: number
  loading: boolean
  error: string | null
  refresh: () => void
}

const LOOP_REFRESH_DEBOUNCE_MS = 250
const LOOP_ACTIVE_POLL_MS = 5000

function isLoopPayload(payload: unknown): payload is { sessionId?: string } {
  return typeof payload === "object" && payload !== null && "loopId" in payload
}

function loopIsActive(loop: LoopSchedule): boolean {
  return loop.state === "active" || loop.state === "paused" || loop.state === "blocked"
}

export function useLoopSchedules(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; turnActive?: boolean; disabled?: boolean } = {},
): LoopSchedulesState {
  const { incognito = false, turnActive = false, disabled = false } = opts
  const [schedules, setSchedules] = useState<LoopSchedule[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchSchedules = useCallback(
    (fetchOpts: { clear?: boolean } = {}) => {
      if (disabled) {
        reqRef.current += 1
        setLoading(false)
        setError(null)
        return
      }
      if (!sessionId || incognito) {
        reqRef.current += 1
        setSchedules([])
        setLoading(false)
        setError(null)
        return
      }
      const req = ++reqRef.current
      setLoading(true)
      setError(null)
      if (fetchOpts.clear) setSchedules([])
      getTransport()
        .call<LoopSchedule[]>("list_loop_schedules", { sessionId })
        .then((next) => {
          if (reqRef.current !== req) return
          setSchedules(Array.isArray(next) ? next : [])
          setLoading(false)
        })
        .catch((e) => {
          if (reqRef.current !== req) return
          const message = e instanceof Error ? e.message : String(e)
          logger.error("ui", "useLoopSchedules", "Failed to load loop schedules", e)
          setError(message)
          setLoading(false)
        })
    },
    [disabled, incognito, sessionId],
  )

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) fetchSchedules({ clear: true })
    })
    return () => {
      cancelled = true
    }
  }, [fetchSchedules])

  const prevTurnActive = useRef(turnActive)
  useEffect(() => {
    const was = prevTurnActive.current
    prevTurnActive.current = turnActive
    if (was && !turnActive) fetchSchedules()
  }, [fetchSchedules, turnActive])

  useEffect(() => {
    if (disabled || !sessionId || incognito) return
    const transport = getTransport()
    const scheduleRefresh = () => {
      if (eventRefreshTimerRef.current !== null) return
      eventRefreshTimerRef.current = window.setTimeout(() => {
        eventRefreshTimerRef.current = null
        fetchSchedules()
      }, LOOP_REFRESH_DEBOUNCE_MS)
    }
    const maybeRefresh = (payload: unknown) => {
      if (isLoopPayload(payload) && payload.sessionId && payload.sessionId !== sessionId) return
      scheduleRefresh()
    }
    const offLoop = transport.listen("loop:changed", maybeRefresh)
    const offCron = transport.listen("cron:run_completed", scheduleRefresh)
    return () => {
      offLoop()
      offCron()
      if (eventRefreshTimerRef.current !== null) {
        window.clearTimeout(eventRefreshTimerRef.current)
        eventRefreshTimerRef.current = null
      }
    }
  }, [disabled, fetchSchedules, incognito, sessionId])

  const activeCount = useMemo(() => schedules.filter(loopIsActive).length, [schedules])

  useEffect(() => {
    if (disabled || !sessionId || incognito || activeCount === 0) return
    const id = window.setInterval(fetchSchedules, LOOP_ACTIVE_POLL_MS)
    return () => window.clearInterval(id)
  }, [activeCount, disabled, fetchSchedules, incognito, sessionId])

  return useMemo(
    () => ({ schedules, activeCount, loading, error, refresh: fetchSchedules }),
    [activeCount, error, fetchSchedules, loading, schedules],
  )
}

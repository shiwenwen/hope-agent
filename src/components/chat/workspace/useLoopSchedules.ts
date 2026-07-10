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
export type LoopTriggerKind = "interval" | "cron" | "condition" | "event" | "dynamic"
export type LoopExecutionStrategy = "continue" | "workflow"
export type LoopWatchKind =
  | "app_event"
  | "job"
  | "subagent"
  | "file"
  | "command"
  | "websocket"
export type LoopProgressState =
  | "progressed"
  | "weak_progress"
  | "no_progress"
  | "blocked"
  | "failed"
  | "awaiting_approval"

export interface LoopSchedule {
  id: string
  sessionId: string
  goalId?: string | null
  goalCriterionId?: string | null
  goalCriterionText?: string | null
  goalCriterionKind?: string | null
  goalRevision?: number | null
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
  progressState?: LoopProgressState | null
  progressSummary?: string | null
  noProgressStreak: number
  failureStreak: number
  maxNoProgressRuns?: number | null
  maxFailures?: number | null
  backoffSecs?: number | null
  nextRunAt?: string | null
  cronStatus?: string | null
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
  progressState?: LoopProgressState | null
  progressDelta?: unknown
  noProgressReason?: string | null
  schedulingDecision?: string | null
  trace: unknown
  usage?: LoopRunUsageSnapshot | null
  startedAt: string
  finishedAt?: string | null
}

export interface LoopRunUsageSnapshot {
  messageCount: number
  userTurns: number
  assistantMessages: number
  inputTokens: number
  outputTokens: number
  totalTokens: number
  attribution: string
  providerEvents?: number
  providerInputTokens?: number
  providerOutputTokens?: number
  providerCacheCreationInputTokens?: number
  providerCacheReadInputTokens?: number
  providerTotalTokens?: number
  providerAttribution?: string
}

export interface LoopWatch {
  id: string
  loopId: string
  kind: LoopWatchKind
  spec: Record<string, unknown>
  active: boolean
  generation: number
  lastEventAt?: string | null
  lastFingerprint?: string | null
  failureCount: number
  lastError?: string | null
  monitorJobId?: string | null
  createdAt: string
  updatedAt: string
}

export interface LoopSnapshot {
  schedule: LoopSchedule
  runs: LoopRun[]
  watches?: LoopWatch[]
}

export interface LoopWatchdogFinding {
  loopId: string
  sessionId: string
  severity: string
  code: "loop_cron_missing" | "loop_due_not_claimed" | "loop_run_maybe_interrupted" | string
  message: string
  nextRunAt?: string | null
  overdueSecs?: number | null
  cronStatus?: string | null
  latestRunId?: string | null
  latestRunState?: string | null
}

export interface LoopSchedulesState {
  schedules: LoopSchedule[]
  watchdogFindings: LoopWatchdogFinding[]
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
  const [watchdogFindings, setWatchdogFindings] = useState<LoopWatchdogFinding[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchSchedules = useCallback(
    (fetchOpts: { clear?: boolean } = {}) => {
      if (disabled) {
        reqRef.current += 1
        setWatchdogFindings([])
        setLoading(false)
        setError(null)
        return
      }
      if (!sessionId || incognito) {
        reqRef.current += 1
        setSchedules([])
        setWatchdogFindings([])
        setLoading(false)
        setError(null)
        return
      }
      const req = ++reqRef.current
      setLoading(true)
      setError(null)
      if (fetchOpts.clear) {
        setSchedules([])
        setWatchdogFindings([])
      }
      getTransport()
        .call<LoopSchedule[]>("list_loop_schedules", { sessionId })
        .then((next) => {
          if (reqRef.current !== req) return
          setSchedules(Array.isArray(next) ? next : [])
          setLoading(false)
          getTransport()
            .call<LoopWatchdogFinding[]>("list_loop_watchdog_findings", {
              sessionId,
              graceSecs: 120,
            })
            .then((findings) => {
              if (reqRef.current !== req) return
              setWatchdogFindings(Array.isArray(findings) ? findings : [])
            })
            .catch((e) => {
              if (reqRef.current !== req) return
              logger.error(
                "ui",
                "useLoopSchedules",
                "Failed to load loop watchdog findings",
                e,
              )
              setWatchdogFindings([])
            })
        })
        .catch((e) => {
          if (reqRef.current !== req) return
          const message = e instanceof Error ? e.message : String(e)
          logger.error("ui", "useLoopSchedules", "Failed to load loop schedules", e)
          setError(message)
          setWatchdogFindings([])
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
    // eslint-disable-next-line react-hooks/set-state-in-effect -- refresh after the foreground turn settles
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
    () => ({
      schedules,
      watchdogFindings,
      activeCount,
      loading,
      error,
      refresh: fetchSchedules,
    }),
    [activeCount, error, fetchSchedules, loading, schedules, watchdogFindings],
  )
}

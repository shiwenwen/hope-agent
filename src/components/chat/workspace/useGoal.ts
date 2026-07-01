import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { Task } from "@/types/chat"
import type { WorkflowRun } from "./useWorkflowRuns"

export type GoalState =
  | "active"
  | "paused"
  | "evaluating"
  | "completed"
  | "failed"
  | "cancelled"
  | "blocked"

export interface Goal {
  id: string
  sessionId: string
  objective: string
  completionCriteria: string
  state: GoalState
  modeSnapshot?: string | null
  budgetTokenLimit?: number | null
  budgetTimeLimitSecs?: number | null
  budgetTurnLimit?: number | null
  createdAt: string
  updatedAt: string
  completedAt?: string | null
  finalSummary?: string | null
  finalEvidence: unknown
  blockedReason?: string | null
  lastEvaluatorResult: unknown
}

export interface GoalEvent {
  id: number
  goalId: string
  seq: number
  kind: string
  payload: unknown
  createdAt: string
}

export interface GoalLink {
  id: number
  goalId: string
  targetType: string
  targetId: string
  relation: string
  metadata: unknown
  createdAt: string
}

export type GoalCriterionStatus = "satisfied" | "missing" | "blocked"

export interface GoalCriterionAudit {
  id: string
  text: string
  status: GoalCriterionStatus
  evidenceIds: string[]
  reason?: string | null
}

export interface GoalEvidenceItem {
  id: string
  sourceType: string
  sourceId: string
  relation: string
  title: string
  summary?: string | null
  metadata: unknown
  createdAt: string
}

export interface GoalTimelineItem {
  id: string
  kind: string
  title: string
  summary?: string | null
  status?: string | null
  sourceType?: string | null
  sourceId?: string | null
  metadata: unknown
  createdAt: string
}

export interface GoalBudgetSnapshot {
  tokenLimit?: number | null
  timeLimitSecs?: number | null
  turnLimit?: number | null
  tokensUsed: number
  elapsedSecs: number
  turnsUsed: number
  tokenRatio?: number | null
  timeRatio?: number | null
  turnRatio?: number | null
  warning: boolean
  exhausted: boolean
  warnings: string[]
  exceeded: string[]
}

export interface GoalSnapshot {
  goal: Goal
  links: GoalLink[]
  events: GoalEvent[]
  criteria?: GoalCriterionAudit[]
  evidence?: GoalEvidenceItem[]
  timeline?: GoalTimelineItem[]
  budget?: GoalBudgetSnapshot
  workflowRuns: WorkflowRun[]
  tasks: Task[]
}

export interface GoalStateSnapshot {
  snapshot: GoalSnapshot | null
  loading: boolean
  error: string | null
  refresh: () => void
  setSnapshot: (snapshot: GoalSnapshot | null) => void
}

const GOAL_EVENT_REFRESH_DEBOUNCE_MS = 250

function isGoalPayload(payload: unknown): payload is Goal {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { id?: unknown }).id === "string" &&
    typeof (payload as { sessionId?: unknown }).sessionId === "string"
  )
}

export function useGoal(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; disabled?: boolean } = {},
): GoalStateSnapshot {
  const { incognito = false, disabled = false } = opts
  const [snapshot, setSnapshot] = useState<GoalSnapshot | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchGoal = useCallback(() => {
    if (disabled) {
      reqRef.current += 1
      setLoading(false)
      setError(null)
      return
    }
    if (!sessionId || incognito) {
      reqRef.current += 1
      setSnapshot(null)
      setLoading(false)
      setError(null)
      return
    }
    const req = ++reqRef.current
    setLoading(true)
    setError(null)
    getTransport()
      .call<GoalSnapshot | null>("get_active_goal", { sessionId })
      .then((next) => {
        if (reqRef.current !== req) return
        setSnapshot(next)
        setLoading(false)
      })
      .catch((e) => {
        if (reqRef.current !== req) return
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useGoal", "Failed to load active goal", e)
        setError(message)
        setLoading(false)
      })
  }, [disabled, incognito, sessionId])

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) fetchGoal()
    })
    return () => {
      cancelled = true
    }
  }, [fetchGoal])

  useEffect(() => {
    if (disabled || !sessionId || incognito) return
    const scheduleRefresh = () => {
      if (eventRefreshTimerRef.current !== null) return
      eventRefreshTimerRef.current = window.setTimeout(() => {
        eventRefreshTimerRef.current = null
        fetchGoal()
      }, GOAL_EVENT_REFRESH_DEBOUNCE_MS)
    }
    const maybeRefreshForGoal = (payload: unknown) => {
      if (isGoalPayload(payload) && payload.sessionId !== sessionId) return
      scheduleRefresh()
    }
    const offCreated = getTransport().listen("goal:created", maybeRefreshForGoal)
    const offUpdated = getTransport().listen("goal:updated", maybeRefreshForGoal)
    const offEvent = getTransport().listen("goal:event", scheduleRefresh)
    const offLink = getTransport().listen("goal:link_updated", scheduleRefresh)
    const offWorkflow = getTransport().listen("workflow:updated", scheduleRefresh)
    return () => {
      offCreated()
      offUpdated()
      offEvent()
      offLink()
      offWorkflow()
      if (eventRefreshTimerRef.current !== null) {
        window.clearTimeout(eventRefreshTimerRef.current)
        eventRefreshTimerRef.current = null
      }
    }
  }, [disabled, fetchGoal, incognito, sessionId])

  return useMemo(
    () => ({
      snapshot,
      loading,
      error,
      refresh: fetchGoal,
      setSnapshot,
    }),
    [error, fetchGoal, loading, snapshot],
  )
}

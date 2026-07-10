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
  revision?: number
  domain?: string | null
  workflowTemplateId?: string | null
  workflowTemplateVersion?: string | null
  workflowTaskType?: string | null
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
  closureDecision?: GoalClosureDecision | null
  closureReason?: string | null
  closedAt?: string | null
  followUpItems?: GoalFollowUpItem[]
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
export type GoalCriterionKind = "required" | "optional" | "follow_up"
export type GoalClosureDecision =
  | "accepted_v1"
  | "needs_strict_evidence"
  | "cancelled"
  | "superseded"

export interface GoalCriterionItem {
  id: string
  text: string
  kind: GoalCriterionKind
  checkKind?:
    | "evidence"
    | "artifact"
    | "test"
    | "semantic"
    | "user_acceptance"
    | "external_state"
    | null
  expectedEvidence?: string[]
  inferred?: boolean
}

export interface GoalCriterionAudit {
  id: string
  text: string
  kind?: GoalCriterionKind
  status: GoalCriterionStatus
  evidenceIds: string[]
  reason?: string | null
}

export interface GoalFollowUpItem {
  id: string
  text: string
  createdAt: string
  source?: string | null
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

export interface GoalGraderRun {
  id: string
  goalId: string
  revision: number
  evaluationKey: string
  strict: boolean
  attempt: number
  state: "running" | "completed" | "failed" | string
  result: unknown
  model?: string | null
  usage: unknown
  error?: string | null
  createdAt: string
  updatedAt: string
}

export interface GoalSnapshot {
  goal: Goal
  links: GoalLink[]
  events: GoalEvent[]
  auditStale?: boolean
  criteriaItems?: GoalCriterionItem[]
  criteria?: GoalCriterionAudit[]
  evidence?: GoalEvidenceItem[]
  timeline?: GoalTimelineItem[]
  budget?: GoalBudgetSnapshot
  workflowRuns: WorkflowRun[]
  tasks: Task[]
  graderRuns?: GoalGraderRun[]
}

export type AutonomyActivityState =
  | "idle"
  | "active"
  | "waiting_user"
  | "waiting_external"
  | "evaluating"
  | "paused"
  | "blocked"
  | "terminal"

export interface ActivityDirective {
  kind: string
  reasonCode: string
  sourceId?: string | null
  label?: string | null
}

export interface ActivitySourceRef {
  kind: string
  id: string
  state: string
  label?: string | null
}

export interface AutonomyActivity {
  sessionId: string
  state: AutonomyActivityState
  headlineCode: string
  currentStep?: string | null
  waitingOn?: ActivityDirective | null
  nextAction?: ActivityDirective | null
  nextWakeupAt?: string | null
  needsUser: boolean
  counts: {
    activeWorkflows: number
    activeTasks: number
    activeLoops: number
    activeJobs: number
    awaitingApproval: number
  }
  sourceRefs: ActivitySourceRef[]
  projectedAt: string
}

export interface GoalWatchdogFinding {
  goalId: string
  sessionId: string
  severity: string
  code: "goal_no_recent_progress" | "goal_stale_evaluating" | string
  message: string
  state: GoalState
  lastActivityAt?: string | null
  staleSecs?: number | null
  latestEventKind?: string | null
  latestEventSeq?: number | null
  activeWorkflowCount: number
  activeTaskCount: number
  activeBackgroundJobCount: number
}

export interface GoalStateSnapshot {
  snapshot: GoalSnapshot | null
  activity: AutonomyActivity | null
  watchdogFindings: GoalWatchdogFinding[]
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
  const [activity, setActivity] = useState<AutonomyActivity | null>(null)
  const [watchdogFindings, setWatchdogFindings] = useState<GoalWatchdogFinding[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchGoal = useCallback(() => {
    if (disabled) {
      reqRef.current += 1
      setSnapshot(null)
      setActivity(null)
      setWatchdogFindings([])
      setLoading(false)
      setError(null)
      return
    }
    if (!sessionId || incognito) {
      reqRef.current += 1
      setSnapshot(null)
      setActivity(null)
      setWatchdogFindings([])
      setLoading(false)
      setError(null)
      return
    }
    const req = ++reqRef.current
    setLoading(true)
    setError(null)
    const transport = getTransport()
    Promise.all([
      transport.call<GoalSnapshot | null>("get_active_goal", { sessionId }),
      transport
        .call<AutonomyActivity>("get_autonomy_activity", { sessionId })
        .catch((e) => {
          logger.warn("ui", "useGoal", "Failed to load autonomy activity projection", e)
          return null
        }),
    ])
      .then(async ([next, nextActivity]) => {
        if (reqRef.current !== req) return
        let nextFindings: GoalWatchdogFinding[] = []
        if (next) {
          try {
            nextFindings = await transport.call<GoalWatchdogFinding[]>(
              "list_goal_watchdog_findings",
              { sessionId, staleSecs: 300 },
            )
          } catch (e) {
            logger.warn("ui", "useGoal", "Failed to load goal watchdog findings", e)
          }
        }
        if (reqRef.current !== req) return
        setSnapshot(next)
        setActivity(nextActivity)
        setWatchdogFindings(Array.isArray(nextFindings) ? nextFindings : [])
        setLoading(false)
      })
      .catch((e) => {
        if (reqRef.current !== req) return
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "useGoal", "Failed to load active goal", e)
        setError(message)
        setActivity(null)
        setWatchdogFindings([])
        setLoading(false)
      })
  }, [disabled, incognito, sessionId])

  const setSnapshotFromOwner = useCallback((next: GoalSnapshot | null) => {
    setSnapshot(next)
    setWatchdogFindings([])
  }, [])

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
    const offLoop = getTransport().listen("loop:changed", scheduleRefresh)
    const offJobCreated = getTransport().listen("job:created", scheduleRefresh)
    const offJobUpdated = getTransport().listen("job:updated", scheduleRefresh)
    const offJobCompleted = getTransport().listen("job:completed", scheduleRefresh)
    return () => {
      offCreated()
      offUpdated()
      offEvent()
      offLink()
      offWorkflow()
      offLoop()
      offJobCreated()
      offJobUpdated()
      offJobCompleted()
      if (eventRefreshTimerRef.current !== null) {
        window.clearTimeout(eventRefreshTimerRef.current)
        eventRefreshTimerRef.current = null
      }
    }
  }, [disabled, fetchGoal, incognito, sessionId])

  return useMemo(
    () => ({
      snapshot,
      activity,
      watchdogFindings,
      loading,
      error,
      refresh: fetchGoal,
      setSnapshot: setSnapshotFromOwner,
    }),
    [activity, error, fetchGoal, loading, setSnapshotFromOwner, snapshot, watchdogFindings],
  )
}

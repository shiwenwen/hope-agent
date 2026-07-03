import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

export type WorkflowRunState =
  | "draft"
  | "awaiting_approval"
  | "running"
  | "awaiting_user"
  | "paused"
  | "recovering"
  | "completed"
  | "failed"
  | "cancelled"
  | "blocked"

export type WorkflowOpState = "pending" | "started" | "completed" | "failed"
export type WorkflowEffectClass = "pure" | "idempotent" | "non_idempotent"

export interface WorkflowRun {
  id: string
  sessionId: string
  kind: string
  state: WorkflowRunState
  executionMode: string
  scriptHash: string
  scriptSource: string
  budget: unknown
  cursorSeq: number
  primaryOwner?: string | null
  blockedReason?: string | null
  parentRunId?: string | null
  origin?: string | null
  goalId?: string | null
  worktreeId?: string | null
  createdAt: string
  updatedAt: string
  completedAt?: string | null
}

export interface WorkflowOp {
  id: string
  runId: string
  opKey: string
  opType: string
  effectClass: WorkflowEffectClass
  inputHash: string
  input: unknown
  state: WorkflowOpState
  output?: unknown
  error?: unknown
  childHandle?: string | null
  startedAt: string
  completedAt?: string | null
}

export interface WorkflowEvent {
  id: number
  runId: string
  seq: number
  eventType: string
  payload: unknown
  createdAt: string
}

export interface WorkflowRunSnapshot {
  run: WorkflowRun
  ops: WorkflowOp[]
  events: WorkflowEvent[]
}

export interface WorkflowGateIssue {
  severity: "error" | "warning"
  code: string
  message: string
  suggestion: string
}

export interface WorkflowGateReport {
  issues: WorkflowGateIssue[]
}

export interface WorkflowPermissionPreview {
  summary: Record<string, unknown>
  calls: Record<string, unknown>[]
  truncated?: boolean
}

export interface WorkflowScriptPreview {
  gate: WorkflowGateReport
  gatePassed: boolean
  gateFeedback: string
  permission: WorkflowPermissionPreview
  canCreate: boolean
  canRunImmediately: boolean
  requiresApproval: boolean
  hasDenials: boolean
}

export interface WorkflowRunsState {
  runs: WorkflowRun[]
  activeCount: number
  loading: boolean
  error: string | null
  refresh: () => void
}

const WORKFLOW_EVENT_REFRESH_DEBOUNCE_MS = 250
const WORKFLOW_ACTIVE_POLL_MS = 4000

function isWorkflowRunPayload(payload: unknown): payload is WorkflowRun {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { id?: unknown }).id === "string" &&
    typeof (payload as { sessionId?: unknown }).sessionId === "string"
  )
}

function workflowRunIsActive(run: WorkflowRun): boolean {
  return (
    run.state === "awaiting_approval" ||
    run.state === "running" ||
    run.state === "awaiting_user" ||
    run.state === "paused" ||
    run.state === "recovering"
  )
}

export function useWorkflowRuns(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; turnActive?: boolean; disabled?: boolean } = {},
): WorkflowRunsState {
  const { incognito = false, turnActive = false, disabled = false } = opts
  const [runs, setRuns] = useState<WorkflowRun[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const reqRef = useRef(0)
  const eventRefreshTimerRef = useRef<number | null>(null)

  const fetchRuns = useCallback(
    (fetchOpts: { clear?: boolean } = {}) => {
      if (disabled) {
        reqRef.current += 1
        setLoading(false)
        setError(null)
        return
      }
      if (!sessionId || incognito) {
        reqRef.current += 1
        setRuns([])
        setLoading(false)
        setError(null)
        return
      }

      const req = ++reqRef.current
      setLoading(true)
      setError(null)
      if (fetchOpts.clear) {
        setRuns([])
      }

      getTransport()
        .call<WorkflowRun[]>("list_workflow_runs", { sessionId })
        .then((next) => {
          if (reqRef.current !== req) return
          setRuns(Array.isArray(next) ? next : [])
          setLoading(false)
        })
        .catch((e) => {
          if (reqRef.current !== req) return
          const message = e instanceof Error ? e.message : String(e)
          logger.error("ui", "useWorkflowRuns", "Failed to load workflow runs", e)
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
      }, WORKFLOW_EVENT_REFRESH_DEBOUNCE_MS)
    }
    const maybeRefreshForRun = (payload: unknown) => {
      if (isWorkflowRunPayload(payload)) {
        if (payload.sessionId !== sessionId) return
        setRuns((current) => {
          const existing = current.findIndex((run) => run.id === payload.id)
          if (existing < 0) return [payload, ...current]
          const next = [...current]
          next[existing] = { ...next[existing], ...payload }
          return next
        })
      }
      scheduleRefresh()
    }
    const refresh = () => scheduleRefresh()
    const offCreated = transport.listen("workflow:created", maybeRefreshForRun)
    const offUpdated = transport.listen("workflow:updated", maybeRefreshForRun)
    const offOp = transport.listen("workflow:op_updated", refresh)
    const offEvent = transport.listen("workflow:event", refresh)
    const offLagged = transport.listen("_lagged", refresh)
    return () => {
      offCreated()
      offUpdated()
      offOp()
      offEvent()
      offLagged()
      if (eventRefreshTimerRef.current !== null) {
        window.clearTimeout(eventRefreshTimerRef.current)
        eventRefreshTimerRef.current = null
      }
    }
  }, [disabled, fetchRuns, incognito, sessionId])

  const activeCount = useMemo(() => runs.filter(workflowRunIsActive).length, [runs])

  useEffect(() => {
    if (disabled || !sessionId || incognito || activeCount === 0) return
    const id = window.setInterval(() => {
      fetchRuns()
    }, WORKFLOW_ACTIVE_POLL_MS)
    return () => window.clearInterval(id)
  }, [activeCount, disabled, fetchRuns, incognito, sessionId])

  useEffect(() => {
    if (disabled || !sessionId || incognito) return
    const refreshOnVisible = () => {
      if (document.visibilityState === "visible") {
        fetchRuns()
      }
    }
    document.addEventListener("visibilitychange", refreshOnVisible)
    return () => document.removeEventListener("visibilitychange", refreshOnVisible)
  }, [disabled, fetchRuns, incognito, sessionId])

  return {
    runs,
    activeCount,
    loading,
    error,
    refresh: fetchRuns,
  }
}

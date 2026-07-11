import { useState, useCallback, useEffect, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload, TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { logger } from "@/lib/logger"
import type { AskUserQuestionGroup } from "../ask-user/AskUserQuestionBlock"

export type PlanModeState = "off" | "planning" | "review" | "executing" | "completed"

export interface PlanCardInfo {
  title: string
}

const PLAN_MODE_STATES = new Set<PlanModeState>([
  "off",
  "planning",
  "review",
  "executing",
  "completed",
])

function unwrapField(value: unknown, key: string): unknown {
  if (value && typeof value === "object" && !Array.isArray(value) && key in value) {
    return (value as Record<string, unknown>)[key]
  }
  return value
}

function normalizePlanModeState(value: unknown): PlanModeState {
  const raw = unwrapField(value, "state")
  return typeof raw === "string" && PLAN_MODE_STATES.has(raw as PlanModeState)
    ? (raw as PlanModeState)
    : "off"
}

function normalizePlanContent(value: unknown): string {
  const raw = unwrapField(value, "content")
  return typeof raw === "string" ? raw : ""
}

function withLocalQuestionDeadline(
  group: AskUserQuestionGroup,
  existing?: AskUserQuestionGroup | null,
): AskUserQuestionGroup {
  if (!group.timeoutAt) return { ...group, localTimeoutAtMs: null }
  if (
    existing?.requestId === group.requestId &&
    existing.timeoutAt === group.timeoutAt &&
    typeof existing.localTimeoutAtMs === "number"
  ) {
    return { ...group, localTimeoutAtMs: existing.localTimeoutAtMs }
  }
  const serverNow = group.serverNow ?? Math.floor(Date.now() / 1000)
  const remainingMs = Math.max(0, (group.timeoutAt - serverNow) * 1000)
  return { ...group, localTimeoutAtMs: Date.now() + remainingMs }
}

function isExpiredQuestionGroup(group: AskUserQuestionGroup): boolean {
  return typeof group.localTimeoutAtMs === "number" && group.localTimeoutAtMs <= Date.now()
}

export interface UsePlanModeReturn {
  planState: PlanModeState
  setPlanState: React.Dispatch<React.SetStateAction<PlanModeState>>
  planContent: string
  setPlanContent: React.Dispatch<React.SetStateAction<string>>
  showPanel: boolean
  setShowPanel: React.Dispatch<React.SetStateAction<boolean>>
  planCardInfo: PlanCardInfo | null
  pendingQuestionGroup: AskUserQuestionGroup | null
  setPendingQuestionGroup: React.Dispatch<React.SetStateAction<AskUserQuestionGroup | null>>
  refreshPendingQuestion: () => Promise<void>
  planSubagentRunning: boolean
  enterPlanMode: () => Promise<void>
  exitPlanMode: () => Promise<void>
  approvePlan: () => Promise<void>
  openPlanPanel: () => Promise<void>
}

export function usePlanMode(
  currentSessionId: string | null,
  externalPlanState?: PlanModeState,
  externalSetPlanState?: React.Dispatch<React.SetStateAction<PlanModeState>>,
): UsePlanModeReturn {
  const [internalPlanState, internalSetPlanState] = useState<PlanModeState>("off")
  // Use external state if provided (for sharing with useChatStream)
  const planState = externalPlanState ?? internalPlanState
  const setPlanState = externalSetPlanState ?? internalSetPlanState
  const [planContent, setPlanContent] = useState<string>("")
  const [showPanel, setShowPanel] = useState(false)
  const [planCardInfo, setPlanCardInfo] = useState<PlanCardInfo | null>(null)
  const [pendingQuestionGroup, setPendingQuestionGroupState] =
    useState<AskUserQuestionGroup | null>(null)
  const [planSubagentRunning, setPlanSubagentRunning] = useState(false)
  const pendingMutationVersionRef = useRef(0)
  const pendingReconcileSequenceRef = useRef(0)
  const pendingAppliedReconcileRef = useRef(0)
  const terminalQuestionIdsRef = useRef<Set<string>>(new Set())

  const rememberTerminalQuestion = useCallback((requestId: string) => {
    const terminal = terminalQuestionIdsRef.current
    terminal.add(requestId)
    // Request ids are globally unique; retain a small tombstone window so a
    // stale in-flight snapshot cannot revive a just-resolved card.
    while (terminal.size > 128) {
      const oldest = terminal.values().next().value
      if (typeof oldest !== "string") break
      terminal.delete(oldest)
    }
  }, [])

  // Any explicit local mutation invalidates older REST reconciliation calls.
  // This prevents a response started before timeout/submission from restoring
  // the stale question after the terminal event has already cleared it.
  const setPendingQuestionGroup = useCallback<
    React.Dispatch<React.SetStateAction<AskUserQuestionGroup | null>>
  >((next) => {
    pendingMutationVersionRef.current += 1
    setPendingQuestionGroupState(next)
  }, [])

  // Track whether plan mode was entered in the current no-session context
  const preSessionPlanRef = useRef(false)
  const lastSessionIdRef = useRef<string | null>(null)

  const reconcilePendingQuestion = useCallback(async () => {
    const sessionId = currentSessionId
    const mutationVersion = pendingMutationVersionRef.current
    const sequence = ++pendingReconcileSequenceRef.current
    if (!sessionId) {
      setPendingQuestionGroupState(null)
      return
    }
    try {
      const group = await getTransport().call<AskUserQuestionGroup | null>(
        "get_pending_ask_user_group",
        { sessionId },
      )
      // A terminal/local mutation always invalidates older reads. Among reads
      // from the same mutation epoch, apply the newest successful response;
      // a newer request that merely failed must not discard this result.
      if (mutationVersion !== pendingMutationVersionRef.current) return
      if (sequence < pendingAppliedReconcileRef.current) return
      pendingAppliedReconcileRef.current = sequence
      const normalized =
        group && !terminalQuestionIdsRef.current.has(group.requestId)
          ? withLocalQuestionDeadline(group)
          : null
      setPendingQuestionGroupState(
        normalized && normalized.sessionId === sessionId && !isExpiredQuestionGroup(normalized)
          ? normalized
          : null,
      )
    } catch {
      // Keep the current state on transient transport errors. The local
      // deadline guard still prevents an expired card accepting responses.
    }
  }, [currentSessionId])

  // Enter Plan Mode
  const enterPlanMode = useCallback(async () => {
    if (!currentSessionId) {
      // Pre-session plan mode: set flag so reset logic doesn't clear it
      preSessionPlanRef.current = true
      setPlanState("planning")
      return
    }
    try {
      await getTransport().call("set_plan_mode", { sessionId: currentSessionId, state: "planning" })
      setPlanState("planning")
    } catch (e) {
      logger.error("plan", "usePlanMode::enter", "Failed to enter plan mode", e)
    }
  }, [currentSessionId, setPlanState])

  // Exit Plan Mode
  const exitPlanMode = useCallback(async () => {
    if (currentSessionId) {
      try {
        await getTransport().call("set_plan_mode", { sessionId: currentSessionId, state: "off" })
      } catch (e) {
        logger.error("plan", "usePlanMode::exit", "Failed to exit plan mode", e)
        return
      }
    }
    // Always reset frontend state (even without a session,
    // since enterPlanMode can set "planning" before a session exists)
    preSessionPlanRef.current = false
    setPlanState("off")
    setShowPanel(false)
    setPlanCardInfo(null)
    setPlanContent("")
    queueMicrotask(() => {
      setPendingQuestionGroup(null)
    })
  }, [currentSessionId, setPlanState])

  // Approve and start execution
  const approvePlan = useCallback(async () => {
    if (!currentSessionId) return
    try {
      await getTransport().call("set_plan_mode", {
        sessionId: currentSessionId,
        state: "executing",
      })
      setPlanState("executing")
    } catch (e) {
      logger.error("plan", "usePlanMode::approve", "Failed to approve plan", e)
    }
  }, [currentSessionId, setPlanState])

  const openPlanPanel = useCallback(async () => {
    if (!currentSessionId) {
      setShowPanel(true)
      return
    }

    try {
      const [rawState, rawContent] = await Promise.all([
        getTransport().call<unknown>("get_plan_mode", { sessionId: currentSessionId }),
        getTransport().call<unknown>("get_plan_content", { sessionId: currentSessionId }),
      ])
      const content = normalizePlanContent(rawContent)
      const state = normalizePlanModeState(rawState)

      setPlanState(state)
      setPlanContent(content)
      // Open the panel for any plan-with-content even when backend state=off
      // (user exited but the file is still on disk) — user explicitly asked
      // to view it via the message-stream SubmitPlanResult card; treat as
      // read-only without resurrecting backend state.
      if (state === "planning" || content.trim()) {
        setShowPanel(true)
      }
    } catch (e) {
      logger.error("plan", "usePlanMode::openPanel", "Failed to open plan panel", e)
      setShowPanel(true)
    }
  }, [currentSessionId, setPlanState])

  // Sync state when session changes
  const planStateRef = useRef(planState)
  useEffect(() => {
    planStateRef.current = planState
  }, [planState])

  useEffect(() => {
    const previousSessionId = lastSessionIdRef.current
    const sessionChanged = previousSessionId !== currentSessionId
    lastSessionIdRef.current = currentSessionId

    // Drop previous session's ask UI on any sessionId change — incl. new-chat (null).
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setPendingQuestionGroup(null)

    if (!currentSessionId) {
      // No session — reset plan state unless user just entered plan mode
      // in this no-session context (pre-session plan mode)
      if (!preSessionPlanRef.current) {
        setPlanState("off")
        queueMicrotask(() => {
          setPlanContent("")
          setShowPanel(false)
          setPlanCardInfo(null)
        })
      }
      return
    }

    const shouldMaterializePreSessionPlan =
      preSessionPlanRef.current && planStateRef.current !== "off"

    // Session exists now — clear pre-session flag
    preSessionPlanRef.current = false

    if (!shouldMaterializePreSessionPlan && sessionChanged) {
      setPlanState("off")
      queueMicrotask(() => {
        setPlanContent("")
        setShowPanel(false)
        setPlanCardInfo(null)
      })
    }

    let cancelled = false

    // Restore any still-pending group for the target session. The versioned
    // reconciliation prevents a response racing timeout from reviving it.
    void reconcilePendingQuestion()

    // If plan mode was explicitly entered before the backend session existed,
    // sync that draft state to the newly materialized session. Do not reuse a
    // non-off state from a different session; that makes ordinary chats look
    // like plan sessions after switching.
    if (shouldMaterializePreSessionPlan) {
      getTransport()
        .call("set_plan_mode", { sessionId: currentSessionId, state: planStateRef.current })
        .catch(() => {})
      return () => {
        cancelled = true
        pendingMutationVersionRef.current += 1
      }
    }

    // Otherwise, load plan state from backend (e.g. restoring a historical session)
    Promise.all([
      getTransport().call<unknown>("get_plan_mode", { sessionId: currentSessionId }),
      getTransport().call<unknown>("get_plan_content", { sessionId: currentSessionId }),
    ])
      .then(([rawState, rawContent]) => {
        if (cancelled) return
        const s = normalizePlanModeState(rawState)
        const content = normalizePlanContent(rawContent)
        const hasPlanData = !!content?.trim()
        if (s !== "off" && s !== "planning" && !hasPlanData) {
          setPlanState("off")
          setPlanContent("")
          setShowPanel(false)
          setPlanCardInfo(null)
          getTransport()
            .call("set_plan_mode", { sessionId: currentSessionId, state: "off" })
            .catch(() => {})
          return
        }
        const restoredState = s
        setPlanState(restoredState)
        setPlanContent(content || "")
        if (restoredState === "off") {
          setShowPanel(false)
          setPlanCardInfo(null)
        }
        // Exclude completed: don't hijack chat area when reopening a finished session.
        if (restoredState !== "off" && restoredState !== "completed" && content) {
          setShowPanel(true)
        }
      })
      .catch(() => {
        if (cancelled) return
        setPlanState("off")
        setPlanContent("")
        setShowPanel(false)
        setPlanCardInfo(null)
      })

    return () => {
      cancelled = true
      pendingMutationVersionRef.current += 1
    }
  }, [currentSessionId, reconcilePendingQuestion, setPendingQuestionGroup, setPlanState])

  // Listen for plan_mode_changed events (auto-transition)
  useEffect(() => {
    return getTransport().listen("plan_mode_changed", (raw) => {
      const payload = raw as { sessionId: string; state: string; reason?: string }
      if (payload.sessionId !== currentSessionId) return
      const next = normalizePlanModeState(payload.state)
      // Skip the React update when the state is already correct so downstream
      // memo-ed consumers (PlanPanel / TitleBar / ChatInput) don't re-render
      // for redundant events.
      setPlanState((prev) => (prev === next ? prev : next))
    })
  }, [currentSessionId, setPlanState])

  useEffect(() => {
    return getTransport().listen("plan_submitted", (raw) => {
      const payload = raw as { sessionId: string; title: string; content?: string }
      if (payload.sessionId !== currentSessionId) return
      setPlanCardInfo({ title: payload.title })
      setShowPanel(true)
      setPlanState((prev) => (prev === "review" ? prev : "review"))
      setPendingQuestionGroup(null)
      if (payload.content) {
        setPlanContent(payload.content)
        return
      }
      // Fallback only when the event arrived without content (older backend
      // builds, or future emit paths that skip the embed). The refetch keeps
      // rapid re-submit paths from depending entirely on event timing.
      getTransport()
        .call<unknown>("get_plan_content", { sessionId: payload.sessionId })
        .then((rawContent) => {
          const fresh = normalizePlanContent(rawContent)
          if (fresh) setPlanContent(fresh)
        })
        .catch(() => {})
    })
  }, [currentSessionId, setPendingQuestionGroup, setPlanState])

  // Listen for ask_user_request events emitted by the ask_user_question tool.
  useEffect(() => {
    const handler = (raw: unknown) => {
      try {
        const group = parsePayload<AskUserQuestionGroup>(raw)
        if (!group) return
        if (group.sessionId !== currentSessionId) return
        if (terminalQuestionIdsRef.current.has(group.requestId)) return
        setPendingQuestionGroup((existing) => {
          const normalized = withLocalQuestionDeadline(group, existing)
          return isExpiredQuestionGroup(normalized) ? null : normalized
        })
      } catch {
        // ignore parse errors
      }
    }
    return getTransport().listen("ask_user_request", handler)
  }, [currentSessionId, setPendingQuestionGroup])

  // Unified terminal event covers answers/cancels on every surface plus Stop
  // and session deletion. Reconcile immediately so another queued owner
  // question becomes visible without waiting for focus/reconnect.
  useEffect(() => {
    return getTransport().listen("ask_user:resolved", (raw) => {
      try {
        const payload = parsePayload<{ requestId?: string; sessionId?: string }>(raw)
        if (!payload || payload.sessionId !== currentSessionId) return
        if (payload.requestId) rememberTerminalQuestion(payload.requestId)
        setPendingQuestionGroup((current) =>
          current?.requestId === payload.requestId ? null : current,
        )
        void reconcilePendingQuestion()
      } catch {
        // ignore parse errors
      }
    })
  }, [
    currentSessionId,
    reconcilePendingQuestion,
    rememberTerminalQuestion,
    setPendingQuestionGroup,
  ])

  // Mirror backend timeout cleanup locally so expired questions no longer
  // accept responses in the active chat UI.
  useEffect(() => {
    return getTransport().listen("ask_user_timed_out", (raw) => {
      try {
        const payload = parsePayload<{ requestId?: string; sessionId?: string }>(raw)
        if (!payload) return
        if (payload.sessionId !== currentSessionId) return
        if (payload.requestId) rememberTerminalQuestion(payload.requestId)
        setPendingQuestionGroup((prev) => (prev?.requestId === payload.requestId ? null : prev))
        void reconcilePendingQuestion()
      } catch {
        // ignore parse errors
      }
    })
  }, [
    currentSessionId,
    reconcilePendingQuestion,
    rememberTerminalQuestion,
    setPendingQuestionGroup,
  ])

  // EventBus delivery is at-most-once. Clear at the durable wall-clock
  // deadline even when the renderer was suspended or the WS event was lost.
  useEffect(() => {
    const requestId = pendingQuestionGroup?.requestId
    const timeoutAtMs = pendingQuestionGroup?.localTimeoutAtMs
    if (!requestId || !timeoutAtMs) return

    let timer: ReturnType<typeof setTimeout> | null = null
    const checkDeadline = () => {
      const remainingMs = timeoutAtMs - Date.now()
      if (remainingMs <= 0) {
        rememberTerminalQuestion(requestId)
        queueMicrotask(() => {
          setPendingQuestionGroup((current) => (current?.requestId === requestId ? null : current))
          void reconcilePendingQuestion()
        })
        return
      }
      // Recheck periodically so clock changes and long owner timeouts do not
      // rely on an oversized/clamped browser timer.
      timer = setTimeout(checkDeadline, Math.min(remainingMs, 60_000))
    }
    checkDeadline()
    return () => {
      if (timer) clearTimeout(timer)
    }
  }, [
    pendingQuestionGroup?.requestId,
    pendingQuestionGroup?.localTimeoutAtMs,
    reconcilePendingQuestion,
    rememberTerminalQuestion,
    setPendingQuestionGroup,
  ])

  // Re-read durable state after HTTP WS reconnect/lag and when a suspended
  // desktop renderer becomes active again. This heals missed terminal events
  // and refreshes owner-plane questions restored after backend restart.
  useEffect(() => {
    const reconcile = () => {
      void reconcilePendingQuestion()
    }
    const offResync = getTransport().listen(TRANSPORT_EVENT_RESYNC_REQUIRED, reconcile)
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") reconcile()
    }
    window.addEventListener("focus", reconcile)
    document.addEventListener("visibilitychange", onVisibilityChange)
    return () => {
      offResync()
      window.removeEventListener("focus", reconcile)
      document.removeEventListener("visibilitychange", onVisibilityChange)
    }
  }, [reconcilePendingQuestion])

  // Listen for plan_subagent_status events (plan sub-agent running/completed)
  useEffect(() => {
    return getTransport().listen("plan_subagent_status", (raw) => {
      const payload = raw as { sessionId: string; status: string; runId: string }
      if (payload.sessionId !== currentSessionId) return
      setPlanSubagentRunning(payload.status === "running")
    })
  }, [currentSessionId])

  // Also clear planSubagentRunning when plan state transitions away from planning
  useEffect(() => {
    if (planState !== "planning") {
      queueMicrotask(() => {
        setPlanSubagentRunning(false)
      })
    }
  }, [planState])

  return {
    planState,
    setPlanState,
    planContent,
    setPlanContent,
    showPanel,
    setShowPanel,
    planCardInfo,
    pendingQuestionGroup,
    setPendingQuestionGroup,
    refreshPendingQuestion: reconcilePendingQuestion,
    planSubagentRunning,
    enterPlanMode,
    exitPlanMode,
    approvePlan,
    openPlanPanel,
  }
}

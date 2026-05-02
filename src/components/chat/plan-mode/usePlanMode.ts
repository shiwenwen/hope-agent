import { useState, useCallback, useEffect, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
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
  const [pendingQuestionGroup, setPendingQuestionGroup] = useState<AskUserQuestionGroup | null>(null)
  const [planSubagentRunning, setPlanSubagentRunning] = useState(false)

  // Track whether plan mode was entered in the current no-session context
  const preSessionPlanRef = useRef(false)
  const lastSessionIdRef = useRef<string | null>(null)

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
    queueMicrotask(() => {
      setPendingQuestionGroup(null)
    })
  }, [currentSessionId, setPlanState])

  // Approve and start execution
  const approvePlan = useCallback(async () => {
    if (!currentSessionId) return
    try {
      await getTransport().call("set_plan_mode", { sessionId: currentSessionId, state: "executing" })
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

      let state = normalizePlanModeState(rawState)
      if (state === "off" && content.trim()) {
        state = "review"
        getTransport()
          .call("set_plan_mode", { sessionId: currentSessionId, state })
          .catch(() => {})
      }

      setPlanState(state)
      setPlanContent(content)
      if (state !== "off" && (state === "planning" || content.trim())) {
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

    // Clear stale question UI, then restore any still-pending group for the
    // target session from the backend (handles "switch away before answering"
    // and "reopen a session that had unanswered questions").
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setPendingQuestionGroup(null)
    getTransport()
      .call<AskUserQuestionGroup | null>("get_pending_ask_user_group", {
        sessionId: currentSessionId,
      })
      .then((group) => {
        if (cancelled) return
        if (group && group.sessionId === currentSessionId) {
          setPendingQuestionGroup(group)
        }
      })
      .catch(() => {})

    // If plan mode was explicitly entered before the backend session existed,
    // sync that draft state to the newly materialized session. Do not reuse a
    // non-off state from a different session; that makes ordinary chats look
    // like plan sessions after switching.
    if (shouldMaterializePreSessionPlan) {
      getTransport().call("set_plan_mode", { sessionId: currentSessionId, state: planStateRef.current })
        .catch(() => {})
      return () => {
        cancelled = true
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
        let restoredState = s
        if (s === "off" && content.trim()) {
          restoredState = "review"
          getTransport()
            .call("set_plan_mode", { sessionId: currentSessionId, state: restoredState })
            .catch(() => {})
        }
        setPlanState(restoredState)
        setPlanContent(content || "")
        if (restoredState === "off") {
          setShowPanel(false)
          setPlanCardInfo(null)
        }
        // Only auto-show panel when plan content exists (not during initial planning)
        if (restoredState !== "off" && content) setShowPanel(true)
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
    }
  }, [currentSessionId, setPlanState])

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

  // Listen for plan_submitted events (LLM submitted a plan via submit_plan tool).
  //
  // Backend embeds `content` in the payload as a fast path, but we ALWAYS
  // refetch from `get_plan_content` afterwards. The refetch is the source of
  // truth: re-submit scenarios (user inline-comments → model resubmits → new
  // plan_submitted event) used to leave the panel showing stale content,
  // which traced back to the embedded payload.content not always propagating
  // through the React state cycle on rapid back-to-back submits. The refetch
  // costs one cheap RPC per submit and guarantees the panel reflects what's
  // actually on disk — that's the only contract that matters here.
  useEffect(() => {
    return getTransport().listen("plan_submitted", (raw) => {
      const payload = raw as { sessionId: string; title: string; content?: string }
      if (payload.sessionId !== currentSessionId) return
      setPlanCardInfo({ title: payload.title })
      setShowPanel(true)
      setPlanState((prev) => (prev === "review" ? prev : "review"))
      setPendingQuestionGroup(null)
      if (payload.content) setPlanContent(payload.content)
      getTransport()
        .call<unknown>("get_plan_content", { sessionId: payload.sessionId })
        .then((rawContent) => {
          const fresh = normalizePlanContent(rawContent)
          if (fresh) setPlanContent(fresh)
        })
        .catch(() => {})
    })
  }, [currentSessionId, setPlanState])

  // Listen for ask_user_request events emitted by the ask_user_question tool.
  useEffect(() => {
    const handler = (raw: unknown) => {
      try {
        const group = parsePayload<AskUserQuestionGroup>(raw)
        if (group.sessionId !== currentSessionId) return
        setPendingQuestionGroup(group)
      } catch {
        // ignore parse errors
      }
    }
    return getTransport().listen("ask_user_request", handler)
  }, [currentSessionId])

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
    planSubagentRunning,
    enterPlanMode,
    exitPlanMode,
    approvePlan,
    openPlanPanel,
  }
}

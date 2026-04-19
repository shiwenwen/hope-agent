import { useState, useCallback, useEffect, useMemo, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import type { AskUserQuestionGroup } from "../ask-user/AskUserQuestionBlock"

export type PlanModeState = "off" | "planning" | "review" | "executing" | "paused" | "completed"

export interface PlanStep {
  index: number
  phase: string
  title: string
  description: string
  status: "pending" | "in_progress" | "completed" | "skipped" | "failed"
  durationMs?: number
}

export interface PlanCardInfo {
  title: string
  stepCount: number
  phaseCount: number
}

export interface UsePlanModeReturn {
  planState: PlanModeState
  setPlanState: React.Dispatch<React.SetStateAction<PlanModeState>>
  planSteps: PlanStep[]
  setPlanSteps: React.Dispatch<React.SetStateAction<PlanStep[]>>
  planContent: string
  setPlanContent: React.Dispatch<React.SetStateAction<string>>
  showPanel: boolean
  setShowPanel: React.Dispatch<React.SetStateAction<boolean>>
  progress: number
  completedCount: number
  planCardInfo: PlanCardInfo | null
  pendingQuestionGroup: AskUserQuestionGroup | null
  setPendingQuestionGroup: React.Dispatch<React.SetStateAction<AskUserQuestionGroup | null>>
  planSubagentRunning: boolean
  enterPlanMode: () => Promise<void>
  exitPlanMode: () => Promise<void>
  approvePlan: () => Promise<void>
  pauseExecution: () => Promise<void>
  resumeExecution: () => Promise<void>
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
  const [planSteps, setPlanSteps] = useState<PlanStep[]>([])
  const [planContent, setPlanContent] = useState<string>("")
  const [showPanel, setShowPanel] = useState(false)
  const [planCardInfo, setPlanCardInfo] = useState<PlanCardInfo | null>(null)
  const [pendingQuestionGroup, setPendingQuestionGroup] = useState<AskUserQuestionGroup | null>(null)
  const [planSubagentRunning, setPlanSubagentRunning] = useState(false)

  // Track whether plan mode was entered in the current no-session context
  const preSessionPlanRef = useRef(false)

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

  // Pause execution
  const pauseExecution = useCallback(async () => {
    if (!currentSessionId) return
    try {
      await getTransport().call("set_plan_mode", { sessionId: currentSessionId, state: "paused" })
      setPlanState("paused")
    } catch (e) {
      logger.error("plan", "usePlanMode::pause", "Failed to pause plan", e)
    }
  }, [currentSessionId, setPlanState])

  // Resume execution
  const resumeExecution = useCallback(async () => {
    if (!currentSessionId) return
    try {
      await getTransport().call("set_plan_mode", { sessionId: currentSessionId, state: "executing" })
      setPlanState("executing")
    } catch (e) {
      logger.error("plan", "usePlanMode::resume", "Failed to resume plan", e)
    }
  }, [currentSessionId, setPlanState])

  // Sync state when session changes
  const planStateRef = useRef(planState)
  useEffect(() => {
    planStateRef.current = planState
  }, [planState])

  useEffect(() => {
    if (!currentSessionId) {
      // No session — reset plan state unless user just entered plan mode
      // in this no-session context (pre-session plan mode)
      if (!preSessionPlanRef.current) {
        setPlanState("off")
        queueMicrotask(() => {
          setPlanSteps([])
          setPlanContent("")
          setShowPanel(false)
          setPlanCardInfo(null)
        })
      }
      return
    }

    // Session exists now — clear pre-session flag
    preSessionPlanRef.current = false

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
        if (group && group.sessionId === currentSessionId) {
          setPendingQuestionGroup(group)
        }
      })
      .catch(() => {})

    // If frontend already has a non-off plan state (entered before session existed),
    // sync it TO the backend instead of reading FROM backend
    if (planStateRef.current !== "off") {
      getTransport().call("set_plan_mode", { sessionId: currentSessionId, state: planStateRef.current })
        .catch(() => {})
      return
    }

    // Otherwise, load plan state from backend (e.g. restoring a historical session)
    Promise.all([
      getTransport().call<string>("get_plan_mode", { sessionId: currentSessionId }),
      getTransport().call<PlanStep[]>("get_plan_steps", { sessionId: currentSessionId }),
      getTransport().call<string | null>("get_plan_content", { sessionId: currentSessionId }),
    ])
      .then(([state, steps, content]) => {
        const s = (state || "off") as PlanModeState
        setPlanState(s)
        setPlanSteps(steps || [])
        setPlanContent(content || "")
        // Only auto-show panel when plan content exists (not during initial planning)
        if (s !== "off" && content) setShowPanel(true)
      })
      .catch(() => {
        setPlanState("off")
      })
  }, [currentSessionId, setPlanState])

  // Listen for plan_content_updated events (backend detected plan in LLM output)
  useEffect(() => {
    return getTransport().listen("plan_content_updated", (raw) => {
      const payload = raw as { sessionId: string; stepCount: number; content: string }
      if (payload.sessionId !== currentSessionId) return
      setPlanContent(payload.content)
      getTransport().call<PlanStep[]>("get_plan_steps", { sessionId: payload.sessionId })
        .then((steps) => {
          if (steps && steps.length > 0) {
            setPlanSteps(steps)
          }
        })
        .catch(() => {})
    })
  }, [currentSessionId])

  // Listen for plan_step_updated events
  useEffect(() => {
    return getTransport().listen("plan_step_updated", (raw) => {
      const payload = raw as { sessionId: string; stepIndex: number; status: string; durationMs?: number }
      if (payload.sessionId !== currentSessionId) return
      setPlanSteps((prev) =>
        prev.map((s) =>
          s.index === payload.stepIndex
            ? {
                ...s,
                status: payload.status as PlanStep["status"],
                durationMs: payload.durationMs ?? s.durationMs,
              }
            : s
        )
      )
    })
  }, [currentSessionId])

  // Listen for plan_mode_changed events (auto-transition)
  useEffect(() => {
    return getTransport().listen("plan_mode_changed", (raw) => {
      const payload = raw as { sessionId: string; state: string; reason?: string }
      if (payload.sessionId !== currentSessionId) return
      setPlanState(payload.state as PlanModeState)
    })
  }, [currentSessionId, setPlanState])

  // Listen for plan_submitted events (LLM submitted a plan via submit_plan tool)
  useEffect(() => {
    return getTransport().listen("plan_submitted", (raw) => {
      const payload = raw as { sessionId: string; title: string; stepCount: number; phaseCount: number; steps: PlanStep[] }
      if (payload.sessionId !== currentSessionId) return
      setPlanCardInfo({
        title: payload.title,
        stepCount: payload.stepCount,
        phaseCount: payload.phaseCount,
      })
      setPlanSteps(payload.steps)
      setPlanState("review")
      setPendingQuestionGroup(null)
      // Load the plan content and auto-show panel
      getTransport().call<string | null>("get_plan_content", { sessionId: currentSessionId })
        .then((content) => {
          if (content) {
            setPlanContent(content)
            setShowPanel(true)
          }
        })
        .catch(() => {})
    })
  }, [currentSessionId, setPlanState])

  // Listen for plan_amended events (steps changed during execution via amend_plan tool)
  useEffect(() => {
    return getTransport().listen("plan_amended", (raw) => {
      const payload = raw as { sessionId: string; steps: PlanStep[]; stepCount: number }
      if (payload.sessionId !== currentSessionId) return
      setPlanSteps(payload.steps)
    })
  }, [currentSessionId])

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

  // Calculate progress
  const completedCount = useMemo(() => {
    return planSteps.filter(
      (s) => s.status === "completed" || s.status === "skipped" || s.status === "failed"
    ).length
  }, [planSteps])

  const progress = useMemo(() => {
    if (planSteps.length === 0) return 0
    return Math.round((completedCount / planSteps.length) * 100)
  }, [planSteps.length, completedCount])

  return {
    planState,
    setPlanState,
    planSteps,
    setPlanSteps,
    planContent,
    setPlanContent,
    showPanel,
    setShowPanel,
    progress,
    completedCount,
    planCardInfo,
    pendingQuestionGroup,
    setPendingQuestionGroup,
    planSubagentRunning,
    enterPlanMode,
    exitPlanMode,
    approvePlan,
    pauseExecution,
    resumeExecution,
  }
}

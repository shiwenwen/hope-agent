import { useState, useCallback, useEffect, useMemo, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { logger } from "@/lib/logger"
import type { PlanQuestionGroup } from "./PlanQuestionBlock"

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
  pendingQuestionGroup: PlanQuestionGroup | null
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
  const [pendingQuestionGroup, setPendingQuestionGroup] = useState<PlanQuestionGroup | null>(null)
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
      await invoke("set_plan_mode", { sessionId: currentSessionId, state: "planning" })
      setPlanState("planning")
    } catch (e) {
      logger.error("plan", "usePlanMode::enter", "Failed to enter plan mode", e)
    }
  }, [currentSessionId, setPlanState])

  // Exit Plan Mode
  const exitPlanMode = useCallback(async () => {
    if (currentSessionId) {
      try {
        await invoke("set_plan_mode", { sessionId: currentSessionId, state: "off" })
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
    setPendingQuestionGroup(null)
  }, [currentSessionId, setPlanState])

  // Approve and start execution
  const approvePlan = useCallback(async () => {
    if (!currentSessionId) return
    try {
      await invoke("set_plan_mode", { sessionId: currentSessionId, state: "executing" })
      setPlanState("executing")
    } catch (e) {
      logger.error("plan", "usePlanMode::approve", "Failed to approve plan", e)
    }
  }, [currentSessionId, setPlanState])

  // Pause execution
  const pauseExecution = useCallback(async () => {
    if (!currentSessionId) return
    try {
      await invoke("set_plan_mode", { sessionId: currentSessionId, state: "paused" })
      setPlanState("paused")
    } catch (e) {
      logger.error("plan", "usePlanMode::pause", "Failed to pause plan", e)
    }
  }, [currentSessionId, setPlanState])

  // Resume execution
  const resumeExecution = useCallback(async () => {
    if (!currentSessionId) return
    try {
      await invoke("set_plan_mode", { sessionId: currentSessionId, state: "executing" })
      setPlanState("executing")
    } catch (e) {
      logger.error("plan", "usePlanMode::resume", "Failed to resume plan", e)
    }
  }, [currentSessionId, setPlanState])

  // Sync state when session changes
  const planStateRef = useRef(planState)
  planStateRef.current = planState

  useEffect(() => {
    if (!currentSessionId) {
      // No session — reset plan state unless user just entered plan mode
      // in this no-session context (pre-session plan mode)
      if (!preSessionPlanRef.current) {
        setPlanState("off")
        setPlanSteps([])
        setPlanContent("")
        setShowPanel(false)
        setPlanCardInfo(null)
      }
      return
    }

    // Session exists now — clear pre-session flag
    preSessionPlanRef.current = false

    // Always clear stale question UI on session switch
    setPendingQuestionGroup(null)

    // If frontend already has a non-off plan state (entered before session existed),
    // sync it TO the backend instead of reading FROM backend
    if (planStateRef.current !== "off") {
      invoke("set_plan_mode", { sessionId: currentSessionId, state: planStateRef.current })
        .catch(() => {})
      return
    }

    // Otherwise, load plan state from backend (e.g. restoring a historical session)
    Promise.all([
      invoke<string>("get_plan_mode", { sessionId: currentSessionId }),
      invoke<PlanStep[]>("get_plan_steps", { sessionId: currentSessionId }),
      invoke<string | null>("get_plan_content", { sessionId: currentSessionId }),
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
    let unlisten: UnlistenFn | undefined
    listen<{ sessionId: string; stepCount: number; content: string }>(
      "plan_content_updated",
      (event) => {
        if (event.payload.sessionId !== currentSessionId) return
        setPlanContent(event.payload.content)
        invoke<PlanStep[]>("get_plan_steps", { sessionId: event.payload.sessionId })
          .then((steps) => {
            if (steps && steps.length > 0) {
              setPlanSteps(steps)
            }
          })
          .catch(() => {})
      }
    ).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [currentSessionId])

  // Listen for plan_step_updated events
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<{ sessionId: string; stepIndex: number; status: string; durationMs?: number }>(
      "plan_step_updated",
      (event) => {
        if (event.payload.sessionId !== currentSessionId) return
        setPlanSteps((prev) =>
          prev.map((s) =>
            s.index === event.payload.stepIndex
              ? {
                  ...s,
                  status: event.payload.status as PlanStep["status"],
                  durationMs: event.payload.durationMs ?? s.durationMs,
                }
              : s
          )
        )
      }
    ).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [currentSessionId])

  // Listen for plan_mode_changed events (auto-transition)
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<{ sessionId: string; state: string; reason?: string }>(
      "plan_mode_changed",
      (event) => {
        if (event.payload.sessionId !== currentSessionId) return
        setPlanState(event.payload.state as PlanModeState)
      }
    ).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [currentSessionId, setPlanState])

  // Listen for plan_submitted events (LLM submitted a plan via submit_plan tool)
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<{ sessionId: string; title: string; stepCount: number; phaseCount: number; steps: PlanStep[] }>(
      "plan_submitted",
      (event) => {
        if (event.payload.sessionId !== currentSessionId) return
        setPlanCardInfo({
          title: event.payload.title,
          stepCount: event.payload.stepCount,
          phaseCount: event.payload.phaseCount,
        })
        setPlanSteps(event.payload.steps)
        setPlanState("review")
        setPendingQuestionGroup(null)
        // Load the plan content and auto-show panel
        invoke<string | null>("get_plan_content", { sessionId: currentSessionId })
          .then((content) => {
            if (content) {
              setPlanContent(content)
              setShowPanel(true)
            }
          })
          .catch(() => {})
      }
    ).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [currentSessionId, setPlanState])

  // Listen for plan_amended events (steps changed during execution via amend_plan tool)
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<{ sessionId: string; steps: PlanStep[]; stepCount: number }>(
      "plan_amended",
      (event) => {
        if (event.payload.sessionId !== currentSessionId) return
        setPlanSteps(event.payload.steps)
      }
    ).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [currentSessionId])

  // Listen for plan_question_request events
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<string>(
      "plan_question_request",
      (event) => {
        try {
          const group: PlanQuestionGroup = JSON.parse(event.payload)
          if (group.sessionId !== currentSessionId) return
          setPendingQuestionGroup(group)
        } catch {
          // ignore parse errors
        }
      }
    ).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [currentSessionId])

  // Listen for plan_subagent_status events (plan sub-agent running/completed)
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<{ sessionId: string; status: string; runId: string }>(
      "plan_subagent_status",
      (event) => {
        if (event.payload.sessionId !== currentSessionId) return
        setPlanSubagentRunning(event.payload.status === "running")
      }
    ).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [currentSessionId])

  // Also clear planSubagentRunning when plan state transitions away from planning
  useEffect(() => {
    if (planState !== "planning") {
      setPlanSubagentRunning(false)
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
    planSubagentRunning,
    enterPlanMode,
    exitPlanMode,
    approvePlan,
    pauseExecution,
    resumeExecution,
  }
}

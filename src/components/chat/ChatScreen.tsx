import { useState, useRef, useEffect, useCallback, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { save } from "@tauri-apps/plugin-dialog"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Brain } from "lucide-react"
import type { AvailableModel, ActiveModel, Message } from "@/types/chat"
import { getEffortOptionsForType } from "@/types/chat"
import type { CommandResult } from "./slash-commands/types"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import ChatSidebar from "@/components/chat/ChatSidebar"
import ChatInput from "@/components/chat/ChatInput"
import ChatTitleBar from "@/components/chat/ChatTitleBar"
import MessageList from "@/components/chat/MessageList"
import CrashRecoveryBanner from "@/components/common/CrashRecoveryBanner"
import CanvasPanel from "@/components/chat/CanvasPanel"
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
  AlertDialogAction,
} from "@/components/ui/alert-dialog"
import { useChatSession } from "./useChatSession"
import { useChatStream } from "./useChatStream"
import { useAutoScroll } from "./useAutoScroll"
import { usePlanMode } from "./plan-mode/usePlanMode"
import SystemPromptDialog from "./SystemPromptDialog"
import { PlanPanel } from "./plan-mode/PlanPanel"

interface ChatScreenProps {
  onOpenAgentSettings?: (agentId: string) => void
  onCodexReauth?: () => void
  initialSessionId?: string
  onSessionNavigated?: () => void
  onUnreadCountChange?: (count: number) => void
  sessionsRefreshTrigger?: number
}

export default function ChatScreen({
  onOpenAgentSettings,
  onCodexReauth,
  initialSessionId,
  onSessionNavigated,
  onUnreadCountChange,
  sessionsRefreshTrigger,
}: ChatScreenProps) {
  const { t } = useTranslation()

  // ── Model State ─────────────────────────────────────────────
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null)
  const [reasoningEffort, setReasoningEffort] = useState("medium")
  const [sessionTemperature, setSessionTemperature] = useState<number | null>(null)
  const globalActiveModelRef = useRef<ActiveModel | null>(null)

  // Sidebar panel width
  const [panelWidth, setPanelWidth] = useState(288)

  // Right panel widths (resizable)
  const [planPanelWidth, setPlanPanelWidth] = useState(400)
  const [canvasPanelWidth, setCanvasPanelWidth] = useState(480)

  // Context compact state
  const [compacting, setCompacting] = useState(false)

  // System prompt viewer state
  const [showSystemPrompt, setShowSystemPrompt] = useState(false)
  const [systemPromptContent, setSystemPromptContent] = useState("")
  const [systemPromptLoading, setSystemPromptLoading] = useState(false)

  // Plan mode state (declared early so useChatStream can access it)
  const [planModeState, setPlanModeState] = useState<"off" | "planning" | "review" | "executing" | "paused" | "completed">("off")

  // Update model display + reasoning effort without persisting to global settings
  const applyModelForDisplay = useCallback(
    (key: string) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      setActiveModel({ providerId, modelId })
      const newModel = availableModels.find(
        (m) => m.providerId === providerId && m.modelId === modelId,
      )
      if (newModel) {
        const validOptions = getEffortOptionsForType(newModel.apiType, t)
        const isValid = validOptions.some((opt) => opt.value === reasoningEffort)
        if (!isValid) {
          const fallback = validOptions.some((o) => o.value === "medium") ? "medium" : "none"
          setReasoningEffort(fallback)
        }
      }
    },
    [availableModels, reasoningEffort, t],
  )

  const handleModelChange = useCallback(
    async (key: string) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      setActiveModel({ providerId, modelId })
      try {
        await invoke("set_active_model", { providerId, modelId })
      } catch (e) {
        logger.error("ui", "ChatScreen::modelChange", "Failed to set model", e)
      }
      const newModel = availableModels.find(
        (m) => m.providerId === providerId && m.modelId === modelId,
      )
      if (newModel) {
        const validOptions = getEffortOptionsForType(newModel.apiType, t)
        const isValid = validOptions.some((opt) => opt.value === reasoningEffort)
        if (!isValid) {
          const fallback = validOptions.some((o) => o.value === "medium") ? "medium" : "none"
          handleEffortChange(fallback)
        }
      }
    },
    [availableModels, reasoningEffort, t],
  )

  async function handleEffortChange(effort: string) {
    setReasoningEffort(effort)
    try {
      await invoke("set_reasoning_effort", { effort })
    } catch (e) {
      logger.error("ui", "ChatScreen::effortChange", "Failed to set reasoning effort", e)
    }
  }

  // ── Session Hook ────────────────────────────────────────────
  const session = useChatSession({
    availableModels,
    setActiveModel,
    globalActiveModelRef,
    handleModelChange,
    applyModelForDisplay,
    initialSessionId,
    onSessionNavigated,
    onUnreadCountChange,
  })

  const isCronSession = useMemo(
    () => session.sessions.find((s) => s.id === session.currentSessionId)?.isCron ?? false,
    [session.sessions, session.currentSessionId],
  )
  const reloadSessions = session.reloadSessions
  const currentAgentId = session.currentAgentId
  const handleNewChat = session.handleNewChat
  const currentSessionId = session.currentSessionId

  // Rename session handler
  const handleRenameSession = useCallback(async (sessionId: string, title: string) => {
    try {
      await invoke("rename_session_cmd", { sessionId, title })
      reloadSessions()
    } catch (err) {
      logger.error("chat", "ChatScreen::renameSession", "Failed to rename session", err)
    }
  }, [reloadSessions])

  // Reload sessions when external trigger changes (e.g. mark-all-read from IconSidebar)
  useEffect(() => {
    if (sessionsRefreshTrigger) {
      reloadSessions()
    }
  }, [sessionsRefreshTrigger, reloadSessions])

  // Listen for tray "new-session" event to trigger new chat
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen("new-session", () => {
      handleNewChat(currentAgentId)
    }).then((fn) => { unlisten = fn })
    return () => { unlisten?.() }
  }, [handleNewChat, currentAgentId])

  // Listen for channel slash command state-sync events
  useEffect(() => {
    const unlisteners: Promise<UnlistenFn>[] = []

    // Model switched from channel (/model)
    unlisteners.push(
      listen("slash:model_switched", (event) => {
        const { providerId, modelId } = event.payload as {
          providerId: string
          modelId: string
        }
        setActiveModel({ providerId, modelId })
        applyModelForDisplay(`${providerId}::${modelId}`)
      }),
    )

    // Effort changed from channel (/think)
    unlisteners.push(
      listen("slash:effort_changed", (event) => {
        setReasoningEffort(event.payload as string)
      }),
    )

    // Session cleared from channel (/clear)
    unlisteners.push(
      listen("slash:session_cleared", (event) => {
        const clearedSid = event.payload as string
        if (clearedSid === session.currentSessionId) {
          session.setMessages([])
        }
        session.reloadSessions()
      }),
    )

    // Plan state changed from channel (/plan)
    unlisteners.push(
      listen("slash:plan_changed", () => {
        session.reloadSessions()
      }),
    )

    return () => {
      unlisteners.forEach((p) => p.then((fn) => fn()))
    }
  }, [session.currentSessionId]) // eslint-disable-line react-hooks/exhaustive-deps

  // Fetch models and current settings on mount
  useEffect(() => {
    ;(async () => {
      try {
        const [models, active, settings, agentConfig] = await Promise.all([
          invoke<AvailableModel[]>("get_available_models"),
          invoke<ActiveModel | null>("get_active_model"),
          invoke<{ model: string; reasoning_effort: string }>("get_current_settings"),
          invoke<{
            name: string
            emoji?: string | null
            avatar?: string | null
          }>("get_agent_config", { id: "default" }).catch(() => null),
        ])
        setAvailableModels(models)
        setActiveModel(active)
        globalActiveModelRef.current = active
        setReasoningEffort(settings.reasoning_effort)
        if (agentConfig) {
          session.setAgentName(agentConfig.name)
        }
      } catch (e) {
        logger.error("ui", "ChatScreen::loadSettings", "Failed to load settings", e)
      }
    })()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // ── Stream Hook ─────────────────────────────────────────────
  const stream = useChatStream({
    messages: session.messages,
    setMessages: session.setMessages,
    currentSessionId: session.currentSessionId,
    setCurrentSessionId: session.setCurrentSessionId,
    currentSessionIdRef: session.currentSessionIdRef,
    currentAgentId: session.currentAgentId,
    agentName: session.agentName,
    loading: session.loading,
    setLoading: session.setLoading,
    loadingSessionsRef: session.loadingSessionsRef,
    setLoadingSessionIds: session.setLoadingSessionIds,
    sessionCacheRef: session.sessionCacheRef,
    sessions: session.sessions,
    agents: session.agents,
    activeModel,
    reloadSessions: session.reloadSessions,
    updateSessionMessages: session.updateSessionMessages,
    planMode: planModeState,
    temperatureOverride: sessionTemperature,
  })

  // ── Plan Mode Hook ─────────────────────────────────────────
  const planMode = usePlanMode(session.currentSessionId, planModeState, setPlanModeState)
  const setPlanState = planMode.setPlanState
  const sendMessage = stream.handleSend

  // ── Auto-scroll Hook ───────────────────────────────────────
  const { scrollContainerRef, bottomRef } = useAutoScroll({
    loading: session.loading,
    messages: session.messages,
    currentSessionId: session.currentSessionId,
  })

  // ── Memory extraction toast ────────────────────────────────
  const [memoryToast, setMemoryToast] = useState<{ count: number } | null>(null)
  const memoryToastTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<{ count: number; sessionId: string }>("memory_extracted", (event) => {
      const { count, sessionId } = event.payload
      // Only show toast for the current session
      if (sessionId === session.currentSessionId && count > 0) {
        setMemoryToast({ count })
        if (memoryToastTimer.current) clearTimeout(memoryToastTimer.current)
        memoryToastTimer.current = setTimeout(() => setMemoryToast(null), 4000)
      }
    }).then((fn) => { unlisten = fn })
    return () => {
      unlisten?.()
      if (memoryToastTimer.current) clearTimeout(memoryToastTimer.current)
    }
  }, [session.currentSessionId])

  // ── Scroll-to-top: load older messages ─────────────────────
  useEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return
    const onScroll = () => {
      if (el.scrollTop < 50) {
        session.handleLoadMore()
      }
    }
    el.addEventListener("scroll", onScroll, { passive: true })
    return () => el.removeEventListener("scroll", onScroll)
  }, [session.handleLoadMore]) // eslint-disable-line react-hooks/exhaustive-deps

  // ── Load system prompt ──────────────────────────────────────────
  const loadSystemPrompt = useCallback(async () => {
    setSystemPromptLoading(true)
    try {
      const prompt = await invoke<string>("get_system_prompt", {
        agentId: session.currentAgentId,
      })
      setSystemPromptContent(prompt)
      setShowSystemPrompt(true)
    } catch (e) {
      logger.error("ui", "ChatScreen::loadSystemPrompt", "Failed to load system prompt", e)
    } finally {
      setSystemPromptLoading(false)
    }
  }, [session.currentAgentId])

  // ── Slash Command Action Handler ──────────────────────────────
  const handleCommandAction = useCallback(
    async (result: CommandResult) => {
      const action = result.action

      // Show command output as an event message (if content is not empty)
      // Skip for newSession: we immediately reset to a blank state so the message would be lost anyway
      if (result.content && action?.type !== "newSession") {
        const eventMsg: Message = {
          role: "event",
          content: result.content,
          timestamp: new Date().toISOString(),
        }
        session.setMessages((prev) => [...prev, eventMsg])
      }

      if (!action) return

      switch (action.type) {
        case "newSession":
          // Behave like the "New Chat" button: clear immediately without showing an empty session
          // in the sidebar. The backend-created session is deleted to avoid DB clutter.
          session.handleNewChat(session.currentAgentId)
          if (action.sessionId) {
            invoke("delete_session_cmd", { sessionId: action.sessionId })
              .then(() => session.reloadSessions())
              .catch(() => {})
          }
          break
        case "switchModel":
          handleModelChange(`${action.providerId}::${action.modelId}`)
          break
        case "setEffort":
          handleEffortChange(action.effort)
          break
        case "switchAgent":
          if (action.sessionId) session.handleSwitchSession(action.sessionId)
          break
        case "stopStream":
          stream.handleStop()
          break
        case "compact":
          if (session.currentSessionId) {
            setCompacting(true)
            try {
              await invoke("compact_context_now", {
                sessionId: session.currentSessionId,
              })
            } catch (e) {
              logger.error("ui", "ChatScreen::slashCompact", "Compact failed", e)
            } finally {
              setCompacting(false)
            }
          }
          break
        case "sessionCleared":
          session.setMessages([])
          session.reloadSessions()
          break
        case "passThrough":
          // Send the message to the LLM as a normal user message
          stream.setInput(action.message)
          // Use a small delay so React can update the input before sending
          setTimeout(() => stream.handleSend(), 50)
          break
        case "exportFile":
          try {
            const filePath = await save({
              defaultPath: action.filename,
              filters: [{ name: "Markdown", extensions: ["md"] }],
            })
            if (filePath) {
              await invoke("write_export_file", {
                path: filePath,
                content: action.content,
              })
            }
          } catch (e) {
            logger.error("ui", "ChatScreen::slashExport", "Export failed", e)
          }
          break
        case "setToolPermission":
          stream.setToolPermissionMode(action.mode as import("@/types/chat").ToolPermissionMode)
          break
        case "displayOnly":
          // Already handled above by adding event message
          break
        case "showModelPicker": {
          const pickerMsg: Message = {
            role: "event",
            content: "",
            timestamp: new Date().toISOString(),
            modelPickerData: {
              models: action.models,
              activeProviderId: action.activeProviderId,
              activeModelId: action.activeModelId,
            },
          }
          session.setMessages((prev) => [...prev, pickerMsg])
          break
        }
        case "enterPlanMode":
          planMode.enterPlanMode()
          break
        case "exitPlanMode":
          planMode.exitPlanMode()
          break
        case "approvePlan":
          handlePlanApprove()
          break
        case "showPlan":
          planMode.setPlanContent(action.planContent)
          planMode.setShowPanel(true)
          break
        case "pausePlan":
          planMode.pauseExecution()
          break
        case "resumePlan":
          planMode.resumeExecution()
          planMode.setShowPanel(true)
          break
        case "viewSystemPrompt":
          loadSystemPrompt()
          break
      }
    },
    [session, stream, handleModelChange, handleEffortChange, compacting, planMode, loadSystemPrompt], // eslint-disable-line react-hooks/exhaustive-deps
  )

  // ── Plan Approve Handler ───────────────────────────────────────
  const handlePlanApprove = useCallback(
    async () => {
      await planMode.approvePlan()
      // Send a short trigger — the full plan is already in the system prompt (Executing state)
      stream.handleSend(t("planMode.executeCommand"))
    },
    [planMode, stream, t]
  )

  // ── Plan Request Changes Handler ──────────────────────────────
  const handleRequestChanges = useCallback(
    (feedback: string) => {
      // Send feedback back to LLM, which will revise the plan
      setPlanState("planning")
      if (currentSessionId) {
        invoke("set_plan_mode", { sessionId: currentSessionId, state: "planning" }).catch(() => {})
      }
      sendMessage(feedback)
    },
    [setPlanState, sendMessage, currentSessionId]
  )

  return (
    <>
      {/* Sidebar */}
      <ChatSidebar
        sessions={session.sessions}
        agents={session.agents}
        currentSessionId={session.currentSessionId}
        loadingSessionIds={session.loadingSessionIds}
        panelWidth={panelWidth}
        onPanelWidthChange={setPanelWidth}
        onSwitchSession={session.handleSwitchSession}
        onNewChat={session.handleNewChat}
        onDeleteSession={session.handleDeleteSession}
        onEditAgent={onOpenAgentSettings}
        onMarkAllRead={session.reloadSessions}
        onRenameSession={handleRenameSession}
        hasMoreSessions={session.hasMoreSessions}
        loadingMoreSessions={session.loadingMoreSessions}
        onLoadMoreSessions={session.handleLoadMoreSessions}
      />

      {/* Command Approval Dialog */}
      <ApprovalDialog
        requests={stream.approvalRequests}
        onRespond={stream.handleApprovalResponse}
      />

      {/* Codex Auth Expired Dialog */}
      <AlertDialog
        open={stream.showCodexAuthExpired}
        onOpenChange={stream.setShowCodexAuthExpired}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("codexAuth.expiredTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("codexAuth.expiredDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            {onCodexReauth && (
              <AlertDialogAction
                onClick={() => {
                  stream.setShowCodexAuthExpired(false)
                  onCodexReauth()
                }}
              >
                {t("codexAuth.reauth")}
              </AlertDialogAction>
            )}
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* System Prompt Viewer Dialog */}
      <SystemPromptDialog
        open={showSystemPrompt}
        onOpenChange={setShowSystemPrompt}
        content={systemPromptContent}
      />

      {/* Chat Area */}
      <div className="flex-1 flex flex-col min-w-0">
        <ChatTitleBar
          agentName={session.agentName}
          currentAgentId={session.currentAgentId}
          currentSessionId={session.currentSessionId}
          sessions={session.sessions}
          messages={session.messages}
          activeModel={activeModel}
          availableModels={availableModels}
          reasoningEffort={reasoningEffort}
          loading={session.loading}
          compacting={compacting}
          setCompacting={setCompacting}
          onOpenAgentSettings={onOpenAgentSettings}
          onRenameSession={handleRenameSession}
          onViewSystemPrompt={loadSystemPrompt}
          systemPromptLoading={systemPromptLoading}
        />

        <CrashRecoveryBanner />

        <MessageList
          messages={session.messages}
          loading={session.loading}
          agents={session.agents}
          hasMore={session.hasMore}
          loadingMore={session.loadingMore}
          onLoadMore={session.handleLoadMore}
          scrollContainerRef={scrollContainerRef}
          bottomRef={bottomRef}
          sessionId={session.currentSessionId}
          pendingQuestionGroup={planMode.pendingQuestionGroup}
          onQuestionSubmitted={() => { /* PlanQuestionBlock handles its own submitted state */ }}
          planCardData={planMode.planCardInfo ? {
            title: planMode.planCardInfo.title,
            steps: planMode.planSteps,
            sessionId: session.currentSessionId || "",
          } : null}
          planState={planMode.planState}
          planSteps={planMode.planSteps}
          onOpenPlanPanel={() => planMode.setShowPanel(true)}
          onApprovePlan={handlePlanApprove}
          onExitPlan={planMode.exitPlanMode}
          onPausePlan={planMode.pauseExecution}
          onResumePlan={planMode.resumeExecution}
          planSubagentRunning={planMode.planSubagentRunning}
          onSwitchModel={(providerId, modelId) => handleModelChange(`${providerId}::${modelId}`)}
        />

        {/* Memory extraction toast */}
        {!isCronSession && (
          <>
            {memoryToast && (
              <div className="flex items-center gap-2 mx-4 mb-2 px-3 py-1.5 rounded-lg bg-secondary/50 text-xs text-muted-foreground animate-in fade-in slide-in-from-bottom-2 duration-300">
                <Brain className="h-3.5 w-3.5 shrink-0" />
                <span>{t("settings.memoryExtractedToast", { count: memoryToast.count })}</span>
                <button onClick={() => setMemoryToast(null)} className="ml-auto text-muted-foreground/60 hover:text-muted-foreground">
                  ×
                </button>
              </div>
            )}

            <ChatInput
              input={stream.input}
              onInputChange={stream.setInput}
              onSend={() => stream.handleSend()}
              loading={session.loading}
              availableModels={availableModels}
              activeModel={activeModel}
              reasoningEffort={reasoningEffort}
              onModelChange={handleModelChange}
              onEffortChange={handleEffortChange}
              attachedFiles={stream.attachedFiles}
              onAttachFiles={(files) => stream.setAttachedFiles((prev) => [...prev, ...files])}
              onRemoveFile={(index) =>
                stream.setAttachedFiles((prev) => prev.filter((_, i) => i !== index))
              }
              pendingMessage={stream.pendingMessage}
              onCancelPending={() => {
                stream.setInput(stream.pendingMessage || "")
                stream.setPendingMessage(null)
              }}
              onDiscardPending={() => {
                stream.setPendingMessage(null)
              }}
              onStop={stream.handleStop}
              currentSessionId={session.currentSessionId}
              currentAgentId={session.currentAgentId}
              onCommandAction={handleCommandAction}
              toolPermissionMode={stream.toolPermissionMode}
              onToolPermissionChange={stream.setToolPermissionMode}
              sessionTemperature={sessionTemperature}
              onSessionTemperatureChange={setSessionTemperature}
              planState={planMode.planState}
              planProgress={planMode.progress}
              onEnterPlanMode={planMode.enterPlanMode}
              onExitPlanMode={planMode.exitPlanMode}
              onTogglePlanPanel={() => planMode.setShowPanel((p) => !p)}
            />
          </>
        )}
      </div>

      {/* Plan Panel (right side) */}
      {planMode.showPanel && planMode.planState !== "off" && (
        <>
          <div
            className="w-1 shrink-0 cursor-col-resize hover:bg-primary/30 active:bg-primary/50 transition-colors"
            onMouseDown={(e) => {
              e.preventDefault()
              const startX = e.clientX
              const startWidth = planPanelWidth
              const onMouseMove = (ev: MouseEvent) => {
                const newWidth = Math.min(800, Math.max(280, startWidth - (ev.clientX - startX)))
                setPlanPanelWidth(newWidth)
              }
              const iframes = document.querySelectorAll("iframe")
              iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = "none"))
              const onMouseUp = () => {
                document.removeEventListener("mousemove", onMouseMove)
                document.removeEventListener("mouseup", onMouseUp)
                document.body.style.cursor = ""
                document.body.style.userSelect = ""
                iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = ""))
              }
              document.addEventListener("mousemove", onMouseMove)
              document.addEventListener("mouseup", onMouseUp)
              document.body.style.cursor = "col-resize"
              document.body.style.userSelect = "none"
            }}
          />
          <PlanPanel
            planState={planMode.planState}
            planSteps={planMode.planSteps}
            planContent={planMode.planContent}
            progress={planMode.progress}
            completedCount={planMode.completedCount}
            sessionId={session.currentSessionId}
            onApprove={handlePlanApprove}
            onExit={planMode.exitPlanMode}
            onClose={() => planMode.setShowPanel(false)}
            onPause={planMode.pauseExecution}
            onResume={planMode.resumeExecution}
            onRequestChanges={handleRequestChanges}
            panelWidth={planPanelWidth}
          />
        </>
      )}

      {/* Canvas Preview Panel */}
      <CanvasPanel panelWidth={canvasPanelWidth} onPanelWidthChange={setCanvasPanelWidth} />
    </>
  )
}

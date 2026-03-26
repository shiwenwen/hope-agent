import { useState, useRef, useEffect, useCallback } from "react"
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
  const globalActiveModelRef = useRef<ActiveModel | null>(null)

  // Sidebar panel width
  const [panelWidth, setPanelWidth] = useState(256)

  // Context compact state
  const [compacting, setCompacting] = useState(false)

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

  // Rename session handler
  const handleRenameSession = useCallback(async (sessionId: string, title: string) => {
    try {
      await invoke("rename_session_cmd", { sessionId, title })
      session.reloadSessions()
    } catch (err) {
      console.error("Failed to rename session:", err)
    }
  }, [session.reloadSessions])

  // Reload sessions when external trigger changes (e.g. mark-all-read from IconSidebar)
  useEffect(() => {
    if (sessionsRefreshTrigger) {
      session.reloadSessions()
    }
  }, [sessionsRefreshTrigger])

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
  })

  // ── Plan Mode Hook ─────────────────────────────────────────
  const planMode = usePlanMode(session.currentSessionId)

  // Reload plan steps from backend when plan_content_updated event is received via chat stream
  // The event is emitted by the backend after detecting plan checklist format in LLM output
  const planContentUpdateRef = useRef(0)
  useEffect(() => {
    // Listen for plan_content_updated in the chat stream events
    const msgs = session.messages
    if (!session.currentSessionId || planMode.planState === "off") return
    // Find the latest assistant message and use it as plan content for the panel
    for (let i = msgs.length - 1; i >= 0; i--) {
      const msg = msgs[i]
      if (msg.role === "assistant" && msg.content) {
        // Only update when content actually changes
        const hash = msg.content.length
        if (hash !== planContentUpdateRef.current) {
          planContentUpdateRef.current = hash
          // Refresh steps from backend (backend already parsed and saved)
          invoke<import("./plan-mode/usePlanMode").PlanStep[]>("get_plan_steps", {
            sessionId: session.currentSessionId,
          }).then((steps) => {
            if (steps && steps.length > 0) {
              planMode.setPlanSteps(steps)
              planMode.setPlanContent(msg.content)
            }
          }).catch(() => {})
        }
        break
      }
    }
  }, [session.messages.length, planMode.planState, session.currentSessionId]) // eslint-disable-line react-hooks/exhaustive-deps

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

  // ── Slash Command Action Handler ──────────────────────────────
  const handleCommandAction = useCallback(
    async (result: CommandResult) => {
      const action = result.action

      // Show command output as an event message (if content is not empty)
      if (result.content) {
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
          session.handleSwitchSession(action.sessionId)
          break
        case "switchModel":
          handleModelChange(`${action.providerId}::${action.modelId}`)
          break
        case "setEffort":
          handleEffortChange(action.effort)
          break
        case "switchAgent":
          session.handleSwitchSession(action.sessionId)
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
        case "enterPlanMode":
          planMode.enterPlanMode()
          planMode.setShowPanel(true)
          break
        case "exitPlanMode":
          planMode.exitPlanMode()
          break
        case "approvePlan":
          handlePlanApprove(action.planContent)
          break
        case "showPlan":
          planMode.setPlanContent(action.planContent)
          planMode.setShowPanel(true)
          break
      }
    },
    [session, stream, handleModelChange, handleEffortChange, compacting, planMode], // eslint-disable-line react-hooks/exhaustive-deps
  )

  // ── Plan Approve Handler ───────────────────────────────────────
  const handlePlanApprove = useCallback(
    async (planContentOverride?: string | null) => {
      await planMode.approvePlan()
      const content = planContentOverride || planMode.planContent
      if (content) {
        stream.setInput(
          `请按照以下计划逐步执行，每完成一步调用 update_plan_step 工具更新进度：\n\n${content}`
        )
        setTimeout(() => stream.handleSend(), 50)
      }
    },
    [planMode, stream]
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
        />

        {/* Memory extraction toast */}
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
          onSend={stream.handleSend}
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
          onStop={stream.handleStop}
          currentSessionId={session.currentSessionId}
          currentAgentId={session.currentAgentId}
          onCommandAction={handleCommandAction}
          toolPermissionMode={stream.toolPermissionMode}
          onToolPermissionChange={stream.setToolPermissionMode}
          planState={planMode.planState}
          planProgress={planMode.progress}
          onEnterPlanMode={async () => {
            // If no session exists, create one first then enter plan mode
            if (!session.currentSessionId) {
              try {
                const meta = await invoke<{ id: string }>("create_session_cmd", {
                  agentId: session.currentAgentId,
                })
                session.handleSwitchSession(meta.id)
                // Small delay to allow session state to update
                setTimeout(async () => {
                  await invoke("set_plan_mode", { sessionId: meta.id, state: "planning" })
                  planMode.setPlanState("planning")
                  planMode.setShowPanel(true)
                }, 50)
              } catch (e) {
                console.error("Failed to create session for plan mode:", e)
              }
            } else {
              await planMode.enterPlanMode()
              planMode.setShowPanel(true)
            }
          }}
          onExitPlanMode={planMode.exitPlanMode}
          onTogglePlanPanel={() => planMode.setShowPanel((p) => !p)}
        />
      </div>

      {/* Plan Panel (right side) */}
      {planMode.showPanel && planMode.planState !== "off" && (
        <PlanPanel
          planState={planMode.planState}
          planSteps={planMode.planSteps}
          planContent={planMode.planContent}
          progress={planMode.progress}
          completedCount={planMode.completedCount}
          onApprove={() => handlePlanApprove()}
          onKeepPlanning={() => planMode.setShowPanel(false)}
          onExit={planMode.exitPlanMode}
          onClose={() => planMode.setShowPanel(false)}
        />
      )}

      {/* Canvas Preview Panel */}
      <CanvasPanel />
    </>
  )
}

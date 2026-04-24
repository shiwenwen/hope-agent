import { useState, useRef, useEffect, useCallback, useMemo } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { save } from "@tauri-apps/plugin-dialog"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Brain } from "lucide-react"
import type { ActiveModel, AvailableModel, Message, ToolPermissionMode } from "@/types/chat"
import { normalizeEffortForModel } from "@/types/chat"
import type { CommandResult } from "./slash-commands/types"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import ChatSidebar from "@/components/chat/ChatSidebar"
import ChatInput from "@/components/chat/ChatInput"
import type { IncognitoDisabledReason } from "@/components/chat/input/IncognitoToggle"
import ChatTitleBar from "@/components/chat/ChatTitleBar"
import MessageList from "@/components/chat/MessageList"
import CrashRecoveryBanner from "@/components/common/CrashRecoveryBanner"
import CanvasPanel from "@/components/chat/CanvasPanel"
import { TeamPanel } from "@/components/team/TeamPanel"
import TeamMiniIndicator from "@/components/team/TeamMiniIndicator"
import { useActiveTeam } from "@/components/team/useTeam"
import SessionSearchBar from "@/components/chat/SessionSearchBar"
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
import { useChatStreamReattach } from "./hooks/useChatStreamReattach"
import { useAutoScroll } from "./useAutoScroll"
import { usePlanMode } from "./plan-mode/usePlanMode"
import { useModelState } from "./hooks/useModelState"
import SystemPromptDialog from "./SystemPromptDialog"
import { PlanPanel } from "./plan-mode/PlanPanel"
import { useProjects } from "./project/hooks/useProjects"
import ProjectDialog from "./project/ProjectDialog"
import ProjectOverviewDialog from "./project/ProjectOverviewDialog"
import type { Project, ProjectMeta } from "@/types/project"

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
  const {
    availableModels,
    setAvailableModels,
    activeModel,
    setActiveModel,
    reasoningEffort,
    setReasoningEffort,
    sessionTemperature,
    setSessionTemperature,
    globalActiveModelRef,
    applyModelForDisplay,
    handleModelChange,
    handleEffortChange,
  } = useModelState()

  // Sidebar panel width
  const [panelWidth, setPanelWidth] = useState(288)

  // Right panel widths (resizable)
  const [planPanelWidth, setPlanPanelWidth] = useState(400)
  const [canvasPanelWidth, setCanvasPanelWidth] = useState(480)

  // Context compact state
  const [compacting, setCompacting] = useState(false)

  // In-session "find in page" search bar state
  const [searchBarOpen, setSearchBarOpen] = useState(false)
  const [searchFocusSignal, setSearchFocusSignal] = useState(0)

  // System prompt viewer state
  const [showSystemPrompt, setShowSystemPrompt] = useState(false)
  const [systemPromptContent, setSystemPromptContent] = useState("")
  const [systemPromptLoading, setSystemPromptLoading] = useState(false)
  const [draftIncognito, setDraftIncognito] = useState(false)
  const [incognitoSaving, setIncognitoSaving] = useState(false)
  const [workingDirSaving, setWorkingDirSaving] = useState(false)

  // Plan mode state (declared early so useChatStream can access it)
  const [planModeState, setPlanModeState] = useState<
    "off" | "planning" | "review" | "executing" | "paused" | "completed"
  >("off")

  // Shared per-session seq cursor for chat stream dedup across the primary
  // per-call Channel/WS path (useChatStream) and the EventBus reattach path
  // (useChatStreamReattach). Owned at this level to break the two-way
  // dependency between the two hooks.
  const streamSeqRef = useRef<Map<string, number>>(new Map())
  const manualModelOverrideRef = useRef<ActiveModel | null>(null)

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
  const currentSessionMeta = useMemo(
    () =>
      session.currentSessionId
        ? (session.sessions.find((s) => s.id === session.currentSessionId) ?? null)
        : null,
    [session.sessions, session.currentSessionId],
  )
  const incognitoEnabled = session.currentSessionId
    ? (currentSessionMeta?.incognito ?? false)
    : draftIncognito
  const incognitoDisabledReason: IncognitoDisabledReason | undefined = currentSessionMeta?.projectId
    ? "project"
    : currentSessionMeta?.channelInfo
      ? "channel"
      : undefined
  const reloadSessions = session.reloadSessions
  const currentAgentId = session.currentAgentId
  const handleNewChat = session.handleNewChat
  const handleNewChatInProject = session.handleNewChatInProject
  const currentSessionId = session.currentSessionId

  // ── Team ──────────────────────────────────────────────────
  const activeTeamId = useActiveTeam(currentSessionId ?? null)
  const [showTeamPanel, setShowTeamPanel] = useState(false)
  const [teamPanelWidth, setTeamPanelWidth] = useState(420)

  const refreshRuntimeModelState = useCallback(async () => {
    try {
      const [models, active, settings, agentConfig] = await Promise.all([
        getTransport().call<AvailableModel[]>("get_available_models"),
        getTransport().call<ActiveModel | null>("get_active_model"),
        getTransport().call<{ model: string; reasoning_effort: string }>("get_current_settings"),
        getTransport()
          .call<{
            name: string
            model?: { primary?: string | null }
            emoji?: string | null
            avatar?: string | null
          }>("get_agent_config", { id: session.currentAgentId })
          .catch(() => null),
      ])

      setAvailableModels(models)
      globalActiveModelRef.current = active

      let displayModel = active
      const manualOverride = manualModelOverrideRef.current
      const manualModel = manualOverride
        ? models.find(
            (m) =>
              m.providerId === manualOverride.providerId && m.modelId === manualOverride.modelId,
          )
        : undefined
      if (manualOverride && !manualModel) {
        manualModelOverrideRef.current = null
      }

      if (manualModel && manualOverride) {
        displayModel = manualOverride
      } else if (currentSessionMeta?.providerId && currentSessionMeta?.modelId) {
        const sessionModel = models.find(
          (m) =>
            m.providerId === currentSessionMeta.providerId &&
            m.modelId === currentSessionMeta.modelId,
        )
        if (sessionModel) {
          displayModel = {
            providerId: sessionModel.providerId,
            modelId: sessionModel.modelId,
          }
        }
      } else if (agentConfig?.model?.primary) {
        const [providerId, modelId] = agentConfig.model.primary.split("::")
        const agentModel = models.find((m) => m.providerId === providerId && m.modelId === modelId)
        if (agentModel) {
          displayModel = { providerId, modelId }
        }
      }

      setActiveModel(displayModel)
      const displayModelInfo = displayModel
        ? models.find(
            (m) => m.providerId === displayModel.providerId && m.modelId === displayModel.modelId,
          )
        : undefined
      setReasoningEffort(normalizeEffortForModel(displayModelInfo, settings.reasoning_effort, t))

      if (agentConfig?.name) {
        session.setAgentName(agentConfig.name)
      }
    } catch (e) {
      logger.error("ui", "ChatScreen::refreshRuntimeModelState", "Failed to refresh model state", e)
    }
  }, [
    currentSessionMeta?.modelId,
    currentSessionMeta?.providerId,
    globalActiveModelRef,
    session,
    setActiveModel,
    setAvailableModels,
    setReasoningEffort,
    t,
  ])

  const handleManualModelChange = useCallback(
    async (key: string) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      manualModelOverrideRef.current = { providerId, modelId }
      await handleModelChange(key)
    },
    [handleModelChange],
  )

  // Auto-show team panel when a team is created
  useEffect(() => {
    if (activeTeamId) setShowTeamPanel(true)
  }, [activeTeamId])

  useEffect(() => {
    manualModelOverrideRef.current = null
  }, [currentSessionId, currentAgentId])

  // ── Projects ────────────────────────────────────────────────
  const {
    projects,
    createProject,
    updateProject,
    deleteProject,
    archiveProject,
    moveSessionToProject,
  } = useProjects()

  // Wrap moveSessionToProject so the sidebar also reloads — otherwise the
  // moved session keeps rendering under the old "Unassigned" group until
  // the user manually refreshes.
  const handleMoveSessionToProject = useCallback(
    async (sessionId: string, projectId: string | null) => {
      await moveSessionToProject(sessionId, projectId)
      await reloadSessions()
    },
    [moveSessionToProject, reloadSessions],
  )

  const [projectDialogOpen, setProjectDialogOpen] = useState(false)
  const [projectDialogMode, setProjectDialogMode] = useState<"create" | "edit">("create")
  const [projectDialogInitial, setProjectDialogInitial] = useState<Project | null>(null)

  const [projectOverviewOpen, setProjectOverviewOpen] = useState(false)
  const [projectOverviewTargetId, setProjectOverviewTargetId] = useState<string | null>(null)
  // Derive the live target from the projects list so mutations (rename,
  // archive, file upload) are reflected immediately in the open dialog.
  const projectOverviewTarget = useMemo(
    () =>
      projectOverviewTargetId
        ? (projects.find((p) => p.id === projectOverviewTargetId) ?? null)
        : null,
    [projects, projectOverviewTargetId],
  )

  const [projectDeleteTarget, setProjectDeleteTarget] = useState<Project | null>(null)

  const openCreateProject = useCallback(() => {
    setProjectDialogMode("create")
    setProjectDialogInitial(null)
    setProjectDialogOpen(true)
  }, [])

  const openEditProject = useCallback((project: Project) => {
    setProjectDialogMode("edit")
    setProjectDialogInitial(project)
    setProjectDialogOpen(true)
  }, [])

  const openProjectOverview = useCallback((project: ProjectMeta) => {
    setProjectOverviewTargetId(project.id)
    setProjectOverviewOpen(true)
  }, [])

  const [deletingProject, setDeletingProject] = useState(false)

  const confirmDeleteProject = useCallback(async () => {
    if (!projectDeleteTarget || deletingProject) return
    const projectName = projectDeleteTarget.name
    setDeletingProject(true)
    try {
      const ok = await deleteProject(projectDeleteTarget.id)
      setProjectDeleteTarget(null)
      if (ok) {
        setProjectOverviewOpen(false)
        reloadSessions()
        toast.success(t("common.deleted"), {
          description: projectName,
        })
      } else {
        toast.error(t("common.deleteFailed"), {
          description: projectName,
        })
      }
    } catch {
      toast.error(t("common.deleteFailed"), {
        description: projectName,
      })
    } finally {
      setDeletingProject(false)
    }
  }, [deleteProject, projectDeleteTarget, deletingProject, reloadSessions, t])

  // Rename session handler
  const handleRenameSession = useCallback(
    async (sessionId: string, title: string) => {
      try {
        await getTransport().call("rename_session_cmd", { sessionId, title })
        reloadSessions()
      } catch (err) {
        logger.error("chat", "ChatScreen::renameSession", "Failed to rename session", err)
      }
    },
    [reloadSessions],
  )

  const handleIncognitoChange = useCallback(
    async (enabled: boolean) => {
      const sid = session.currentSessionId
      if (!sid) {
        setDraftIncognito(enabled)
        return
      }

      // Project / Channel sessions can never be incognito; the toggle is
      // disabled in the UI but a stale prop or external trigger could still
      // call us. Reject before hitting the backend.
      if (enabled && incognitoDisabledReason !== undefined) return

      const previous = currentSessionMeta?.incognito ?? false
      if (previous === enabled) return
      session.updateSessionMeta(sid, (prev) =>
        prev.incognito === enabled ? prev : { ...prev, incognito: enabled },
      )
      setIncognitoSaving(true)
      try {
        await getTransport().call("set_session_incognito", {
          sessionId: sid,
          enabled,
        })
      } catch (err) {
        session.updateSessionMeta(sid, (prev) =>
          prev.incognito === previous ? prev : { ...prev, incognito: previous },
        )
        logger.error("chat", "ChatScreen::setIncognito", "Failed to update incognito mode", err)
      } finally {
        setIncognitoSaving(false)
      }
    },
    [session, currentSessionMeta?.incognito, incognitoDisabledReason],
  )

  const handleWorkingDirChange = useCallback(
    async (workingDir: string | null) => {
      const sid = session.currentSessionId
      if (!sid) return
      const previous = currentSessionMeta?.workingDir ?? null
      if (previous === workingDir) return
      session.updateSessionMeta(sid, (prev) =>
        prev.workingDir === workingDir ? prev : { ...prev, workingDir },
      )
      setWorkingDirSaving(true)
      try {
        const result = await getTransport().call<{
          updated: boolean
          workingDir: string | null
        } | null>("set_session_working_dir", {
          sessionId: sid,
          workingDir,
        })
        // Backend returns the canonical path; sync if it differs from the
        // user-typed form (e.g. trailing slash stripped, symlinks resolved).
        const canonical = result?.workingDir ?? null
        if (canonical !== workingDir) {
          session.updateSessionMeta(sid, (prev) =>
            prev.workingDir === canonical ? prev : { ...prev, workingDir: canonical },
          )
        }
      } catch (err) {
        session.updateSessionMeta(sid, (prev) =>
          prev.workingDir === previous ? prev : { ...prev, workingDir: previous },
        )
        logger.error(
          "chat",
          "ChatScreen::setWorkingDir",
          "Failed to update working directory",
          err,
        )
        toast.error(t("chat.workingDir.invalid"), {
          description: err instanceof Error ? err.message : String(err),
        })
      } finally {
        setWorkingDirSaving(false)
      }
    },
    [session, currentSessionMeta?.workingDir, t],
  )

  // Reload sessions when external trigger changes (e.g. mark-all-read from IconSidebar)
  useEffect(() => {
    if (sessionsRefreshTrigger) {
      reloadSessions()
    }
  }, [sessionsRefreshTrigger, reloadSessions])

  // Close the in-session search bar whenever the active session changes.
  useEffect(() => {
    setSearchBarOpen(false)
  }, [currentSessionId])

  // Cmd/Ctrl+F: open in-session search bar (or re-focus its input if
  // already open). Only active when a session is loaded.
  useEffect(() => {
    if (!currentSessionId) return
    const handler = (e: KeyboardEvent) => {
      const isFindKey =
        (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "f"
      if (!isFindKey) return
      // Don't hijack the shortcut if the user is editing inside an
      // unrelated contenteditable (e.g. a markdown canvas field). Free
      // inputs (ChatInput textarea etc.) are fine to preempt since there
      // is no browser find-in-page equivalent for chat history anyway.
      const target = e.target as HTMLElement | null
      if (target?.isContentEditable) return
      e.preventDefault()
      // Open (or keep open) and bump the focus signal so the search bar
      // re-focuses its input even if Cmd+F is pressed while it's already
      // visible.
      setSearchBarOpen(true)
      setSearchFocusSignal((n) => n + 1)
    }
    window.addEventListener("keydown", handler)
    return () => window.removeEventListener("keydown", handler)
  }, [currentSessionId])

  // Listen for tray "new-session" event to trigger new chat
  useEffect(() => {
    return getTransport().listen("new-session", () => {
      handleNewChat(currentAgentId)
    })
  }, [handleNewChat, currentAgentId])

  // Listen for channel slash command state-sync events
  useEffect(() => {
    const unlisteners: Array<() => void> = []

    // Model switched from channel (/model)
    unlisteners.push(
      getTransport().listen("slash:model_switched", (payload) => {
        const { providerId, modelId } = payload as {
          providerId: string
          modelId: string
        }
        manualModelOverrideRef.current = { providerId, modelId }
        applyModelForDisplay(`${providerId}::${modelId}`)
      }),
    )

    // Effort changed from channel (/think)
    unlisteners.push(
      getTransport().listen("slash:effort_changed", (payload) => {
        setReasoningEffort(payload as string)
      }),
    )

    // Session cleared from channel (/clear)
    unlisteners.push(
      getTransport().listen("slash:session_cleared", (payload) => {
        const clearedSid = payload as string
        if (clearedSid === session.currentSessionId) {
          session.setMessages([])
        }
        session.reloadSessions()
      }),
    )

    // Plan state changed from channel (/plan)
    unlisteners.push(
      getTransport().listen("slash:plan_changed", () => {
        session.reloadSessions()
      }),
    )

    return () => {
      unlisteners.forEach((fn) => fn())
    }
  }, [session.currentSessionId]) // eslint-disable-line react-hooks/exhaustive-deps

  // Fetch models and current settings on mount
  useEffect(() => {
    void refreshRuntimeModelState()
  }, [refreshRuntimeModelState])

  useEffect(() => {
    const offConfig = getTransport().listen("config:changed", () => {
      void refreshRuntimeModelState()
    })
    const offAgents = getTransport().listen("agents:changed", () => {
      void refreshRuntimeModelState()
    })
    const onWindowAgentsChanged = () => {
      void refreshRuntimeModelState()
    }
    window.addEventListener("agents-changed", onWindowAgentsChanged)
    return () => {
      offConfig()
      offAgents()
      window.removeEventListener("agents-changed", onWindowAgentsChanged)
    }
  }, [refreshRuntimeModelState])

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
    lastSeqRef: streamSeqRef,
    planMode: planModeState,
    temperatureOverride: sessionTemperature,
    incognitoEnabled,
  })

  // Restore the per-session tool permission toggle on session switch. The ref
  // guards against re-applying when `sessions` later reloads with the same
  // sid — that would clobber the user's in-session edits.
  //
  // When `sid` becomes null (new-chat transition), also reset to "auto" so the
  // toggle doesn't carry the previous session's `full_approve` / `ask_every_time`
  // into a fresh chat — otherwise the first tool call would silently fall back
  // once `session_created` arrives and the DB default ("auto") is restored.
  const restoredTpmForSidRef = useRef<string | null>(null)
  useEffect(() => {
    const sid = session.currentSessionId
    if (!sid) {
      if (restoredTpmForSidRef.current !== null) {
        stream.setToolPermissionMode("auto")
      }
      restoredTpmForSidRef.current = null
      return
    }
    if (restoredTpmForSidRef.current === sid) return
    const meta = session.sessions.find((s) => s.id === sid)
    if (!meta) return // wait until the sessions list has the meta
    const mode: ToolPermissionMode =
      (meta.toolPermissionMode as ToolPermissionMode | undefined) ?? "auto"
    restoredTpmForSidRef.current = sid
    stream.setToolPermissionMode(mode)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.currentSessionId, session.sessions, stream.setToolPermissionMode])

  // ── Stream Reattach Hook ────────────────────────────────────
  // Rehydrates chat streaming after frontend reload / window reopen / browser
  // refresh via EventBus-backed events, deduplicated against `streamSeqRef`
  // which the primary per-call Channel path also updates.
  useChatStreamReattach({
    currentSessionId: session.currentSessionId,
    currentSessionIdRef: session.currentSessionIdRef,
    lastSeqRef: streamSeqRef,
    updateSessionMessages: session.updateSessionMessages,
    setShowCodexAuthExpired: stream.setShowCodexAuthExpired,
    setMessages: session.setMessages,
    setLoading: session.setLoading,
    loadingSessionsRef: session.loadingSessionsRef,
    setLoadingSessionIds: session.setLoadingSessionIds,
    sessionCacheRef: session.sessionCacheRef,
    reloadSessions: session.reloadSessions,
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
    const unlisten = getTransport().listen("memory_extracted", (raw) => {
      const { count, sessionId } = raw as { count: number; sessionId: string }
      // Only show toast for the current session
      if (sessionId === session.currentSessionId && count > 0) {
        setMemoryToast({ count })
        if (memoryToastTimer.current) clearTimeout(memoryToastTimer.current)
        memoryToastTimer.current = setTimeout(() => setMemoryToast(null), 4000)
      }
    })
    return () => {
      unlisten()
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
      const prompt = await getTransport().call<string>("get_system_prompt", {
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

      // Skip the event chip for newSession (we clear anyway) and skill passThrough
      // (the user bubble already shows "/skillname args", the chip would duplicate).
      if (result.content && action?.type !== "newSession" && !result._isSkillPassThrough) {
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
            getTransport()
              .call("delete_session_cmd", { sessionId: action.sessionId })
              .then(() => session.reloadSessions())
              .catch(() => {})
          }
          break
        case "switchModel":
          handleManualModelChange(`${action.providerId}::${action.modelId}`)
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
              await getTransport().call("compact_context_now", {
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
          if (result._isSkillPassThrough) {
            // User bubble shows "/skillname args"; LLM gets the expanded prompt.
            await stream.handleSend(action.message, {
              displayText: result._skillCommandText,
            })
          } else {
            stream.setInput(action.message)
            setTimeout(() => stream.handleSend(), 50)
          }
          break
        case "exportFile":
          try {
            const filePath = await save({
              defaultPath: action.filename,
              filters: [{ name: "Markdown", extensions: ["md"] }],
            })
            if (filePath) {
              await getTransport().call("write_export_file", {
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
          await planMode.approvePlan()
          stream.handleSend(t("planMode.executeCommand"))
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
        case "showContextBreakdown": {
          const contextMsg: Message = {
            role: "event",
            content: "",
            timestamp: new Date().toISOString(),
            contextBreakdownData: action.breakdown,
          }
          session.setMessages((prev) => [...prev, contextMsg])
          break
        }
      }
    },
    [session, stream, handleManualModelChange, handleEffortChange, planMode, loadSystemPrompt, t],
  )

  // ── Plan Approve Handler ───────────────────────────────────────
  const handlePlanApprove = useCallback(async () => {
    await planMode.approvePlan()
    // Send a short trigger — the full plan is already in the system prompt (Executing state)
    stream.handleSend(t("planMode.executeCommand"))
  }, [planMode, stream, t])

  // ── Plan Request Changes Handler ──────────────────────────────
  const handleRequestChanges = useCallback(
    (feedback: string) => {
      // Send feedback back to LLM, which will revise the plan
      setPlanState("planning")
      if (currentSessionId) {
        getTransport()
          .call("set_plan_mode", { sessionId: currentSessionId, state: "planning" })
          .catch(() => {})
      }
      sendMessage(feedback)
    },
    [setPlanState, sendMessage, currentSessionId],
  )

  return (
    <>
      {/* Sidebar */}
      <ChatSidebar
        sessions={session.sessions}
        agents={session.agents}
        projects={projects}
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
        onOpenProject={openProjectOverview}
        onAddProject={openCreateProject}
        onMoveSessionToProject={handleMoveSessionToProject}
      />

      {/* Project create/edit dialog */}
      <ProjectDialog
        open={projectDialogOpen}
        mode={projectDialogMode}
        initialProject={projectDialogInitial}
        agents={session.agents}
        onOpenChange={setProjectDialogOpen}
        onCreate={createProject}
        onUpdate={updateProject}
      />

      {/* Project overview dialog (tabs: overview/sessions/files/instructions) */}
      <ProjectOverviewDialog
        open={projectOverviewOpen}
        project={projectOverviewTarget}
        onOpenChange={setProjectOverviewOpen}
        onEdit={(p) => {
          setProjectOverviewOpen(false)
          openEditProject(p)
        }}
        onDelete={(p) => setProjectDeleteTarget(p)}
        onArchive={async (p, archived) => {
          await archiveProject(p.id, archived)
          // Close the dialog since archived projects vanish from the sidebar
          if (archived) setProjectOverviewOpen(false)
        }}
        onNewSessionInProject={(projectId, defaultAgentId) => {
          void handleNewChatInProject(projectId, defaultAgentId, draftIncognito)
        }}
        onOpenSession={(sid) => session.handleSwitchSession(sid)}
        onUpdateProject={updateProject}
      />

      {/* Project delete confirmation */}
      <AlertDialog
        open={!!projectDeleteTarget}
        onOpenChange={(o) => !o && setProjectDeleteTarget(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("project.deleteConfirm.title")}</AlertDialogTitle>
            <AlertDialogDescription>{t("project.deleteConfirm.body")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={confirmDeleteProject}
              disabled={deletingProject}
            >
              {deletingProject ? t("common.saving") : t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Command Approval Dialog */}
      <ApprovalDialog
        requests={stream.approvalRequests}
        onRespond={stream.handleApprovalResponse}
      />

      {/* Codex Auth Expired Dialog */}
      <AlertDialog open={stream.showCodexAuthExpired} onOpenChange={stream.setShowCodexAuthExpired}>
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
          onCommandAction={handleCommandAction}
          onToggleSearch={() => {
            setSearchBarOpen((v) => !v)
            setSearchFocusSignal((n) => n + 1)
          }}
          searchOpen={searchBarOpen}
        />

        {activeTeamId && !showTeamPanel && (
          <div className="px-3 py-1 border-b border-border">
            <TeamMiniIndicator teamId={activeTeamId} onClick={() => setShowTeamPanel(true)} />
          </div>
        )}

        {searchBarOpen && session.currentSessionId && (
          <SessionSearchBar
            sessionId={session.currentSessionId}
            onJumpTo={session.jumpToMessage}
            onClose={() => setSearchBarOpen(false)}
            focusSignal={searchFocusSignal}
          />
        )}

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
          pendingScrollTarget={session.pendingScrollTarget}
          onScrollTargetHandled={session.clearPendingScrollTarget}
          pendingQuestionGroup={planMode.pendingQuestionGroup}
          onQuestionSubmitted={() => planMode.setPendingQuestionGroup(null)}
          planCardData={
            planMode.planCardInfo
              ? {
                  title: planMode.planCardInfo.title,
                  steps: planMode.planSteps,
                  sessionId: session.currentSessionId || "",
                }
              : null
          }
          planState={planMode.planState}
          planSteps={planMode.planSteps}
          onOpenPlanPanel={() => planMode.setShowPanel(true)}
          onApprovePlan={handlePlanApprove}
          onExitPlan={planMode.exitPlanMode}
          onPausePlan={planMode.pauseExecution}
          onResumePlan={planMode.resumeExecution}
          planSubagentRunning={planMode.planSubagentRunning}
          onSwitchModel={(providerId, modelId) =>
            handleManualModelChange(`${providerId}::${modelId}`)
          }
          onViewSystemPrompt={loadSystemPrompt}
          onSwitchSession={(sid) => {
            void session.handleSwitchSession(sid)
          }}
        />

        {/* Memory extraction toast */}
        {!isCronSession && (
          <>
            {memoryToast && (
              <div className="flex items-center gap-2 mx-4 mb-2 px-3 py-1.5 rounded-lg bg-secondary/50 text-xs text-muted-foreground animate-in fade-in slide-in-from-bottom-2 duration-300">
                <Brain className="h-3.5 w-3.5 shrink-0" />
                <span>{t("settings.memoryExtractedToast", { count: memoryToast.count })}</span>
                <button
                  onClick={() => setMemoryToast(null)}
                  className="ml-auto text-muted-foreground/60 hover:text-muted-foreground"
                >
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
              incognitoEnabled={incognitoEnabled}
              incognitoSaving={incognitoSaving}
              incognitoDisabledReason={incognitoDisabledReason}
              onIncognitoChange={handleIncognitoChange}
              workingDir={currentSessionMeta?.workingDir ?? null}
              workingDirSaving={workingDirSaving}
              onWorkingDirChange={
                session.currentSessionId ? handleWorkingDirChange : undefined
              }
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
      <CanvasPanel
        panelWidth={canvasPanelWidth}
        onPanelWidthChange={setCanvasPanelWidth}
        currentSessionId={currentSessionId}
      />

      {/* Team Panel */}
      {activeTeamId && showTeamPanel && (
        <TeamPanel
          teamId={activeTeamId}
          panelWidth={teamPanelWidth}
          onPanelWidthChange={setTeamPanelWidth}
          onClose={() => setShowTeamPanel(false)}
          onSwitchSession={session.handleSwitchSession}
        />
      )}
    </>
  )
}

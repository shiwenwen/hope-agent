import { useState, useRef, useEffect, useCallback, useMemo } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { save } from "@tauri-apps/plugin-dialog"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Brain } from "lucide-react"
import type { ActiveModel, AvailableModel, Message, SessionMode } from "@/types/chat"
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
import { usePlanMode } from "./plan-mode/usePlanMode"
import { useTaskProgressSnapshot } from "./tasks/useTaskProgressSnapshot"
import { useDiffPanel } from "./diff-panel/useDiffPanel"
import { DiffPanel } from "./diff-panel/DiffPanel"
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
  const [planPanelWidth, setPlanPanelWidth] = useState(520)
  const [canvasPanelWidth, setCanvasPanelWidth] = useState(480)

  // Right side diff panel (write/edit/apply_patch metadata viewer)
  const diffPanel = useDiffPanel()

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
  // Draft working dir picked before a session exists. Materialized into the new
  // session by the backend `chat` command on first send, then cleared via the
  // `currentSessionId` transition effect below.
  const [draftWorkingDir, setDraftWorkingDir] = useState<string | null>(null)
  const [workingDirSaving, setWorkingDirSaving] = useState(false)

  // Plan mode state (declared early so useChatStream can access it)
  const [planModeState, setPlanModeState] = useState<
    "off" | "planning" | "review" | "executing" | "completed"
  >("off")

  // Shared stream identity state for dedup across the primary per-call
  // Channel/WS path (useChatStream) and the EventBus reattach path
  // (useChatStreamReattach). Cursors are keyed by session + stream id so a
  // delayed frame from a finished stream cannot mutate the next DB snapshot.
  const streamSeqRef = useRef<Map<string, number>>(new Map())
  const endedStreamIdsRef = useRef<Map<string, string>>(new Map())
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
  const isSubagentSession = useMemo(
    () => !!session.sessions.find((s) => s.id === session.currentSessionId)?.parentSessionId,
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
  const setAgentName = session.setAgentName

  const handleStartNewChat = useCallback(
    async (agentId: string, opts?: { incognito?: boolean }) => {
      setDraftIncognito(opts?.incognito ?? false)
      await handleNewChat(agentId)
    },
    [handleNewChat],
  )

  /**
   * Title-bar agent switch handler. Backend rejects the switch when the
   * session already has user/assistant messages (defense layer); the UI
   * additionally hides the dropdown via `disabled` once messages exist, so
   * we only really get called for empty sessions.
   *
   * Branches:
   *  - Existing session (already materialized) → call backend so the change
   *    is persisted across reloads.
   *  - Draft session (no `currentSessionId` yet) → just update front-end
   *    state; the agent_id is baked in when the first message materializes
   *    the session.
   */
  const handleChangeAgent = useCallback(
    async (agentId: string) => {
      if (!agentId || agentId === session.currentAgentId) return
      const transport = getTransport()
      try {
        if (session.currentSessionId) {
          await transport.call("update_session_agent_cmd", {
            sessionId: session.currentSessionId,
            agentId,
          })
        }
        const agent = session.agents.find((a) => a.id === agentId)
        session.setCurrentAgentId(agentId)
        if (agent) session.setAgentName(agent.name)
        // Apply the new agent's preferred model (best-effort).
        try {
          const cfg = await transport.call<{
            model?: { primary?: string | null }
          }>("get_agent_config", { id: agentId })
          const primary = cfg.model?.primary
          if (primary) {
            const exists = availableModels.some((m) => `${m.providerId}::${m.modelId}` === primary)
            if (exists) {
              const [providerId, modelId] = primary.split("::")
              if (providerId && modelId) {
                setActiveModel({ providerId, modelId })
              }
            }
          }
        } catch {
          /* ignore */
        }
        await session.reloadSessions()
      } catch (err) {
        logger.warn("chat", "ChatScreen::handleChangeAgent", "failed", err)
      }
    },
    [session, availableModels, setActiveModel],
  )

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
          }>("get_agent_config", { id: currentAgentId })
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
        setAgentName(agentConfig.name)
      }
    } catch (e) {
      logger.error("ui", "ChatScreen::refreshRuntimeModelState", "Failed to refresh model state", e)
    }
  }, [
    currentSessionMeta?.modelId,
    currentSessionMeta?.providerId,
    currentAgentId,
    globalActiveModelRef,
    setActiveModel,
    setAgentName,
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

  const sessionWorkingDir = currentSessionMeta?.workingDir ?? null
  const projectWorkingDir = useMemo(
    () =>
      currentSessionMeta?.projectId
        ? (projects.find((p) => p.id === currentSessionMeta.projectId)?.workingDir ?? null)
        : null,
    [projects, currentSessionMeta?.projectId],
  )
  const effectiveWorkingDir = sessionWorkingDir ?? projectWorkingDir
  const workingDirSource: "session" | "project" | undefined = sessionWorkingDir
    ? "session"
    : projectWorkingDir
      ? "project"
      : undefined

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
    (enabled: boolean) => {
      if (session.currentSessionId) return
      setDraftIncognito(enabled)
    },
    [session.currentSessionId],
  )

  const handleWorkingDirChange = useCallback(
    async (workingDir: string | null) => {
      const sid = session.currentSessionId
      // No session yet — stash the choice. The backend `chat` command applies
      // it on the auto-create branch when the first message ships.
      if (!sid) {
        setDraftWorkingDir(workingDir)
        return
      }
      const previous = currentSessionMeta?.workingDir ?? null
      if (previous === workingDir) return
      session.updateSessionMeta(sid, (prev) =>
        prev.workingDir === workingDir ? prev : { ...prev, workingDir },
      )
      setWorkingDirSaving(true)
      try {
        await getTransport().call("set_session_working_dir", {
          sessionId: sid,
          workingDir,
        })
      } catch (err) {
        session.updateSessionMeta(sid, (prev) =>
          prev.workingDir === previous ? prev : { ...prev, workingDir: previous },
        )
        logger.error("chat", "ChatScreen::setWorkingDir", "Failed to update working directory", err)
        toast.error(t("chat.workingDir.invalid"), {
          description: err instanceof Error ? err.message : String(err),
        })
      } finally {
        setWorkingDirSaving(false)
      }
    },
    [session, currentSessionMeta?.workingDir, t],
  )

  // Once the auto-created session lands (chat command emits `session_created`),
  // the draft has been materialized server-side — drop the local copy so the
  // sidebar/sessions metadata becomes the single source of truth.
  useEffect(() => {
    if (session.currentSessionId && draftWorkingDir !== null) {
      setDraftWorkingDir(null)
    }
  }, [session.currentSessionId, draftWorkingDir])

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

  // The diff panel holds change metadata from a specific tool call in the
  // outgoing session — keeping it open across switches would render the
  // previous session's file content alongside a different message list.
  const closeDiff = diffPanel.closeDiff
  useEffect(() => {
    closeDiff()
  }, [currentSessionId, closeDiff])

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
      void handleStartNewChat(currentAgentId)
    })
  }, [handleStartNewChat, currentAgentId])

  // Listen for tray "focus-session" event — emitted when the user clicks an
  // in-progress regular conversation entry inside the system tray dropdown.
  useEffect(() => {
    return getTransport().listen("tray:focus-session", (raw) => {
      const sessionId = (raw as { sessionId?: string } | undefined)?.sessionId
      if (sessionId) void session.handleSwitchSession(sessionId)
    })
  }, [session.handleSwitchSession])

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
    endedStreamIdsRef,
    planMode: planModeState,
    temperatureOverride: sessionTemperature,
    incognitoEnabled,
    draftWorkingDir,
  })

  // Restore the per-session permission mode on session switch. The ref
  // guards against re-applying when `sessions` later reloads with the same
  // sid — that would clobber the user's in-session edits. When `sid` becomes
  // null (new-chat transition), reset to "default" so the previous session's
  // `yolo` / `smart` doesn't bleed into a fresh chat.
  const restoredModeForSidRef = useRef<string | null>(null)
  useEffect(() => {
    const sid = session.currentSessionId
    if (!sid) {
      if (restoredModeForSidRef.current !== null) {
        stream.setPermissionMode("default")
      }
      restoredModeForSidRef.current = null
      return
    }
    if (restoredModeForSidRef.current === sid) return
    const meta = session.sessions.find((s) => s.id === sid)
    if (!meta) return // wait until the sessions list has the meta
    const mode: SessionMode = meta.permissionMode ?? "default"
    restoredModeForSidRef.current = sid
    stream.setPermissionMode(mode)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.currentSessionId, session.sessions, stream.setPermissionMode])

  // ── Stream Reattach Hook ────────────────────────────────────
  // Rehydrates chat streaming after frontend reload / window reopen / browser
  // refresh via EventBus-backed events, deduplicated against `streamSeqRef`
  // which the primary per-call Channel path also updates.
  useChatStreamReattach({
    currentSessionId: session.currentSessionId,
    currentSessionIdRef: session.currentSessionIdRef,
    lastSeqRef: streamSeqRef,
    endedStreamIdsRef,
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
  const taskProgressSnapshot = useTaskProgressSnapshot(
    session.currentSessionId,
    session.messages,
  )
  const setPlanState = planMode.setPlanState
  const sendMessage = stream.handleSend

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
          await handleStartNewChat(session.currentAgentId)
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
          stream.setPermissionMode(action.mode)
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
          stream.handleSend(t("planMode.executeCommand"), {
            planMode: "executing",
            displayText: t("planMode.executionApproved"),
            isPlanTrigger: true,
          })
          break
        case "showPlan":
          planMode.setPlanContent(action.planContent)
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
        case "showProjectPicker": {
          // Render a markdown list of projects so the user can either click
          // back to /project <name> or visually pick from the sidebar's
          // project tree. A full clickable picker card is a follow-up.
          const lines = [t("project.openProject") + ":"]
          for (const p of action.projects) {
            const icon = p.emoji ? `${p.emoji} ` : "📁 "
            lines.push(`- ${icon}**${p.name}** · ${p.sessionCount}`)
          }
          lines.push("")
          lines.push(`> \`/project <${t("project.projectName")}>\``)
          const pickerMsg: Message = {
            role: "event",
            content: lines.join("\n"),
            timestamp: new Date().toISOString(),
          }
          session.setMessages((prev) => [...prev, pickerMsg])
          break
        }
        case "enterProject": {
          setDraftIncognito(false)
          void handleNewChatInProject(action.projectId, undefined, false)
          break
        }
        // result.content (rendered above as an event chip) is the only
        // user-facing surface today; richer wiring tracked in F-033.
        case "recapCard":
        case "openDashboardTab":
        case "skillFork":
          break
      }
    },
    [
      session,
      stream,
      handleStartNewChat,
      handleManualModelChange,
      handleEffortChange,
      planMode,
      loadSystemPrompt,
      handleNewChatInProject,
      t,
    ],
  )

  // ── Plan Approve Handler ───────────────────────────────────────
  const handlePlanApprove = useCallback(async () => {
    await planMode.approvePlan()
    // Send a short trigger — the full plan is already in the system prompt (Executing state)
    stream.handleSend(t("planMode.executeCommand"), {
      planMode: "executing",
      displayText: t("planMode.executionApproved"),
      isPlanTrigger: true,
    })
  }, [planMode, stream, t])

  const handlePlanContinue = useCallback(async () => {
    stream.handleSend(t("planMode.executeCommand"), {
      planMode: "executing",
      displayText: t("planMode.executionResumed"),
      isPlanTrigger: true,
    })
  }, [stream, t])

  const handleMessageSwitchModel = useCallback(
    (providerId: string, modelId: string) => {
      void handleManualModelChange(`${providerId}::${modelId}`)
    },
    [handleManualModelChange],
  )

  // ── Plan Request Changes Handler ──────────────────────────────
  // `prompt` carries the full XML payload sent to the LLM; `displayText` is
  // the friendly markdown stored in the user bubble (quote + comment). The
  // split exists so the chat history doesn't show the raw <plan-inline-comment>
  // XML to the user — see planCommentMessage.ts.
  const handleRequestChanges = useCallback(
    (prompt: string, displayText: string) => {
      setPlanState("planning")
      if (currentSessionId) {
        getTransport()
          .call("set_plan_mode", { sessionId: currentSessionId, state: "planning" })
          .catch(() => {})
      }
      sendMessage(prompt, { displayText })
    },
    [setPlanState, sendMessage, currentSessionId],
  )

  const shouldShowPlanPanel =
    planMode.showPanel &&
    planMode.planState !== "off" &&
    (planMode.planState === "planning" || planMode.planContent.trim().length > 0)

  const handlePlanPanelDragStart = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault()
      const startX = e.clientX
      const startWidth = planPanelWidth
      const maxWidth = Math.min(860, Math.max(420, window.innerWidth * 0.55))
      const onMouseMove = (ev: MouseEvent) => {
        const newWidth = Math.min(maxWidth, Math.max(360, startWidth - (ev.clientX - startX)))
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
    },
    [planPanelWidth],
  )

  const handleDiffPanelDragStart = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault()
      const startX = e.clientX
      const startWidth = diffPanel.panelWidth
      const maxWidth = Math.min(860, Math.max(420, window.innerWidth * 0.55))
      const onMouseMove = (ev: MouseEvent) => {
        const newWidth = Math.min(maxWidth, Math.max(360, startWidth - (ev.clientX - startX)))
        diffPanel.setPanelWidth(newWidth)
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
    },
    [diffPanel],
  )

  // Three right-side panels (PlanPanel / CanvasPanel / DiffPanel) are
  // mutually exclusive at the visual level — opening one closes the others
  // but keeps their internal state so re-toggling restores the prior view.
  useEffect(() => {
    if (diffPanel.showPanel) {
      planMode.setShowPanel(false)
    }
  }, [diffPanel.showPanel, planMode])

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
        onNewChat={handleStartNewChat}
        onDeleteSession={session.handleDeleteSession}
        onEditAgent={onOpenAgentSettings}
        onMarkAllRead={session.reloadSessions}
        onRenameSession={handleRenameSession}
        hasMoreSessions={session.hasMoreSessions}
        loadingMoreSessions={session.loadingMoreSessions}
        onLoadMoreSessions={session.handleLoadMoreSessions}
        onOpenProjectSettings={openProjectOverview}
        onAddProject={openCreateProject}
        onNewChatInProject={(projectId, opts) => {
          // Project + incognito are mutually exclusive — backend coerces to
          // false anyway; we strip here for UI consistency. Using the
          // project's default_agent (resolved server-side) by passing
          // `undefined` to handleNewChatInProject.
          setDraftIncognito(false)
          void handleNewChatInProject(projectId, undefined, opts?.incognito ?? false)
        }}
        onArchiveProject={(projectId, archived) => {
          void archiveProject(projectId, archived)
        }}
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
          setDraftIncognito(false)
          void handleNewChatInProject(projectId, defaultAgentId, false)
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

      {/* Conversation workspace */}
      <div className="flex-1 flex flex-col min-w-0 bg-background">
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
          effectiveWorkingDir={effectiveWorkingDir}
          workingDirSource={workingDirSource}
          project={
            session.currentSessionId
              ? (projects.find((p) => p.id === currentSessionMeta?.projectId) ?? null)
              : null
          }
          onOpenProjectSettings={openProjectOverview}
          agents={session.agents}
          onChangeAgent={handleChangeAgent}
        />

        <div className="flex-1 flex min-h-0 overflow-hidden">
          <div className="flex-1 flex flex-col min-w-0">
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
              sessionId={session.currentSessionId}
              incognito={incognitoEnabled}
              pendingScrollTarget={session.pendingScrollTarget}
              onScrollTargetHandled={session.clearPendingScrollTarget}
              pendingQuestionGroup={planMode.pendingQuestionGroup}
              onQuestionSubmitted={() => planMode.setPendingQuestionGroup(null)}
              planCardData={
                planMode.planCardInfo ? { title: planMode.planCardInfo.title } : null
              }
              planState={planMode.planState}
              onOpenPlanPanel={planMode.openPlanPanel}
              onApprovePlan={handlePlanApprove}
              onExitPlan={planMode.exitPlanMode}
              planSubagentRunning={planMode.planSubagentRunning}
              onSwitchModel={handleMessageSwitchModel}
              onViewSystemPrompt={loadSystemPrompt}
              onSwitchSession={(sid) => {
                void session.handleSwitchSession(sid)
              }}
              onOpenDiff={diffPanel.openDiff}
            />

            {/* Memory extraction toast */}
            {!isCronSession && !isSubagentSession && (
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
                  permissionMode={stream.permissionMode}
                  onPermissionModeChange={stream.setPermissionMode}
                  sessionTemperature={sessionTemperature}
                  onSessionTemperatureChange={setSessionTemperature}
                  incognitoEnabled={incognitoEnabled}
                  incognitoDisabledReason={incognitoDisabledReason}
                  onIncognitoChange={handleIncognitoChange}
                  workingDir={session.currentSessionId ? effectiveWorkingDir : draftWorkingDir}
                  workingDirInherited={
                    session.currentSessionId ? workingDirSource === "project" : false
                  }
                  workingDirSaving={workingDirSaving}
                  onWorkingDirChange={handleWorkingDirChange}
                  planState={planMode.planState}
                  onEnterPlanMode={planMode.enterPlanMode}
                  onExitPlanMode={planMode.exitPlanMode}
                  onTogglePlanPanel={() => planMode.setShowPanel((p) => !p)}
                  taskProgressSnapshot={taskProgressSnapshot}
                />
              </>
            )}
          </div>

          {/* Diff panel (right side; mutually exclusive with PlanPanel) */}
          {diffPanel.showPanel && diffPanel.activeChanges.length > 0 && (
            <div
              className="relative flex h-full min-h-0 shrink-0 min-w-[360px] max-w-[55%] p-3 pl-2"
              style={{ width: diffPanel.panelWidth }}
            >
              <div
                className="group absolute left-0 top-3 bottom-3 z-10 flex w-3 cursor-col-resize items-center justify-center"
                onMouseDown={handleDiffPanelDragStart}
                role="separator"
                aria-orientation="vertical"
                aria-label={t("diffPanel.resizePanel", "Resize diff panel")}
              >
                <div className="h-full w-px rounded-full bg-transparent transition-colors group-hover:bg-primary/35 group-active:bg-primary/50" />
              </div>
              <DiffPanel
                changes={diffPanel.activeChanges}
                activeIndex={diffPanel.activeIndex}
                onActiveIndexChange={diffPanel.setActiveIndex}
                onClose={diffPanel.closeDiff}
                embedded
              />
            </div>
          )}

          {/* Plan workspace (right side, integrated under the shared title bar) */}
          {shouldShowPlanPanel && (
            <div
              className="relative flex h-full min-h-0 shrink-0 min-w-[360px] max-w-[55%] p-3 pl-2"
              style={{ width: planPanelWidth }}
            >
              <div
                className="group absolute left-0 top-3 bottom-3 z-10 flex w-3 cursor-col-resize items-center justify-center"
                onMouseDown={handlePlanPanelDragStart}
                role="separator"
                aria-orientation="vertical"
                aria-label={t("planMode.resizePanel", "Resize plan panel")}
              >
                <div className="h-full w-px rounded-full bg-transparent transition-colors group-hover:bg-primary/35 group-active:bg-primary/50" />
              </div>
              <PlanPanel
                planState={planMode.planState}
                planContent={planMode.planContent}
                sessionId={session.currentSessionId}
                onApprove={handlePlanApprove}
                onExit={planMode.exitPlanMode}
                onClose={() => planMode.setShowPanel(false)}
                onContinue={handlePlanContinue}
                isExecutionActive={session.loading && planMode.planState === "executing"}
                onRequestChanges={handleRequestChanges}
                embedded
              />
            </div>
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
        </div>
      </div>
    </>
  )
}

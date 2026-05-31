import { useState, useRef, useEffect, useLayoutEffect, useCallback, useMemo } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { save } from "@tauri-apps/plugin-dialog"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import {
  Brain,
  ChevronLeft,
  ChevronRight,
  ClipboardList,
  FolderTree,
  GitCompare,
  Globe,
  Monitor,
  MousePointer2,
  Users,
  type LucideIcon,
} from "lucide-react"
import type {
  ActiveModel,
  AvailableModel,
  ChatDisplayMode,
  Message,
  SessionMode,
} from "@/types/chat"
import { normalizeEffortForModel } from "@/types/chat"
import type { CommandResult } from "./slash-commands/types"
import type { AgentConfig } from "@/components/settings/types"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import ChatSidebar from "@/components/chat/ChatSidebar"
import ChatInput from "@/components/chat/ChatInput"
import { FileBrowserPanel } from "@/components/chat/FileBrowserPanel"
import type { QuotePayload } from "@/components/chat/project/file-browser/FilePreviewPane"
import type { IncognitoDisabledReason } from "@/components/chat/input/IncognitoToggle"
import ChatTitleBar from "@/components/chat/ChatTitleBar"
import HandoverDialog from "@/components/chat/HandoverDialog"
import MessageList from "@/components/chat/MessageList"
import CrashRecoveryBanner from "@/components/common/CrashRecoveryBanner"
import CanvasPanel from "@/components/chat/CanvasPanel"
import BrowserPanel from "@/components/chat/BrowserPanel"
import MacControlPanel from "@/components/chat/MacControlPanel"
import { TeamPanel } from "@/components/team/TeamPanel"
import TeamMiniIndicator from "@/components/team/TeamMiniIndicator"
import { useActiveTeam } from "@/components/team/useTeam"
import SessionSearchBar from "@/components/chat/SessionSearchBar"
import { IconTip } from "@/components/ui/tooltip"
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
import type { BuiltPlanComment } from "./plan-mode/planCommentMessage"
import { RightPanelShell } from "./right-panel/RightPanelShell"
import { useProjects } from "./project/hooks/useProjects"
import ProjectDialog from "./project/ProjectDialog"
import ProjectOverviewDialog from "./project/ProjectOverviewDialog"
import {
  CHAT_DISPLAY_MODE_EVENT,
  normalizeChatDisplayMode,
  readChatDisplayModePreference,
  writeChatDisplayModePreference,
} from "./chatDisplayModePreference"
import {
  CHAT_SIDEBAR_DEFAULT_WIDTH,
  CHAT_SIDEBAR_MAX_WIDTH,
  CHAT_SIDEBAR_MIN_WIDTH,
  CHAT_SIDEBAR_WIDTH_STORAGE_KEY,
} from "./sidebar/types"
import { generateClientId } from "./chatScrollKeys"
import type { Project, ProjectMeta } from "@/types/project"

interface ChatScreenProps {
  onOpenAgentSettings?: (agentId: string) => void
  onCodexReauth?: () => void
  initialSessionId?: string
  onSessionNavigated?: () => void
  onUnreadCountChange?: (count: number) => void
  onOpenDashboardTab?: (tab: string, initialReportId?: string | null) => void
  sessionsRefreshTrigger?: number
  /** Free-form text to append to the chat input on next render (e.g. `@plan:abcd:v0`). */
  pendingChatInsert?: string
  /** Called once the insert has been consumed so App can clear the pending slot. */
  onChatInsertConsumed?: () => void
}

type ExclusiveRightPanel = "diff" | "plan" | "files" | "browser" | "mac-control" | "canvas" | "team"
type ExclusiveRightPanelVisibility = Record<ExclusiveRightPanel, boolean>

const EXCLUSIVE_RIGHT_PANEL_ORDER: readonly ExclusiveRightPanel[] = [
  "diff",
  "plan",
  "files",
  "browser",
  "mac-control",
  "canvas",
  "team",
]

const EMPTY_RIGHT_PANEL_VISIBILITY: ExclusiveRightPanelVisibility = {
  diff: false,
  plan: false,
  files: false,
  browser: false,
  "mac-control": false,
  canvas: false,
  team: false,
}

const EXCLUSIVE_RIGHT_PANEL_ICONS: Record<ExclusiveRightPanel, LucideIcon> = {
  diff: GitCompare,
  plan: ClipboardList,
  files: FolderTree,
  browser: Globe,
  "mac-control": MousePointer2,
  canvas: Monitor,
  team: Users,
}

const DEFAULT_RIGHT_PANEL_WIDTH = 520

interface MacControlFrameOpenHint {
  mediaId?: string | null
  path?: string | null
}

function clampChatSidebarWidth(width: number): number {
  return Math.min(CHAT_SIDEBAR_MAX_WIDTH, Math.max(CHAT_SIDEBAR_MIN_WIDTH, width))
}

function isSessionMode(value: unknown): value is SessionMode {
  return value === "default" || value === "smart" || value === "yolo"
}

function readActionString(action: object, camelKey: string, snakeKey: string): string | null {
  const record = action as Record<string, unknown>
  const value = record[camelKey] ?? record[snakeKey]
  return typeof value === "string" && value.length > 0 ? value : null
}

type ClientEventMessage = Omit<Message, "role" | "_clientId">

function makeClientEventMessage(message: ClientEventMessage): Message {
  return {
    role: "event",
    _clientId: generateClientId(),
    ...message,
  }
}

export default function ChatScreen({
  onOpenAgentSettings,
  onCodexReauth,
  initialSessionId,
  onSessionNavigated,
  onUnreadCountChange,
  onOpenDashboardTab,
  sessionsRefreshTrigger,
  pendingChatInsert,
  onChatInsertConsumed,
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
  const [panelWidth, setPanelWidth] = useState(() => {
    if (typeof window === "undefined") return CHAT_SIDEBAR_DEFAULT_WIDTH
    const stored = window.localStorage.getItem(CHAT_SIDEBAR_WIDTH_STORAGE_KEY)
    if (!stored) return CHAT_SIDEBAR_DEFAULT_WIDTH

    const storedWidth = Number(stored)
    return Number.isFinite(storedWidth)
      ? clampChatSidebarWidth(storedWidth)
      : CHAT_SIDEBAR_DEFAULT_WIDTH
  })
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    if (typeof window === "undefined") return false
    return window.localStorage.getItem("hope.chatSidebarCollapsed") === "true"
  })

  useEffect(() => {
    if (typeof window === "undefined") return
    window.localStorage.setItem(CHAT_SIDEBAR_WIDTH_STORAGE_KEY, String(panelWidth))
  }, [panelWidth])

  useEffect(() => {
    if (typeof window === "undefined") return
    window.localStorage.setItem("hope.chatSidebarCollapsed", String(sidebarCollapsed))
  }, [sidebarCollapsed])

  const [defaultDisplayMode, setDefaultDisplayMode] = useState<ChatDisplayMode>(() =>
    readChatDisplayModePreference(),
  )
  const [sessionDisplayModeOverrides, setSessionDisplayModeOverrides] = useState<
    Record<string, ChatDisplayMode>
  >({})

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<{ chatDisplayMode?: unknown }>("get_user_config")
      .then((cfg) => {
        const mode = normalizeChatDisplayMode(cfg.chatDisplayMode)
        if (!mode || cancelled) return
        setDefaultDisplayMode(mode)
        writeChatDisplayModePreference(mode, { emit: false })
      })
      .catch((e: unknown) =>
        logger.warn(
          "settings",
          "ChatScreen::loadDisplayMode",
          "Failed to load chat display mode",
          e,
        ),
      )

    const handlePreferenceChange = (event: Event) => {
      const mode = normalizeChatDisplayMode((event as CustomEvent).detail?.mode)
      if (mode) setDefaultDisplayMode(mode)
    }
    window.addEventListener(CHAT_DISPLAY_MODE_EVENT, handlePreferenceChange)
    return () => {
      cancelled = true
      window.removeEventListener(CHAT_DISPLAY_MODE_EVENT, handlePreferenceChange)
    }
  }, [])

  // Right panel width (shared by all switchable right panels)
  const [rightPanelWidth, setRightPanelWidth] = useState(DEFAULT_RIGHT_PANEL_WIDTH)
  const [canvasPanelOpen, setCanvasPanelOpen] = useState(false)

  // Right side diff panel (write/edit/apply_patch metadata viewer)
  const diffPanel = useDiffPanel()

  // Browser live-mirror panel. Auto-opens on the **first** `browser:frame`
  // push of a session. After the user manually closes it, further frames in
  // the same session never re-pop the panel — `browserPanelDismissedRef`
  // tracks the dismissal until a session switch resets it.
  const [showBrowserPanel, setShowBrowserPanel] = useState(false)
  const browserPanelDismissedRef = useRef(false)
  const [showFilesPanel, setShowFilesPanel] = useState(false)
  // Clicking a staged quote chip reveals that file in the browser. The nonce
  // makes each click a fresh signal, even when re-revealing the same path.
  const revealQuoteNonce = useRef(0)
  const [revealFile, setRevealFile] = useState<{
    path: string
    name: string
    startLine: number
    endLine: number
    nonce: number
  } | null>(null)
  const [showMacControlPanel, setShowMacControlPanel] = useState(false)
  const macControlPanelDismissedRef = useRef(false)

  // Context compact state
  const [compacting, setCompacting] = useState(false)

  // In-session "find in page" search bar state
  const [searchBarOpen, setSearchBarOpen] = useState(false)
  const [searchFocusSignal, setSearchFocusSignal] = useState(0)
  const [handoverSessionId, setHandoverSessionId] = useState<string | null>(null)
  const openSessionSearch = useCallback(() => {
    setSearchBarOpen(true)
    setSearchFocusSignal((n) => n + 1)
  }, [])

  // Sidebar global-history search focus tick (Cmd+Shift+F).
  const [globalSearchFocusSignal, setGlobalSearchFocusSignal] = useState(0)
  const focusGlobalSearch = useCallback(() => {
    setGlobalSearchFocusSignal((n) => n + 1)
  }, [])

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

  // ── Projects ────────────────────────────────────────────────
  const {
    projects,
    createProject,
    updateProject,
    deleteProject,
    archiveProject,
    moveSessionToProject,
    reloadProjects,
  } = useProjects({ includeArchived: true })

  const refreshProjectAggregates = useCallback(() => {
    void reloadProjects()
  }, [reloadProjects])

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
    onSidebarAggregatesChanged: refreshProjectAggregates,
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
  const displayModeSessionKey = currentSessionId ?? "draft"
  const displayMode = sessionDisplayModeOverrides[displayModeSessionKey] ?? defaultDisplayMode
  const previousDisplayModeSessionKeyRef = useRef(displayModeSessionKey)
  const setAgentName = session.setAgentName
  const updateSessionMeta = session.updateSessionMeta
  const handleSwitchSession = session.handleSwitchSession

  useEffect(() => {
    const previousKey = previousDisplayModeSessionKeyRef.current
    if (previousKey === "draft" && currentSessionId) {
      setSessionDisplayModeOverrides((prev) => {
        const draftMode = prev.draft
        if (!draftMode || prev[currentSessionId]) return prev
        const next = { ...prev, [currentSessionId]: draftMode }
        delete next.draft
        return next
      })
    }
    previousDisplayModeSessionKeyRef.current = displayModeSessionKey
  }, [currentSessionId, displayModeSessionKey])

  const handleDisplayModeChange = useCallback(
    (mode: ChatDisplayMode) => {
      setSessionDisplayModeOverrides((prev) =>
        prev[displayModeSessionKey] === mode ? prev : { ...prev, [displayModeSessionKey]: mode },
      )
    },
    [displayModeSessionKey],
  )

  const handleSessionEffortChange = useCallback(
    async (effort: string) => {
      const sid = session.currentSessionId
      if (sid) {
        updateSessionMeta(sid, (prev) =>
          prev.reasoningEffort === effort ? prev : { ...prev, reasoningEffort: effort },
        )
      }
      await handleEffortChange(effort, sid, session.currentAgentId)
    },
    [handleEffortChange, session.currentAgentId, session.currentSessionId, updateSessionMeta],
  )

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

  const refreshRuntimeModelState = useCallback(async () => {
    try {
      const [models, active, settings, agentConfig] = await Promise.all([
        getTransport().call<AvailableModel[]>("get_available_models"),
        getTransport().call<ActiveModel | null>("get_active_model"),
        getTransport().call<{ model: string; reasoning_effort: string }>("get_current_settings"),
        getTransport()
          .call<AgentConfig>("get_agent_config", { id: currentAgentId })
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
      const effort =
        currentSessionMeta?.reasoningEffort ??
        agentConfig?.model?.reasoningEffort ??
        settings.reasoning_effort
      setReasoningEffort(normalizeEffortForModel(displayModelInfo, effort, t))

      if (agentConfig?.name) {
        setAgentName(agentConfig.name)
      }
    } catch (e) {
      logger.error("ui", "ChatScreen::refreshRuntimeModelState", "Failed to refresh model state", e)
    }
  }, [
    currentSessionMeta?.modelId,
    currentSessionMeta?.providerId,
    currentSessionMeta?.reasoningEffort,
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
      await handleModelChange(key, currentSessionId, session.currentAgentId)
    },
    [handleModelChange, currentSessionId, session.currentAgentId],
  )

  // Auto-show team panel when a team is created
  useEffect(() => {
    if (activeTeamId) setShowTeamPanel(true)
  }, [activeTeamId])

  useEffect(() => {
    manualModelOverrideRef.current = null
  }, [currentSessionId, currentAgentId])

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

  const refreshUnreadState = useCallback(async () => {
    await Promise.all([reloadSessions(), reloadProjects()])
  }, [reloadSessions, reloadProjects])

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
      reloadProjects()
    }
  }, [sessionsRefreshTrigger, reloadSessions, reloadProjects])

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

  // Cmd/Ctrl+F: in-session search; Cmd/Ctrl+Shift+F: global sidebar search.
  // The in-session bar requires an active session (search target is a single
  // session); the global one is always available.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const modKey = e.metaKey || e.ctrlKey
      if (!modKey || e.altKey) return
      if (e.key.toLowerCase() !== "f") return
      // Don't hijack the shortcut if the user is editing inside an
      // unrelated contenteditable (e.g. a markdown canvas field). Free
      // inputs (ChatInput textarea etc.) are fine to preempt since there
      // is no browser find-in-page equivalent for chat history anyway.
      const target = e.target as HTMLElement | null
      if (target?.isContentEditable) return

      if (e.shiftKey) {
        e.preventDefault()
        focusGlobalSearch()
        return
      }
      if (!currentSessionId) return
      e.preventDefault()
      openSessionSearch()
    }
    window.addEventListener("keydown", handler)
    return () => window.removeEventListener("keydown", handler)
  }, [currentSessionId, openSessionSearch, focusGlobalSearch])

  // Cmd/Ctrl+N: start a fresh draft chat with the current agent, matching the
  // sidebar New Chat button and tray "new-session" behavior.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const modKey = e.metaKey || e.ctrlKey
      if (!modKey || e.altKey || e.shiftKey || e.repeat) return
      if (e.key.toLowerCase() !== "n") return

      const target = e.target as HTMLElement | null
      if (target?.isContentEditable) return

      e.preventDefault()
      void handleStartNewChat(currentAgentId)
    }
    window.addEventListener("keydown", handler)
    return () => window.removeEventListener("keydown", handler)
  }, [handleStartNewChat, currentAgentId])

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
      if (sessionId) void handleSwitchSession(sessionId)
    })
  }, [handleSwitchSession])

  // Listen for channel slash command state-sync events.
  // Deps narrowed to currentSessionId + refreshUnreadState: re-subscribing on
  // every applyModelForDisplay / session.* identity churn would create a
  // listener storm. The other call sites used in the closure are either
  // stable useState setters or only matter for the active session — which is
  // re-keyed by currentSessionId here.
  useEffect(() => {
    const unlisteners: Array<() => void> = []

    // Session model pinned (from IM /model, set_session_model command, or
    // PATCH /api/sessions/{id}/model). The IM channel may target a different
    // session than the one the desktop UI currently has open — gate by
    // sessionId so we don't apply a remote change to the wrong picker.
    unlisteners.push(
      getTransport().listen("session:model_updated", (payload) => {
        const { sessionId, providerId, modelId } = payload as {
          sessionId: string
          providerId: string
          modelId: string
        }
        if (!sessionId || sessionId !== session.currentSessionId) return
        manualModelOverrideRef.current = { providerId, modelId }
        applyModelForDisplay(`${providerId}::${modelId}`)
      }),
    )

    // Effort changed from channel (/thinking)
    unlisteners.push(
      getTransport().listen("slash:effort_changed", (payload) => {
        const data = payload as string | { sessionId?: string; effort?: string }
        const effort = typeof data === "string" ? data : data.effort
        if (!effort) return
        const sid = typeof data === "string" ? undefined : data.sessionId
        if (!sid || sid === session.currentSessionId) {
          setReasoningEffort(effort)
        }
        if (sid) {
          session.updateSessionMeta(sid, (prev) =>
            prev.reasoningEffort === effort ? prev : { ...prev, reasoningEffort: effort },
          )
        }
        session.reloadSessions()
      }),
    )

    // Session cleared from channel (/clear)
    unlisteners.push(
      getTransport().listen("slash:session_cleared", (payload) => {
        const clearedSid = payload as string
        if (clearedSid === session.currentSessionId) {
          session.setMessages([])
        }
        void refreshUnreadState()
      }),
    )

    // Plan state changed from channel (/plan)
    unlisteners.push(
      getTransport().listen("slash:plan_changed", () => {
        void refreshUnreadState()
      }),
    )

    return () => {
      unlisteners.forEach((fn) => fn())
    }
  }, [session.currentSessionId, refreshUnreadState]) // eslint-disable-line react-hooks/exhaustive-deps

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
    capMessagesForSession: session.capMessagesForSession,
    touchSessionCacheLru: session.touchSessionCacheLru,
    sessions: session.sessions,
    agents: session.agents,
    activeModel,
    reloadSessions: refreshUnreadState,
    updateSessionMessages: session.updateSessionMessages,
    lastSeqRef: streamSeqRef,
    endedStreamIdsRef,
    planMode: planModeState,
    temperatureOverride: sessionTemperature,
    reasoningEffort,
    incognitoEnabled,
    draftWorkingDir,
  })

  useEffect(() => {
    return getTransport().listen("permission:mode_changed", (payload) => {
      const data = payload as { sessionId?: unknown; mode?: unknown }
      if (typeof data.sessionId !== "string" || !isSessionMode(data.mode)) return
      const sessionId = data.sessionId
      const mode = data.mode

      updateSessionMeta(sessionId, (prev) =>
        prev.permissionMode === mode ? prev : { ...prev, permissionMode: mode },
      )
      if (sessionId === currentSessionId) {
        stream.setPermissionMode(mode)
      }
      void reloadSessions()
    })
  }, [currentSessionId, reloadSessions, stream.setPermissionMode, updateSessionMeta])

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

  // Consume a `@plan:xxx` (or any free-form text) injection from the global
  // Plans view: append once with a leading space, then notify App so the slot
  // clears. Runs after `stream` is initialized so `setInput` is available.
  useEffect(() => {
    if (!pendingChatInsert) return
    const sep = stream.input && !stream.input.endsWith(" ") ? " " : ""
    stream.setInput(`${stream.input}${sep}${pendingChatInsert} `)
    onChatInsertConsumed?.()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingChatInsert])

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
    reloadSessions: refreshUnreadState,
    onTurnStarted: stream.handleTurnStarted,
    onTurnEnded: stream.handleTurnEnded,
  })

  // ── Plan Mode Hook ─────────────────────────────────────────
  const planMode = usePlanMode(session.currentSessionId, planModeState, setPlanModeState)
  const taskProgressSnapshot = useTaskProgressSnapshot(session.currentSessionId, session.messages)
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

      // Skip history rows for session-spawning commands and skill passThrough
      // (the real user bubble already shows "/skillname args"). Other slash
      // controls are persisted by the backend as event rows; mirror that
      // immediately in the current in-memory timeline.
      const shouldShowSlashHistory =
        action?.type !== "newSession" &&
        action?.type !== "switchAgent" &&
        action?.type !== "passThrough" &&
        !result._isSkillPassThrough
      const actionRendersResult =
        action?.type === "showProjectPicker" ||
        action?.type === "showSessionPicker" ||
        action?.type === "recapCard" ||
        action?.type === "skillFork"
      const shouldAppendResultContent = result.content && !actionRendersResult
      const slashHistoryMessages: Message[] = []
      if (shouldShowSlashHistory && result._slashCommandText) {
        const now = new Date().toISOString()
        slashHistoryMessages.push(
          makeClientEventMessage({
            content: result._slashCommandText,
            timestamp: now,
            slashEvent: { kind: "command", displayAs: "user" },
          }),
        )
        if (shouldAppendResultContent) {
          slashHistoryMessages.push(
            makeClientEventMessage({
              content: result.content,
              timestamp: now,
              slashEvent: { kind: "result", command: result._slashCommandText },
            }),
          )
        }
      } else if (shouldAppendResultContent && shouldShowSlashHistory) {
        slashHistoryMessages.push(
          makeClientEventMessage({
            content: result.content,
            timestamp: new Date().toISOString(),
          }),
        )
      }
      if (slashHistoryMessages.length > 0) {
        session.setMessages((prev) => [...prev, ...slashHistoryMessages])
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
          handleSessionEffortChange(action.effort)
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
          session.setMessages(slashHistoryMessages)
          void refreshUnreadState()
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
            const ext = (action.filename.split(".").pop() ?? "md").toLowerCase()
            const filterName = ext === "json" ? "JSON" : ext === "html" ? "HTML" : "Markdown"
            const filePath = await save({
              defaultPath: action.filename,
              filters: [{ name: filterName, extensions: [ext] }],
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
          const pickerMsg: Message = makeClientEventMessage({
            content: "",
            timestamp: new Date().toISOString(),
            modelPickerData: {
              models: action.models,
              activeProviderId: action.activeProviderId,
              activeModelId: action.activeModelId,
            },
          })
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
          const contextMsg: Message = makeClientEventMessage({
            content: "",
            timestamp: new Date().toISOString(),
            contextBreakdownData: action.breakdown,
          })
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
          const pickerMsg: Message = makeClientEventMessage({
            content: lines.join("\n"),
            timestamp: new Date().toISOString(),
          })
          session.setMessages((prev) => [...prev, pickerMsg])
          break
        }
        case "enterProject": {
          setDraftIncognito(false)
          void handleNewChatInProject(action.projectId, undefined, false)
          break
        }
        case "assignProject": {
          // IM-mode action — desktop falls back to the "create new chat in
          // project" flow so users still get a usable outcome if they
          // somehow reach this branch from the GUI.
          setDraftIncognito(false)
          void handleNewChatInProject(action.projectId, undefined, false)
          break
        }
        case "showSessionPicker": {
          // Markdown list, mirroring the showProjectPicker fallback. Each
          // row carries the short id + title + agent / project / channel
          // chips so users can spot the right session at a glance; the
          // user types `/session <id>` (or clicks in the sidebar) to switch.
          const lines = [t("chat.pickSession") + ":"]
          for (const s of action.sessions) {
            const idShort = s.id.slice(0, 8)
            const chips: string[] = []
            if (s.agentLabel) chips.push(`agent: ${s.agentLabel}`)
            if (s.projectLabel) chips.push(`project: ${s.projectLabel}`)
            if (s.channelLabel) chips.push(s.channelLabel)
            const suffix = chips.length ? ` · _${chips.join(" · ")}_` : ""
            lines.push(`- \`${idShort}\` · ${s.title}${suffix}`)
            if (s.snippet) lines.push(`  > ${s.snippet}`)
          }
          lines.push("")
          lines.push("> `/session <id>` · `/sessions <query>`")
          const pickerMsg: Message = makeClientEventMessage({
            content: lines.join("\n"),
            timestamp: new Date().toISOString(),
          })
          session.setMessages((prev) => [...prev, pickerMsg])
          break
        }
        case "enterSession":
        case "attachToSession": {
          // Desktop has no chat-to-session binding; both reduce to "switch
          // to that session". Reuse the sidebar's switch path so history /
          // pagination / agent restore behave identically.
          void session.handleSwitchSession(action.sessionId)
          break
        }
        case "detachFromSession": {
          // GUI has no IM-attach to release. Surface a hint so /session exit
          // typed from the desktop slash menu doesn't appear silent.
          const msg: Message = makeClientEventMessage({
            content: t("chat.detachOnDesktopNoop"),
            timestamp: new Date().toISOString(),
          })
          session.setMessages((prev) => [...prev, msg])
          break
        }
        case "handoverToChannel": {
          // Slash-form handover — mirrors what the title-bar Send icon does
          // through HandoverDialog, but skips the dialog when the user
          // already supplied channel:account:chat[:thread] inline.
          try {
            await getTransport().call<void>("channel_handover_session", {
              sessionId: action.sessionId,
              channelId: action.channelId,
              accountId: action.accountId,
              chatId: action.chatId,
              threadId: action.threadId ?? null,
            })
            const msg: Message = makeClientEventMessage({
              content: t("chat.handover.done"),
              timestamp: new Date().toISOString(),
            })
            session.setMessages((prev) => [...prev, msg])
          } catch (e) {
            const msg: Message = makeClientEventMessage({
              content: t("chat.handover.failed", { error: String(e) }),
              timestamp: new Date().toISOString(),
            })
            session.setMessages((prev) => [...prev, msg])
          }
          break
        }
        case "recapCard": {
          const reportId = readActionString(action, "reportId", "report_id")
          if (!reportId) {
            logger.warn("chat", "ChatScreen::slashRecapCard", "Missing report id", action)
            break
          }
          const msg: Message = makeClientEventMessage({
            content: "",
            timestamp: new Date().toISOString(),
            recapCardData: { reportId },
          })
          session.setMessages((prev) => [...prev, msg])
          break
        }
        case "openDashboardTab":
          onOpenDashboardTab?.(action.tab)
          break
        case "skillFork": {
          const runId = readActionString(action, "runId", "run_id")
          if (!runId) {
            logger.warn("chat", "ChatScreen::slashSkillFork", "Missing run id", action)
            break
          }
          const skillName =
            readActionString(action, "skillName", "skill_name") ??
            t("skills.defaultName", { defaultValue: "skill" })
          const msg: Message = makeClientEventMessage({
            content: "",
            timestamp: new Date().toISOString(),
            skillForkData: {
              runId,
              skillName,
            },
          })
          session.setMessages((prev) => [...prev, msg])
          break
        }
      }
    },
    [
      session,
      stream,
      handleStartNewChat,
      handleManualModelChange,
      handleSessionEffortChange,
      planMode,
      loadSystemPrompt,
      handleNewChatInProject,
      refreshUnreadState,
      onOpenDashboardTab,
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
  // See `planCommentMessage.ts` for the prompt vs displayText vs payload split.
  const handleRequestChanges = useCallback(
    ({ prompt, displayText, payload }: BuiltPlanComment) => {
      setPlanState("planning")
      if (currentSessionId) {
        getTransport()
          .call("set_plan_mode", { sessionId: currentSessionId, state: "planning" })
          .catch(() => {})
      }
      sendMessage(prompt, { displayText, planComment: payload })
    },
    [setPlanState, sendMessage, currentSessionId],
  )

  const shouldShowPlanPanel =
    planMode.showPanel &&
    planMode.planState !== "off" &&
    (planMode.planState === "planning" || planMode.planContent.trim().length > 0)
  const isDiffPanelVisible = diffPanel.showPanel && diffPanel.activeChanges.length > 0
  const [activeExclusiveRightPanel, setActiveExclusiveRightPanel] =
    useState<ExclusiveRightPanel | null>(null)
  const [rightPanelCollapsed, setRightPanelCollapsed] = useState(false)
  const rightPanelVisibility = useMemo<ExclusiveRightPanelVisibility>(
    () => ({
      diff: isDiffPanelVisible,
      plan: shouldShowPlanPanel,
      files: showFilesPanel && !!effectiveWorkingDir,
      browser: showBrowserPanel,
      "mac-control": showMacControlPanel,
      canvas: canvasPanelOpen,
      team: !!activeTeamId && showTeamPanel,
    }),
    [
      activeTeamId,
      canvasPanelOpen,
      effectiveWorkingDir,
      isDiffPanelVisible,
      shouldShowPlanPanel,
      showBrowserPanel,
      showFilesPanel,
      showMacControlPanel,
      showTeamPanel,
    ],
  )
  const openExclusiveRightPanels = useMemo(
    () => EXCLUSIVE_RIGHT_PANEL_ORDER.filter((panel) => rightPanelVisibility[panel]),
    [rightPanelVisibility],
  )
  const hasOpenExclusiveRightPanel = openExclusiveRightPanels.length > 0
  const renderedExclusiveRightPanel =
    activeExclusiveRightPanel && rightPanelVisibility[activeExclusiveRightPanel]
      ? activeExclusiveRightPanel
      : (openExclusiveRightPanels[0] ?? null)
  const shouldRenderRightPanelContent =
    !!renderedExclusiveRightPanel && !rightPanelCollapsed
  const rightPanelToggleLabel = rightPanelCollapsed
    ? t("chat.rightPanel.expand", "Expand right panel")
    : t("chat.rightPanel.collapse", "Collapse right panel")
  const getRightPanelLabel = useCallback(
    (panel: ExclusiveRightPanel) => {
      switch (panel) {
        case "diff":
          return t("diffPanel.title", "Diff")
        case "plan":
          return t("planMode.panelTitle", "Plan")
        case "files":
          return t("fileBrowser.panelTitle", "Files")
        case "browser":
          return t("browser.panelTitle", "Browser")
        case "mac-control":
          return t("macControl.panelTitle", "Mac Control")
        case "canvas":
          return t("canvas.panelTitle", "Canvas")
        case "team":
          return t("team.panelTitle", "Team")
      }
    },
    [t],
  )

  // Stage a "quote to chat" reference as a removable chip above the composer.
  // On send it becomes a quote attachment: the model sees a <file_reference>
  // block, the user only ever sees a friendly quote card.
  const handleFileQuote = useCallback(
    (q: QuotePayload) => {
      stream.setPendingQuotes((prev) => [...prev, q])
    },
    [stream],
  )
  // Reveal a quoted file in the browser: open the files panel + signal target.
  const handleQuoteJump = useCallback((q: QuotePayload) => {
    setShowFilesPanel(true)
    revealQuoteNonce.current += 1
    setRevealFile({
      path: q.path,
      name: q.name,
      startLine: q.startLine,
      endLine: q.endLine,
      nonce: revealQuoteNonce.current,
    })
  }, [])
  const rightPanelKey = renderedExclusiveRightPanel
  const lastRightPanelKeyRef = useRef<string | null>(rightPanelKey)

  useEffect(() => {
    if (rightPanelKey && rightPanelKey !== lastRightPanelKeyRef.current) {
      setSidebarCollapsed(true)
    }
    lastRightPanelKeyRef.current = rightPanelKey
  }, [rightPanelKey])

  useEffect(() => {
    if (!hasOpenExclusiveRightPanel && rightPanelCollapsed) {
      setRightPanelCollapsed(false)
    }
  }, [hasOpenExclusiveRightPanel, rightPanelCollapsed])

  // Plan / Diff / Browser / Mac Control / Canvas / Team share the same right
  // rail. Track rising edges so the panel that just opened wins while the
  // others remain open in the background and can be switched back to.
  const previousRightPanelVisibilityRef = useRef<ExclusiveRightPanelVisibility>(
    EMPTY_RIGHT_PANEL_VISIBILITY,
  )
  useLayoutEffect(() => {
    const previous = previousRightPanelVisibilityRef.current
    const newlyOpened =
      EXCLUSIVE_RIGHT_PANEL_ORDER.find(
        (panel) => rightPanelVisibility[panel] && !previous[panel],
      ) ?? null
    const stillActive =
      activeExclusiveRightPanel && rightPanelVisibility[activeExclusiveRightPanel]
        ? activeExclusiveRightPanel
        : null
    const active = newlyOpened ?? stillActive ?? openExclusiveRightPanels[0] ?? null

    previousRightPanelVisibilityRef.current = rightPanelVisibility
    if (activeExclusiveRightPanel !== active) {
      setActiveExclusiveRightPanel(active)
    }
  }, [activeExclusiveRightPanel, openExclusiveRightPanels, rightPanelVisibility])

  // Reset dismissal flags (and any open panel state) on session switch so each
  // session gets a fresh chance to auto-open live mirror panels.
  useEffect(() => {
    browserPanelDismissedRef.current = false
    macControlPanelDismissedRef.current = false
    setShowBrowserPanel(false)
    setShowMacControlPanel(false)
  }, [session.currentSessionId])

  // Auto-open the BrowserPanel only on the first `browser:frame` of a session
  // and only if the user hasn't already dismissed it.
  useEffect(() => {
    const unlisten = getTransport().listen("browser:frame", () => {
      if (browserPanelDismissedRef.current) return
      setShowBrowserPanel((prev) => (prev ? prev : true))
    })
    return () => {
      try {
        unlisten?.()
      } catch {
        // ignore
      }
    }
  }, [])

  useEffect(() => {
    const unlisten = getTransport().listen("mac_control:frame", (raw) => {
      if (macControlPanelDismissedRef.current) return
      const payload = parsePayload<MacControlFrameOpenHint>(raw)
      const isToolScreenshotFrame = !!(payload?.mediaId || payload?.path)
      setShowMacControlPanel((prev) => (prev ? prev : isToolScreenshotFrame))
    })
    return () => {
      try {
        unlisten?.()
      } catch {
        // ignore
      }
    }
  }, [])

  const emptySessionInputHero =
    session.messages.length === 0 &&
    !session.loading &&
    !planMode.pendingQuestionGroup &&
    !planMode.planCardInfo &&
    !planMode.planSubagentRunning &&
    !searchBarOpen

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
        sidebarCollapsed={sidebarCollapsed}
        onPanelWidthChange={setPanelWidth}
        onSidebarCollapsedChange={setSidebarCollapsed}
        onSwitchSession={session.handleSwitchSession}
        onNewChat={handleStartNewChat}
        onDeleteSession={session.handleDeleteSession}
        onEditAgent={onOpenAgentSettings}
        onToggleSessionPinned={session.handleToggleSessionPinned}
        onReorderAgents={session.handleReorderAgents}
        onMarkAllRead={refreshUnreadState}
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
        searchFocusSignal={globalSearchFocusSignal}
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
          onRenameSession={handleRenameSession}
          onViewSystemPrompt={loadSystemPrompt}
          systemPromptLoading={systemPromptLoading}
          onCommandAction={handleCommandAction}
          onOpenSearch={openSessionSearch}
          searchOpen={searchBarOpen}
          effectiveWorkingDir={effectiveWorkingDir}
          workingDirSource={workingDirSource}
          project={
            session.currentSessionId
              ? (projects.find((p) => p.id === currentSessionMeta?.projectId) ?? null)
              : null
          }
          onOpenProjectSettings={openProjectOverview}
          onOpenHandover={(sid) => setHandoverSessionId(sid)}
          agents={session.agents}
          onChangeAgent={handleChangeAgent}
          sidebarCollapsed={sidebarCollapsed}
          onExpandSidebar={() => setSidebarCollapsed(false)}
          displayMode={displayMode}
          onDisplayModeChange={handleDisplayModeChange}
          incognitoEnabled={incognitoEnabled}
          incognitoDisabledReason={incognitoDisabledReason}
          onIncognitoChange={handleIncognitoChange}
          onToggleFilesPanel={
            effectiveWorkingDir ? () => setShowFilesPanel((p) => !p) : undefined
          }
          filesPanelOpen={showFilesPanel}
        />

        <div className="flex-1 flex min-h-0 overflow-hidden">
          <div className="relative flex-1 flex flex-col min-w-0">
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
              executionState={
                session.currentSessionId
                  ? (stream.executionStateBySession.get(session.currentSessionId) ?? null)
                  : null
              }
              agents={session.agents}
              hasMore={session.hasMore}
              loadingMore={session.loadingMore}
              onLoadMore={session.handleLoadMore}
              hasMoreAfter={session.hasMoreAfter}
              loadingMoreAfter={session.loadingMoreAfter}
              onLoadMoreAfter={session.handleLoadMoreAfter}
              onResetToLatest={session.resetToLatest}
              sessionId={session.currentSessionId}
              incognito={incognitoEnabled}
              pendingScrollIntent={session.pendingScrollIntent}
              onScrollTargetHandled={session.clearPendingScrollIntent}
              pendingQuestionGroup={planMode.pendingQuestionGroup}
              onQuestionSubmitted={() => planMode.setPendingQuestionGroup(null)}
              planCardData={planMode.planCardInfo ? { title: planMode.planCardInfo.title } : null}
              planState={planMode.planState}
              onOpenPlanPanel={planMode.openPlanPanel}
              onApprovePlan={handlePlanApprove}
              onExitPlan={planMode.exitPlanMode}
              planSubagentRunning={planMode.planSubagentRunning}
              onSwitchModel={handleMessageSwitchModel}
              onViewSystemPrompt={loadSystemPrompt}
              onOpenDashboardTab={onOpenDashboardTab}
              onSwitchSession={(sid) => {
                void session.handleSwitchSession(sid)
              }}
              onOpenDiff={diffPanel.openDiff}
              onResume={(message) => {
                void stream.handleSend(message)
              }}
              displayMode={displayMode}
            />

            {/* Memory extraction toast — absolute-positioned above ChatInput
             * so it doesn't shrink the MessageList scroll container when it
             * appears/disappears. */}
            {!isCronSession && !isSubagentSession && (
              <div
                className={cn(
                  "relative",
                  emptySessionInputHero &&
                    "absolute inset-x-0 top-[48%] z-20 flex -translate-y-1/2 justify-center px-5 sm:px-8",
                )}
              >
                {memoryToast && (
                  <div
                    className={cn(
                      "absolute bottom-full mb-2 flex items-center gap-2 px-3 py-1.5 rounded-lg bg-secondary/50 text-xs text-muted-foreground animate-in fade-in slide-in-from-bottom-2 duration-300 z-10",
                      emptySessionInputHero
                        ? "inset-x-5 mx-auto max-w-[920px] sm:inset-x-8"
                        : "left-0 right-0 mx-4",
                    )}
                  >
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

                <div className={cn(emptySessionInputHero && "w-full max-w-[920px]")}>
                  <ChatInput
                    input={stream.input}
                    onInputChange={stream.setInput}
                    onSend={() => stream.handleSend()}
                    loading={session.loading}
                    availableModels={availableModels}
                    activeModel={activeModel}
                    reasoningEffort={reasoningEffort}
                    onModelChange={handleManualModelChange}
                    onEffortChange={handleSessionEffortChange}
                    attachedFiles={stream.attachedFiles}
                    onAttachFiles={(files) =>
                      stream.setAttachedFiles((prev) => [...prev, ...files])
                    }
                    onRemoveFile={(index) =>
                      stream.setAttachedFiles((prev) => prev.filter((_, i) => i !== index))
                    }
                    pendingQuotes={stream.pendingQuotes}
                    onRemoveQuote={(index) => {
                      stream.setPendingQuotes((prev) => prev.filter((_, i) => i !== index))
                      setRevealFile(null) // dropping a quote clears its reveal highlight
                    }}
                    onJumpToQuote={handleQuoteJump}
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
                    workingDir={session.currentSessionId ? effectiveWorkingDir : draftWorkingDir}
                    workingDirInherited={
                      session.currentSessionId ? workingDirSource === "project" : false
                    }
                    workingDirSaving={workingDirSaving}
                    onWorkingDirChange={
                      currentSessionMeta?.projectId ? undefined : handleWorkingDirChange
                    }
                    planState={planMode.planState}
                    onEnterPlanMode={planMode.enterPlanMode}
                    onExitPlanMode={planMode.exitPlanMode}
                    onTogglePlanPanel={() => planMode.setShowPanel((p) => !p)}
                    taskProgressSnapshot={taskProgressSnapshot}
                    executionState={
                      session.currentSessionId
                        ? (stream.executionStateBySession.get(session.currentSessionId) ?? null)
                        : null
                    }
                    hero={emptySessionInputHero}
                  />
                </div>
              </div>
            )}
          </div>

          {hasOpenExclusiveRightPanel && (
            <div className="flex h-full shrink-0 items-start py-3 pl-1">
              <div className="flex flex-col gap-1 rounded-lg border border-border-soft bg-surface-panel/95 p-1 shadow-panel">
                {openExclusiveRightPanels.length > 1 &&
                  openExclusiveRightPanels.map((panel) => {
                    const PanelIcon = EXCLUSIVE_RIGHT_PANEL_ICONS[panel]
                    const label = getRightPanelLabel(panel)
                    const isActive = renderedExclusiveRightPanel === panel

                    return (
                      <IconTip key={panel} label={label} side="left">
                        <button
                          type="button"
                          aria-label={label}
                          aria-pressed={isActive}
                          onClick={() => {
                            setActiveExclusiveRightPanel(panel)
                            setRightPanelCollapsed(false)
                          }}
                          className={cn(
                            "flex h-8 w-8 items-center justify-center rounded-md transition-colors",
                            isActive
                              ? "bg-secondary text-foreground shadow-sm"
                              : "text-muted-foreground hover:bg-secondary/70 hover:text-foreground",
                          )}
                        >
                          <PanelIcon className="h-4 w-4" />
                        </button>
                      </IconTip>
                    )
                  })}
                {openExclusiveRightPanels.length > 1 && (
                  <div className="mx-1 my-0.5 h-px bg-border-soft" />
                )}
                <IconTip label={rightPanelToggleLabel} side="left">
                  <button
                    type="button"
                    aria-label={rightPanelToggleLabel}
                    aria-expanded={!rightPanelCollapsed}
                    onClick={() => setRightPanelCollapsed((collapsed) => !collapsed)}
                    className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/70 hover:text-foreground"
                  >
                    {rightPanelCollapsed ? (
                      <ChevronLeft className="h-4 w-4" />
                    ) : (
                      <ChevronRight className="h-4 w-4" />
                    )}
                  </button>
                </IconTip>
              </div>
            </div>
          )}

          {/* Diff panel (right side, selected from the shared panel rail) */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "diff" && (
            <RightPanelShell
              width={rightPanelWidth}
              onWidthChange={setRightPanelWidth}
              resizeLabel={t("diffPanel.resizePanel", "Resize diff panel")}
              maxWidth={860}
            >
              <DiffPanel
                changes={diffPanel.activeChanges}
                activeIndex={diffPanel.activeIndex}
                onActiveIndexChange={diffPanel.setActiveIndex}
                onClose={diffPanel.closeDiff}
                embedded
              />
            </RightPanelShell>
          )}

          {/* Plan workspace (right side, integrated under the shared title bar) */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "plan" && (
            <RightPanelShell
              width={rightPanelWidth}
              onWidthChange={setRightPanelWidth}
              resizeLabel={t("planMode.resizePanel", "Resize plan panel")}
              maxWidth={860}
            >
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
            </RightPanelShell>
          )}

          {/* Project file browser (right side, scoped to the working dir) */}
          {/* File browser panel — permanently mounted (like CanvasPanel) and
              toggled via `visible`, so a popped-out window survives panel
              switches / collapses. */}
          <FileBrowserPanel
            scope="session"
            scopeId={session.currentSessionId}
            rootPath={effectiveWorkingDir}
            sessionId={session.currentSessionId}
            visible={shouldRenderRightPanelContent && renderedExclusiveRightPanel === "files"}
            panelWidth={rightPanelWidth}
            onPanelWidthChange={setRightPanelWidth}
            onQuote={handleFileQuote}
            revealFile={revealFile}
            onClose={() => setShowFilesPanel(false)}
          />

          {/* Canvas Preview Panel */}
          <CanvasPanel
            panelWidth={rightPanelWidth}
            onPanelWidthChange={setRightPanelWidth}
            currentSessionId={currentSessionId}
            onOpenChange={setCanvasPanelOpen}
            visible={
              shouldRenderRightPanelContent && renderedExclusiveRightPanel === "canvas"
            }
          />

          {/* Browser live-mirror panel — open on first `browser:frame` push,
              close-only by user, then switchable from the shared panel rail. */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "browser" && (
            <BrowserPanel
              panelWidth={rightPanelWidth}
              onPanelWidthChange={setRightPanelWidth}
              onClose={() => {
                browserPanelDismissedRef.current = true
                setShowBrowserPanel(false)
              }}
            />
          )}

          {/* Mac Control live-mirror panel — opens on tool-produced managed
              screenshot frames; panel polling frames only refresh an already
              open panel and must not re-open after a session switch. */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "mac-control" && (
            <MacControlPanel
              panelWidth={rightPanelWidth}
              onPanelWidthChange={setRightPanelWidth}
              onClose={() => {
                macControlPanelDismissedRef.current = true
                setShowMacControlPanel(false)
              }}
            />
          )}

          {/* Team Panel */}
          {shouldRenderRightPanelContent &&
            renderedExclusiveRightPanel === "team" &&
            activeTeamId && (
              <TeamPanel
                teamId={activeTeamId}
                panelWidth={rightPanelWidth}
                onPanelWidthChange={setRightPanelWidth}
                onClose={() => setShowTeamPanel(false)}
                onSwitchSession={session.handleSwitchSession}
              />
            )}
        </div>
      </div>

      <HandoverDialog
        open={!!handoverSessionId}
        onOpenChange={(o) => {
          if (!o) setHandoverSessionId(null)
        }}
        sessionId={handoverSessionId}
      />
    </>
  )
}

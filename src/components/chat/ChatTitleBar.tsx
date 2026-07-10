import { useState, useRef, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { useAppVersion } from "@/lib/appMeta"
import { basename } from "@/lib/path"
import { IconTip } from "@/components/ui/tooltip"
import { Input } from "@/components/ui/input"
import { FloatingMenu } from "@/components/ui/floating-menu"
import {
  Copy,
  BarChart3,
  Pencil,
  Check,
  X,
  FileText,
  Folder,
  FolderCheck,
  FolderOpen,
  Layers,
  LayoutDashboard,
  Loader2,
  Search,
  Send,
  Ghost,
  Share2,
  PanelLeftDashed,
  PanelRight,
  PanelRightDashed,
  type LucideIcon,
} from "lucide-react"
import { ExportSessionDialog } from "@/components/chat/export/ExportSessionDialog"
import ChannelIcon from "@/components/common/ChannelIcon"
import { formatCacheUsageDisplay, formatCompactTokenCount } from "./cacheUsageDisplay"
import {
  computeContextUsage,
  contextUsageBarClass,
  formatMessageTime,
  type ContextUsageInfo,
} from "./chatUtils"
import {
  type CompactResult,
  compactResultMessage,
  computeCacheStats,
  resolveCurrentModel,
  runViewContext,
} from "./sessionStatus"
import { INCOGNITO_BADGE_LABEL_CLASSES } from "./input/incognitoStyles"
import IncognitoToggle, { type IncognitoDisabledReason } from "./input/IncognitoToggle"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import AgentSwitcher from "./AgentSwitcher"
import { PROJECT_TEXT_COLOR_MAP } from "./project/colors"
import {
  DEFAULT_SIDEBAR_DISPLAY_MODE,
  normalizeSidebarDisplayMode,
  type SidebarDisplayMode,
} from "./sidebar/types"
import type {
  Message,
  AvailableModel,
  ActiveModel,
  SessionMeta,
  AgentSummaryForSidebar,
} from "@/types/chat"
import type { ProjectMeta } from "@/types/project"

interface RightPanelTitleBarItem {
  id: string
  label: string
  icon: LucideIcon
}

interface ChatTitleBarProps {
  agentName: string
  currentAgentId: string
  currentSessionId: string | null
  sessions: SessionMeta[]
  messages: Message[]
  contextUsageOverride?: ContextUsageInfo | null
  activeModel: ActiveModel | null
  availableModels: AvailableModel[]
  reasoningEffort: string
  loading: boolean
  compacting: boolean
  onCompactContext?: () => Promise<CompactResult | null>
  onRenameSession?: (sessionId: string, title: string) => void
  onViewSystemPrompt?: () => void
  systemPromptLoading?: boolean
  /**
   * Dispatches a slash command action back to ChatScreen's handler.
   * Used by the "View context" popover button to trigger `/context`
   * without going through the text input.
   */
  onCommandAction?: (result: import("@/components/chat/slash-commands/types").CommandResult) => void
  /** Opens or refocuses the in-session "find in page" search bar. */
  onOpenSearch?: () => void
  /** Whether the in-session search bar is currently open (controls active styling). */
  searchOpen?: boolean
  /**
   * Currently effective working directory for this session — session-level
   * value if set, otherwise the parent project's default. `null` when neither
   * is set, in which case the chip is hidden.
   */
  effectiveWorkingDir?: string | null
  /** Source of the effective path; only read when `effectiveWorkingDir` is set. */
  workingDirSource?: "session" | "project"
  /** Project this session belongs to. Surface a chip linking to its settings. */
  project?: ProjectMeta | null
  /** Triggered by the project chip click — opens the settings sheet. */
  onOpenProjectSettings?: (project: ProjectMeta) => void
  /** Opens the GUI handover dialog for the current session. */
  onOpenHandover?: (sessionId: string) => void
  /** Available agents for the title-bar agent switcher. */
  agents?: AgentSummaryForSidebar[]
  /**
   * Triggered when the user picks a different agent from the title-bar
   * dropdown. Only invoked while `messages.length === 0` — once messages
   * exist, the dropdown is hidden.
   */
  onChangeAgent?: (agentId: string) => void
  /** Whether the session sidebar is currently collapsed. */
  sidebarCollapsed?: boolean
  /** Expands the session sidebar from the title bar. */
  onExpandSidebar?: () => void
  /** Draft/new-session incognito toggle, surfaced in the title bar. */
  incognitoEnabled?: boolean
  incognitoSaving?: boolean
  incognitoDisabledReason?: IncognitoDisabledReason
  onIncognitoChange?: (enabled: boolean) => void
  /** Toggle the right-side file browser. Undefined when no working directory. */
  onToggleFilesPanel?: () => void
  /** Whether the file browser panel is currently open (controls active styling). */
  filesPanelOpen?: boolean
  /** Toggle the right-side workspace panel (tasks / files / sources). */
  onToggleWorkspacePanel?: () => void
  /** Whether the workspace panel is currently open (controls active styling). */
  workspacePanelOpen?: boolean
  /** Toggle the right-side background-jobs panel (R4). */
  onToggleBackgroundJobsPanel?: () => void
  /** Whether the background-jobs panel is currently open (controls active styling). */
  backgroundJobsPanelOpen?: boolean
  /** Count of running background jobs in this session (drives the header badge). */
  backgroundJobsRunningCount?: number
  /** Open right-side panels available for switching/collapsing. */
  rightPanels?: RightPanelTitleBarItem[]
  /** Active right-side panel id. */
  activeRightPanelId?: string | null
  /** Whether the active right-side panel is collapsed. */
  rightPanelCollapsed?: boolean
  /** Switch to an already-open right-side panel. */
  onSelectRightPanel?: (panelId: string) => void
  /** Collapse/expand the active right-side panel. */
  onToggleRightPanelCollapsed?: () => void
}

export default function ChatTitleBar({
  agentName,
  currentAgentId,
  currentSessionId,
  sessions,
  messages,
  contextUsageOverride,
  activeModel,
  availableModels,
  reasoningEffort,
  loading,
  compacting,
  onCompactContext,
  onRenameSession,
  onViewSystemPrompt,
  systemPromptLoading,
  onCommandAction,
  onOpenSearch,
  searchOpen,
  effectiveWorkingDir,
  workingDirSource,
  project,
  onOpenProjectSettings,
  onOpenHandover,
  agents = [],
  onChangeAgent,
  sidebarCollapsed,
  onExpandSidebar,
  incognitoEnabled = false,
  incognitoSaving = false,
  incognitoDisabledReason,
  onIncognitoChange,
  onToggleFilesPanel,
  filesPanelOpen = false,
  onToggleWorkspacePanel,
  workspacePanelOpen = false,
  onToggleBackgroundJobsPanel,
  backgroundJobsPanelOpen = false,
  backgroundJobsRunningCount = 0,
  rightPanels = [],
  activeRightPanelId,
  rightPanelCollapsed = false,
  onSelectRightPanel,
  onToggleRightPanelCollapsed,
}: ChatTitleBarProps) {
  const { t } = useTranslation()
  const appVersion = useAppVersion()
  const [showStatus, setShowStatus] = useState(false)
  const statusRef = useRef<HTMLDivElement>(null)

  // Compact result toast
  const [compactToast, setCompactToast] = useState<{ success: boolean; message: string } | null>(
    null,
  )
  const compactToastTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Session ID copy feedback
  const [sessionIdCopied, setSessionIdCopied] = useState(false)
  const sessionIdCopiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Inline title editing
  const [editingTitle, setEditingTitle] = useState(false)
  const [titleValue, setTitleValue] = useState("")
  const [exportOpen, setExportOpen] = useState(false)
  const [sidebarDisplayMode, setSidebarDisplayMode] = useState<SidebarDisplayMode>(
    DEFAULT_SIDEBAR_DISPLAY_MODE,
  )
  const titleInputRef = useRef<HTMLInputElement>(null)

  const currentSession = currentSessionId ? sessions.find((s) => s.id === currentSessionId) : null
  const sessionTitle = currentSession?.title || ""
  const compactTitleAgent = sidebarDisplayMode === "compact"
  const projectFolderColorClass =
    (project?.color && PROJECT_TEXT_COLOR_MAP[project.color]) || "text-muted-foreground/70"

  const startEditTitle = useCallback(() => {
    setTitleValue(sessionTitle || t("chat.newChat") || "")
    setEditingTitle(true)
    setTimeout(() => {
      titleInputRef.current?.focus()
      titleInputRef.current?.select()
    }, 0)
  }, [sessionTitle, t])

  const commitTitle = useCallback(() => {
    if (currentSessionId && titleValue.trim() && onRenameSession) {
      onRenameSession(currentSessionId, titleValue.trim())
    }
    setEditingTitle(false)
  }, [currentSessionId, titleValue, onRenameSession])

  const cancelEditTitle = useCallback(() => {
    setEditingTitle(false)
  }, [])

  // Close status popover on outside click
  useEffect(() => {
    if (!showStatus) return
    const handler = (e: MouseEvent) => {
      if (statusRef.current && !statusRef.current.contains(e.target as Node)) {
        setShowStatus(false)
      }
    }
    document.addEventListener("mousedown", handler)
    return () => document.removeEventListener("mousedown", handler)
  }, [showStatus])

  useEffect(() => {
    return () => {
      if (sessionIdCopiedTimer.current) clearTimeout(sessionIdCopiedTimer.current)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<string>("get_sidebar_display_mode")
      .then((mode) => {
        if (!cancelled) setSidebarDisplayMode(normalizeSidebarDisplayMode(mode))
      })
      .catch((err) => {
        logger.error(
          "ui",
          "ChatTitleBar::loadSidebarDisplayMode",
          "Failed to load sidebar display mode",
          err,
        )
      })

    const handleModeChanged = (event: Event) => {
      const detail = (event as CustomEvent<{ mode?: unknown }>).detail
      setSidebarDisplayMode(normalizeSidebarDisplayMode(detail?.mode))
    }
    window.addEventListener("sidebar-display-mode-changed", handleModeChanged)
    return () => {
      cancelled = true
      window.removeEventListener("sidebar-display-mode-changed", handleModeChanged)
    }
  }, [])

  const handleCopySessionId = useCallback(async () => {
    if (!currentSessionId) return
    try {
      await navigator.clipboard.writeText(currentSessionId)
    } catch (e) {
      logger.error("ui", "ChatTitleBar::copySessionId", "Copy failed", e)
      return
    }
    setSessionIdCopied(true)
    if (sessionIdCopiedTimer.current) clearTimeout(sessionIdCopiedTimer.current)
    sessionIdCopiedTimer.current = setTimeout(() => setSessionIdCopied(false), 1500)
  }, [currentSessionId])

  const currentModel = resolveCurrentModel(activeModel, availableModels)
  const activeRightPanel =
    rightPanels.find((panel) => panel.id === activeRightPanelId) ?? rightPanels[0] ?? null
  const rightPanelToggleLabel = rightPanelCollapsed
    ? t("chat.rightPanel.expand", "展开右侧面板")
    : t("chat.rightPanel.collapse", "收起右侧面板")
  const hasRightPanelControls =
    !!onToggleFilesPanel ||
    !!onToggleWorkspacePanel ||
    !!onToggleBackgroundJobsPanel ||
    (rightPanels.length > 0 && (rightPanels.length > 1 || !!onToggleRightPanelCollapsed))
  const workingDirChip = effectiveWorkingDir ? (
    <IconTip
      label={
        workingDirSource === "project"
          ? `${t("chat.workingDir.titleBarInherited")}: ${effectiveWorkingDir}`
          : `${t("chat.workingDir.titleBarSession")}: ${effectiveWorkingDir}`
      }
    >
      <span
        className={cn(
          "inline-flex items-center gap-1 shrink-0 text-[11px] px-1.5 py-0.5 rounded font-mono max-w-[260px]",
          workingDirSource === "project"
            ? "text-muted-foreground bg-muted/60"
            : "text-primary bg-primary/10",
        )}
      >
        <FolderCheck className="h-3 w-3 shrink-0" />
        <span className="truncate">{basename(effectiveWorkingDir)}</span>
      </span>
    </IconTip>
  ) : null
  const shouldShowWorkingDirChip = !project || workingDirSource === "session"
  const rightPanelControls = hasRightPanelControls ? (
    <div className="ml-1 flex items-center gap-0.5 border-l border-border-soft pl-1">
      {onToggleFilesPanel && (
        <IconTip label={t("fileBrowser.open", "Show files")}>
          <button
            type="button"
            className={cn(
              "flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/70 hover:text-foreground",
              filesPanelOpen && "text-foreground",
            )}
            aria-label={t("fileBrowser.open", "Show files")}
            aria-pressed={filesPanelOpen}
            onClick={onToggleFilesPanel}
          >
            <FolderOpen className="h-4 w-4" />
          </button>
        </IconTip>
      )}
      {onToggleWorkspacePanel && (
        <IconTip label={t("workspace.openPanel", "Open workspace")}>
          <button
            type="button"
            className={cn(
              "flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/70 hover:text-foreground",
              workspacePanelOpen && "text-foreground",
            )}
            aria-label={t("workspace.openPanel", "Open workspace")}
            aria-pressed={workspacePanelOpen}
            onClick={onToggleWorkspacePanel}
          >
            <LayoutDashboard className="h-4 w-4" />
          </button>
        </IconTip>
      )}
      {onToggleBackgroundJobsPanel && (
        <IconTip label={t("backgroundJobs.openPanel", "后台任务")}>
          <button
            type="button"
            className={cn(
              "relative flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary/70 hover:text-foreground",
              backgroundJobsPanelOpen && "text-foreground",
            )}
            aria-label={t("backgroundJobs.openPanel", "后台任务")}
            aria-pressed={backgroundJobsPanelOpen}
            onClick={onToggleBackgroundJobsPanel}
          >
            <Layers className="h-4 w-4" />
            {backgroundJobsRunningCount > 0 && (
              <span className="absolute -right-1 -top-1 z-10 flex h-[15px] min-w-[15px] items-center justify-center rounded-full border border-background bg-amber-500 px-0.5 text-[9px] font-semibold leading-none text-white tabular-nums">
                {backgroundJobsRunningCount > 99 ? "99+" : backgroundJobsRunningCount}
              </span>
            )}
          </button>
        </IconTip>
      )}
      {rightPanels.length > 1 && activeRightPanel && (
        <div
          className="flex h-7 max-w-[184px] items-center gap-0.5 overflow-x-auto rounded-lg bg-secondary/40 p-0.5"
          role="tablist"
          aria-label={t("chat.rightPanel.switch", "切换右侧面板")}
        >
          {rightPanels.map((panel) => {
            const PanelIcon = panel.icon
            const active = panel.id === activeRightPanel.id
            return (
              <IconTip key={panel.id} label={panel.label}>
                <button
                  type="button"
                  role="tab"
                  aria-selected={active}
                  aria-label={panel.label}
                  className={cn(
                    "flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-all hover:bg-background/80 hover:text-foreground",
                    active && "bg-background text-foreground shadow-sm ring-1 ring-border/60",
                  )}
                  onClick={() => onSelectRightPanel?.(panel.id)}
                >
                  <PanelIcon className="h-3.5 w-3.5" />
                </button>
              </IconTip>
            )
          })}
        </div>
      )}
      {onToggleRightPanelCollapsed && (
        <IconTip label={rightPanelToggleLabel}>
          <button
            type="button"
            className={cn(
              "flex h-7 w-7 items-center justify-center rounded-md transition-colors hover:bg-secondary/70 hover:text-foreground",
              rightPanelCollapsed ? "text-muted-foreground" : "text-foreground",
            )}
            aria-label={rightPanelToggleLabel}
            aria-expanded={!rightPanelCollapsed}
            onClick={onToggleRightPanelCollapsed}
          >
            {rightPanelCollapsed ? (
              <PanelRightDashed className="h-4 w-4" />
            ) : (
              <PanelRight className="h-4 w-4" />
            )}
          </button>
        </IconTip>
      )}
    </div>
  ) : null

  return (
    <div
      className="h-10 flex items-end justify-between px-4 bg-background shrink-0"
      data-tauri-drag-region
    >
      <div className="flex items-end gap-2 min-w-0 pb-1.5">
        {sidebarCollapsed && onExpandSidebar && (
          <IconTip label={t("chat.expandSidebar")}>
            <button
              className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-secondary/70 hover:text-foreground"
              aria-label={t("chat.expandSidebar")}
              onClick={onExpandSidebar}
            >
              <PanelLeftDashed className="h-4 w-4" />
            </button>
          </IconTip>
        )}
        {project && (
          <>
            <div className="inline-flex items-center gap-1 shrink-0 min-w-0">
              <button
                onClick={() => onOpenProjectSettings?.(project)}
                className="inline-flex items-center gap-1 shrink-0 text-[12px] px-1.5 py-0.5 rounded hover:bg-accent/40 transition-colors"
                title={project.description ?? project.name}
              >
                <Folder className={cn("h-3.5 w-3.5 shrink-0", projectFolderColorClass)} />
                <span className="truncate max-w-[140px] text-foreground/80">{project.name}</span>
              </button>
            </div>
            <span className="text-muted-foreground/40 text-sm shrink-0">/</span>
          </>
        )}
        <AgentSwitcher
          agents={agents}
          currentAgentId={currentAgentId}
          agentName={agentName || t("chat.mainAgent")}
          compactLabel={compactTitleAgent}
          // Allow switching only before any messages exist — system prompt
          // and history are pinned to the agent once a message is sent.
          disabled={messages.length > 0 || !onChangeAgent}
          onSelect={(agentId) => onChangeAgent?.(agentId)}
        />
        {currentSessionId && (
          <>
            <span className="text-muted-foreground/40 text-sm shrink-0">/</span>
            {editingTitle ? (
              <div className="flex items-center gap-1 min-w-0">
                <Input
                  ref={titleInputRef}
                  className="h-auto w-auto rounded-none border-0 border-b border-primary bg-transparent px-0 py-0 shadow-none text-sm text-foreground/80 outline-none min-w-[80px] max-w-[300px] focus-visible:border-primary"
                  value={titleValue}
                  onChange={(e) => setTitleValue(e.target.value)}
                  onBlur={commitTitle}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault()
                      commitTitle()
                    } else if (e.key === "Escape") {
                      e.preventDefault()
                      cancelEditTitle()
                    }
                  }}
                  placeholder={t("chat.renameSessionPlaceholder")}
                />
              </div>
            ) : (
              <button
                className="group flex items-center gap-1 min-w-0 text-sm text-foreground/60 hover:text-foreground transition-colors truncate"
                onClick={startEditTitle}
              >
                <span className="truncate max-w-[300px]">{sessionTitle || t("chat.newChat")}</span>
                <Pencil className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-60 transition-opacity" />
              </button>
            )}
            {currentSession?.channelInfo && (
              <span className="inline-flex items-center gap-1 shrink-0 text-[11px] text-blue-500 bg-blue-500/10 px-1.5 py-0.5 rounded">
                <ChannelIcon channelId={currentSession.channelInfo.channelId} />
                {currentSession.channelInfo.channelId}
                {currentSession.channelInfo.senderName && (
                  <span className="text-blue-400">· {currentSession.channelInfo.senderName}</span>
                )}
              </span>
            )}
            {currentSession?.incognito && (
              <span className={INCOGNITO_BADGE_LABEL_CLASSES}>
                <Ghost className="h-3 w-3" strokeWidth={1.75} />
                {t("chat.incognito")}
              </span>
            )}
            {shouldShowWorkingDirChip && workingDirChip}
          </>
        )}
      </div>
      <div className="flex items-end gap-1">
        {!currentSessionId && onIncognitoChange && (
          <IncognitoToggle
            sessionId={null}
            enabled={incognitoEnabled}
            saving={incognitoSaving}
            disabledReason={incognitoDisabledReason}
            variant="titlebar"
            showLabel={false}
            onChange={onIncognitoChange}
          />
        )}
        {/* In-session Search Button */}
        {currentSessionId && onOpenSearch && (
          <IconTip label={t("chat.sessionSearch")}>
            <button
              className={cn(
                "pb-1.5 text-muted-foreground hover:text-foreground transition-colors",
                searchOpen && "text-foreground",
              )}
              onClick={onOpenSearch}
            >
              <Search className="h-4 w-4" />
            </button>
          </IconTip>
        )}
        {/* Session Status Button */}
        <div className="relative" ref={statusRef}>
          <IconTip label={t("chat.sessionStatus")}>
            <button
              className={cn(
                "pb-1.5 text-muted-foreground hover:text-foreground transition-colors",
                showStatus && "text-foreground",
              )}
              onClick={() => setShowStatus((v) => !v)}
            >
              <BarChart3 className="h-4 w-4" />
            </button>
          </IconTip>
          <FloatingMenu
            open={showStatus}
            positionClassName="top-full right-0 mt-1.5"
            originClassName="origin-top-right"
            className="ha-menu-from-top min-w-[260px] rounded-xl border-border bg-popover p-3.5 shadow-xl"
            onEscapeKeyDown={() => setShowStatus(false)}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="space-y-2 text-xs">
              {/* App version */}
              <div className="flex items-center justify-between gap-2">
                <span className="text-muted-foreground">🖥️ Hope Agent</span>
                <span className="font-medium text-foreground tabular-nums">v{appVersion}</span>
              </div>
              <div className="border-t border-border" />
              {/* Model + Auth */}
              {(() => {
                const modelLabel = currentModel
                  ? `${currentModel.providerName}/${currentModel.modelName || currentModel.modelId}`
                  : activeModel?.modelId || "—"
                const apiType = currentModel?.apiType || "—"
                const authLabel = apiType === "codex" ? "oauth" : "api-key"
                return (
                  <>
                    <div className="flex items-start gap-2">
                      <span className="text-muted-foreground shrink-0">
                        🧠 {t("chat.statusModel")}
                      </span>
                      <span className="font-medium text-foreground text-right ml-auto">
                        {modelLabel}
                      </span>
                    </div>
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">🔑 {t("chat.statusAuth")}</span>
                      <span className="font-medium text-foreground">{authLabel}</span>
                    </div>
                  </>
                )
              })()}
              {/* Context window usage. See `getContextUsageTokens` for the
               *  cumulative-vs-last-round rule. */}
              {(() => {
                const usage = contextUsageOverride ?? (currentModel
                  ? computeContextUsage(messages, currentModel.contextWindow)
                  : null)
                if (!usage) return null
                const { usedTokens, usedK, ctxK, pct } = usage
                const barColor = contextUsageBarClass(pct)
                return (
                  <div className="space-y-1.5">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">📚 {t("chat.statusContext")}</span>
                      <span className="font-medium text-foreground tabular-nums">
                        {usedK}k/{ctxK}k ({pct}%)
                      </span>
                    </div>
                    <div className="h-1.5 w-full bg-secondary rounded-full overflow-hidden">
                      <div
                        className={`h-full rounded-full transition-all duration-300 ${barColor}`}
                        style={{ width: `${Math.min(pct, 100)}%` }}
                      />
                    </div>
                    {currentSessionId && usedTokens > 0 && (
                      <button
                        className="w-full mt-1 px-2 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:text-foreground hover:bg-secondary/60 transition-colors disabled:opacity-50"
                        disabled={compacting || loading}
                        onClick={async () => {
                          if (!currentSessionId) return
                          try {
                            const result = await onCompactContext?.()
                            if (!result) return
                            if (compactToastTimer.current) clearTimeout(compactToastTimer.current)
                            setCompactToast({ success: true, message: compactResultMessage(t, result) })
                            compactToastTimer.current = setTimeout(
                              () => setCompactToast(null),
                              3000,
                            )
                            if (result.messagesAffected > 0) {
                              setShowStatus(false)
                            }
                          } catch (e) {
                            logger.error("ui", "ChatTitleBar::compact", "Compact failed", e)
                            if (compactToastTimer.current) clearTimeout(compactToastTimer.current)
                            setCompactToast({ success: false, message: t("chat.compactFailed") })
                            compactToastTimer.current = setTimeout(
                              () => setCompactToast(null),
                              3000,
                            )
                          }
                        }}
                      >
                        {compacting ? t("chat.compacting") : t("chat.compactNow")}
                      </button>
                    )}
                    {/* View context breakdown */}
                    <button
                      className="w-full mt-1 px-2 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:text-foreground hover:bg-secondary/60 transition-colors flex items-center justify-center gap-1"
                      onClick={async () => {
                        if (!currentSessionId) return
                        try {
                          const result = await runViewContext(currentSessionId, currentAgentId)
                          setShowStatus(false)
                          onCommandAction?.(result)
                        } catch (e) {
                          logger.error("ui", "ChatTitleBar::viewContext", "View context failed", e)
                        }
                      }}
                    >
                      <BarChart3 className="h-3 w-3" />
                      {t("chat.viewContext", "View context")}
                    </button>
                  </div>
                )
              })()}
              {/* Cache info (Anthropic) */}
              {(() => {
                const cache = computeCacheStats(messages)
                if (!cache) return null
                const { created, read, lastInput } = cache
                return (
                  <div className="space-y-1">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">
                        🗄️ {t("chat.statusCache")} · {t("chat.statusCumulative")}
                      </span>
                      <span className="font-medium text-foreground tabular-nums">
                        {formatCacheUsageDisplay({
                          created,
                          read,
                          writeLabel: t("chat.statusCacheWrite"),
                          hitLabel: t("chat.statusCacheHit"),
                        })}
                      </span>
                    </div>
                    {lastInput != null && (
                      <div className="flex items-center justify-between gap-2 text-[11px]">
                        <span className="text-muted-foreground">
                          {t("chat.lastRoundInputTokens")}
                        </span>
                        <span className="font-medium text-foreground tabular-nums">
                          {formatCompactTokenCount(lastInput)}
                        </span>
                      </div>
                    )}
                  </div>
                )
              })()}
              <div className="border-t border-border" />
              {/* Agent */}
              <div className="flex items-center justify-between gap-2">
                <span className="text-muted-foreground">🤖 {t("chat.statusAgent")}</span>
                <span className="font-medium text-foreground">
                  {agentName || t("chat.mainAgent")}
                </span>
              </div>
              {/* Session */}
              <div className="flex items-start gap-2">
                <span className="text-muted-foreground shrink-0">🧵 {t("chat.statusSession")}</span>
                <span className="font-medium text-foreground text-right ml-auto truncate max-w-[160px]">
                  {currentSessionId
                    ? (() => {
                        const sess = sessions.find((s) => s.id === currentSessionId)
                        return sess?.title || currentSessionId.slice(0, 8)
                      })()
                    : t("chat.statusNewSession")}
                </span>
              </div>
              {/* Session ID */}
              {currentSessionId && (
                <div className="flex items-center justify-between gap-2 overflow-hidden">
                  <span className="text-muted-foreground shrink-0">
                    🆔 {t("chat.statusSessionId")}
                  </span>
                  <IconTip label={sessionIdCopied ? t("chat.copied") : t("chat.copy")}>
                    <div
                      role="button"
                      tabIndex={0}
                      className={cn(
                        "flex items-center gap-1.5 ml-auto overflow-hidden cursor-pointer transition-colors group",
                        sessionIdCopied
                          ? "text-green-600 dark:text-green-500"
                          : "text-muted-foreground/80 hover:text-foreground",
                      )}
                      onClick={handleCopySessionId}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault()
                          handleCopySessionId()
                        }
                      }}
                    >
                      <span className="font-mono text-[11px] truncate select-all">
                        {currentSessionId}
                      </span>
                      {sessionIdCopied ? (
                        <Check className="h-3.5 w-3.5 shrink-0" />
                      ) : (
                        <Copy className="h-3.5 w-3.5 shrink-0 opacity-70 group-hover:opacity-100 transition-opacity" />
                      )}
                    </div>
                  </IconTip>
                </div>
              )}
              {/* Message count */}
              <div className="flex items-center justify-between gap-2">
                <span className="text-muted-foreground">
                  📊 {t("chat.statusMessages", { count: messages.length })}
                </span>
              </div>
              <div className="border-t border-border" />
              {/* Runtime: Thinking */}
              <div className="flex items-center justify-between gap-2">
                <span className="text-muted-foreground">⚙️ {t("chat.statusThinking")}</span>
                <span className="font-medium text-foreground">
                  {t(`effort.${reasoningEffort}`)}
                </span>
              </div>
              {/* Updated */}
              {currentSessionId &&
                (() => {
                  const sess = sessions.find((s) => s.id === currentSessionId)
                  if (!sess) return null
                  return (
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">🕒 {t("chat.statusUpdated")}</span>
                      <span className="font-medium text-foreground tabular-nums">
                        {formatMessageTime(sess.updatedAt)}
                      </span>
                    </div>
                  )
                })()}
              {/* View System Prompt */}
              {onViewSystemPrompt && (
                <>
                  <div className="border-t border-border" />
                  <button
                    className="w-full px-2 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:text-foreground hover:bg-secondary/60 transition-colors disabled:opacity-50 flex items-center justify-center gap-1.5"
                    disabled={systemPromptLoading}
                    onClick={() => {
                      onViewSystemPrompt()
                      setShowStatus(false)
                    }}
                  >
                    {systemPromptLoading ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      <FileText className="h-3 w-3" />
                    )}
                    {t("chat.viewSystemPrompt")}
                  </button>
                </>
              )}
            </div>
          </FloatingMenu>
          {compactToast && (
            <div
              className={cn(
                "absolute top-full right-0 mt-1.5 z-50 whitespace-nowrap rounded-lg border px-2.5 py-1.5 text-xs shadow-lg animate-in fade-in slide-in-from-top-1 duration-200",
                compactToast.success
                  ? "border-border bg-popover text-popover-foreground"
                  : "border-destructive/30 bg-destructive/10 text-destructive",
              )}
            >
              <div className="flex items-center gap-1.5">
                {compactToast.success ? (
                  <Check className="h-3 w-3 text-green-500" />
                ) : (
                  <X className="h-3 w-3" />
                )}
                {compactToast.message}
              </div>
            </div>
          )}
        </div>
        {/* Export Button — open the export-conversation dialog. */}
        {currentSessionId && (
          <IconTip label={t("chat.exportSession.menuItem")}>
            <button
              className="pb-1.5 text-muted-foreground hover:text-foreground transition-colors"
              onClick={() => setExportOpen(true)}
            >
              <Share2 className="h-4 w-4" />
            </button>
          </IconTip>
        )}
        {/* Handover Button — push the current session to an IM chat. */}
        {onOpenHandover && currentSessionId && (
          <IconTip label={t("chat.handover.button")}>
            <button
              className="pb-1.5 text-muted-foreground hover:text-foreground transition-colors"
              onClick={() => onOpenHandover(currentSessionId)}
            >
              <Send className="h-4 w-4" />
            </button>
          </IconTip>
        )}
        {rightPanelControls}
      </div>
      {currentSessionId && exportOpen && (
        <ExportSessionDialog
          open={exportOpen}
          onOpenChange={setExportOpen}
          sessionId={currentSessionId}
          sessionTitle={sessions.find((s) => s.id === currentSessionId)?.title ?? null}
        />
      )}
    </div>
  )
}

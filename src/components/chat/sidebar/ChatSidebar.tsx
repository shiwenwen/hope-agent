import { useState, useRef, useEffect, useCallback } from "react"
import { flushSync } from "react-dom"
import { useTranslation } from "react-i18next"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { IconTip } from "@/components/ui/tooltip"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import { Bot, ListCollapse, Loader2, MessageSquarePlus, PanelLeft, Rows3, Search, X } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { SessionSearchResult } from "@/types/chat"
import {
  CHAT_SIDEBAR_MAX_WIDTH,
  CHAT_SIDEBAR_MIN_WIDTH,
  DEFAULT_SIDEBAR_DISPLAY_MODE,
  type ChatSidebarProps,
  type SidebarDisplayMode,
  type SessionFilterType,
  normalizeSidebarDisplayMode,
} from "./types"
import { sortSessionSearchResults } from "../chatUtils"
import { SEARCH_LIMIT } from "../hooks/constants"
import AgentSection from "./AgentSection"
import SessionList from "./SessionList"
import ProjectSection from "../project/ProjectSection"

const AGENTS_EXPANDED_STORAGE_KEY = "hope.chatSidebarAgentsExpanded"
const PROJECTS_EXPANDED_STORAGE_KEY = "hope.chatSidebarProjectsExpanded"

function readStoredBoolean(key: string, fallback: boolean): boolean {
  try {
    if (typeof window === "undefined") return fallback
    const raw = window.localStorage.getItem(key)
    if (raw === null) return fallback
    return raw === "true"
  } catch {
    return fallback
  }
}

function writeStoredBoolean(key: string, value: boolean) {
  try {
    window.localStorage.setItem(key, String(value))
  } catch {
    // localStorage may be unavailable in restricted browser modes.
  }
}

export default function ChatSidebar({
  sessions,
  agents,
  projects = [],
  projectsLoading = false,
  currentSessionId,
  loadingSessionIds,
  sessionsLoading = false,
  panelWidth,
  sidebarCollapsed,
  onPanelWidthChange,
  onSidebarCollapsedChange,
  onSwitchSession,
  onNewChat,
  onDeleteSession,
  onEditAgent,
  onToggleSessionPinned,
  onReorderAgents,
  onReorderProjects,
  onMarkAllRead,
  onRenameSession,
  hasMoreSessions,
  loadingMoreSessions,
  onLoadMoreSessions,
  onOpenProjectSettings,
  onAddProject,
  onNewChatInProject,
  onArchiveProject,
  onMoveSessionToProject,
  searchFocusSignal,
}: ChatSidebarProps) {
  const { t } = useTranslation()
  const [agentsExpanded, setAgentsExpandedState] = useState(() =>
    readStoredBoolean(AGENTS_EXPANDED_STORAGE_KEY, false),
  )
  const [projectsExpanded, setProjectsExpandedState] = useState(() =>
    readStoredBoolean(PROJECTS_EXPANDED_STORAGE_KEY, true),
  )
  const [showNewChatMenu, setShowNewChatMenu] = useState(false)
  const newChatMenuRef = useRef<HTMLDivElement>(null)
  const [deleteConfirmSessionId, setDeleteConfirmSessionId] = useState<string | null>(null)
  const [sidebarDisplayMode, setSidebarDisplayMode] = useState<SidebarDisplayMode>(
    DEFAULT_SIDEBAR_DISPLAY_MODE,
  )
  const [sidebarDisplayModeReady, setSidebarDisplayModeReady] = useState(false)

  const setAgentsExpanded = useCallback((expanded: boolean) => {
    setAgentsExpandedState(expanded)
    writeStoredBoolean(AGENTS_EXPANDED_STORAGE_KEY, expanded)
  }, [])

  const setProjectsExpanded = useCallback((expanded: boolean) => {
    setProjectsExpandedState(expanded)
    writeStoredBoolean(PROJECTS_EXPANDED_STORAGE_KEY, expanded)
  }, [])

  // Inline rename state
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null)
  const [renameValue, setRenameValue] = useState("")
  const renameInputRef = useRef<HTMLInputElement>(null)

  const startRename = useCallback((sessionId: string, currentTitle: string) => {
    setRenamingSessionId(sessionId)
    setRenameValue(currentTitle)
    // Focus input after render
    setTimeout(() => renameInputRef.current?.focus(), 0)
  }, [])

  const commitRename = useCallback(() => {
    if (renamingSessionId && renameValue.trim() && onRenameSession) {
      onRenameSession(renamingSessionId, renameValue.trim())
    }
    setRenamingSessionId(null)
    setRenameValue("")
  }, [renamingSessionId, renameValue, onRenameSession])

  const cancelRename = useCallback(() => {
    setRenamingSessionId(null)
    setRenameValue("")
  }, [])

  // Session type filter
  const [sessionFilter, setSessionFilter] = useState<SessionFilterType>("session")

  // Agent filter state (single-select)
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null)

  // ── History search ─────────────────────────────────────────────
  const [searchQuery, setSearchQuery] = useState("")
  const [searchResults, setSearchResults] = useState<SessionSearchResult[] | null>(null)
  const [searching, setSearching] = useState(false)
  const searchInputRef = useRef<HTMLInputElement>(null)
  const searchTruncated = (searchResults?.length ?? 0) >= SEARCH_LIMIT
  const isHistorySearching = searchQuery.trim().length > 0

  useEffect(() => {
    if (searchFocusSignal === undefined || searchFocusSignal === 0) return
    onSidebarCollapsedChange(false)
  }, [searchFocusSignal, onSidebarCollapsedChange])

  useEffect(() => {
    if (searchFocusSignal === undefined || searchFocusSignal === 0 || sidebarCollapsed) return
    const frame = window.requestAnimationFrame(() => {
      const input = searchInputRef.current
      if (!input) return
      input.focus({ preventScroll: true })
      input.select()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [searchFocusSignal, sidebarCollapsed])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<string>("get_sidebar_display_mode")
      .then((mode) => {
        if (!cancelled) {
          setSidebarDisplayMode(normalizeSidebarDisplayMode(mode))
          setSidebarDisplayModeReady(true)
        }
      })
      .catch((err) => {
        logger.error("chat", "ChatSidebar::loadDisplayMode", "failed to load sidebar mode", err)
        if (!cancelled) setSidebarDisplayModeReady(true)
      })

    const handleModeChanged = (event: Event) => {
      const detail = (event as CustomEvent<{ mode?: unknown }>).detail
      setSidebarDisplayMode(normalizeSidebarDisplayMode(detail?.mode))
      setSidebarDisplayModeReady(true)
    }
    window.addEventListener("sidebar-display-mode-changed", handleModeChanged)
    return () => {
      cancelled = true
      window.removeEventListener("sidebar-display-mode-changed", handleModeChanged)
    }
  }, [])

  const toggleSidebarDisplayMode = useCallback(async () => {
    const next: SidebarDisplayMode = sidebarDisplayMode === "compact" ? "detailed" : "compact"
    const previous = sidebarDisplayMode
    setSidebarDisplayMode(next)
    window.dispatchEvent(new CustomEvent("sidebar-display-mode-changed", { detail: { mode: next } }))
    try {
      await getTransport().call("set_sidebar_display_mode", { mode: next })
    } catch (err) {
      setSidebarDisplayMode(previous)
      window.dispatchEvent(
        new CustomEvent("sidebar-display-mode-changed", { detail: { mode: previous } }),
      )
      logger.error("chat", "ChatSidebar::toggleDisplayMode", "failed to save sidebar mode", err)
    }
  }, [sidebarDisplayMode])

  useEffect(() => {
    const q = searchQuery.trim()
    if (!q) {
      setSearchResults(null)
      setSearching(false)
      return
    }
    setSearching(true)
    const timer = setTimeout(async () => {
      try {
        const results = await getTransport().call<SessionSearchResult[]>("search_sessions_cmd", {
          query: q,
          limit: SEARCH_LIMIT,
          // Exclude cron at the backend so the fixed SEARCH_LIMIT isn't consumed
          // by hidden cron hits (they live in the cron panel's history view, not
          // the sidebar) — otherwise a regular match ranked just below a burst of
          // cron matches could fall outside the limit and never render.
          types: ["regular", "subagent", "channel"],
        })
        setSearchResults(sortSessionSearchResults(results ?? []))
      } catch (err) {
        logger.error("chat", "ChatSidebar::search", "search failed", err)
        setSearchResults([])
      } finally {
        setSearching(false)
      }
    }, 300)
    return () => clearTimeout(timer)
  }, [searchQuery])

  const filteredSessions = (() => {
    const list =
      selectedAgentId === null ? sessions : sessions.filter((s) => s.agentId === selectedAgentId)
    switch (sessionFilter) {
      case "session":
        // Project-bound sessions render under their project group above —
        // exclude them here to avoid duplicate rows. IM-channel sessions
        // are surfaced inline (the row already shows a channel icon)
        // since the dedicated "channel" tab was retired in Phase B3.
        return list.filter((s) => !s.isCron && !s.parentSessionId && !s.projectId)
      case "subagent":
        return list.filter((s) => !!s.parentSessionId)
      default:
        return list
    }
  })()

  const toggleAgentFilter = useCallback(
    (agentId: string) => {
      setSelectedAgentId((prev) => {
        if (prev === agentId) {
          return null
        }
        return agentId
      })
      // Move parent callbacks outside the state updater to avoid
      // updating ChatScreen state during ChatSidebar render
      if (selectedAgentId !== agentId) {
        const firstSession = sessions.find((s) => s.agentId === agentId)
        if (firstSession) {
          onSwitchSession(firstSession.id)
        } else {
          onNewChat(agentId)
        }
      }
    },
    [selectedAgentId, sessions, onSwitchSession, onNewChat],
  )

  // Drag handler for resizable panel
  const isDragging = useRef(false)
  const [isResizing, setIsResizing] = useState(false)
  const [isResizeHandleHovered, setIsResizeHandleHovered] = useState(false)
  const handleDragStart = (e: React.MouseEvent) => {
    e.preventDefault()
    isDragging.current = true
    setIsResizing(true)
    const startX = e.clientX
    const startWidth = panelWidth

    const onMouseMove = (ev: MouseEvent) => {
      if (!isDragging.current) return
      const delta = ev.clientX - startX
      const newWidth = Math.min(
        CHAT_SIDEBAR_MAX_WIDTH,
        Math.max(CHAT_SIDEBAR_MIN_WIDTH, startWidth + delta),
      )
      onPanelWidthChange(newWidth)
    }

    const onMouseUp = () => {
      isDragging.current = false
      setIsResizing(false)
      document.removeEventListener("mousemove", onMouseMove)
      document.removeEventListener("mouseup", onMouseUp)
      document.body.style.cursor = ""
      document.body.style.userSelect = ""
    }

    document.addEventListener("mousemove", onMouseMove)
    document.addEventListener("mouseup", onMouseUp)
    document.body.style.cursor = "col-resize"
    document.body.style.userSelect = "none"
  }

  // Close new-chat menu on outside click
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (newChatMenuRef.current && !newChatMenuRef.current.contains(e.target as Node)) {
        setShowNewChatMenu(false)
      }
    }
    if (showNewChatMenu) {
      document.addEventListener("mousedown", handleClickOutside)
      return () => document.removeEventListener("mousedown", handleClickOutside)
    }
  }, [showNewChatMenu])

  useEffect(() => {
    if (agents.length <= 1 && showNewChatMenu) {
      setShowNewChatMenu(false)
    }
  }, [agents.length, showNewChatMenu])

  const getAgentInfo = useCallback(
    (agentId: string) => {
      return agents.find((a) => a.id === agentId)
    },
    [agents],
  )

  const formatRelativeTime = useCallback(
    (dateStr: string) => {
      const date = new Date(dateStr)
      if (isNaN(date.getTime())) return ""
      const now = new Date()
      const diff = now.getTime() - date.getTime()
      const minutes = Math.floor(diff / 60000)
      if (minutes < 1) return t("chat.justNow")
      if (minutes < 60) return t("chat.minutesAgo", { count: minutes })
      const hours = Math.floor(minutes / 60)
      if (hours < 24) return t("chat.hoursAgo", { count: hours })
      const days = Math.floor(hours / 24)
      if (days < 7) return t("chat.daysAgo", { count: days })
      const weeks = Math.floor(days / 7)
      if (days < 30) return t("chat.weeksAgo", { count: weeks })
      return date.toLocaleDateString()
    },
    [t],
  )

  function handleDeleteClick(sessionId: string, e: React.MouseEvent) {
    e.stopPropagation()
    setDeleteConfirmSessionId(sessionId)
  }

  function confirmDelete() {
    if (!deleteConfirmSessionId) return
    onDeleteSession(deleteConfirmSessionId)
    setDeleteConfirmSessionId(null)
  }

  const focusSearchInput = useCallback(() => {
    searchInputRef.current?.focus({ preventScroll: true })
  }, [])

  const sidebarContentReady = sidebarDisplayModeReady && !sessionsLoading && !projectsLoading
  const [sidebarMotionEnabled, setSidebarMotionEnabled] = useState(false)

  useEffect(() => {
    if (!sidebarContentReady) {
      setSidebarMotionEnabled(false)
    }
  }, [sidebarContentReady])

  const enableSidebarMotion = useCallback(() => {
    if (!sidebarContentReady || sidebarMotionEnabled) return
    flushSync(() => setSidebarMotionEnabled(true))
  }, [sidebarContentReady, sidebarMotionEnabled])

  const sidebarMotionDisabled = !sidebarMotionEnabled

  const handleSearchSurfaceMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const target = e.target as HTMLElement | null
      if (target?.closest("button")) return
      if (target !== searchInputRef.current) {
        e.preventDefault()
      }
      focusSearchInput()
    },
    [focusSearchInput],
  )

  const handleSearchKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key !== "Escape") return
      e.preventDefault()
      if (searchQuery.length > 0) {
        setSearchQuery("")
      } else {
        searchInputRef.current?.blur()
      }
    },
    [searchQuery],
  )

  const sessionListNode = (
    <SessionList
      sessions={sessions}
      filteredSessions={filteredSessions}
      sessionFilter={sessionFilter}
      setSessionFilter={setSessionFilter}
      selectedAgentId={selectedAgentId}
      currentSessionId={currentSessionId}
      loadingSessionIds={loadingSessionIds}
      sessionsLoading={sessionsLoading}
      loadingMoreSessions={loadingMoreSessions}
      onSwitchSession={onSwitchSession}
      onDeleteClick={handleDeleteClick}
      onMarkAllRead={onMarkAllRead}
      renamingSessionId={renamingSessionId}
      renameValue={renameValue}
      renameInputRef={renameInputRef}
      onStartRename={startRename}
      onRenameValueChange={setRenameValue}
      onCommitRename={commitRename}
      onCancelRename={cancelRename}
      getAgentInfo={getAgentInfo}
      formatRelativeTime={formatRelativeTime}
      searchQuery={searchQuery}
      searchResults={searchResults}
      searchTruncated={searchTruncated}
      searching={searching}
      agents={agents}
      projects={projects}
      onMoveToProject={onMoveSessionToProject}
      onToggleSessionPinned={onToggleSessionPinned}
      displayMode={sidebarDisplayMode}
    />
  )

  return (
    <>
      <div
        style={{ width: sidebarCollapsed ? 0 : panelWidth }}
        className={cn(
          "relative h-full shrink-0",
          !isResizing &&
            !sidebarMotionDisabled &&
            "transition-[width] duration-[250ms] ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[width] motion-reduce:transition-none",
        )}
      >
        <div className="h-full overflow-hidden">
          <div
            style={{ width: panelWidth }}
            aria-hidden={sidebarCollapsed}
            inert={sidebarCollapsed ? true : undefined}
            onPointerDownCapture={enableSidebarMotion}
            onKeyDownCapture={enableSidebarMotion}
            className={cn(
              "h-full border-r bg-surface-panel shadow-panel flex flex-col [contain:layout_paint]",
              isResizing
                ? "border-r-primary/50"
                : isResizeHandleHovered
                  ? "border-r-primary/35"
                  : "border-r-border-soft",
              !sidebarMotionDisabled &&
                "transition-[opacity,transform,border-color] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[opacity,transform] motion-reduce:transition-none",
              sidebarCollapsed
                ? "pointer-events-none -translate-x-4 opacity-0"
                : "translate-x-0 opacity-100",
            )}
          >
            {/* Title bar */}
            <div className="h-12 flex items-end px-3.5 shrink-0" data-tauri-drag-region>
              <h2 className="text-sm font-semibold text-foreground pb-2">
                {t("chat.conversations")}
              </h2>
              <div className="ml-auto flex items-center gap-1 pb-2">
                <IconTip label={t("chat.collapseSidebar")}>
                  <button
                    className="flex h-7 w-7 items-center justify-center rounded-md text-foreground transition-colors hover:bg-surface-subtle"
                    aria-label={t("chat.collapseSidebar")}
                    onClick={(e) => {
                      e.currentTarget.blur()
                      onSidebarCollapsedChange(true)
                    }}
                  >
                    <PanelLeft className="h-4 w-4" />
                  </button>
                </IconTip>
                <IconTip
                  label={
                    sidebarDisplayMode === "compact"
                      ? t("chat.sidebarDetailedMode", "切换到普通模式")
                      : t("chat.sidebarCompactMode", "切换到简约模式")
                  }
                >
                  <button
                    className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-surface-subtle hover:text-foreground"
                    aria-label={
                      sidebarDisplayMode === "compact"
                        ? t("chat.sidebarDetailedMode", "切换到普通模式")
                        : t("chat.sidebarCompactMode", "切换到简约模式")
                    }
                    onClick={(e) => {
                      e.currentTarget.blur()
                      void toggleSidebarDisplayMode()
                    }}
                  >
                    {sidebarDisplayMode === "compact" ? (
                      <Rows3 className="h-4 w-4" />
                    ) : (
                      <ListCollapse className="h-4 w-4" />
                    )}
                  </button>
                </IconTip>
                {/* New Chat button */}
                <div className="relative" ref={newChatMenuRef}>
                  <IconTip label={t("chat.newChat")}>
                    <button
                      className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-surface-subtle hover:text-foreground"
                      onClick={() => {
                        if (agents.length === 1) {
                          onNewChat(agents[0].id)
                        } else if (agents.length > 1) {
                          setShowNewChatMenu(!showNewChatMenu)
                        }
                      }}
                    >
                      <MessageSquarePlus className="h-4 w-4" />
                    </button>
                  </IconTip>
                  {/* Agent selector popup */}
                  <FloatingMenu
                    open={showNewChatMenu && agents.length > 1}
                    positionClassName="right-0 top-full mt-1"
                    originClassName="origin-top-right"
                    className="ha-menu-from-top min-w-[180px] p-1.5"
                    onEscapeKeyDown={() => setShowNewChatMenu(false)}
                  >
                    {agents.map((agent) => (
                      <button
                        key={agent.id}
                        className="flex items-center gap-2 w-full px-2.5 py-1.5 text-[13px] rounded-md text-foreground/80 hover:bg-surface-subtle hover:text-foreground transition-colors"
                        onClick={() => {
                          onNewChat(agent.id)
                          setShowNewChatMenu(false)
                        }}
                      >
                        {sidebarDisplayMode === "detailed" && (
                          <div className="w-5 h-5 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden">
                            {agent.avatar ? (
                              <img
                                src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
                                className="w-full h-full object-cover"
                                alt=""
                              />
                            ) : agent.emoji ? (
                              <span>{agent.emoji}</span>
                            ) : (
                              <Bot className="h-3 w-3" />
                            )}
                          </div>
                        )}
                        <span className="truncate">{agent.name}</span>
                      </button>
                    ))}
                  </FloatingMenu>
                </div>
              </div>
            </div>

            <div className="shrink-0 border-b border-border/40 px-3 pb-2 pt-1">
              <div className="relative" onMouseDown={handleSearchSurfaceMouseDown}>
                <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground/60 pointer-events-none" />
                <Input
                  ref={searchInputRef}
                  aria-label={t("chat.searchPlaceholder") || ""}
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  onKeyDown={handleSearchKeyDown}
                  placeholder={t("chat.searchPlaceholder")}
                  className="h-7 pl-7 pr-7 text-xs border-none shadow-none bg-muted/50 rounded-md focus-visible:ring-0 focus-visible:bg-muted/80 placeholder:text-muted-foreground/50"
                />
                {searchQuery && (
                  <button
                    onClick={() => setSearchQuery("")}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                    aria-label={t("common.clear") || "Clear"}
                  >
                    <X className="h-3 w-3" />
                  </button>
                )}
              </div>
            </div>

            <div
              className={cn(
                "flex-1 overflow-y-auto overflow-x-hidden [overscroll-behavior-y:none]",
                sidebarMotionDisabled && "[&_*]:!animate-none [&_*]:!transition-none",
              )}
              onScroll={(e) => {
                if (isHistorySearching) return
                if (!hasMoreSessions || loadingMoreSessions || !onLoadMoreSessions) return
                const el = e.currentTarget
                // Trigger when scrolled within 100px of the bottom
                if (el.scrollHeight - el.scrollTop - el.clientHeight < 100) {
                  onLoadMoreSessions()
                }
              }}
            >
              {!sidebarContentReady ? (
                <div className="flex justify-center py-8">
                  <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                </div>
              ) : isHistorySearching ? (
                sessionListNode
              ) : (
                <>
                  {/* Collapsible Agents section */}
                  {agents.length > 1 && (
                    <AgentSection
                      agents={agents}
                      agentsExpanded={agentsExpanded}
                      setAgentsExpanded={setAgentsExpanded}
                      selectedAgentId={selectedAgentId}
                      toggleAgentFilter={toggleAgentFilter}
                      onNewChat={onNewChat}
                      onEditAgent={onEditAgent}
                      onReorderAgents={onReorderAgents}
                      panelWidth={panelWidth}
                      displayMode={sidebarDisplayMode}
                      motionDisabled={sidebarMotionDisabled}
                    />
                  )}

                  {/* Projects section — shown below agents. Falls back silently
                      when no handler is wired (backwards-compat). */}
                  {(projects.length > 0 || onAddProject) && (
                    <ProjectSection
                      projects={projects}
                      sessions={sessions}
                      agents={agents}
                      currentSessionId={currentSessionId}
                      loadingSessionIds={loadingSessionIds}
                      expanded={projectsExpanded}
                      setExpanded={setProjectsExpanded}
                      onAddProject={() => onAddProject?.()}
                      onOpenProjectSettings={(p) => onOpenProjectSettings?.(p)}
                      onNewChatInProject={(pid, opts) => onNewChatInProject?.(pid, opts)}
                      onArchiveProject={(pid, archived) => onArchiveProject?.(pid, archived)}
                      onReorderProjects={onReorderProjects}
                      onSwitchSession={onSwitchSession}
                      onDeleteSession={handleDeleteClick}
                      onMarkAllRead={onMarkAllRead}
                      renamingSessionId={renamingSessionId}
                      renameValue={renameValue}
                      renameInputRef={renameInputRef}
                      onStartRename={startRename}
                      onRenameValueChange={setRenameValue}
                      onCommitRename={commitRename}
                      onCancelRename={cancelRename}
                      onMoveSessionToProject={onMoveSessionToProject}
                      onToggleSessionPinned={onToggleSessionPinned}
                      getAgentInfo={getAgentInfo}
                      formatRelativeTime={formatRelativeTime}
                      displayMode={sidebarDisplayMode}
                      motionDisabled={sidebarMotionDisabled}
                    />
                  )}

                  {/* Session filter tabs + session list */}
                  {sessionListNode}
                </>
              )}
            </div>
          </div>
        </div>
        {/* Transparent resize hit area; the existing sidebar border provides
            the hover/drag affordance so no second line can drift out of place. */}
        <div
          className={cn(
            "absolute inset-y-0 right-0 z-20 cursor-col-resize",
            !sidebarMotionDisabled &&
              "transition-[width,opacity] duration-200 ease-out",
            sidebarCollapsed ? "w-0 pointer-events-none opacity-0" : "w-3 opacity-100",
          )}
          onMouseDown={handleDragStart}
          onMouseEnter={() => setIsResizeHandleHovered(true)}
          onMouseLeave={() => setIsResizeHandleHovered(false)}
        />
      </div>

      {/* Delete session confirmation dialog */}
      <AlertDialog
        open={!!deleteConfirmSessionId}
        onOpenChange={(open) => !open && setDeleteConfirmSessionId(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("chat.deleteSessionTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("chat.deleteSessionWarning")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={confirmDelete}
            >
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

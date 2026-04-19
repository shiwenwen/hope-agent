import { useState, useRef, useEffect, useCallback } from "react"
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
import {
  Bot,
  MessageSquarePlus,
} from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { SessionSearchResult } from "@/types/chat"
import type { ChatSidebarProps, SessionFilterType } from "./types"
import AgentSection from "./AgentSection"
import SessionList from "./SessionList"
import ProjectSection from "../project/ProjectSection"

export default function ChatSidebar({
  sessions,
  agents,
  projects = [],
  currentSessionId,
  loadingSessionIds,
  panelWidth,
  onPanelWidthChange,
  onSwitchSession,
  onNewChat,
  onDeleteSession,
  onEditAgent,
  onMarkAllRead,
  onRenameSession,
  hasMoreSessions,
  loadingMoreSessions,
  onLoadMoreSessions,
  onOpenProject,
  onAddProject,
  onMoveSessionToProject,
}: ChatSidebarProps) {
  const { t } = useTranslation()
  const [agentsExpanded, setAgentsExpanded] = useState(true)
  const [projectsExpanded, setProjectsExpanded] = useState(true)
  const [showNewChatMenu, setShowNewChatMenu] = useState(false)
  const newChatMenuRef = useRef<HTMLDivElement>(null)
  const [deleteConfirmSessionId, setDeleteConfirmSessionId] = useState<string | null>(null)

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
        const results = await getTransport().call<SessionSearchResult[]>(
          "search_sessions_cmd",
          {
            query: q,
            agentId: selectedAgentId ?? undefined,
            limit: 80,
          },
        )
        setSearchResults(results)
      } catch (err) {
        logger.error("chat", "ChatSidebar::search", "search failed", err)
        setSearchResults([])
      } finally {
        setSearching(false)
      }
    }, 300)
    return () => clearTimeout(timer)
  }, [searchQuery, selectedAgentId])

  const filteredSessions = (() => {
    const list =
      selectedAgentId === null ? sessions : sessions.filter((s) => s.agentId === selectedAgentId)
    switch (sessionFilter) {
      case "session":
        return list.filter((s) => !s.isCron && !s.parentSessionId && !s.channelInfo)
      case "cron":
        return list.filter((s) => s.isCron)
      case "subagent":
        return list.filter((s) => !!s.parentSessionId)
      case "channel":
        return list.filter((s) => !!s.channelInfo)
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
  const handleDragStart = (e: React.MouseEvent) => {
    e.preventDefault()
    isDragging.current = true
    const startX = e.clientX
    const startWidth = panelWidth

    const onMouseMove = (ev: MouseEvent) => {
      if (!isDragging.current) return
      const delta = ev.clientX - startX
      const newWidth = Math.min(400, Math.max(180, startWidth + delta))
      onPanelWidthChange(newWidth)
    }

    const onMouseUp = () => {
      isDragging.current = false
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

  return (
    <>
      <div
          style={{ width: panelWidth }}
          className="shrink-0 border-r border-border bg-background flex flex-col"
        >
          {/* Title bar */}
          <div className="h-10 flex items-end px-4 shrink-0" data-tauri-drag-region>
            <h2 className="text-sm font-semibold text-foreground pb-1.5">
              {t("chat.conversations")}
            </h2>
            {/* New Chat button */}
            <div className="ml-auto relative" ref={newChatMenuRef}>
              <IconTip label={t("chat.newChat")}>
                <button
                  className="text-muted-foreground hover:text-foreground transition-colors pb-1.5"
                  onClick={() => {
                    if (agents.length === 1) {
                      onNewChat(agents[0].id)
                    } else {
                      setShowNewChatMenu(!showNewChatMenu)
                    }
                  }}
                >
                  <MessageSquarePlus className="h-4 w-4" />
                </button>
              </IconTip>
              {/* Agent selector popup */}
              {showNewChatMenu && (
                <div className="absolute right-0 top-full mt-1 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-lg z-50 min-w-[180px] p-1.5 animate-in fade-in-0 zoom-in-95 duration-150">
                  {agents.map((agent) => (
                    <button
                      key={agent.id}
                      className="flex items-center gap-2 w-full px-2.5 py-1.5 text-[13px] rounded-md text-foreground/80 hover:bg-secondary/60 hover:text-foreground transition-colors"
                      onClick={() => {
                        onNewChat(agent.id)
                        setShowNewChatMenu(false)
                      }}
                    >
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
                      <span className="truncate">{agent.name}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          </div>

          <div
            className="flex-1 overflow-y-auto overflow-x-hidden"
            onScroll={(e) => {
              if (!hasMoreSessions || loadingMoreSessions || !onLoadMoreSessions) return
              const el = e.currentTarget
              // Trigger when scrolled within 100px of the bottom
              if (el.scrollHeight - el.scrollTop - el.clientHeight < 100) {
                onLoadMoreSessions()
              }
            }}
          >
            {/* Projects section — shown above agents so users reach their
                active workspace first. Falls back silently when no handler
                is wired (backwards-compat). */}
            {(projects.length > 0 || onAddProject) && (
              <ProjectSection
                projects={projects}
                expanded={projectsExpanded}
                setExpanded={setProjectsExpanded}
                onAddProject={() => onAddProject?.()}
                onOpenProject={(p) => onOpenProject?.(p)}
              />
            )}

            {/* Collapsible Agents section */}
            <AgentSection
              agents={agents}
              agentsExpanded={agentsExpanded}
              setAgentsExpanded={setAgentsExpanded}
              selectedAgentId={selectedAgentId}
              toggleAgentFilter={toggleAgentFilter}
              onNewChat={onNewChat}
              onEditAgent={onEditAgent}
              panelWidth={panelWidth}
            />

            {/* Session filter tabs + session list */}
            <SessionList
              sessions={sessions}
              filteredSessions={filteredSessions}
              sessionFilter={sessionFilter}
              setSessionFilter={setSessionFilter}
              selectedAgentId={selectedAgentId}
              currentSessionId={currentSessionId}
              loadingSessionIds={loadingSessionIds}
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
              onSearchQueryChange={setSearchQuery}
              searchResults={searchResults}
              searching={searching}
              agents={agents}
              projects={projects}
              onMoveToProject={onMoveSessionToProject}
            />
          </div>
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
      {/* Drag Handle */}
      <div
        className="w-1 shrink-0 cursor-col-resize hover:bg-primary/30 active:bg-primary/50 transition-colors"
        onMouseDown={handleDragStart}
      />
    </>
  )
}

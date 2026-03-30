import { useState, useRef, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { convertFileSrc } from "@tauri-apps/api/core"
import { logger } from "@/lib/logger"
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
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  ChevronRight,
  MessageSquare,
  Bot,
  Trash2,
  MessageSquarePlus,
  Loader2,
  Timer,
  Pencil,
  Network,
  CheckCheck,
  MessageCircle,
} from "lucide-react"
import type { SessionMeta, AgentSummaryForSidebar } from "@/types/chat"
import ChannelIcon from "@/components/common/ChannelIcon"

interface ChatSidebarProps {
  sessions: SessionMeta[]
  agents: AgentSummaryForSidebar[]
  currentSessionId: string | null
  loadingSessionIds: Set<string>
  panelWidth: number
  onPanelWidthChange: (width: number) => void
  onSwitchSession: (sessionId: string) => void
  onNewChat: (agentId: string) => void
  onDeleteSession: (sessionId: string) => void
  onEditAgent?: (agentId: string) => void
  onMarkAllRead?: () => void
  onRenameSession?: (sessionId: string, title: string) => void
  hasMoreSessions?: boolean
  loadingMoreSessions?: boolean
  onLoadMoreSessions?: () => void
}

export default function ChatSidebar({
  sessions,
  agents,
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
}: ChatSidebarProps) {
  const { t } = useTranslation()
  const [agentsExpanded, setAgentsExpanded] = useState(true)
  const [showNewChatMenu, setShowNewChatMenu] = useState(false)
  const newChatMenuRef = useRef<HTMLDivElement>(null)
  const [deleteConfirmSessionId, setDeleteConfirmSessionId] = useState<string | null>(null)
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

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
  type SessionFilterType = "all" | "session" | "cron" | "subagent" | "channel"
  const [sessionFilter, setSessionFilter] = useState<SessionFilterType>("session")

  // Agent filter state (single-select)
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null)

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

  const getAgentInfo = (agentId: string) => {
    return agents.find((a) => a.id === agentId)
  }

  const formatRelativeTime = (dateStr: string) => {
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
  }

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
                            src={
                              agent.avatar.startsWith("/")
                                ? convertFileSrc(agent.avatar)
                                : agent.avatar
                            }
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
            className="flex-1 overflow-y-auto"
            onScroll={(e) => {
              if (!hasMoreSessions || loadingMoreSessions || !onLoadMoreSessions) return
              const el = e.currentTarget
              // Trigger when scrolled within 100px of the bottom
              if (el.scrollHeight - el.scrollTop - el.clientHeight < 100) {
                onLoadMoreSessions()
              }
            }}
          >
            {/* Collapsible Agents section */}
            <div className="border-b border-border/50">
              <div className="flex items-center">
                <button
                  className="flex items-center gap-1.5 flex-1 px-4 py-2 text-[11px] font-semibold text-muted-foreground uppercase tracking-wider hover:text-foreground transition-colors"
                  onClick={() => setAgentsExpanded(!agentsExpanded)}
                >
                  <ChevronRight
                    className={cn(
                      "h-3 w-3 transition-transform duration-200",
                      agentsExpanded && "rotate-90",
                    )}
                  />
                  <span>Agents</span>
                  <span className="font-normal normal-case text-muted-foreground/60 ml-0.5">
                    ({agents.length})
                  </span>
                </button>
              </div>
              <div
                className={cn(
                  "overflow-hidden transition-all duration-200 ease-out",
                  agentsExpanded ? "max-h-[500px] opacity-100" : "max-h-0 opacity-0",
                )}
              >
                <div
                  className={cn(
                    "px-2 pb-2 grid gap-1",
                    panelWidth >= 280 ? "grid-cols-2" : "grid-cols-1",
                  )}
                >
                  {agents.map((agent) => {
                    const isSelected = selectedAgentId === agent.id
                    return (
                      <ContextMenu key={agent.id}>
                        <ContextMenuTrigger asChild>
                          <div
                            className={cn(
                              "flex items-center gap-2 px-2 py-1.5 rounded-lg text-xs transition-colors truncate group/agent",
                              isSelected ? "bg-primary/10" : "hover:bg-secondary/60",
                            )}
                            title={agent.description || agent.name}
                          >
                            {/* Clickable area: single click = toggle filter, double click = new chat */}
                            <button
                              className="flex items-center gap-2 flex-1 min-w-0"
                              onClick={() => {
                                if (clickTimerRef.current) {
                                  clearTimeout(clickTimerRef.current)
                                  clickTimerRef.current = null
                                }
                                clickTimerRef.current = setTimeout(() => {
                                  toggleAgentFilter(agent.id)
                                  clickTimerRef.current = null
                                }, 250)
                              }}
                              onDoubleClick={() => {
                                if (clickTimerRef.current) {
                                  clearTimeout(clickTimerRef.current)
                                  clickTimerRef.current = null
                                }
                                onNewChat(agent.id)
                              }}
                            >
                              <div
                                className={cn(
                                  "w-6 h-6 rounded-full flex items-center justify-center shrink-0 text-[10px] overflow-hidden",
                                  isSelected
                                    ? "bg-primary/25 text-primary"
                                    : "bg-primary/15 text-primary",
                                )}
                              >
                                {agent.avatar ? (
                                  <img
                                    src={
                                      agent.avatar.startsWith("/")
                                        ? convertFileSrc(agent.avatar)
                                        : agent.avatar
                                    }
                                    className="w-full h-full object-cover"
                                    alt=""
                                  />
                                ) : agent.emoji ? (
                                  <span>{agent.emoji}</span>
                                ) : (
                                  <Bot className="h-3 w-3" />
                                )}
                              </div>
                              <span
                                className={cn(
                                  "truncate",
                                  isSelected ? "text-primary font-medium" : "text-foreground/80",
                                )}
                              >
                                {agent.name}
                                {agent.emoji ? ` ${agent.emoji}` : ""}
                              </span>
                            </button>
                            {/* New chat button */}
                            <IconTip label={t("chat.newChat")}>
                              <button
                                className="shrink-0 p-0.5 rounded text-muted-foreground/0 group-hover/agent:text-muted-foreground/60 hover:!text-primary transition-colors"
                                onClick={(e) => {
                                  e.stopPropagation()
                                  onNewChat(agent.id)
                                }}
                              >
                                <MessageSquarePlus className="h-3 w-3" />
                              </button>
                            </IconTip>
                          </div>
                        </ContextMenuTrigger>
                        {onEditAgent && (
                          <ContextMenuContent>
                            <ContextMenuItem onClick={() => onEditAgent(agent.id)}>
                              <Pencil className="h-3 w-3 mr-2" />
                              {t("common.edit")}
                            </ContextMenuItem>
                          </ContextMenuContent>
                        )}
                      </ContextMenu>
                    )
                  })}
                </div>
              </div>
            </div>

            {/* Session type filter tabs */}
            <div className="flex items-center gap-0.5 px-3 py-1.5 border-b border-border/40">
              {(["all", "session", "channel", "cron", "subagent"] as const).map((filter) => {
                const label = {
                  all: t("chat.filterAll"),
                  session: t("chat.filterSessions"),
                  channel: t("chat.filterChannel"),
                  cron: t("chat.filterCron"),
                  subagent: t("chat.filterSubagent"),
                }[filter]
                const filterSessions = {
                  all: sessions,
                  session: sessions.filter((s) => !s.isCron && !s.parentSessionId && !s.channelInfo),
                  channel: sessions.filter((s) => !!s.channelInfo),
                  cron: sessions.filter((s) => s.isCron),
                  subagent: sessions.filter((s) => !!s.parentSessionId),
                }[filter]
                const count = filter === "channel"
                  ? 0  // Channel sessions don't show unread counts
                  : filterSessions.reduce((sum, s) => sum + s.unreadCount, 0)
                const isActive = sessionFilter === filter
                const handleMarkAllRead = async () => {
                  const unreadSessions = filterSessions.filter((s) => s.unreadCount > 0)
                  if (unreadSessions.length === 0) return
                  try {
                    await invoke("mark_session_read_batch_cmd", {
                      sessionIds: unreadSessions.map((s) => s.id),
                    })
                    if (onMarkAllRead) onMarkAllRead()
                  } catch (err) {
                    logger.error("chat", "ChatSidebar::markSessionsRead", "Failed to mark sessions as read", err)
                  }
                }

                return (
                  <ContextMenu key={filter}>
                    <ContextMenuTrigger asChild>
                      <button
                        className={cn(
                          "relative px-2 py-1 text-[11px] rounded-md transition-colors whitespace-nowrap",
                          isActive
                            ? "text-foreground font-semibold"
                            : "text-muted-foreground hover:text-foreground/70",
                        )}
                        onClick={() => setSessionFilter(filter)}
                      >
                        {label}
                        {count > 0 && (
                          <span className="ml-0.5 text-[10px] text-muted-foreground/50">
                            {count > 99 ? "99+" : count}
                          </span>
                        )}
                        {isActive && (
                          <span className="absolute bottom-0 left-1/2 -translate-x-1/2 w-3/5 h-[2px] rounded-full bg-primary" />
                        )}
                      </button>
                    </ContextMenuTrigger>
                    <ContextMenuContent>
                      <ContextMenuItem onClick={handleMarkAllRead} disabled={count === 0}>
                        {t("chat.markAllRead") || "全部已读"}
                      </ContextMenuItem>
                    </ContextMenuContent>
                  </ContextMenu>
                )
              })}
            </div>

            {/* Session list */}
            <div className="p-2 space-y-0.5">
              {filteredSessions.length === 0 ? (
                <div className="text-center py-8">
                  <MessageSquare className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
                  <p className="text-xs text-muted-foreground/60">
                    {selectedAgentId !== null
                      ? t("chat.noMatchingSessions") || "No matching sessions"
                      : t("chat.startConversation")}
                  </p>
                </div>
              ) : (
                filteredSessions.map((session) => {
                  const agent = getAgentInfo(session.agentId)
                  const isActive = session.id === currentSessionId
                  const isLoading = loadingSessionIds.has(session.id)
                  const handleMarkAsRead = async () => {
                    if (session.unreadCount === 0) return
                    try {
                      await invoke("mark_session_read_cmd", {
                        sessionId: session.id,
                      })
                      if (onMarkAllRead) onMarkAllRead()
                    } catch (err) {
                      logger.error("chat", "ChatSidebar::markSessionRead", "Failed to mark session as read", err)
                    }
                  }
                  return (
                    <ContextMenu key={session.id}>
                      <ContextMenuTrigger asChild>
                    <div
                      role="button"
                      tabIndex={0}
                      className={cn(
                        "flex items-center gap-2.5 w-full px-2.5 py-2 rounded-lg text-left transition-colors group cursor-pointer",
                        isActive
                          ? "bg-secondary/70 border border-border/50"
                          : "hover:bg-secondary/40",
                      )}
                      onClick={() => onSwitchSession(session.id)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault()
                          onSwitchSession(session.id)
                        }
                      }}
                    >
                      {/* Agent avatar (small) — with loading spinner overlay + unread dot */}
                      <div className="relative shrink-0">
                        <div className="w-7 h-7 rounded-full bg-primary/10 flex items-center justify-center text-primary text-[10px] overflow-hidden">
                          {isLoading ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
                          ) : agent?.avatar ? (
                            <img
                              src={
                                agent.avatar.startsWith("/")
                                  ? convertFileSrc(agent.avatar)
                                  : agent.avatar
                              }
                              className="w-full h-full object-cover"
                              alt=""
                            />
                          ) : agent?.emoji ? (
                            <span>{agent.emoji}</span>
                          ) : (
                            <Bot className="h-3.5 w-3.5" />
                          )}
                        </div>
                        {!isActive && !session.channelInfo && session.unreadCount > 0 && (
                          <span
                            className="absolute -top-1 -right-1.5 z-10 min-w-[16px] h-[16px] px-0.5 rounded-full text-white text-[9px] font-bold flex items-center justify-center border border-background pointer-events-none leading-none"
                            style={{
                              background:
                                "linear-gradient(135deg, #ff6b6b 0%, #ee3333 50%, #cc1111 100%)",
                              boxShadow:
                                "0 2px 6px rgba(220, 38, 38, 0.45), inset 0 1px 1px rgba(255, 255, 255, 0.25)",
                            }}
                          >
                            {session.unreadCount > 99 ? "99+" : session.unreadCount}
                          </span>
                        )}
                      </div>

                      {/* Title + meta */}
                      <div className="flex-1 min-w-0">
                        <div className="text-[13px] font-medium text-foreground truncate flex items-center gap-1">
                          {session.isCron && (
                            <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-orange-500/15 text-orange-500">
                              <Timer className="w-2.5 h-2.5" />
                            </span>
                          )}
                          {session.parentSessionId &&
                            (() => {
                              const parentSession = sessions.find(
                                (s) => s.id === session.parentSessionId,
                              )
                              const parentAgent = parentSession
                                ? getAgentInfo(parentSession.agentId)
                                : undefined
                              return (
                                <IconTip
                                  label={t("chat.subagentFrom", {
                                    agent: parentAgent?.name || parentSession?.agentId || "unknown",
                                  })}
                                >
                                  <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-purple-500/15 text-purple-500">
                                    <Network className="w-2.5 h-2.5" />
                                  </span>
                                </IconTip>
                              )
                            })()}
                          {session.channelInfo && (
                            <IconTip
                              label={`${session.channelInfo.channelId} · ${session.channelInfo.senderName || session.channelInfo.chatId}`}
                            >
                              <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-blue-500/15 text-blue-500">
                                <ChannelIcon channelId={session.channelInfo.channelId} className="w-2.5 h-2.5" />
                              </span>
                            </IconTip>
                          )}
                          {renamingSessionId === session.id ? (
                            <input
                              ref={renameInputRef}
                              className="flex-1 min-w-0 bg-transparent border-b border-primary text-[13px] font-medium text-foreground outline-none py-0"
                              value={renameValue}
                              onChange={(e) => setRenameValue(e.target.value)}
                              onBlur={commitRename}
                              onKeyDown={(e) => {
                                if (e.key === "Enter") {
                                  e.preventDefault()
                                  commitRename()
                                } else if (e.key === "Escape") {
                                  e.preventDefault()
                                  cancelRename()
                                }
                              }}
                              onClick={(e) => e.stopPropagation()}
                              placeholder={t("chat.renameSessionPlaceholder")}
                            />
                          ) : (
                            <span className="truncate">
                              {session.title || t("chat.newChat") || "New Chat"}
                            </span>
                          )}
                        </div>
                        <div className="text-[11px] text-muted-foreground truncate">
                          {agent?.name || session.agentId}
                          <span className="mx-1">·</span>
                          {isLoading ? (
                            <span className="text-primary animate-pulse">
                              {t("chat.thinking") || "执行中..."}
                            </span>
                          ) : (
                            formatRelativeTime(session.updatedAt)
                          )}
                        </div>
                      </div>

                      {/* Delete button (hover) */}
                      <IconTip label={t("common.delete")}>
                        <button
                          className="shrink-0 text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-destructive transition-colors p-0.5"
                          onClick={(e) => handleDeleteClick(session.id, e)}
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </IconTip>
                    </div>
                      </ContextMenuTrigger>
                      <ContextMenuContent>
                        <ContextMenuItem
                          onClick={() => startRename(session.id, session.title || t("chat.newChat") || "New Chat")}
                        >
                          <Pencil className="h-4 w-4 mr-2" />
                          {t("chat.renameSession")}
                        </ContextMenuItem>
                        <ContextMenuItem
                          onClick={handleMarkAsRead}
                          disabled={session.unreadCount === 0}
                        >
                          <CheckCheck className="h-4 w-4 mr-2" />
                          {t("chat.markAsRead")}
                        </ContextMenuItem>
                      </ContextMenuContent>
                    </ContextMenu>
                  )
                })
              )}
              {loadingMoreSessions && (
                <div className="flex justify-center py-3">
                  <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                </div>
              )}
            </div>
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

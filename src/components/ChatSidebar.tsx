import { useState, useRef, useEffect, useCallback } from "react"
import { convertFileSrc } from "@tauri-apps/api/core"
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
import {
  ChevronDown,
  ChevronRight,
  MessageSquare,
  Bot,
  Trash2,
  MessageSquarePlus,
  X,
} from "lucide-react"
import type { SessionMeta, AgentSummaryForSidebar } from "@/types/chat"

interface ChatSidebarProps {
  sessions: SessionMeta[]
  agents: AgentSummaryForSidebar[]
  currentSessionId: string | null
  panelWidth: number
  onPanelWidthChange: (width: number) => void
  onSwitchSession: (sessionId: string) => void
  onNewChat: (agentId: string) => void
  onDeleteSession: (sessionId: string) => void
}

export default function ChatSidebar({
  sessions,
  agents,
  currentSessionId,
  panelWidth,
  onPanelWidthChange,
  onSwitchSession,
  onNewChat,
  onDeleteSession,
}: ChatSidebarProps) {
  const { t } = useTranslation()
  const [agentsExpanded, setAgentsExpanded] = useState(true)
  const [showNewChatMenu, setShowNewChatMenu] = useState(false)
  const newChatMenuRef = useRef<HTMLDivElement>(null)
  const [deleteConfirmSessionId, setDeleteConfirmSessionId] = useState<string | null>(null)

  // Agent filter state
  const [selectedAgentIds, setSelectedAgentIds] = useState<Set<string>>(new Set())

  const filteredSessions = selectedAgentIds.size === 0
    ? sessions
    : sessions.filter(s => selectedAgentIds.has(s.agentId))

  const toggleAgentFilter = useCallback((agentId: string) => {
    setSelectedAgentIds(prev => {
      const next = new Set(prev)
      if (next.has(agentId)) {
        next.delete(agentId)
      } else {
        next.add(agentId)
      }
      return next
    })
  }, [])

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
    return agents.find(a => a.id === agentId)
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
          <h2 className="text-sm font-semibold text-foreground pb-1.5">{t("chat.conversations")}</h2>
          {/* New Chat button */}
          <div className="ml-auto relative" ref={newChatMenuRef}>
            <button
              className="text-muted-foreground hover:text-foreground transition-colors pb-1.5"
              onClick={() => setShowNewChatMenu(!showNewChatMenu)}
              title={t("chat.newChat") || "New Chat"}
            >
              <MessageSquarePlus className="h-4 w-4" />
            </button>
            {/* Agent selector popup */}
            {showNewChatMenu && (
              <div className="absolute right-0 top-full mt-1 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-lg z-50 min-w-[180px] p-1.5">
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
                        <img src={agent.avatar.startsWith("/") ? convertFileSrc(agent.avatar) : agent.avatar} className="w-full h-full object-cover" alt="" />
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

        <div className="flex-1 overflow-y-auto">
          {/* Collapsible Agents section */}
          <div className="border-b border-border/50">
            <div className="flex items-center">
              <button
                className="flex items-center gap-1.5 flex-1 px-4 py-2 text-[11px] font-semibold text-muted-foreground uppercase tracking-wider hover:text-foreground transition-colors"
                onClick={() => setAgentsExpanded(!agentsExpanded)}
              >
                {agentsExpanded ? (
                  <ChevronDown className="h-3 w-3" />
                ) : (
                  <ChevronRight className="h-3 w-3" />
                )}
                <span>Agents</span>
                <span className="font-normal normal-case text-muted-foreground/60 ml-0.5">({agents.length})</span>
              </button>
              {/* Clear all agent filters */}
              {selectedAgentIds.size > 0 && (
                <button
                  className="mr-3 flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[10px] text-primary bg-primary/10 hover:bg-primary/20 transition-colors"
                  onClick={() => setSelectedAgentIds(new Set())}
                  title={t("chat.clearFilter") || "Clear filter"}
                >
                  <X className="h-2.5 w-2.5" />
                  <span>{selectedAgentIds.size}</span>
                </button>
              )}
            </div>
            {agentsExpanded && (
              <div className={cn("px-2 pb-2 grid gap-1", panelWidth >= 280 ? "grid-cols-2" : "grid-cols-1")}>
                {agents.map((agent) => {
                  const isSelected = selectedAgentIds.has(agent.id)
                  return (
                    <div
                      key={agent.id}
                      className={cn(
                        "flex items-center gap-2 px-2 py-1.5 rounded-lg text-xs transition-colors truncate group/agent",
                        isSelected
                          ? "bg-primary/10"
                          : "hover:bg-secondary/60"
                      )}
                      title={agent.description || agent.name}
                    >
                      {/* Clickable area: toggle filter */}
                      <button
                        className="flex items-center gap-2 flex-1 min-w-0"
                        onClick={() => toggleAgentFilter(agent.id)}
                      >
                        <div className={cn(
                          "w-6 h-6 rounded-full flex items-center justify-center shrink-0 text-[10px] overflow-hidden",
                          isSelected ? "bg-primary/25 text-primary" : "bg-primary/15 text-primary"
                        )}>
                          {agent.avatar ? (
                            <img src={agent.avatar.startsWith("/") ? convertFileSrc(agent.avatar) : agent.avatar} className="w-full h-full object-cover" alt="" />
                          ) : agent.emoji ? (
                            <span>{agent.emoji}</span>
                          ) : (
                            <Bot className="h-3 w-3" />
                          )}
                        </div>
                        <span className={cn("truncate", isSelected ? "text-primary font-medium" : "text-foreground/80")}>
                          {agent.name}{agent.emoji ? ` ${agent.emoji}` : ""}
                        </span>
                      </button>
                      {/* New chat button */}
                      <button
                        className="shrink-0 p-0.5 rounded text-muted-foreground/0 group-hover/agent:text-muted-foreground/60 hover:!text-primary transition-colors"
                        onClick={(e) => {
                          e.stopPropagation()
                          onNewChat(agent.id)
                        }}
                        title={t("chat.newChat") || "New Chat"}
                      >
                        <MessageSquarePlus className="h-3 w-3" />
                      </button>
                    </div>
                  )
                })}
              </div>
            )}
          </div>

          {/* Session list */}
          <div className="p-2 space-y-0.5">
            {filteredSessions.length === 0 ? (
              <div className="text-center py-8">
                <MessageSquare className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
                <p className="text-xs text-muted-foreground/60">
                  {selectedAgentIds.size > 0
                    ? (t("chat.noMatchingSessions") || "No matching sessions")
                    : t("chat.startConversation")}
                </p>
              </div>
            ) : (
              filteredSessions.map((session) => {
                const agent = getAgentInfo(session.agentId)
                const isActive = session.id === currentSessionId
                return (
                  <button
                    key={session.id}
                    className={cn(
                      "flex items-center gap-2.5 w-full px-2.5 py-2 rounded-lg text-left transition-colors group",
                      isActive
                        ? "bg-secondary/70 border border-border/50"
                        : "hover:bg-secondary/40"
                    )}
                    onClick={() => onSwitchSession(session.id)}
                  >
                    {/* Agent avatar (small) */}
                    <div className="w-7 h-7 rounded-full bg-primary/10 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden">
                      {agent?.avatar ? (
                        <img src={agent.avatar.startsWith("/") ? convertFileSrc(agent.avatar) : agent.avatar} className="w-full h-full object-cover" alt="" />
                      ) : agent?.emoji ? (
                        <span>{agent.emoji}</span>
                      ) : (
                        <Bot className="h-3.5 w-3.5" />
                      )}
                    </div>

                    {/* Title + meta */}
                    <div className="flex-1 min-w-0">
                      <div className="text-[13px] font-medium text-foreground truncate">
                        {session.title || t("chat.newChat") || "New Chat"}
                      </div>
                      <div className="text-[11px] text-muted-foreground truncate">
                        {agent?.name || session.agentId}
                        <span className="mx-1">·</span>
                        {formatRelativeTime(session.updatedAt)}
                      </div>
                    </div>

                    {/* Delete button (hover) */}
                    <button
                      className="shrink-0 text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-destructive transition-colors p-0.5"
                      onClick={(e) => handleDeleteClick(session.id, e)}
                      title="Delete"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </button>
                )
              })
            )}
          </div>
        </div>
      </div>

      {/* Delete session confirmation dialog */}
      <AlertDialog open={!!deleteConfirmSessionId} onOpenChange={(open) => !open && setDeleteConfirmSessionId(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("chat.deleteSessionTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("chat.deleteSessionWarning")}
            </AlertDialogDescription>
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

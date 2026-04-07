import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  MessageSquare,
  Loader2,
} from "lucide-react"
import type { SessionMeta, AgentSummaryForSidebar } from "@/types/chat"
import type { SessionFilterType } from "./types"
import SessionItem from "./SessionItem"

interface SessionListProps {
  sessions: SessionMeta[]
  filteredSessions: SessionMeta[]
  sessionFilter: SessionFilterType
  setSessionFilter: (filter: SessionFilterType) => void
  selectedAgentId: string | null
  currentSessionId: string | null
  loadingSessionIds: Set<string>
  loadingMoreSessions?: boolean
  onSwitchSession: (sessionId: string) => void
  onDeleteClick: (sessionId: string, e: React.MouseEvent) => void
  onMarkAllRead?: () => void
  // Rename props
  renamingSessionId: string | null
  renameValue: string
  renameInputRef: React.RefObject<HTMLInputElement | null>
  onStartRename: (sessionId: string, currentTitle: string) => void
  onRenameValueChange: (value: string) => void
  onCommitRename: () => void
  onCancelRename: () => void
  // Helpers
  getAgentInfo: (agentId: string) => AgentSummaryForSidebar | undefined
  formatRelativeTime: (dateStr: string) => string
}

export default function SessionList({
  sessions,
  filteredSessions,
  sessionFilter,
  setSessionFilter,
  selectedAgentId,
  currentSessionId,
  loadingSessionIds,
  loadingMoreSessions,
  onSwitchSession,
  onDeleteClick,
  onMarkAllRead,
  renamingSessionId,
  renameValue,
  renameInputRef,
  onStartRename,
  onRenameValueChange,
  onCommitRename,
  onCancelRename,
  getAgentInfo,
  formatRelativeTime,
}: SessionListProps) {
  const { t } = useTranslation()

  return (
    <>
      {/* Session type filter tabs */}
      <div className="flex items-center gap-0.5 px-3 py-1.5 border-b border-border/40 overflow-x-auto scrollbar-none">
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
            : filterSessions.reduce((sum, s) => sum + (s.channelInfo ? 0 : s.unreadCount), 0)
          const isActive = sessionFilter === filter
          const handleMarkAllRead = async () => {
            const unreadSessions = filterSessions.filter((s) => s.unreadCount > 0)
            if (unreadSessions.length === 0) return
            try {
              await getTransport().call("mark_session_read_batch_cmd", {
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
            return (
              <SessionItem
                key={session.id}
                session={session}
                sessions={sessions}
                agent={agent}
                isActive={isActive}
                isLoading={isLoading}
                renamingSessionId={renamingSessionId}
                renameValue={renameValue}
                renameInputRef={renameInputRef}
                onSwitchSession={onSwitchSession}
                onDeleteClick={onDeleteClick}
                onStartRename={onStartRename}
                onRenameValueChange={onRenameValueChange}
                onCommitRename={onCommitRename}
                onCancelRename={onCancelRename}
                onMarkAllRead={onMarkAllRead}
                getAgentInfo={getAgentInfo}
                formatRelativeTime={formatRelativeTime}
              />
            )
          })
        )}
        {loadingMoreSessions && (
          <div className="flex justify-center py-3">
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          </div>
        )}
      </div>
    </>
  )
}

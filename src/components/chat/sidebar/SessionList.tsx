import { useMemo } from "react"
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
import { Input } from "@/components/ui/input"
import {
  MessageSquare,
  Loader2,
  Search,
  X,
} from "lucide-react"
import type {
  SessionMeta,
  AgentSummaryForSidebar,
  SessionSearchResult,
} from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import type { SessionFilterType } from "./types"
import SessionItem from "./SessionItem"
import SearchResultItem from "./SearchResultItem"

// Classify a search result into one of the sidebar filter types.
function classifyResult(r: SessionSearchResult): SessionFilterType {
  if (r.channelType) return "channel"
  if (r.isCron) return "cron"
  if (r.parentSessionId) return "subagent"
  return "session"
}

interface SessionListProps {
  sessions: SessionMeta[]
  filteredSessions: SessionMeta[]
  sessionFilter: SessionFilterType
  setSessionFilter: (filter: SessionFilterType) => void
  selectedAgentId: string | null
  currentSessionId: string | null
  loadingSessionIds: Set<string>
  loadingMoreSessions?: boolean
  onSwitchSession: (sessionId: string, opts?: { targetMessageId?: number }) => void
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
  // Search
  searchQuery: string
  onSearchQueryChange: (q: string) => void
  searchResults: SessionSearchResult[] | null
  searching: boolean
  agents: AgentSummaryForSidebar[]
  // Projects — drives the per-session "Move to project" submenu
  projects?: ProjectMeta[]
  onMoveToProject?: (sessionId: string, projectId: string | null) => void
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
  searchQuery,
  onSearchQueryChange,
  searchResults,
  searching,
  agents,
  projects = [],
  onMoveToProject,
}: SessionListProps) {
  const { t } = useTranslation()

  const isSearching = searchQuery.trim().length > 0

  // Client-side second-level filter by session type for search results.
  const visibleResults = useMemo(() => {
    if (!searchResults) return []
    if (sessionFilter === "all") return searchResults
    return searchResults.filter((r) => classifyResult(r) === sessionFilter)
  }, [searchResults, sessionFilter])

  return (
    <>
      {/* Search input */}
      <div className="relative px-3 pt-2 pb-1.5">
        <Search className="absolute left-5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground pointer-events-none" />
        <Input
          value={searchQuery}
          onChange={(e) => onSearchQueryChange(e.target.value)}
          placeholder={t("chat.searchPlaceholder")}
          className="h-7 pl-7 pr-7 text-xs"
        />
        {searchQuery && (
          <button
            onClick={() => onSearchQueryChange("")}
            className="absolute right-5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            aria-label={t("common.clear") || "Clear"}
          >
            <X className="h-3 w-3" />
          </button>
        )}
      </div>

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

          // In search mode, show result counts per type instead of unread counts.
          let count = 0
          if (isSearching && searchResults) {
            if (filter === "all") {
              count = searchResults.length
            } else {
              count = searchResults.filter((r) => classifyResult(r) === filter).length
            }
          } else {
            const filterSessions = {
              all: sessions,
              session: sessions.filter((s) => !s.isCron && !s.parentSessionId && !s.channelInfo),
              channel: sessions.filter((s) => !!s.channelInfo),
              cron: sessions.filter((s) => s.isCron),
              subagent: sessions.filter((s) => !!s.parentSessionId),
            }[filter]
            count = filter === "channel"
              ? 0  // Channel sessions don't show unread counts
              : filterSessions.reduce((sum, s) => sum + (s.channelInfo ? 0 : s.unreadCount), 0)
          }

          const isActive = sessionFilter === filter
          const handleMarkAllRead = async () => {
            if (isSearching) return
            const filterSessions = {
              all: sessions,
              session: sessions.filter((s) => !s.isCron && !s.parentSessionId && !s.channelInfo),
              channel: sessions.filter((s) => !!s.channelInfo),
              cron: sessions.filter((s) => s.isCron),
              subagent: sessions.filter((s) => !!s.parentSessionId),
            }[filter]
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
                <ContextMenuItem onClick={handleMarkAllRead} disabled={isSearching || count === 0}>
                  {t("chat.markAllRead") || "全部已读"}
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          )
        })}
      </div>

      {/* Search results or session list */}
      {isSearching ? (
        <div className="p-2 space-y-0.5">
          {searching && visibleResults.length === 0 ? (
            <div className="flex justify-center py-6">
              <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
            </div>
          ) : visibleResults.length === 0 ? (
            <div className="text-center py-8">
              <Search className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
              <p className="text-xs text-muted-foreground/60">
                {t("chat.noSearchResults")}
              </p>
            </div>
          ) : (
            visibleResults.map((result) => (
              <SearchResultItem
                key={`${result.sessionId}-${result.messageId}`}
                result={result}
                isActive={result.sessionId === currentSessionId}
                agent={getAgentInfo(result.agentId)}
                agents={agents}
                sessionMeta={sessions.find((s) => s.id === result.sessionId)}
                onSwitch={() =>
                  onSwitchSession(result.sessionId, { targetMessageId: result.messageId })
                }
                formatRelativeTime={formatRelativeTime}
              />
            ))
          )}
        </div>
      ) : (
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
                  projects={projects}
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
                  onMoveToProject={onMoveToProject}
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
      )}
    </>
  )
}

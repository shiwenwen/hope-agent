import { useMemo } from "react"
import { parseHighlightTerms } from "@/lib/inlineHighlight"
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
  Search,
} from "lucide-react"
import type {
  SessionMeta,
  AgentSummaryForSidebar,
  SessionSearchResult,
} from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import type { SessionFilterType, SidebarDisplayMode } from "./types"
import SessionItem from "./SessionItem"
import SearchResultItem from "./SearchResultItem"

// Classify a search result into one of the sidebar filter types. Channel
// results fold into `session` — IM-driven conversations are still user
// conversations, just sourced from a different surface.
function classifyResult(r: SessionSearchResult): SessionFilterType {
  if (r.isCron) return "cron"
  if (r.parentSessionId) return "subagent"
  return "session"
}

function unreadCountForDisplay(session: SessionMeta): number {
  if (session.channelInfo || session.parentSessionId) return 0
  return session.unreadCount
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
  onSwitchSession: (
    sessionId: string,
    opts?: { targetMessageId?: number; highlightTerms?: string[] },
  ) => void
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
  searchResults: SessionSearchResult[] | null
  /** True when the result set hit the backend limit and may have been
   *  truncated. Surfaced as a hint above the result list. */
  searchTruncated?: boolean
  searching: boolean
  agents: AgentSummaryForSidebar[]
  // Projects — drives the per-session "Move to project" submenu
  projects?: ProjectMeta[]
  onMoveToProject?: (sessionId: string, projectId: string | null) => void
  onToggleSessionPinned?: (sessionId: string, pinned: boolean) => void
  displayMode: SidebarDisplayMode
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
  searchResults,
  searchTruncated = false,
  searching,
  agents,
  projects = [],
  onMoveToProject,
  onToggleSessionPinned,
  displayMode,
}: SessionListProps) {
  const { t } = useTranslation()

  const isSearching = searchQuery.trim().length > 0

  // Client-side second-level filter by session type for search results.
  const visibleResults = useMemo(() => {
    if (!searchResults) return []
    if (sessionFilter === "all") return searchResults
    return searchResults.filter((r) => classifyResult(r) === sessionFilter)
  }, [searchResults, sessionFilter])

  const highlightTerms = useMemo(
    () => parseHighlightTerms(searchQuery),
    [searchQuery],
  )
  const projectsById = useMemo(() => new Map(projects.map((p) => [p.id, p])), [projects])

  return (
    <>
      {/* Session type filter tabs */}
      <div className="flex items-center gap-0.5 px-3 py-1.5 border-b border-border/40 overflow-x-auto scrollbar-none">
        {(["all", "session", "cron", "subagent"] as const).map((filter) => {
          const label = {
            all: t("chat.filterAll"),
            session: t("chat.filterSessions"),
            cron: t("chat.filterCron"),
            subagent: t("chat.filterSubagent"),
          }[filter]

          // In search mode, show result counts per type instead of unread counts.
          let count: number
          if (isSearching && searchResults) {
            if (filter === "all") {
              count = searchResults.length
            } else {
              count = searchResults.filter((r) => classifyResult(r) === filter).length
            }
          } else {
            const filterSessions = {
              all: sessions,
              // Channel-bound conversations land under the unified "session"
              // tab — they're still user chats, just surfaced from IM.
              session: sessions.filter((s) => !s.isCron && !s.parentSessionId),
              cron: sessions.filter((s) => s.isCron),
              subagent: sessions.filter((s) => !!s.parentSessionId),
            }[filter]
            count = filterSessions.reduce((sum, s) => sum + unreadCountForDisplay(s), 0)
          }

          const isActive = sessionFilter === filter
          const handleMarkAllRead = async () => {
            if (isSearching) return
            const filterSessions = {
              all: sessions,
              session: sessions.filter((s) => !s.isCron && !s.parentSessionId),
              cron: sessions.filter((s) => s.isCron),
              subagent: sessions.filter((s) => !!s.parentSessionId),
            }[filter]
            const unreadSessions = filterSessions.filter((s) => unreadCountForDisplay(s) > 0)
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
        <div
          key={`search-${sessionFilter}-${displayMode}`}
          className={cn(
            "p-2 animate-in fade-in-0 slide-in-from-bottom-1 duration-150",
            displayMode === "compact" ? "space-y-1" : "space-y-0.5",
          )}
        >
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
            <>
              {searchTruncated && (
                <div className="px-2 py-1 mb-1 text-[10px] text-muted-foreground/70 leading-snug">
                  {t("chat.searchTruncatedHint", { count: visibleResults.length })}
                </div>
              )}
              {visibleResults.map((result) => {
                const sessionMeta = sessions.find((s) => s.id === result.sessionId)
                const projectId = result.projectId ?? sessionMeta?.projectId ?? null
                return (
                  <SearchResultItem
                    key={`${result.sessionId}-${result.messageId}`}
                    result={result}
                    isActive={result.sessionId === currentSessionId}
                    agent={getAgentInfo(result.agentId)}
                    agents={agents}
                    sessionMeta={sessionMeta}
                    project={projectId ? projectsById.get(projectId) : undefined}
                    projectId={projectId}
                    onSwitch={() =>
                      onSwitchSession(result.sessionId, {
                        targetMessageId: result.messageId,
                        highlightTerms,
                      })
                    }
                    formatRelativeTime={formatRelativeTime}
                    displayMode={displayMode}
                  />
                )
              })}
            </>
          )}
        </div>
      ) : (
        <div
          key={`sessions-${sessionFilter}-${selectedAgentId ?? "all"}-${displayMode}`}
          className={cn(
            "p-2 animate-in fade-in-0 slide-in-from-bottom-1 duration-150",
            displayMode === "compact" ? "space-y-1" : "space-y-0.5",
          )}
        >
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
                  onTogglePinned={onToggleSessionPinned}
                  getAgentInfo={getAgentInfo}
                  formatRelativeTime={formatRelativeTime}
                  displayMode={displayMode}
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

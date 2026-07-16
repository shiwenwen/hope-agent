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
import { MessageSquare, Loader2, Search } from "lucide-react"
import type {
  SessionMeta,
  AgentSummaryForSidebar,
  SessionSearchResult,
  UnreadSessionTarget,
} from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import type { SessionFilterType, SidebarDisplayMode } from "./types"
import SessionItem from "./SessionItem"
import SearchResultItem from "./SearchResultItem"
import { filterGlobalSessionSearchResults } from "./sessionListModel"

interface SessionListProps {
  sessions: SessionMeta[]
  sessionsByFilter: Record<SessionFilterType, SessionMeta[]>
  filteredSessions: SessionMeta[]
  sessionFilter: SessionFilterType
  setSessionFilter: (filter: SessionFilterType) => void
  selectedAgentId: string | null
  currentSessionId: string | null
  readableSessionId: string | null
  loadingSessionIds: Set<string>
  sessionsLoading?: boolean
  totalUnreadCount: number
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
  /** Number of visible 32px sticky section headers above the filter tabs. */
  stickyHeaderCount?: number
  unreadFocusTarget?: (UnreadSessionTarget & { signal: number }) | null
}

export default function SessionList({
  sessions,
  sessionsByFilter,
  filteredSessions,
  sessionFilter,
  setSessionFilter,
  selectedAgentId,
  currentSessionId,
  readableSessionId,
  loadingSessionIds,
  sessionsLoading = false,
  totalUnreadCount,
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
  stickyHeaderCount = 0,
  unreadFocusTarget,
}: SessionListProps) {
  const { t } = useTranslation()

  const isSearching = searchQuery.trim().length > 0
  const showInitialSessionLoading =
    !isSearching && sessionsLoading && sessions.length === 0 && filteredSessions.length === 0

  // Search is deliberately global and independent of the browsing tab. This
  // preserves discovery across regular, project, channel, and subagent chats
  // after removing the redundant "All" browsing tab.
  const visibleResults = useMemo(
    () => filterGlobalSessionSearchResults(searchResults),
    [searchResults],
  )

  const highlightTerms = useMemo(() => parseHighlightTerms(searchQuery), [searchQuery])
  const projectsById = useMemo(() => new Map(projects.map((p) => [p.id, p])), [projects])
  const sessionContext = useMemo(() => {
    const byId = new Map(sessions.map((session) => [session.id, session]))
    for (const session of [...sessionsByFilter.session, ...sessionsByFilter.subagent]) {
      byId.set(session.id, session)
    }
    return [...byId.values()]
  }, [sessions, sessionsByFilter])

  return (
    <>
      {/* Browsing tabs are hidden during search because search always spans all
          supported conversation types, regardless of the previously active tab. */}
      {!isSearching && (
        <div
          className={cn(
            "sticky z-20 flex items-center gap-0.5 px-3 py-1.5 border-b border-border/40 bg-surface-panel overflow-x-auto scrollbar-none",
            stickyHeaderCount === 0 ? "top-0" : stickyHeaderCount === 1 ? "top-8" : "top-16",
          )}
        >
          {(["session", "subagent"] as const).map((filter) => {
            const label = {
              session: t("chat.filterSessions"),
              subagent: t("chat.filterSubagent"),
            }[filter]

            const isActive = sessionFilter === filter
            const handleMarkAllRead = async () => {
              if (filter !== "session") return
              try {
                await getTransport().call("mark_all_sessions_read_cmd")
                onMarkAllRead?.()
              } catch (err) {
                logger.error(
                  "chat",
                  "ChatSidebar::markSessionsRead",
                  "Failed to mark all regular sessions as read",
                  err,
                )
              }
            }

            return (
              <ContextMenu key={filter}>
                <ContextMenuTrigger asChild>
                  <button
                    className={cn(
                      "relative px-2 py-1 text-[11px] rounded-md whitespace-nowrap",
                      isActive
                        ? "text-foreground font-semibold"
                        : "text-muted-foreground hover:text-foreground/70",
                    )}
                    onClick={() => setSessionFilter(filter)}
                  >
                    {label}
                    {isActive && (
                      <span className="absolute bottom-0 left-1/2 -translate-x-1/2 w-3/5 h-[2px] rounded-full bg-primary" />
                    )}
                  </button>
                </ContextMenuTrigger>
                <ContextMenuContent variant="floating">
                  <ContextMenuItem
                    onClick={handleMarkAllRead}
                    disabled={filter !== "session" || totalUnreadCount === 0}
                  >
                    {t("chat.markAllRead")}
                  </ContextMenuItem>
                </ContextMenuContent>
              </ContextMenu>
            )
          })}
        </div>
      )}

      {/* Search results or session list */}
      {isSearching ? (
        <div
          key="search-global"
          className={cn("p-2", displayMode === "compact" ? "space-y-1" : "space-y-0.5")}
        >
          {searching && visibleResults.length === 0 ? (
            <div className="flex justify-center py-6">
              <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
            </div>
          ) : visibleResults.length === 0 ? (
            <div className="text-center py-8">
              <Search className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
              <p className="text-xs text-muted-foreground/60">{t("chat.noSearchResults")}</p>
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
                    key={`${result.sessionId}-${result.matchKind}-${result.messageId}`}
                    result={result}
                    isActive={result.sessionId === currentSessionId}
                    agent={getAgentInfo(result.agentId)}
                    agents={agents}
                    sessionMeta={sessionMeta}
                    project={projectId ? projectsById.get(projectId) : undefined}
                    projectId={projectId}
                    onSwitch={() => {
                      if (result.matchKind === "title") {
                        onSwitchSession(result.sessionId)
                        return
                      }
                      onSwitchSession(result.sessionId, {
                        targetMessageId: result.messageId,
                        highlightTerms,
                      })
                    }}
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
          key={`sessions-${sessionFilter}-${selectedAgentId ?? "all"}`}
          className={cn("p-2", displayMode === "compact" ? "space-y-1" : "space-y-0.5")}
        >
          {showInitialSessionLoading ? (
            <div className="flex justify-center py-8">
              <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
            </div>
          ) : filteredSessions.length === 0 ? (
            <div className="text-center py-8">
              <MessageSquare className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
              <p className="text-xs text-muted-foreground/60">
                {selectedAgentId !== null
                  ? t("chat.noMatchingSessions")
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
                  sessions={sessionContext}
                  agent={agent}
                  projects={projects}
                  isActive={isActive}
                  isReadable={session.id === readableSessionId}
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
                  revealSignal={
                    unreadFocusTarget?.sessionId === session.id
                      ? unreadFocusTarget.signal
                      : undefined
                  }
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

import { useCallback, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"

import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { cn } from "@/lib/utils"
import type { AgentSummaryForSidebar, SessionMeta, UnreadSessionTarget } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import SessionItem from "./SessionItem"
import SidebarSectionHeader from "./SidebarSectionHeader"
import type { SidebarDisplayMode } from "./types"

const PINNED_EXPANDED_STORAGE_KEY = "hope.chatSidebarPinnedExpanded"

function readExpanded(): boolean {
  try {
    if (typeof window === "undefined") return true
    const raw = window.localStorage.getItem(PINNED_EXPANDED_STORAGE_KEY)
    return raw === null ? true : raw === "true"
  } catch {
    return true
  }
}

interface PinnedSectionProps {
  pinnedSessions: SessionMeta[]
  sessions: SessionMeta[]
  projects: ProjectMeta[]
  currentSessionId: string | null
  readableSessionId: string | null
  loadingSessionIds: Set<string>
  onSwitchSession: (sessionId: string) => void
  onArchiveClick: (sessionId: string, event: React.MouseEvent) => void
  onMarkAllRead?: () => void
  renamingSessionId: string | null
  renameValue: string
  renameInputRef: React.RefObject<HTMLInputElement | null>
  onStartRename: (sessionId: string, currentTitle: string) => void
  onRenameValueChange: (value: string) => void
  onCommitRename: () => void
  onCancelRename: () => void
  onMoveToProject?: (sessionId: string, projectId: string | null) => void
  onToggleSessionPinned?: (session: SessionMeta, pinned: boolean) => void
  getAgentInfo: (agentId: string) => AgentSummaryForSidebar | undefined
  formatRelativeTime: (dateStr: string) => string
  displayMode: SidebarDisplayMode
  motionDisabled?: boolean
  unreadFocusTarget?: (UnreadSessionTarget & { signal: number }) | null
}

export default function PinnedSection({
  pinnedSessions,
  sessions,
  projects,
  currentSessionId,
  readableSessionId,
  loadingSessionIds,
  onSwitchSession,
  onArchiveClick,
  onMarkAllRead,
  renamingSessionId,
  renameValue,
  renameInputRef,
  onStartRename,
  onRenameValueChange,
  onCommitRename,
  onCancelRename,
  onMoveToProject,
  onToggleSessionPinned,
  getAgentInfo,
  formatRelativeTime,
  displayMode,
  motionDisabled = false,
  unreadFocusTarget,
}: PinnedSectionProps) {
  const { t } = useTranslation()
  const [expanded, setExpandedState] = useState(readExpanded)
  const visibleExpanded = expanded || unreadFocusTarget?.pinned === true
  const sessionContext = useMemo(() => {
    const byId = new Map(sessions.map((session) => [session.id, session]))
    for (const session of pinnedSessions) byId.set(session.id, session)
    return [...byId.values()]
  }, [pinnedSessions, sessions])

  const setExpanded = useCallback((next: boolean) => {
    setExpandedState(next)
    try {
      window.localStorage.setItem(PINNED_EXPANDED_STORAGE_KEY, String(next))
    } catch {
      // localStorage may be unavailable in restricted browser modes.
    }
  }, [])

  return (
    <div className="contents">
      <SidebarSectionHeader
        title={t("chat.pinnedSessions")}
        count={pinnedSessions.length}
        expanded={visibleExpanded}
        onToggle={() => setExpanded(!expanded)}
        className="sticky top-0 z-20 mb-0 flex h-8 items-center border-b border-border/50 bg-surface-panel px-3"
      />
      <AnimatedCollapse open={visibleExpanded} durationMs={motionDisabled ? 0 : undefined}>
        <div
          className={cn("px-2 pb-1 pt-1", displayMode === "compact" ? "space-y-1" : "space-y-0.5")}
        >
          {pinnedSessions.map((session) => (
            <SessionItem
              key={session.id}
              session={session}
              sessions={sessionContext}
              agent={getAgentInfo(session.agentId)}
              showSubagentBadge
              projects={projects}
              isActive={session.id === currentSessionId}
              isReadable={session.id === readableSessionId}
              isLoading={loadingSessionIds.has(session.id)}
              renamingSessionId={renamingSessionId}
              renameValue={renameValue}
              renameInputRef={renameInputRef}
              onSwitchSession={onSwitchSession}
              onArchiveClick={onArchiveClick}
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
                unreadFocusTarget?.pinned && unreadFocusTarget.sessionId === session.id
                  ? unreadFocusTarget.signal
                  : undefined
              }
            />
          ))}
        </div>
      </AnimatedCollapse>
    </div>
  )
}

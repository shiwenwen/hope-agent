import type { SessionMeta, AgentSummaryForSidebar } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"

export interface ChatSidebarProps {
  sessions: SessionMeta[]
  agents: AgentSummaryForSidebar[]
  /** Projects visible in the sidebar. Empty array when none exist. */
  projects?: ProjectMeta[]
  currentSessionId: string | null
  loadingSessionIds: Set<string>
  panelWidth: number
  onPanelWidthChange: (width: number) => void
  onSwitchSession: (
    sessionId: string,
    opts?: { targetMessageId?: number; highlightTerms?: string[] },
  ) => void
  onNewChat: (agentId: string, opts?: { incognito?: boolean }) => void
  onDeleteSession: (sessionId: string) => void
  onEditAgent?: (agentId: string) => void
  onMarkAllRead?: () => void
  onRenameSession?: (sessionId: string, title: string) => void
  hasMoreSessions?: boolean
  loadingMoreSessions?: boolean
  onLoadMoreSessions?: () => void
  /** Triggered by the gear button / right-click → "Settings" entry on a project row. */
  onOpenProjectSettings?: (project: ProjectMeta) => void
  /** Triggered by the "+ New Project" sidebar button. */
  onAddProject?: () => void
  /** Triggered by the hover "+" button or right-click → "New chat" on a project row. */
  onNewChatInProject?: (projectId: string, opts?: { incognito?: boolean }) => void
  /** Triggered by the right-click → "Archive / Unarchive" entry on a project row. */
  onArchiveProject?: (projectId: string, archived: boolean) => void
  /**
   * Triggered by the per-session "Move to project" context-menu entry.
   * Passing `projectId=null` removes the session from its current project.
   */
  onMoveSessionToProject?: (sessionId: string, projectId: string | null) => void
  /**
   * Incremented by the parent (e.g. via `Cmd+Shift+F`) to focus the sidebar
   * search input. Each new value triggers a focus-and-select on the input,
   * even if the same value is sent twice (the parent should monotonically
   * increment).
   */
  searchFocusSignal?: number
}

export type SessionFilterType = "all" | "session" | "cron" | "subagent" | "channel"

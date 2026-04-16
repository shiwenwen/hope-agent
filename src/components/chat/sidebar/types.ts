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
  onSwitchSession: (sessionId: string, opts?: { targetMessageId?: number }) => void
  onNewChat: (agentId: string, projectId?: string | null) => void
  onDeleteSession: (sessionId: string) => void
  onEditAgent?: (agentId: string) => void
  onMarkAllRead?: () => void
  onRenameSession?: (sessionId: string, title: string) => void
  hasMoreSessions?: boolean
  loadingMoreSessions?: boolean
  onLoadMoreSessions?: () => void
  /** Triggered when the user clicks a project row in the sidebar. */
  onOpenProject?: (project: ProjectMeta) => void
  /** Triggered by the "+ New Project" sidebar button. */
  onAddProject?: () => void
  /**
   * Triggered by the per-session "Move to project" context-menu entry.
   * Passing `projectId=null` removes the session from its current project.
   */
  onMoveSessionToProject?: (sessionId: string, projectId: string | null) => void
}

export type SessionFilterType = "all" | "session" | "cron" | "subagent" | "channel"

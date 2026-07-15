import type { SessionMeta, AgentSummaryForSidebar } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"

export const CHAT_SIDEBAR_WIDTH_STORAGE_KEY = "hope.chatSidebarPanelWidth"
export const CHAT_SIDEBAR_LEGACY_DEFAULT_WIDTH = 260
export const CHAT_SIDEBAR_DEFAULT_WIDTH = 280
export const CHAT_SIDEBAR_MIN_WIDTH = 220
export const CHAT_SIDEBAR_MAX_WIDTH = 360

export type SidebarDisplayMode = "compact" | "detailed"
export const DEFAULT_SIDEBAR_DISPLAY_MODE: SidebarDisplayMode = "compact"

export function normalizeSidebarDisplayMode(value: unknown): SidebarDisplayMode {
  return value === "compact" || value === "detailed" ? value : DEFAULT_SIDEBAR_DISPLAY_MODE
}

export interface ChatSidebarProps {
  sessions: SessionMeta[]
  agents: AgentSummaryForSidebar[]
  /** Projects visible in the sidebar. Empty array when none exist. */
  projects?: ProjectMeta[]
  projectsLoading?: boolean
  currentSessionId: string | null
  /** Session whose latest assistant output is actually inside the reading surface. */
  readableSessionId: string | null
  loadingSessionIds: Set<string>
  sessionsLoading?: boolean
  /** Authoritative whole-database regular unread-session aggregate. */
  totalUnreadCount: number
  panelWidth: number
  sidebarCollapsed: boolean
  onPanelWidthChange: (width: number) => void
  onSidebarCollapsedChange: (collapsed: boolean) => void
  onSwitchSession: (
    sessionId: string,
    opts?: { targetMessageId?: number; highlightTerms?: string[] },
  ) => void
  onNewChat: (agentId: string, opts?: { incognito?: boolean }) => void
  onDeleteSession: (sessionId: string) => void | Promise<void>
  onEditAgent?: (agentId: string) => void
  onToggleSessionPinned?: (sessionId: string, pinned: boolean) => void
  onReorderAgents?: (agentIds: string[]) => void
  onReorderProjects?: (projectIds: string[]) => void
  onMarkAllRead?: () => void
  onRenameSession?: (sessionId: string, title: string) => void
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
  onMoveSessionToProject?: (sessionId: string, projectId: string | null) => void | Promise<void>
  /**
   * Incremented by the parent (e.g. via `Cmd+Shift+F`) to focus the sidebar
   * search input. Each new value triggers a focus-and-select on the input,
   * even if the same value is sent twice (the parent should monotonically
   * increment).
   */
  searchFocusSignal?: number
  /** Reveal the first unread regular conversation in sidebar order. */
  unreadFocusSignal?: number
}

export type SessionFilterType = "session" | "subagent"

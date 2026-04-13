import type { SessionMeta, AgentSummaryForSidebar } from "@/types/chat"

export interface ChatSidebarProps {
  sessions: SessionMeta[]
  agents: AgentSummaryForSidebar[]
  currentSessionId: string | null
  loadingSessionIds: Set<string>
  panelWidth: number
  onPanelWidthChange: (width: number) => void
  onSwitchSession: (sessionId: string, opts?: { targetMessageId?: number }) => void
  onNewChat: (agentId: string) => void
  onDeleteSession: (sessionId: string) => void
  onEditAgent?: (agentId: string) => void
  onMarkAllRead?: () => void
  onRenameSession?: (sessionId: string, title: string) => void
  hasMoreSessions?: boolean
  loadingMoreSessions?: boolean
  onLoadMoreSessions?: () => void
}

export type SessionFilterType = "all" | "session" | "cron" | "subagent" | "channel"

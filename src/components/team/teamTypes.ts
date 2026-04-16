// ── Team Types ──────────────────────────────────────────────────

export type TeamStatus = "active" | "paused" | "dissolved"
export type MemberRole = "lead" | "worker" | "reviewer"
export type MemberStatus = "idle" | "working" | "paused" | "completed" | "error" | "killed"
export type TeamMessageType = "chat" | "task_update" | "handoff" | "system"

export interface TeamConfig {
  maxMembers: number
  autoDissolveOnComplete: boolean
  sharedContext?: string
}

export interface Team {
  teamId: string
  name: string
  description?: string
  leadSessionId: string
  leadAgentId: string
  status: TeamStatus
  createdAt: string
  updatedAt: string
  templateId?: string
  config: TeamConfig
}

export interface TeamMember {
  memberId: string
  teamId: string
  name: string
  agentId: string
  role: MemberRole
  status: MemberStatus
  runId?: string
  sessionId?: string
  color: string
  currentTaskId?: number
  modelOverride?: string
  joinedAt: string
  lastActiveAt?: string
  inputTokens?: number
  outputTokens?: number
}

export interface TeamMessage {
  messageId: string
  teamId: string
  fromMemberId: string
  toMemberId?: string
  content: string
  messageType: TeamMessageType
  timestamp: string
}

export interface TeamTask {
  id: number
  teamId: string
  content: string
  status: string
  ownerMemberId?: string
  priority: number
  blockedBy: number[]
  blocks: number[]
  columnName: string
  createdAt: string
  updatedAt: string
}

export interface TeamTemplate {
  templateId: string
  name: string
  description: string
  members: TeamTemplateMember[]
  builtin: boolean
}

export interface TeamTemplateMember {
  name: string
  role: MemberRole
  agentId: string
  color: string
  description: string
}

export interface TeamEvent {
  type: string
  payload: unknown
}

export interface TeamSummary {
  totalMembers: number
  activeMembers: number
  completedMembers: number
  totalTasks: number
  completedTasks: number
  totalInputTokens: number
  totalOutputTokens: number
}

// Status display config
export const MEMBER_STATUS_CONFIG: Record<
  MemberStatus,
  { color: string; bgColor: string; label: string }
> = {
  idle: { color: "text-gray-500", bgColor: "bg-gray-100", label: "Idle" },
  working: { color: "text-blue-500", bgColor: "bg-blue-100", label: "Working" },
  paused: { color: "text-yellow-500", bgColor: "bg-yellow-100", label: "Paused" },
  completed: { color: "text-green-500", bgColor: "bg-green-100", label: "Completed" },
  error: { color: "text-red-500", bgColor: "bg-red-100", label: "Error" },
  killed: { color: "text-gray-400", bgColor: "bg-gray-100", label: "Killed" },
}

export const KANBAN_COLUMNS = ["todo", "doing", "review", "done"] as const
export type KanbanColumn = (typeof KANBAN_COLUMNS)[number]

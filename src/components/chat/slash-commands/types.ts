/** Matches Rust CommandCategory enum */
export type CommandCategory = "session" | "model" | "memory" | "agent" | "utility" | "skill"

/** Slash command definition from backend */
export interface SlashCommandDef {
  name: string
  category: CommandCategory
  descriptionKey: string
  hasArgs: boolean
  argsOptional?: boolean
  argPlaceholder?: string
  argOptions?: string[]
  /** Raw description for skill commands (no i18n key). */
  descriptionRaw?: string
}

/** Matches Rust CommandAction enum (tagged union via "type" field) */
export type CommandAction =
  | { type: "newSession"; sessionId: string }
  | { type: "switchModel"; providerId: string; modelId: string }
  | { type: "setEffort"; effort: string }
  | { type: "switchAgent"; agentId: string; sessionId: string }
  | { type: "stopStream" }
  | { type: "compact" }
  | { type: "sessionCleared" }
  | { type: "passThrough"; message: string }
  | { type: "exportFile"; content: string; filename: string }
  | { type: "setToolPermission"; mode: "default" | "smart" | "yolo" }
  | { type: "displayOnly" }
  | { type: "showModelPicker"; models: ModelPickerItem[]; activeProviderId?: string; activeModelId?: string }
  | { type: "enterPlanMode" }
  | { type: "exitPlanMode"; planContent?: string }
  | { type: "approvePlan"; planContent?: string }
  | { type: "showPlan"; planContent: string }
  | { type: "pausePlan" }
  | { type: "resumePlan" }
  | { type: "viewSystemPrompt" }
  | { type: "showContextBreakdown"; breakdown: ContextBreakdown }
  | { type: "showProjectPicker"; projects: ProjectPickerItem[] }
  | { type: "enterProject"; projectId: string }
  | { type: "assignProject"; projectId: string }
  | { type: "showSessionPicker"; sessions: SessionPickerItem[] }
  | { type: "enterSession"; sessionId: string }
  | { type: "attachToSession"; sessionId: string }
  | { type: "detachFromSession" }
  | {
      type: "handoverToChannel"
      sessionId: string
      channelId: string
      accountId: string
      chatId: string
      threadId?: string | null
    }
  | { type: "skillFork"; runId: string; skillName: string }
  | { type: "recapCard"; reportId: string }
  | { type: "openDashboardTab"; tab: string }

/** Per-category context window usage snapshot (mirrors Rust `ContextBreakdown`). */
export interface ContextBreakdown {
  contextWindow: number
  maxOutputTokens: number
  systemPromptTokens: number
  toolSchemasTokens: number
  toolDescriptionsTokens: number
  memoryTokens: number
  skillTokens: number
  messagesTokens: number
  usedTotal: number
  freeSpace: number
  usagePct: number
  lastCompactTier?: number | null
  lastCompactSecsAgo?: number | null
  nextCompactAllowedInSecs?: number | null
  activeModel: string
  activeProvider: string
  activeAgent: string
  messageCount: number
}

/** A model entry in the model picker card */
export interface ModelPickerItem {
  providerId: string
  providerName: string
  modelId: string
  modelName: string
}

/** A project entry surfaced by the `/project` picker. Mirrors Rust
 *  `ProjectPickerItem`. */
export interface ProjectPickerItem {
  id: string
  name: string
  emoji?: string | null
  logo?: string | null
  color?: string | null
  description?: string | null
  sessionCount: number
}

/** A session entry surfaced by the `/sessions` picker. Mirrors Rust
 *  `SessionPickerItem`. */
export interface SessionPickerItem {
  id: string
  title: string
  agentId: string
  /** Friendly agent name (`AgentConfig.name`), falling back to `agentId`.
   *  Resolved server-side. */
  agentLabel: string
  projectId?: string | null
  /** Project display label (emoji + name) when the session is assigned to
   *  a project. Resolved server-side. */
  projectLabel?: string | null
  /** When set, the session is currently surfaced by an IM chat. Display as
   *  a small chip so the user can spot IM-shared sessions. */
  channelLabel?: string | null
  /** RFC3339 timestamp matching `SessionMeta.updatedAt` shape. */
  updatedAt: string
  /** When set, the session was matched via FTS5 message-body search.
   *  Already sanitized (FTS5 sentinels stripped, newlines collapsed,
   *  truncated). Pickers should render it on a second indented line. */
  snippet?: string | null
}

/** Matches Rust CommandResult struct */
export interface CommandResult {
  content: string
  action?: CommandAction
  /** Frontend-only: set by useSlashCommands when a skill passThrough is detected */
  _isSkillPassThrough?: boolean
  /** Frontend-only: user arguments extracted from skill command (e.g. "把主题改成深色") */
  _skillArgs?: string
  /** Frontend-only: the raw slash command text the user typed, e.g. "/drawio 画网络图".
   *  Used so the UI shows what the user typed instead of the expanded skill prompt. */
  _skillCommandText?: string
}

/** Category display order */
export const CATEGORY_ORDER: CommandCategory[] = [
  "session",
  "model",
  "memory",
  "agent",
  "utility",
  "skill",
]

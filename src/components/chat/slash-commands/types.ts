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
  | { type: "setToolPermission"; mode: string }
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

/** Matches Rust CommandResult struct */
export interface CommandResult {
  content: string
  action?: CommandAction
  /** Frontend-only: set by useSlashCommands when a skill passThrough is detected */
  _isSkillPassThrough?: boolean
  /** Frontend-only: user arguments extracted from skill command (e.g. "把主题改成深色") */
  _skillArgs?: string
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

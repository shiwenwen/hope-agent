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

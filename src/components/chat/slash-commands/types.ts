/** Matches Rust CommandCategory enum */
export type CommandCategory = "session" | "model" | "memory" | "agent" | "utility"

/** Slash command definition from backend */
export interface SlashCommandDef {
  name: string
  category: CommandCategory
  descriptionKey: string
  hasArgs: boolean
  argPlaceholder?: string
  argOptions?: string[]
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
  | { type: "displayOnly" }

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
]

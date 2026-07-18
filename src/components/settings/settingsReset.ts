import type { SettingsSection } from "./types"

export type SettingsResetScope =
  | "general"
  | "tools"
  | "memory"
  | "knowledge"
  | "design"
  | "chat"
  | "cron"
  | "plan"
  | "recap"
  | "server"
  | "files"
  | "sandbox"
  | "browser"
  | "acp"
  | "notifications"
  | "approval"
  | "security"
  | "logs"

export type SettingsResetSection =
  | "appearance"
  | "system"
  | "network"
  | "general"
  | "web_search"
  | "web_fetch"
  | "media_gen"
  | "canvas"
  | "async_tools"
  | "issue_reporting"
  | "basic"
  | "awareness"
  | "context_compact"
  | "dangerous"
  | "ssrf"
  | "global"
  | "startup"
  | "extract"
  | "recall_summary"
  | "budget"
  | "retrieval"
  | "dreaming"
  | "compile"
  | "vision"
  | "note_tools"
  | "search"
  | "passive_recall"
  | "source_limits"
  | "media_retention"
  | "maintenance"
  | "sprite"
  | "protected_paths"
  | "edit_commands"
  | "dangerous_commands"

export type SettingsResetLevel = "page" | "tab" | "region"

export const RESET_SCOPE_BY_SECTION: Partial<Record<SettingsSection, SettingsResetScope>> = {
  general: "general",
  tools: "tools",
  memory: "memory",
  knowledge: "knowledge",
  design: "design",
  chat: "chat",
  cron: "cron",
  plan: "plan",
  recap: "recap",
  server: "server",
  files: "files",
  sandbox: "sandbox",
  browser: "browser",
  acp: "acp",
  notifications: "notifications",
  approval: "approval",
  security: "security",
  logs: "logs",
}

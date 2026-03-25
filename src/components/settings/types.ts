export type SettingsSection =
  | "general"
  | "modelConfig"
  | "tools"
  | "skills"
  | "agents"
  | "memory"
  | "cron"
  | "notifications"
  | "sandbox"
  | "permissions"
  | "profile"
  | "chat"
  | "logs"
  | "health"
  | "about"
  | "developer"

export interface SettingsSectionItem {
  id: SettingsSection
  icon: React.ReactNode
  labelKey: string
}

export interface AvailableModel {
  providerId: string
  providerName: string
  apiType: string
  modelId: string
  modelName: string
  inputTypes: string[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
}

export interface ActiveModelRef {
  providerId: string
  modelId: string
}

export interface SkillSummary {
  name: string
  description: string
  source: string
  base_dir: string
  enabled: boolean
  requires_env: string[]
  skill_key?: string
  user_invocable?: boolean
  disable_model_invocation?: boolean
  has_install?: boolean
  any_bins?: string[]
  always?: boolean
}

export interface SkillInstallSpec {
  kind: string
  formula?: string
  package?: string
  go_module?: string
  bins?: string[]
  label?: string
  os?: string[]
}

export interface SkillStatusEntry {
  name: string
  source: string
  eligible: boolean
  disabled: boolean
  blocked_by_allowlist: boolean
  missing_bins?: string[]
  missing_any_bins?: string[]
  missing_env?: string[]
  missing_config?: string[]
  has_install: boolean
  always: boolean
}

export interface AgentSummary {
  id: string
  name: string
  description?: string | null
  emoji?: string | null
  avatar?: string | null
  hasAgentMd: boolean
  hasPersona: boolean
  hasToolsGuide: boolean
}

export interface PersonalityConfig {
  role?: string | null
  vibe?: string | null
  tone?: string | null
  traits: string[]
  principles: string[]
  boundaries?: string | null
  quirks?: string | null
  communicationStyle?: string | null
}

export interface AgentConfig {
  name: string
  description?: string | null
  emoji?: string | null
  avatar?: string | null
  model: { primary?: string | null; fallbacks: string[] }
  skills: { allow: string[]; deny: string[] }
  tools: { allow: string[]; deny: string[] }
  personality: PersonalityConfig
  behavior: {
    maxToolRounds: number
    requireApproval: string[]
    sandbox: boolean
    skillEnvCheck: boolean
  }
  useCustomPrompt: boolean
  notifyOnComplete?: boolean | null
  subagents: {
    enabled: boolean
    allowedAgents: string[]
    deniedAgents: string[]
    maxConcurrent: number
    defaultTimeoutSecs: number
    model?: string | null
  }
}

// ── Log Types ────────────────────────────────────────────────────

export interface LogEntry {
  id: number
  timestamp: string
  level: string
  category: string
  source: string
  message: string
  details?: string | null
  sessionId?: string | null
  agentId?: string | null
}

export interface LogFilter {
  levels: string[] | null
  categories: string[] | null
  keyword: string | null
  sessionId: string | null
  startTime: string | null
  endTime: string | null
}

export interface LogConfig {
  enabled: boolean
  level: string
  maxAgeDays: number
  maxSizeMb: number
  fileEnabled: boolean
  fileMaxSizeMb: number
}

export interface LogFileInfo {
  name: string
  sizeBytes: number
  modified: string
}

export interface LogStats {
  total: number
  byLevel: Record<string, number>
  byCategory: Record<string, number>
  dbSizeBytes: number
}

export interface LogQueryResult {
  logs: LogEntry[]
  total: number
}

export const DEFAULT_PERSONALITY: PersonalityConfig = {
  role: null,
  vibe: null,
  tone: null,
  traits: [],
  principles: [],
  boundaries: null,
  quirks: null,
  communicationStyle: null,
}

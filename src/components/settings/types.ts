export const SETTINGS_SECTION_IDS = [
  "general",
  "modelConfig",
  "tools",
  "skills",
  "agents",
  "teams",
  "memory",
  "notifications",
  "sandbox",
  "acp",
  "permissions",
  "profile",
  "chat",
  "plan",
  "recap",
  "logs",
  "health",
  "about",
  "channels",
  "developer",
  "server",
  "mcp",
  "security",
  "browser",
] as const

export type SettingsSection = (typeof SETTINGS_SECTION_IDS)[number]

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
  thinkingStyle?: "openai" | "anthropic" | "zai" | "qwen" | "none"
}

export interface ActiveModelRef {
  providerId: string
  modelId: string
}

/** Lifecycle status for a skill. `draft` entries are auto-created and hidden
 * from discovery until a user promotes them via the Skills panel. */
export type SkillStatus = "active" | "draft" | "archived"

/** Display-only metadata aggregated from frontmatter top-level + vendor
 * namespaces (`metadata.openclaw`, `metadata.hermes`). Mirrors
 * [`ha_core::skills::SkillDisplay`](../../crates/ha-core/src/skills/types.rs). */
export interface SkillDisplay {
  emoji?: string
  version?: string
  license?: string
  /** Short SPDX-ish label derived from `license` for badge rendering. */
  license_label?: string
  /** True when `license` is not a recognized OSS family. Backend-derived. */
  is_proprietary?: boolean
  author?: string
  tags?: string[]
  related_skills?: string[]
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
  status?: SkillStatus
  authored_by?: string
  display?: SkillDisplay
}

/** A discoverable third-party skill catalog (Claude Code, Anthropic
 * marketplace, OpenClaw, Hermes Agent). Returned by
 * `discover_preset_skill_sources`. */
export interface PresetSkillSource {
  id: string
  labelKey: string
  warningKey?: string
  candidates: PresetCandidate[]
}

export interface PresetCandidate {
  path: string
  exists: boolean
  skillCount: number
  alreadyAdded: boolean
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

export type PersonaMode = "structured" | "soulMd"

/// Active Memory pre-reply recall configuration (Phase B1).
export interface ActiveMemoryConfig {
  enabled: boolean
  timeoutMs: number
  maxChars: number
  cacheTtlSecs: number
  budgetTokens: number
  candidateLimit: number
}

/// Per-section character budgets for the SQLite Layer 3 memory block.
export interface SqliteSectionBudgets {
  userProfile: number
  aboutUser: number
  preferences: number
  projectContext: number
  references: number
}

/// Memory section budget (mirrors Rust MemoryBudgetConfig).
export interface MemoryBudgetConfig {
  totalChars: number
  coreMemoryFileChars: number
  sqliteEntryMaxChars: number
  sqliteSections: SqliteSectionBudgets
}

/// Agent-level memory configuration (mirrors Rust MemoryConfig).
export interface AgentMemoryConfig {
  enabled: boolean
  shared: boolean
  promptBudget: number
  autoExtract?: boolean | null
  extractProviderId?: string | null
  extractModelId?: string | null
  flushBeforeCompact?: boolean | null
  extractTokenThreshold?: number | null
  extractTimeThresholdSecs?: number | null
  extractMessageThreshold?: number | null
  extractIdleTimeoutSecs?: number | null
  activeMemory: ActiveMemoryConfig
  /// `null`/`undefined` means "inherit global AppConfig.memoryBudget";
  /// a full `MemoryBudgetConfig` replaces the default wholesale.
  budget?: MemoryBudgetConfig | null
}

export const DEFAULT_ACTIVE_MEMORY: ActiveMemoryConfig = {
  enabled: true,
  timeoutMs: 3000,
  maxChars: 220,
  cacheTtlSecs: 15,
  budgetTokens: 512,
  candidateLimit: 20,
}

export const DEFAULT_SQLITE_SECTION_BUDGETS: SqliteSectionBudgets = {
  userProfile: 1500,
  aboutUser: 2000,
  preferences: 2000,
  projectContext: 3000,
  references: 1500,
}

export const DEFAULT_MEMORY_BUDGET: MemoryBudgetConfig = {
  totalChars: 10_000,
  coreMemoryFileChars: 8_000,
  sqliteEntryMaxChars: 500,
  sqliteSections: DEFAULT_SQLITE_SECTION_BUDGETS,
}

export interface PersonalityConfig {
  mode?: PersonaMode
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
  model: { primary?: string | null; fallbacks: string[]; planModel?: string | null; temperature?: number | null }
  personality: PersonalityConfig
  capabilities: {
    maxToolRounds: number
    requireApproval: string[]
    sandbox: boolean
    skillEnvCheck: boolean
    tools: { allow: string[]; deny: string[] }
    skills: { allow: string[]; deny: string[] }
    /** MCP master switch (default true). When false all MCP tools are excluded. */
    mcpEnabled?: boolean
    /** Tier 3 capability toggles. `null/undefined` means "use tier default". */
    capabilityToggles?: {
      webSearch?: boolean | null
      imageGenerate?: boolean | null
      canvas?: boolean | null
      sendNotification?: boolean | null
      subagent?: boolean | null
      acpSpawn?: boolean | null
    }
  }
  openclawMode: boolean
  notifyOnComplete?: boolean | null
  memory?: AgentMemoryConfig
  subagents: {
    allowedAgents: string[]
    deniedAgents: string[]
    maxConcurrent: number
    defaultTimeoutSecs: number
    maxSpawnDepth?: number | null
    maxBatchSize?: number | null
    announceTimeoutSecs?: number | null
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
  mode: "structured",
  role: null,
  vibe: null,
  tone: null,
  traits: [],
  principles: [],
  boundaries: null,
  quirks: null,
  communicationStyle: null,
}

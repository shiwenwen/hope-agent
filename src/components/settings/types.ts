import type { SandboxMode } from "@/types/chat"

export const SETTINGS_SECTION_IDS = [
  "general",
  "modelConfig",
  "tools",
  "skills",
  "agents",
  "teams",
  "memory",
  "knowledge",
  "notifications",
  "sandbox",
  "acp",
  "approval",
  "hooks",
  "permissions",
  "profile",
  "chat",
  "cron",
  "plan",
  "recap",
  "logs",
  "health",
  "about",
  "updates",
  "channels",
  "developer",
  "server",
  "mcp",
  "security",
  "browser",
  "voice",
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
  hard_blocked?: boolean
  needs_setup?: boolean
  disabled: boolean
  blocked_by_allowlist: boolean
  current_os?: string | null
  supported_os?: string[]
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
  /// Active Memory v2: also shortlist structured claims as recall candidates.
  includeClaims: boolean
}

/// Procedure Memory soft workflow guidance configuration (P5).
export interface ProcedureMemoryConfig {
  enabled: boolean
  maxProcedures: number
  maxChars: number
  minConfidence: number
}

/// Entity relationship trace configuration (P4 Graph Memory).
export interface GraphMemoryConfig {
  enabled: boolean
  maxCenters: number
  maxEdges: number
}

/// Cross-source candidate fusion for Retrieval Planner diagnostics.
export interface RetrievalPlannerConfig {
  intentAware: boolean
  maxTraceRefs: number
  maxCandidatesPerOrigin: number
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
  procedureMemory: ProcedureMemoryConfig
  graphMemory: GraphMemoryConfig
  retrievalPlanner: RetrievalPlannerConfig
  /// `null`/`undefined` means "inherit global AppConfig.memoryBudget";
  /// a full `MemoryBudgetConfig` replaces the default wholesale.
  budget?: MemoryBudgetConfig | null
}

export const DEFAULT_ACTIVE_MEMORY: ActiveMemoryConfig = {
  enabled: false,
  timeoutMs: 8000,
  maxChars: 220,
  cacheTtlSecs: 15,
  budgetTokens: 512,
  candidateLimit: 10,
  includeClaims: false,
}

export const DEFAULT_PROCEDURE_MEMORY: ProcedureMemoryConfig = {
  enabled: true,
  maxProcedures: 1,
  maxChars: 800,
  minConfidence: 0.7,
}

export const DEFAULT_GRAPH_MEMORY: GraphMemoryConfig = {
  enabled: true,
  maxCenters: 3,
  maxEdges: 6,
}

export const DEFAULT_RETRIEVAL_PLANNER: RetrievalPlannerConfig = {
  intentAware: true,
  maxTraceRefs: 24,
  maxCandidatesPerOrigin: 4,
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

/**
 * Per-agent backgrounding policy for async-capable tools
 * (`exec` / `web_search` / `image_generate`). Independent of approval.
 *
 * - `model-decide` (default): honor `run_in_background:true` from the
 *   model; otherwise auto-background after `asyncTools.autoBackgroundSecs`.
 * - `always-background`: every async-capable tool call detaches
 *   immediately and returns a job id the model can poll with `job_status`.
 * - `never-background`: disable backgrounding entirely; tools run sync.
 */
export type AsyncToolPolicy = "model-decide" | "always-background" | "never-background"

export interface AgentConfig {
  name: string
  description?: string | null
  emoji?: string | null
  avatar?: string | null
  model: {
    primary?: string | null
    fallbacks: string[]
    planModel?: string | null
    temperature?: number | null
    reasoningEffort?: string | null
  }
  personality: PersonalityConfig
  capabilities: {
    maxToolRounds: number
    sandbox: boolean
    /**
     * Per-agent default sandbox mode for newly opened sessions.
     * `null/undefined` preserves legacy `sandbox` boolean semantics.
     */
    defaultSandboxMode?: SandboxMode | null
    skillEnvCheck: boolean
    tools: { allow: string[]; deny: string[] }
    skills: { allow: string[]; deny: string[] }
    /** MCP master switch (default true). When false all MCP tools are excluded. */
    mcpEnabled?: boolean
    /**
     * Permission system v2 — when true, the tools listed in
     * `customApprovalTools` also require approval in Default mode.
     */
    enableCustomToolApproval?: boolean
    /** Per-tool extra approval list. Only consumed in Default mode. */
    customApprovalTools?: string[]
    /**
     * Per-agent default permission mode for newly opened sessions.
     * `null/undefined` falls back to the global default ("default").
     */
    defaultSessionPermissionMode?: "default" | "smart" | "yolo" | null
    /** See [`AsyncToolPolicy`] for value semantics. */
    asyncToolPolicy?: AsyncToolPolicy
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

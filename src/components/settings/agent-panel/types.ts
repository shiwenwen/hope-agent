// Re-export types from the shared settings types
export type {
  AgentSummary,
  AgentConfig,
  PersonalityConfig,
  AvailableModel,
  ActiveModelRef,
  SkillSummary,
  ActiveMemoryConfig,
  AgentMemoryConfig,
  MemoryBudgetConfig,
  PersonaMode,
  SqliteSectionBudgets,
} from "../types"
export {
  DEFAULT_PERSONALITY,
  DEFAULT_ACTIVE_MEMORY,
  DEFAULT_MEMORY_BUDGET,
  DEFAULT_SQLITE_SECTION_BUDGETS,
} from "../types"

export type AgentTab = "identity" | "personality" | "capabilities" | "model" | "memory" | "subagent" | "custom"

export const TONE_PRESETS = [
  { value: "formal", labelKey: "settings.agentToneFormal" },
  { value: "casual", labelKey: "settings.agentToneCasual" },
  { value: "playful", labelKey: "settings.agentTonePlayful" },
  { value: "professional", labelKey: "settings.agentToneProfessional" },
  { value: "warm", labelKey: "settings.agentToneWarm" },
  { value: "direct", labelKey: "settings.agentToneDirect" },
]

export const TABS: { id: AgentTab; labelKey: string }[] = [
  { id: "identity", labelKey: "settings.agentIdentity" },
  { id: "personality", labelKey: "settings.agentPersonalityTab" },
  { id: "custom", labelKey: "settings.agentOpenClawMode" },
  { id: "capabilities", labelKey: "settings.agentCapabilities" },
  { id: "model", labelKey: "settings.agentModel" },
  { id: "memory", labelKey: "settings.memory" },
  { id: "subagent", labelKey: "settings.subagentTitle" },
]

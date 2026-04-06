// Re-export types from the shared settings types
export type {
  AgentSummary,
  AgentConfig,
  PersonalityConfig,
  AvailableModel,
  ActiveModelRef,
  SkillSummary,
} from "../types"
export { DEFAULT_PERSONALITY } from "../types"

export type AgentTab = "identity" | "personality" | "behavior" | "model" | "memory" | "subagent" | "custom"

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
  { id: "behavior", labelKey: "settings.agentBehavior" },
  { id: "model", labelKey: "settings.agentModel" },
  { id: "memory", labelKey: "settings.memory" },
  { id: "subagent", labelKey: "settings.subagentTitle" },
]

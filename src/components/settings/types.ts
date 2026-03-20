export type SettingsSection = "providers" | "models" | "skills" | "agents" | "profile" | "chat" | "appearance" | "language" | "about"

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
  behavior: { maxToolRounds: number; requireApproval: string[]; sandbox: boolean; skillEnvCheck: boolean }
  useCustomPrompt: boolean
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

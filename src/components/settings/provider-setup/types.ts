// ── Types ─────────────────────────────────────────────────────────

export type ApiType = "anthropic" | "openai-chat" | "openai-responses" | "codex"
export type ThinkingStyleType = "openai" | "anthropic" | "zai" | "qwen" | "none"

export interface ModelConfig {
  id: string
  name: string
  inputTypes: string[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
  costInput: number
  costOutput: number
}

export interface AuthProfile {
  id: string
  label: string
  apiKey: string
  baseUrl?: string
  enabled: boolean
}

export interface ProviderConfig {
  id: string
  name: string
  apiType: ApiType
  baseUrl: string
  apiKey: string
  authProfiles: AuthProfile[]
  models: ModelConfig[]
  enabled: boolean
  userAgent: string
  thinkingStyle: ThinkingStyleType
  allowPrivateNetwork?: boolean
}

export interface ProviderTemplate {
  key: string
  name: string
  description: string
  icon: string // emoji
  apiType: ApiType
  baseUrl: string
  apiKeyPlaceholder: string
  requiresApiKey: boolean
  models: ModelConfig[]
  thinkingStyle?: ThinkingStyleType
}

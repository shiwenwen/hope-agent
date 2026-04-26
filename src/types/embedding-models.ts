export type EmbeddingProviderType = "openai-compatible" | "google" | "local" | "auto"

export interface EmbeddingConfig {
  enabled: boolean
  providerType: EmbeddingProviderType | string
  apiBaseUrl?: string | null
  apiKey?: string | null
  apiModel?: string | null
  apiDimensions?: number | null
  localModelId?: string | null
}

export interface EmbeddingModelConfig {
  id: string
  name: string
  providerType: EmbeddingProviderType
  apiBaseUrl?: string | null
  apiKey?: string | null
  apiModel?: string | null
  apiDimensions?: number | null
  source?: string | null
}

export interface EmbeddingModelTemplate {
  name: string
  providerType: EmbeddingProviderType
  baseUrl: string
  defaultModel: string
  defaultDimensions: number
}

export type EmbeddingPreset = EmbeddingModelTemplate

export interface MemoryEmbeddingSelection {
  enabled: boolean
  modelConfigId?: string | null
  activeSignature?: string | null
  lastReembeddedSignature?: string | null
}

export interface MemoryEmbeddingState {
  selection: MemoryEmbeddingSelection
  currentModel?: EmbeddingModelConfig | null
  needsReembed: boolean
}

export interface MemoryEmbeddingSetDefaultResult {
  state: MemoryEmbeddingState
  reembedded: number
  reembedError?: string | null
}

export function embeddingProviderLabel(model: EmbeddingModelConfig): string {
  if (model.source === "ollama") return "Ollama"
  if (model.providerType === "google") return "Google"
  return model.apiBaseUrl?.replace(/^https?:\/\//, "") ?? model.providerType
}

export function openEmbeddingModelSettings() {
  window.dispatchEvent(
    new CustomEvent("settings:navigate", {
      detail: { section: "modelConfig", modelTab: "embeddingModels" },
    }),
  )
}

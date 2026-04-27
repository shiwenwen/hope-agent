export type OllamaPhase = "not-installed" | "installed" | "running"

export interface OllamaStatus {
  phase: OllamaPhase
  baseUrl: string
  installScriptSupported: boolean
}

export interface OllamaModelDetails {
  parentModel?: string | null
  format?: string | null
  family?: string | null
  families?: string[] | null
  parameterSize?: string | null
  quantizationLevel?: string | null
}

export interface LocalModelUsage {
  activeModel: boolean
  fallbackModel: boolean
  providerModel: boolean
  embeddingConfig: boolean
  embeddingModel: boolean
  running: boolean
  providerId?: string | null
  embeddingConfigId?: string | null
}

export interface LocalOllamaModel {
  id: string
  name: string
  sizeBytes?: number | null
  modifiedAt?: string | null
  digest?: string | null
  details?: OllamaModelDetails | null
  contextWindow?: number | null
  capabilities: string[]
  inputTypes: string[]
  running: boolean
  expiresAt?: string | null
  sizeVramBytes?: number | null
  usage: LocalModelUsage
}

export interface ModelCandidate {
  id: string
  displayName: string
  family: string
  sizeMb: number
  contextWindow: number
  reasoning: boolean
}

export type BudgetSource = "unified-memory" | "dedicated-vram" | "system-memory"

export interface GpuInfo {
  name: string
  vramMb?: number | null
}

export interface HardwareInfo {
  os: string
  totalMemoryMb: number
  availableMemoryMb: number
  gpu?: GpuInfo | null
  budgetSource: BudgetSource
  budgetMb: number
}

export interface ModelRecommendation {
  hardware?: HardwareInfo
  recommended?: ModelCandidate | null
  alternatives: ModelCandidate[]
  reason?: string
}

export interface OllamaLibraryModel {
  name: string
  href: string
  description: string
  capabilities: string[]
  sizes: string[]
  pullCount?: string | null
  tagCount?: number | null
  updated?: string | null
}

export interface OllamaLibraryTag {
  id: string
  sizeLabel?: string | null
  sizeBytes?: number | null
  contextLabel?: string | null
  contextWindow?: number | null
  inputTypes: string[]
  digest?: string | null
  updated?: string | null
  cloudOnly: boolean
}

export interface OllamaLibrarySearchResponse {
  query: string
  models: OllamaLibraryModel[]
  fromCache: boolean
  stale: boolean
}

export interface OllamaLibraryModelDetail {
  model: OllamaLibraryModel
  summary: string
  downloads?: string | null
  updated?: string | null
  tags: OllamaLibraryTag[]
  fromCache: boolean
  stale: boolean
}

export interface OllamaPullRequest {
  modelId: string
  displayName?: string | null
}

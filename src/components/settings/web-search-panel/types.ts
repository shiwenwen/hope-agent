// ── Types ────────────────────────────────────────────────────────

export interface ProviderEntry {
  id: string
  enabled: boolean
  apiKey: string | null
  apiKey2: string | null
  baseUrl: string | null
}

export interface WebSearchConfig {
  providers: ProviderEntry[]
  searxngDockerManaged: boolean | null
  searxngDockerUseProxy: boolean
  defaultResultCount: number
  timeoutSeconds: number
  cacheTtlMinutes: number
  defaultCountry: string | null
  defaultLanguage: string | null
  defaultFreshness: string | null
}

export interface SearxngDockerStatus {
  dockerInstalled: boolean
  dockerNotRunning: boolean
  hostOs?: string
  containerExists: boolean
  containerRunning: boolean
  port: number | null
  healthOk: boolean
  deploying: boolean
  deployStep: string | null
  deployLogs: string[]
  searchOk: boolean
  searchResultCount: number
  unresponsiveEngines: string[]
}

export interface ProviderMeta {
  id: string
  labelKey: string
  badges?: ProviderBadge[]
  needsApiKey: boolean
  url: string
  fields: FieldDef[]
}

export type ProviderBadgeTone = "positive" | "info" | "warning" | "danger"

export interface ProviderBadge {
  labelKey: string
  tone: ProviderBadgeTone
}

export interface FieldDef {
  configKey: "apiKey" | "apiKey2" | "baseUrl"
  labelKey: string
  placeholder: string
  secret?: boolean
}

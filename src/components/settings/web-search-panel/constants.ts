import type { ProviderMeta, ProviderEntry } from "./types"

export const PROVIDER_META: Record<string, ProviderMeta> = {
  "duck-duck-go": {
    id: "duck-duck-go",
    labelKey: "settings.webSearchProviderDDG",
    free: true,
    needsApiKey: false,
    url: "https://duckduckgo.com",
    fields: [],
  },
  searxng: {
    id: "searxng",
    labelKey: "settings.webSearchProviderSearXNG",
    free: true,
    needsApiKey: false,
    url: "https://docs.searxng.org",
    fields: [
      {
        configKey: "baseUrl",
        labelKey: "settings.webSearchInstanceUrl",
        placeholder: "http://localhost:8080",
      },
    ],
  },
  brave: {
    id: "brave",
    labelKey: "settings.webSearchProviderBrave",
    free: false,
    needsApiKey: true,
    url: "https://brave.com/search/api/",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "BSA...",
        secret: true,
      },
    ],
  },
  perplexity: {
    id: "perplexity",
    labelKey: "settings.webSearchProviderPerplexity",
    free: false,
    needsApiKey: true,
    url: "https://docs.perplexity.ai",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "pplx-...",
        secret: true,
      },
    ],
  },
  google: {
    id: "google",
    labelKey: "settings.webSearchProviderGoogle",
    free: false,
    needsApiKey: true,
    url: "https://developers.google.com/custom-search/v1/overview",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "AIza...",
        secret: true,
      },
      {
        configKey: "apiKey2",
        labelKey: "settings.webSearchGoogleCx",
        placeholder: "Search Engine ID",
      },
    ],
  },
  grok: {
    id: "grok",
    labelKey: "settings.webSearchProviderGrok",
    free: false,
    needsApiKey: true,
    url: "https://console.x.ai",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "xai-...",
        secret: true,
      },
    ],
  },
  kimi: {
    id: "kimi",
    labelKey: "settings.webSearchProviderKimi",
    free: false,
    needsApiKey: true,
    url: "https://platform.moonshot.cn",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "sk-...",
        secret: true,
      },
    ],
  },
  tavily: {
    id: "tavily",
    labelKey: "settings.webSearchProviderTavily",
    free: false,
    needsApiKey: true,
    url: "https://tavily.com",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "tvly-...",
        secret: true,
      },
    ],
  },
}

export function hasRequiredCredentials(entry: ProviderEntry): boolean {
  const meta = PROVIDER_META[entry.id]
  if (!meta) return false
  // DuckDuckGo: always ready
  if (entry.id === "duck-duck-go") return true
  // SearXNG: needs baseUrl (instance address)
  if (entry.id === "searxng") return !!entry.baseUrl?.trim()
  // Paid providers: need apiKey
  if (!entry.apiKey?.trim()) return false
  // Google also needs apiKey2 (CX)
  if (entry.id === "google" && !entry.apiKey2?.trim()) return false
  return true
}

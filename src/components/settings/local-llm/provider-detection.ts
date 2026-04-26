interface ProviderLike {
  enabled?: boolean
  apiType: string
  baseUrl: string
}

const LOCAL_OLLAMA_HOST_RE = /(127\.0\.0\.1|localhost|ollama\.local):11434/i

export function hasLocalOllamaProvider(providers: ProviderLike[]): boolean {
  return providers.some(
    (p) =>
      p.enabled !== false && p.apiType === "openai-chat" && LOCAL_OLLAMA_HOST_RE.test(p.baseUrl),
  )
}

interface ProviderLike {
  enabled?: boolean
  apiType: string
  baseUrl: string
}

export interface KnownLocalBackend {
  key: string
  name: string
  apiType: string
  baseUrl: string
  hosts: string[]
  port: number
}

function parseHostPort(baseUrl: string): { host: string; port: number } | null {
  const trimmed = baseUrl.trim()
  if (!trimmed) return null

  try {
    const url = new URL(trimmed)
    const host = url.hostname.replace(/^\[|\]$/g, "").toLowerCase()
    const port = url.port ? Number(url.port) : url.protocol === "https:" ? 443 : 80
    if (!Number.isFinite(port)) return null
    return { host, port }
  } catch {
    return null
  }
}

export function providerMatchesKnownLocalBackend(
  provider: ProviderLike,
  backend: KnownLocalBackend,
): boolean {
  if (provider.enabled === false || provider.apiType !== backend.apiType) return false
  const parsed = parseHostPort(provider.baseUrl)
  if (!parsed || parsed.port !== backend.port) return false
  return backend.hosts.some((host) => host.toLowerCase() === parsed.host)
}

export function hasKnownLocalBackend(
  providers: ProviderLike[],
  backends: KnownLocalBackend[],
  backendKey: string,
): boolean {
  const backend = backends.find((entry) => entry.key === backendKey)
  if (!backend) return false
  return providers.some((provider) => providerMatchesKnownLocalBackend(provider, backend))
}

import type { ProviderConfig, ProviderTemplate } from "./types"

function normalizeLoopbackHost(hostname: string): string {
  const normalized = hostname.toLowerCase()
  if (normalized === "localhost" || normalized === "::1") {
    return "127.0.0.1"
  }
  return normalized
}

function normalizeBaseUrl(baseUrl: string): string {
  const trimmed = baseUrl.trim()
  if (!trimmed) return ""

  try {
    const url = new URL(trimmed)
    const pathname = url.pathname.replace(/\/+$/, "")
    const port = url.port ? `:${url.port}` : ""
    return `${url.protocol.toLowerCase()}//${normalizeLoopbackHost(url.hostname)}${port}${pathname}`
  } catch {
    return trimmed.replace(/\/+$/, "").toLowerCase()
  }
}

export function hasConfiguredTemplate(
  template: Pick<ProviderTemplate, "apiType" | "baseUrl">,
  configuredProviders: Pick<ProviderConfig, "apiType" | "baseUrl">[],
): boolean {
  const normalizedTemplateUrl = normalizeBaseUrl(template.baseUrl)
  return configuredProviders.some(
    (provider) =>
      provider.apiType === template.apiType &&
      normalizeBaseUrl(provider.baseUrl) === normalizedTemplateUrl,
  )
}

export function getConfiguredTemplateKeys(
  templates: Pick<ProviderTemplate, "key" | "apiType" | "baseUrl">[],
  configuredProviders: Pick<ProviderConfig, "apiType" | "baseUrl">[],
): Set<string> {
  return new Set(
    templates
      .filter((template) => hasConfiguredTemplate(template, configuredProviders))
      .map((template) => template.key),
  )
}

export function hasConfiguredCodexProvider(
  configuredProviders: Pick<ProviderConfig, "apiType">[],
): boolean {
  return configuredProviders.some((provider) => provider.apiType === "codex")
}

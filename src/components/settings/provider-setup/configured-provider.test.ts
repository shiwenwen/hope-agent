import { test, expect } from "vitest"

import {
  getConfiguredTemplateKeys,
  hasConfiguredCodexProvider,
} from "./configured-provider.ts"
import type { ProviderConfig, ProviderTemplate } from "./types"

function makeTemplate(
  overrides: Partial<ProviderTemplate> & Pick<ProviderTemplate, "key" | "apiType" | "baseUrl">,
): ProviderTemplate {
  return {
    key: overrides.key,
    name: overrides.name ?? overrides.key,
    description: overrides.description ?? "",
    icon: overrides.icon ?? "",
    apiType: overrides.apiType,
    baseUrl: overrides.baseUrl,
    apiKeyPlaceholder: overrides.apiKeyPlaceholder ?? "",
    requiresApiKey: overrides.requiresApiKey ?? true,
    models: overrides.models ?? [],
    thinkingStyle: overrides.thinkingStyle,
  }
}

function makeProvider(
  overrides: Partial<ProviderConfig> & Pick<ProviderConfig, "apiType" | "baseUrl">,
): ProviderConfig {
  return {
    id: overrides.id ?? "provider-id",
    name: overrides.name ?? "Provider",
    apiType: overrides.apiType,
    baseUrl: overrides.baseUrl,
    apiKey: overrides.apiKey ?? "",
    authProfiles: overrides.authProfiles ?? [],
    models: overrides.models ?? [],
    enabled: overrides.enabled ?? true,
    userAgent: overrides.userAgent ?? "claude-code/0.1.0",
    thinkingStyle: overrides.thinkingStyle ?? "openai",
    allowPrivateNetwork: overrides.allowPrivateNetwork ?? false,
  }
}

test("matches templates by api type and normalized base URL", () => {
  const templates = [
    makeTemplate({
      key: "openai",
      apiType: "openai-responses",
      baseUrl: "https://api.openai.com",
    }),
    makeTemplate({
      key: "ollama",
      apiType: "openai-chat",
      baseUrl: "http://127.0.0.1:11434",
      requiresApiKey: false,
    }),
  ]

  const configured = [
    makeProvider({
      apiType: "openai-responses",
      baseUrl: "https://api.openai.com/",
    }),
    makeProvider({
      id: "ollama-id",
      apiType: "openai-chat",
      baseUrl: "http://localhost:11434/",
    }),
  ]

  expect(getConfiguredTemplateKeys(templates, configured)).toEqual(new Set(["openai", "ollama"]))
})

test("does not mark templates when only the base URL matches", () => {
  const templates = [
    makeTemplate({
      key: "openai",
      apiType: "openai-responses",
      baseUrl: "https://api.openai.com",
    }),
  ]

  const configured = [
    makeProvider({
      apiType: "openai-chat",
      baseUrl: "https://api.openai.com",
    }),
  ]

  expect(getConfiguredTemplateKeys(templates, configured)).toEqual(new Set())
})

test("detects an already configured codex provider", () => {
  const configured = [
    makeProvider({
      apiType: "codex",
      baseUrl: "",
    }),
  ]

  expect(hasConfiguredCodexProvider(configured)).toBe(true)
})

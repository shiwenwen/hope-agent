import { describe, expect, it } from "vitest"
import {
  applyModelCatalogMetadata,
  applyModelCatalogPricing,
  buildModelCatalog,
  getModelCatalogPricing,
  searchModelCatalog,
} from "./model-catalog"
import type { ModelConfig, ProviderTemplate } from "./types"

function model(id: string, overrides: Partial<ModelConfig> = {}): ModelConfig {
  return {
    id,
    name: id,
    inputTypes: ["text"],
    contextWindow: 128_000,
    maxTokens: 8_192,
    reasoning: false,
    costInput: null,
    costOutput: null,
    ...overrides,
  }
}

function template(
  key: string,
  name: string,
  models: ModelConfig[],
  currency: ProviderTemplate["currency"] = "USD",
): ProviderTemplate {
  return {
    key,
    name,
    description: name,
    icon: "",
    apiType: "openai-chat",
    baseUrl: `https://${key}.example.com`,
    apiKeyPlaceholder: "",
    requiresApiKey: true,
    models,
    currency,
  }
}

describe("model catalog", () => {
  it("keeps provider-specific capability variants and ignores placeholders", () => {
    const catalog = buildModelCatalog([
      template("direct", "Direct", [
        model("alpha", { name: "Alpha", contextWindow: 200_000, costInput: 1 }),
        model("your-model-id"),
      ]),
      template("relay", "Relay", [
        model("alpha", { name: "Relay Alpha", contextWindow: 64_000, costInput: 2 }),
      ]),
      template("mirror", "Mirror", [
        model("alpha", { name: "Mirror Alpha", contextWindow: 200_000, costInput: 3 }),
      ]),
    ])

    expect(catalog).toHaveLength(2)
    expect(catalog.find((entry) => entry.contextWindow === 200_000)).toMatchObject({
      id: "alpha",
      name: "Alpha",
      contextWindow: 200_000,
      sourceNames: ["Direct", "Mirror"],
    })
    expect(catalog.find((entry) => entry.contextWindow === 200_000)?.pricing).toHaveLength(2)
    expect(catalog.find((entry) => entry.contextWindow === 64_000)).toMatchObject({
      id: "alpha",
      name: "Relay Alpha",
      sourceNames: ["Relay"],
    })
  })

  it("searches IDs, display names, and Provider names with exact IDs first", () => {
    const catalog = buildModelCatalog([
      template("anthropic", "Anthropic", [
        model("claude-sonnet-5", { name: "Claude Sonnet 5" }),
        model("claude-sonnet-5-fast", { name: "Claude Sonnet 5 Fast" }),
      ]),
      template("gateway", "Special Gateway", [model("vendor/other-model")]),
    ])

    expect(searchModelCatalog(catalog, "claude sonnet").map((entry) => entry.id)).toEqual([
      "claude-sonnet-5",
      "claude-sonnet-5-fast",
    ])
    expect(searchModelCatalog(catalog, "claude-sonnet-5")[0].id).toBe("claude-sonnet-5")
    expect(searchModelCatalog(catalog, "claudesonnet5")[0].id).toBe("claude-sonnet-5")
    expect(searchModelCatalog(catalog, "special")[0].id).toBe("vendor/other-model")
  })

  it("fills capabilities while pricing remains an explicit, currency-safe choice", () => {
    const current = model("", {
      name: "",
      costInput: 9,
      costOutput: 18,
    })
    const [entry] = buildModelCatalog([
      template(
        "source",
        "Source",
        [
          model("alpha", {
            name: "Alpha",
            inputTypes: ["text", "image"],
            contextWindow: 1_000_000,
            maxTokens: 64_000,
            reasoning: true,
            costInput: 1,
            costOutput: 3,
          }),
        ],
        "USD",
      ),
    ])

    const withMetadata = applyModelCatalogMetadata(current, entry)
    expect(withMetadata).toMatchObject({
      id: "alpha",
      name: "Alpha",
      inputTypes: ["text", "image"],
      contextWindow: 1_000_000,
      maxTokens: 64_000,
      reasoning: true,
      costInput: 9,
      costOutput: 18,
    })

    expect(getModelCatalogPricing(entry, "CNY")).toBeNull()
    const pricing = getModelCatalogPricing(entry, "USD")
    expect(pricing).not.toBeNull()
    expect(applyModelCatalogPricing(withMetadata, pricing!)).toMatchObject({
      costInput: 1,
      costOutput: 3,
    })
  })
})

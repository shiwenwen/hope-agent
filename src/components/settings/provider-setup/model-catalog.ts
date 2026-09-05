import { PROVIDER_TEMPLATES } from "./templates"
import type { Currency, ModelConfig, ProviderTemplate } from "./types"

const PLACEHOLDER_MODEL_IDS = new Set(["your-model-id"])

export interface ModelCatalogPricing {
  currency: Currency
  costInput: number | null
  costOutput: number | null
  sourceName: string
}

export interface ModelCatalogEntry extends Pick<
  ModelConfig,
  "id" | "name" | "inputTypes" | "contextWindow" | "maxTokens" | "reasoning"
> {
  catalogKey: string
  sourceNames: string[]
  pricing: ModelCatalogPricing[]
}

function modelCatalogKey(model: ModelConfig, id: string) {
  return JSON.stringify([
    id,
    [...model.inputTypes].sort(),
    model.contextWindow,
    model.maxTokens,
    model.reasoning,
  ])
}

function catalogPricing(
  template: ProviderTemplate,
  model: ModelConfig,
): ModelCatalogPricing | null {
  if (model.costInput == null && model.costOutput == null) return null
  return {
    currency: template.currency ?? "USD",
    costInput: model.costInput,
    costOutput: model.costOutput,
    sourceName: template.name,
  }
}

/**
 * Build one searchable model library from every built-in Provider template.
 *
 * Model IDs are case-sensitive at the API boundary. Entries are merged only
 * when both the ID and effective capability limits match; gateways can expose
 * the same ID with a smaller context/output window or different modalities.
 */
export function buildModelCatalog(templates: ProviderTemplate[]): ModelCatalogEntry[] {
  const byVariant = new Map<string, ModelCatalogEntry>()

  for (const template of templates) {
    for (const model of template.models) {
      const id = model.id.trim()
      if (!id || PLACEHOLDER_MODEL_IDS.has(id)) continue

      const pricing = catalogPricing(template, model)
      const catalogKey = modelCatalogKey(model, id)
      const existing = byVariant.get(catalogKey)
      if (!existing) {
        byVariant.set(catalogKey, {
          catalogKey,
          id,
          name: model.name.trim() || id,
          inputTypes: [...model.inputTypes],
          contextWindow: model.contextWindow,
          maxTokens: model.maxTokens,
          reasoning: model.reasoning,
          sourceNames: [template.name],
          pricing: pricing ? [pricing] : [],
        })
        continue
      }

      if (!existing.sourceNames.includes(template.name)) {
        existing.sourceNames.push(template.name)
      }
      if (
        pricing &&
        !existing.pricing.some(
          (candidate) =>
            candidate.currency === pricing.currency &&
            candidate.costInput === pricing.costInput &&
            candidate.costOutput === pricing.costOutput,
        )
      ) {
        existing.pricing.push(pricing)
      }
    }
  }

  return [...byVariant.values()]
}

function normalized(value: string) {
  return value.trim().toLowerCase()
}

function compact(value: string) {
  return normalized(value).replace(/[\s_.:/-]+/g, "")
}

function matchRank(entry: ModelCatalogEntry, query: string) {
  const id = normalized(entry.id)
  const name = normalized(entry.name)
  const sources = normalized(entry.sourceNames.join(" "))
  const searchText = `${id} ${name} ${sources}`
  const terms = normalized(query).split(/\s+/).filter(Boolean)
  const needle = normalized(query)
  const compactNeedle = compact(query)
  const compactMatch =
    compactNeedle.length > 0 &&
    [entry.id, entry.name, ...entry.sourceNames].some((value) =>
      compact(value).includes(compactNeedle),
    )
  if (!terms.every((term) => searchText.includes(term)) && !compactMatch) {
    return Number.POSITIVE_INFINITY
  }

  if (id === needle) return 0
  if (name === needle) return 1
  if (id.startsWith(needle)) return 2
  if (name.startsWith(needle)) return 3
  if (compactNeedle && compact(entry.id).startsWith(compactNeedle)) return 4
  if (id.includes(needle)) return 5
  if (name.includes(needle)) return 6
  return 7
}

export function searchModelCatalog(
  catalog: ModelCatalogEntry[],
  query: string,
  limit = 8,
): ModelCatalogEntry[] {
  if (!query.trim() || limit <= 0) return []

  return catalog
    .map((entry) => ({ entry, rank: matchRank(entry, query) }))
    .filter(({ rank }) => Number.isFinite(rank))
    .sort(
      (a, b) =>
        a.rank - b.rank ||
        a.entry.id.length - b.entry.id.length ||
        a.entry.id.localeCompare(b.entry.id),
    )
    .slice(0, limit)
    .map(({ entry }) => entry)
}

export function applyModelCatalogMetadata(
  current: ModelConfig,
  entry: ModelCatalogEntry,
): ModelConfig {
  return {
    ...current,
    id: entry.id,
    name: entry.name,
    inputTypes: [...entry.inputTypes],
    contextWindow: entry.contextWindow,
    maxTokens: entry.maxTokens,
    reasoning: entry.reasoning,
  }
}

export function getModelCatalogPricing(
  entry: ModelCatalogEntry,
  currency: Currency,
): ModelCatalogPricing | null {
  return entry.pricing.find((pricing) => pricing.currency === currency) ?? null
}

export function applyModelCatalogPricing(
  current: ModelConfig,
  pricing: ModelCatalogPricing,
): ModelConfig {
  return {
    ...current,
    costInput: pricing.costInput,
    costOutput: pricing.costOutput,
  }
}

export const MODEL_CATALOG = buildModelCatalog(PROVIDER_TEMPLATES)

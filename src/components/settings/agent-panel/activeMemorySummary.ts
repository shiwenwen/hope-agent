import type { ActiveMemoryConfig } from "./types"
import {
  isRecommendedActiveMemory,
  RECOMMENDED_ACTIVE_MEMORY,
} from "./activeMemoryPreset"

export type ActiveMemorySummaryItem =
  | { id: "timeout"; value: string }
  | { id: "cache"; value: string }
  | { id: "candidates"; value: string }
  | { id: "maxChars"; value: string }
  | { id: "claims"; enabled: boolean }

export type ActiveMemoryReadinessItem = {
  id:
    | "recommended"
    | "agentMemoryOff"
    | "disabled"
    | "claimsOff"
    | "slowTimeout"
    | "tightBudget"
    | "lowCandidates"
    | "shortSnippets"
    | "custom"
  tone: "ok" | "notice" | "warning"
}

function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "0ms"
  if (ms < 1000) return `${Math.round(ms)}ms`
  const seconds = ms / 1000
  return `${Number.isInteger(seconds) ? seconds : seconds.toFixed(1)}s`
}

function formatDurationSecs(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0s"
  return `${Number.isInteger(seconds) ? seconds : seconds.toFixed(1)}s`
}

function formatCount(value: number): string {
  return Number.isFinite(value) ? String(Math.round(value)) : "0"
}

export function activeMemorySummaryItems(config: ActiveMemoryConfig): ActiveMemorySummaryItem[] {
  return [
    { id: "timeout", value: formatDurationMs(config.timeoutMs) },
    { id: "cache", value: formatDurationSecs(config.cacheTtlSecs) },
    { id: "candidates", value: formatCount(config.candidateLimit) },
    { id: "maxChars", value: formatCount(config.maxChars) },
    { id: "claims", enabled: config.includeClaims },
  ]
}

export function activeMemoryReadinessItems(
  config: ActiveMemoryConfig,
  options: { agentMemoryEnabled?: boolean } = {},
): ActiveMemoryReadinessItem[] {
  if (options.agentMemoryEnabled === false) return [{ id: "agentMemoryOff", tone: "warning" }]
  if (!config.enabled) return [{ id: "disabled", tone: "warning" }]
  if (isRecommendedActiveMemory(config)) return [{ id: "recommended", tone: "ok" }]

  const items: ActiveMemoryReadinessItem[] = []
  if (!config.includeClaims) items.push({ id: "claimsOff", tone: "notice" })
  if (config.timeoutMs > RECOMMENDED_ACTIVE_MEMORY.timeoutMs) {
    items.push({ id: "slowTimeout", tone: "notice" })
  }
  if (config.budgetTokens < RECOMMENDED_ACTIVE_MEMORY.budgetTokens) {
    items.push({ id: "tightBudget", tone: "notice" })
  }
  if (config.candidateLimit < RECOMMENDED_ACTIVE_MEMORY.candidateLimit) {
    items.push({ id: "lowCandidates", tone: "notice" })
  }
  if (config.maxChars < RECOMMENDED_ACTIVE_MEMORY.maxChars) {
    items.push({ id: "shortSnippets", tone: "notice" })
  }
  return items.length > 0 ? items : [{ id: "custom", tone: "ok" }]
}

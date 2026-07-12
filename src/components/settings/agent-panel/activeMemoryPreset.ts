import type { ActiveMemoryConfig, AgentConfig, AgentMemoryConfig } from "./types"
import {
  DEFAULT_ACTIVE_MEMORY,
  DEFAULT_GRAPH_MEMORY,
  DEFAULT_PROCEDURE_MEMORY,
  DEFAULT_RETRIEVAL_PLANNER,
} from "./types"

export const DEFAULT_AGENT_MEMORY: AgentMemoryConfig = {
  enabled: true,
  shared: true,
  promptBudget: 5000,
  activeMemory: DEFAULT_ACTIVE_MEMORY,
  procedureMemory: DEFAULT_PROCEDURE_MEMORY,
  graphMemory: DEFAULT_GRAPH_MEMORY,
  retrievalPlanner: DEFAULT_RETRIEVAL_PLANNER,
}

export const RECOMMENDED_ACTIVE_MEMORY: ActiveMemoryConfig = {
  ...DEFAULT_ACTIVE_MEMORY,
  enabled: true,
  timeoutMs: 4500,
  maxChars: 220,
  cacheTtlSecs: 60,
  candidateLimit: 12,
  includeClaims: true,
}

export function isRecommendedActiveMemory(config: ActiveMemoryConfig): boolean {
  return (
    config.enabled === RECOMMENDED_ACTIVE_MEMORY.enabled &&
    config.timeoutMs === RECOMMENDED_ACTIVE_MEMORY.timeoutMs &&
    config.maxChars === RECOMMENDED_ACTIVE_MEMORY.maxChars &&
    config.cacheTtlSecs === RECOMMENDED_ACTIVE_MEMORY.cacheTtlSecs &&
    config.budgetTokens === RECOMMENDED_ACTIVE_MEMORY.budgetTokens &&
    config.candidateLimit === RECOMMENDED_ACTIVE_MEMORY.candidateLimit &&
    config.includeClaims === RECOMMENDED_ACTIVE_MEMORY.includeClaims
  )
}

export function withRecommendedActiveMemory(config: AgentConfig): AgentConfig {
  const prevMemory = { ...DEFAULT_AGENT_MEMORY, ...(config.memory ?? {}) }
  return {
    ...config,
    memory: {
      ...prevMemory,
      enabled: true,
      activeMemory: { ...RECOMMENDED_ACTIVE_MEMORY },
    },
  }
}

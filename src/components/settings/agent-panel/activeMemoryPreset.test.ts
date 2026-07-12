import { describe, expect, it } from "vitest"

import {
  DEFAULT_AGENT_MEMORY,
  isRecommendedActiveMemory,
  RECOMMENDED_ACTIVE_MEMORY,
  withRecommendedActiveMemory,
} from "./activeMemoryPreset"
import { DEFAULT_ACTIVE_MEMORY, type AgentConfig } from "./types"

function makeAgentConfig(patch: Partial<AgentConfig> = {}): AgentConfig {
  return {
    name: "Main",
    model: {
      fallbacks: [],
    },
    personality: {
      traits: [],
      principles: [],
    },
    capabilities: {
      maxToolRounds: 10,
      sandbox: true,
      skillEnvCheck: true,
      tools: { allow: [], deny: [] },
      skills: { allow: [], deny: [] },
    },
    openclawMode: false,
    subagents: {
      allowedAgents: [],
      deniedAgents: [],
      maxConcurrent: 2,
      defaultTimeoutSecs: 600,
    },
    ...patch,
  }
}

describe("active memory recommended preset", () => {
  it("keeps the default conservative and opt-in", () => {
    expect(DEFAULT_ACTIVE_MEMORY.enabled).toBe(false)
    expect(DEFAULT_ACTIVE_MEMORY.includeClaims).toBe(false)
    expect(isRecommendedActiveMemory(DEFAULT_ACTIVE_MEMORY)).toBe(false)
  })

  it("uses the bounded recommended recall preset", () => {
    expect(RECOMMENDED_ACTIVE_MEMORY).toMatchObject({
      enabled: true,
      timeoutMs: 4500,
      maxChars: 220,
      cacheTtlSecs: 60,
      candidateLimit: 12,
      includeClaims: true,
    })
    expect(isRecommendedActiveMemory(RECOMMENDED_ACTIVE_MEMORY)).toBe(true)
  })

  it("treats partial drift as no longer recommended", () => {
    expect(
      isRecommendedActiveMemory({
        ...RECOMMENDED_ACTIVE_MEMORY,
        timeoutMs: RECOMMENDED_ACTIVE_MEMORY.timeoutMs + 100,
      }),
    ).toBe(false)
    expect(
      isRecommendedActiveMemory({
        ...RECOMMENDED_ACTIVE_MEMORY,
        budgetTokens: RECOMMENDED_ACTIVE_MEMORY.budgetTokens + 1,
      }),
    ).toBe(false)
  })

  it("applies the recommended preset without mutating the agent config", () => {
    const config = makeAgentConfig({
      description: "existing description",
      memory: {
        ...DEFAULT_AGENT_MEMORY,
        enabled: false,
        shared: false,
        promptBudget: 7000,
        activeMemory: {
          ...DEFAULT_ACTIVE_MEMORY,
          enabled: false,
          timeoutMs: 9000,
        },
      },
    })

    const updated = withRecommendedActiveMemory(config)

    expect(updated).not.toBe(config)
    expect(updated.memory).not.toBe(config.memory)
    expect(config.memory?.activeMemory.timeoutMs).toBe(9000)
    expect(updated.description).toBe("existing description")
    expect(updated.memory?.enabled).toBe(true)
    expect(updated.memory?.shared).toBe(false)
    expect(updated.memory?.promptBudget).toBe(7000)
    expect(updated.memory?.activeMemory).toEqual(RECOMMENDED_ACTIVE_MEMORY)
  })

  it("fills missing memory defaults when applying the preset", () => {
    const updated = withRecommendedActiveMemory(makeAgentConfig())

    expect(updated.memory).toMatchObject({
      enabled: true,
      shared: true,
      promptBudget: 5000,
      activeMemory: RECOMMENDED_ACTIVE_MEMORY,
    })
  })
})

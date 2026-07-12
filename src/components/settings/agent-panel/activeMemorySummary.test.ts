import { describe, expect, it } from "vitest"

import {
  activeMemoryReadinessItems,
  activeMemorySummaryItems,
} from "./activeMemorySummary"
import { RECOMMENDED_ACTIVE_MEMORY } from "./activeMemoryPreset"

describe("activeMemorySummaryItems", () => {
  it("summarizes the recommended preset for the overview card", () => {
    expect(activeMemorySummaryItems(RECOMMENDED_ACTIVE_MEMORY)).toEqual([
      { id: "timeout", value: "4.5s" },
      { id: "cache", value: "60s" },
      { id: "candidates", value: "12" },
      { id: "maxChars", value: "220" },
      { id: "claims", enabled: true },
    ])
  })

  it("handles sub-second and disabled-looking custom values", () => {
    expect(
      activeMemorySummaryItems({
        ...RECOMMENDED_ACTIVE_MEMORY,
        timeoutMs: 250,
        cacheTtlSecs: 0,
        candidateLimit: 3.4,
        includeClaims: false,
      }),
    ).toEqual([
      { id: "timeout", value: "250ms" },
      { id: "cache", value: "0s" },
      { id: "candidates", value: "3" },
      { id: "maxChars", value: "220" },
      { id: "claims", enabled: false },
    ])
  })
})

describe("activeMemoryReadinessItems", () => {
  it("explains when active recall is ready with the recommended preset", () => {
    expect(activeMemoryReadinessItems(RECOMMENDED_ACTIVE_MEMORY)).toEqual([
      { id: "recommended", tone: "ok" },
    ])
  })

  it("explains when active recall is disabled", () => {
    expect(activeMemoryReadinessItems({ ...RECOMMENDED_ACTIVE_MEMORY, enabled: false })).toEqual([
      { id: "disabled", tone: "warning" },
    ])
  })

  it("explains when the agent-level memory gate is off", () => {
    expect(
      activeMemoryReadinessItems(RECOMMENDED_ACTIVE_MEMORY, { agentMemoryEnabled: false }),
    ).toEqual([{ id: "agentMemoryOff", tone: "warning" }])
  })

  it("surfaces custom settings that may reduce recall quality or speed", () => {
    expect(
      activeMemoryReadinessItems({
        ...RECOMMENDED_ACTIVE_MEMORY,
        includeClaims: false,
        timeoutMs: RECOMMENDED_ACTIVE_MEMORY.timeoutMs + 1,
        budgetTokens: RECOMMENDED_ACTIVE_MEMORY.budgetTokens - 1,
        candidateLimit: RECOMMENDED_ACTIVE_MEMORY.candidateLimit - 1,
        maxChars: RECOMMENDED_ACTIVE_MEMORY.maxChars - 1,
      }),
    ).toEqual([
      { id: "claimsOff", tone: "notice" },
      { id: "slowTimeout", tone: "notice" },
      { id: "tightBudget", tone: "notice" },
      { id: "lowCandidates", tone: "notice" },
      { id: "shortSnippets", tone: "notice" },
    ])
  })

  it("keeps high-capacity custom recall as an ok custom strategy", () => {
    expect(
      activeMemoryReadinessItems({
        ...RECOMMENDED_ACTIVE_MEMORY,
        timeoutMs: RECOMMENDED_ACTIVE_MEMORY.timeoutMs - 500,
        candidateLimit: RECOMMENDED_ACTIVE_MEMORY.candidateLimit + 3,
      }),
    ).toEqual([{ id: "custom", tone: "ok" }])
  })
})

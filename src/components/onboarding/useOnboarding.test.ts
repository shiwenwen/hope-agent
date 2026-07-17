import { describe, expect, test } from "vitest"

import { ONBOARDING_STEPS, stepsForMode } from "./types"
import {
  hydrateOnboardingDraft,
  mergeOnboardingDraft,
  restoreOnboardingStep,
} from "./useOnboarding"

describe("mergeOnboardingDraft", () => {
  test("merges restored nested draft values without dropping seeded fields", () => {
    const merged = mergeOnboardingDraft(
      {
        language: "en",
        profile: {
          name: "Ada",
          timezone: "UTC",
          aiExperience: "intermediate",
        },
        server: {
          bindMode: "local",
          apiKeyEnabled: true,
          apiKey: "existing-key",
        },
      },
      {
        language: "zh",
        profile: {
          responseStyle: "concise",
        },
        server: {
          bindMode: "lan",
          apiKeyEnabled: true,
        },
      },
    )

    expect(merged.language).toBe("zh")
    expect(merged.profile).toEqual({
      name: "Ada",
      timezone: "UTC",
      aiExperience: "intermediate",
      responseStyle: "concise",
    })
    expect(merged.server).toEqual({
      bindMode: "lan",
      apiKeyEnabled: true,
      apiKey: "existing-key",
    })
  })

  test("uses canonical defaults for partial server and remote drafts", () => {
    const merged = mergeOnboardingDraft(
      {},
      {
        server: { bindMode: "lan", apiKeyEnabled: true },
        remote: { apiKey: "remote-secret", url: "" },
      },
    )

    expect(merged.server).toEqual({
      bindMode: "lan",
      apiKeyEnabled: true,
    })
    expect(merged.remote).toEqual({
      url: "",
      apiKey: "remote-secret",
    })
  })
})

describe("hydrateOnboardingDraft", () => {
  test("does not treat an unconnected remote draft as an effective remote config", () => {
    const hydrated = hydrateOnboardingDraft(
      { serverMode: "local" },
      {
        serverMode: "remote",
        remote: { url: "http://192.168.1.10:8420", apiKey: "draft-key" },
      },
    )

    expect(hydrated.serverMode).toBe("local")
    expect(hydrated.remote).toEqual({
      url: "http://192.168.1.10:8420",
      apiKey: "draft-key",
    })
  })

  test("preserves an established remote mode seeded from effective config", () => {
    const hydrated = hydrateOnboardingDraft(
      {
        serverMode: "remote",
        remote: { url: "https://agent.example.com", apiKey: "saved-key" },
      },
      { serverMode: "local" },
    )

    expect(hydrated.serverMode).toBe("remote")
  })
})

describe("onboarding step order", () => {
  test("adds search provider after model provider for local setup", () => {
    expect(ONBOARDING_STEPS).toEqual([
      "welcome",
      "provider",
      "search-provider",
      "profile",
      "safety",
      "channels",
    ])
    expect(stepsForMode("local")).toEqual(ONBOARDING_STEPS)
  })

  test("keeps remote setup short-circuited before local provider steps", () => {
    expect(stepsForMode("remote")).toEqual(["welcome"])
  })

  test("does not include third-party migration in first-run setup", () => {
    expect(ONBOARDING_STEPS).not.toContain("import-openclaw")
  })

  test("resumes removed v2 steps at the next visible step", () => {
    expect(restoreOnboardingStep(5, 2, ONBOARDING_STEPS)).toBe(4)
    expect(restoreOnboardingStep(7, 2, ONBOARDING_STEPS)).toBe(5)
    expect(restoreOnboardingStep(8, 2, ONBOARDING_STEPS)).toBe(5)
    expect(restoreOnboardingStep(10, 2, ONBOARDING_STEPS)).toBe(5)
  })

  test("resumes the removed v3 mode step at provider setup", () => {
    expect(restoreOnboardingStep(1, 3, ONBOARDING_STEPS)).toBe(1)
    expect(restoreOnboardingStep(6, 3, ONBOARDING_STEPS)).toBe(5)
  })
})

import { describe, expect, test } from "vitest"

import { mergeOnboardingDraft } from "./useOnboarding"

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

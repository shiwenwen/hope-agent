import { describe, expect, it } from "vitest"

import { hasRequiredCredentials, PROVIDER_META } from "./constants"
import type { ProviderEntry } from "./types"

describe("keyless search setup", () => {
  it("can be enabled without credentials or a deployed service", () => {
    const entry: ProviderEntry = {
      id: "keyless",
      enabled: false,
      apiKey: null,
      apiKey2: null,
      baseUrl: null,
    }
    expect(hasRequiredCredentials(entry)).toBe(true)
    expect(PROVIDER_META.keyless.fields).toEqual([])
    expect(PROVIDER_META.keyless.needsApiKey).toBe(false)
    expect(hasRequiredCredentials({ ...entry, id: "searxng" })).toBe(false)
    expect(hasRequiredCredentials({ ...entry, id: "brave" })).toBe(false)
  })
})

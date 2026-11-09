import { describe, expect, it } from "vitest"
import { figmaImportError } from "./figmaImportError"

describe("Figma import diagnostics", () => {
  it("handles desktop strings and HTTP errors without returning private text", () => {
    expect(figmaImportError("Figma API returned 401 (private)")).toEqual({ kind: "permission" })
    expect(
      figmaImportError(new Error('HTTP 500: {"error":"Figma API returned 403 (private)"}')),
    ).toEqual({ kind: "permission" })
    expect(figmaImportError(new Error("private file key and model failure"))).toEqual({
      kind: "other",
    })
    expect(figmaImportError({ private: "data" })).toEqual({ kind: "other" })
  })

  it("returns only bounded server cooldowns", () => {
    const error = (value: string) =>
      new Error(
        `HTTP 500: Figma API returned 429 (rate limited; retry_after_seconds=${value}; rate_limit_type=low)`,
      )
    expect(figmaImportError(error("60"))).toEqual({ kind: "rateLimited", retryAfterSeconds: 60 })
    expect(figmaImportError(error("2678400")).retryAfterSeconds).toBe(2678400)
    for (const value of ["0", "-1", "2678401", "999999999999", "60private", "1.5"]) {
      expect(figmaImportError(error(value))).toEqual({
        kind: "rateLimited",
        retryAfterSeconds: undefined,
      })
    }
    expect(figmaImportError("Figma API returned 429 (rate limited)")).toEqual({
      kind: "rateLimited",
      retryAfterSeconds: undefined,
    })
    expect(figmaImportError("Figma API returned 4290")).toEqual({ kind: "other" })
  })
})

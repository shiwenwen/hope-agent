import { describe, expect, it } from "vitest"

import { sanitizeDiagnosticText } from "./diagnosticRedaction"

describe("sanitizeDiagnosticText", () => {
  it("redacts common diagnostic credentials while preserving useful shape", () => {
    const diagnostic = sanitizeDiagnosticText(
      "request failed https://api.example.test?api-key=query-secret&safe=1 Authorization: Bearer bearer-secret apiKey: camel-secret accessToken=access-secret sk-live-secret AIzaSyA123456789012345678901234567890",
    )

    expect(diagnostic).toContain("api-key=[redacted]")
    expect(diagnostic).toContain("Authorization: Bearer [redacted]")
    expect(diagnostic).toContain("apiKey: [redacted]")
    expect(diagnostic).toContain("accessToken=[redacted]")
    expect(diagnostic).toContain("sk-[redacted]")
    expect(diagnostic).toContain("AIza[redacted]")
    expect(diagnostic).not.toContain("query-secret")
    expect(diagnostic).not.toContain("bearer-secret")
    expect(diagnostic).not.toContain("camel-secret")
    expect(diagnostic).not.toContain("access-secret")
    expect(diagnostic).not.toContain("sk-live-secret")
    expect(diagnostic).not.toContain("AIzaSyA123456789012345678901234567890")
  })

  it("normalizes whitespace and bounds copied diagnostics", () => {
    const diagnostic = sanitizeDiagnosticText(
      `first line\nsecond line password: db-secret ${"x".repeat(120)}`,
      80,
    )

    expect(diagnostic).toContain("first line second line password: [redacted]")
    expect(diagnostic).toContain("[truncated]")
    expect(diagnostic).not.toContain("db-secret")
    expect(diagnostic.length).toBeLessThanOrEqual(80)
  })
})

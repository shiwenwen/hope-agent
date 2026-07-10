import { describe, expect, it } from "vitest"

import { noteMentionErrorDetail } from "./noteMentionFeedback"

describe("note mention feedback", () => {
  it("redacts referenceable note load failures", () => {
    expect(noteMentionErrorDetail(new Error("sqlite busy"))).toBe("sqlite busy")
    expect(
      noteMentionErrorDetail(
        "load failed Authorization: Bearer bearer-secret token=query-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "load failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(noteMentionErrorDetail("   ")).toBeNull()
  })
})

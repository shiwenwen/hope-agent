import { describe, expect, it } from "vitest"

import { shouldConsumeWorkspaceFocus } from "./workspaceFocus"

describe("shouldConsumeWorkspaceFocus", () => {
  it("accepts a fresh request for the current session", () => {
    expect(shouldConsumeWorkspaceFocus({ sessionId: "session-a", nonce: 2 }, "session-a", 1)).toBe(
      true,
    )
  })

  it("rejects a stale request after switching sessions", () => {
    expect(shouldConsumeWorkspaceFocus({ sessionId: "session-a", nonce: 2 }, "session-b", 1)).toBe(
      false,
    )
  })

  it("rejects an already consumed nonce", () => {
    expect(shouldConsumeWorkspaceFocus({ sessionId: "session-a", nonce: 2 }, "session-a", 2)).toBe(
      false,
    )
  })
})

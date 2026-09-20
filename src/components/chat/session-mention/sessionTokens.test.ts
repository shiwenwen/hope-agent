import { describe, expect, it } from "vitest"

import { formatSessionInsertion, safeSessionMentionLabel } from "./sessionTokens"

describe("session mention tokens", () => {
  it("normalizes Markdown label delimiters including a trailing backslash", () => {
    expect(safeSessionMentionLabel("部署到 [C:\\]\\")).toBe("部署到  C:")
    expect(
      formatSessionInsertion({
        id: "session-2",
        title: "C:\\",
        agentId: "ha-main",
      }),
    ).toBe("[@C:](#session:session-2)")
  })
})

import { describe, expect, it } from "vitest"

import { parseMemoryScopeFocusTarget } from "./scopeFocus"

describe("parseMemoryScopeFocusTarget", () => {
  it("keeps valid agent tab hints", () => {
    expect(
      parseMemoryScopeFocusTarget({
        kind: "agent",
        id: " ha-main ",
        agentTab: "memory",
      }),
    ).toEqual({
      kind: "agent",
      id: "ha-main",
      agentTab: "memory",
    })
  })

  it("drops invalid tab hints without dropping the target", () => {
    expect(
      parseMemoryScopeFocusTarget({
        kind: "agent",
        id: "ha-main",
        agentTab: "missing",
      }),
    ).toEqual({
      kind: "agent",
      id: "ha-main",
    })
  })

  it("rejects empty or unknown targets", () => {
    expect(parseMemoryScopeFocusTarget({ kind: "agent", id: " " })).toBeNull()
    expect(parseMemoryScopeFocusTarget({ kind: "workspace", id: "p1" })).toBeNull()
    expect(parseMemoryScopeFocusTarget(null)).toBeNull()
  })
})

import { describe, expect, test } from "vitest"

import {
  isLoopCreateSlashCommand,
  loopSlashCommandDisplay,
  parseLoopCreateSlashCommand,
} from "./loopSlashCommand"

describe("loop slash command parsing", () => {
  test("treats control words as normal slash controls", () => {
    expect(parseLoopCreateSlashCommand("/loop status")).toBeNull()
    expect(isLoopCreateSlashCommand("/loop pause loop_123")).toBe(false)
    expect(loopSlashCommandDisplay("/loop status")).toEqual({
      content: "Show loops",
      mode: "loop",
    })
  })

  test("treats prompt forms as loop creation and hides the slash prefix", () => {
    expect(parseLoopCreateSlashCommand("/loop every 10m: check release blockers")).toBe(
      "every 10m: check release blockers",
    )
    expect(isLoopCreateSlashCommand("/loop Build release notes every 10m")).toBe(true)
    expect(loopSlashCommandDisplay("/loop Build release notes every 10m")).toEqual({
      content: "Build release notes every 10m",
      mode: "loop",
    })
  })
})

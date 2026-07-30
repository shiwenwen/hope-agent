// @vitest-environment jsdom

import { beforeEach, describe, expect, test } from "vitest"

import { clearMemoryFocusUrl } from "./memoryFocus"

describe("memory focus URL lifecycle", () => {
  beforeEach(() => {
    window.history.replaceState(null, "", "/workspace?mode=desktop")
  })

  test("clears a claims hash without changing the current page or query", () => {
    window.history.replaceState(
      { navigation: "settings" },
      "",
      "/workspace?mode=desktop#memory/claims?status=active",
    )

    expect(clearMemoryFocusUrl()).toBe(true)
    expect(window.location.pathname).toBe("/workspace")
    expect(window.location.search).toBe("?mode=desktop")
    expect(window.location.hash).toBe("")
    expect(window.history.state).toEqual({ navigation: "settings" })
  })

  test("leaves unrelated hashes untouched", () => {
    window.history.replaceState(null, "", "/workspace#dashboard/reports")

    expect(clearMemoryFocusUrl()).toBe(false)
    expect(window.location.hash).toBe("#dashboard/reports")
  })
})

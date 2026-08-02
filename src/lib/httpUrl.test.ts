import { describe, expect, test } from "vitest"

import { normalizeHttpBaseUrl } from "./httpUrl"

describe("normalizeHttpBaseUrl", () => {
  test("canonicalizes equivalent HTTP URL spellings", () => {
    expect(normalizeHttpBaseUrl(" https://AGENT.example:443/a/../%7e/ ")).toBe(
      "https://agent.example/~",
    )
  })

  test("normalizes reserved escape casing without decoding the delimiter", () => {
    expect(normalizeHttpBaseUrl("https://agent.example/prefix%2fchild")).toBe(
      "https://agent.example/prefix%2Fchild",
    )
  })
})

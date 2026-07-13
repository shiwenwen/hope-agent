import { describe, it, expect } from "vitest"

import { formatSizeDisplay } from "./inspectorFormat"

describe("formatSizeDisplay", () => {
  it("hides unset keywords so the friendly placeholder shows instead of raw CSS", () => {
    expect(formatSizeDisplay("none")).toBe("")
    expect(formatSizeDisplay("auto")).toBe("")
    expect(formatSizeDisplay("normal")).toBe("")
    expect(formatSizeDisplay("")).toBe("")
    expect(formatSizeDisplay("  auto ")).toBe("")
  })

  it("rounds sub-pixel / long-decimal lengths to at most 2 places", () => {
    expect(formatSizeDisplay("563.90625px")).toBe("563.91px")
    expect(formatSizeDisplay("52px")).toBe("52px")
    expect(formatSizeDisplay("0px")).toBe("0px")
    expect(formatSizeDisplay("33.33333%")).toBe("33.33%")
    expect(formatSizeDisplay("1.5em")).toBe("1.5em")
  })

  it("passes through values it does not recognize (keeps them editable)", () => {
    expect(formatSizeDisplay("calc(100% - 8px)")).toBe("calc(100% - 8px)")
    expect(formatSizeDisplay("fit-content")).toBe("fit-content")
    expect(formatSizeDisplay("100%")).toBe("100%")
  })
})

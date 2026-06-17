import { describe, expect, it } from "vitest"

import { parseGfmTable } from "./livePreviewExtensions"

describe("parseGfmTable", () => {
  it("parses header, delimiter alignment and rows", () => {
    const t = parseGfmTable("| 名称 | 值 |\n| :-- | --: |\n| a | 1 |\n| b | 2 |")
    expect(t).not.toBeNull()
    expect(t!.header).toEqual(["名称", "值"])
    expect(t!.aligns).toEqual(["left", "right"])
    expect(t!.rows).toEqual([
      ["a", "1"],
      ["b", "2"],
    ])
  })

  it("recognizes center alignment and default (null)", () => {
    const t = parseGfmTable("| a | b | c |\n| :-: | --- | :-- |\n| 1 | 2 | 3 |")
    expect(t!.aligns).toEqual(["center", null, "left"])
  })

  it("honors escaped pipes inside cells", () => {
    const t = parseGfmTable("| a | b |\n| - | - |\n| x \\| y | z |")
    expect(t!.rows).toEqual([["x | y", "z"]])
  })

  it("tolerates missing leading / trailing pipes", () => {
    const t = parseGfmTable("a | b\n--- | ---\n1 | 2")
    expect(t!.header).toEqual(["a", "b"])
    expect(t!.rows).toEqual([["1", "2"]])
  })

  it("keeps ragged rows as-is (caller pads to header width)", () => {
    const t = parseGfmTable("| a | b | c |\n| - | - | - |\n| 1 |")
    expect(t!.rows).toEqual([["1"]])
  })

  it("ignores blank lines between rows", () => {
    const t = parseGfmTable("| a | b |\n| - | - |\n\n| 1 | 2 |\n")
    expect(t!.rows).toEqual([["1", "2"]])
  })

  it("returns null when the delimiter row is missing or malformed", () => {
    expect(parseGfmTable("| a | b |\n| 1 | 2 |")).toBeNull()
    expect(parseGfmTable("| a | b |")).toBeNull()
    expect(parseGfmTable("just text")).toBeNull()
  })
})

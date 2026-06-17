import { describe, expect, it } from "vitest"

import { parseHeadings } from "./outline"

describe("parseHeadings", () => {
  it("extracts headings with level and 1-based line", () => {
    const md = "# A\n\ntext\n## B\n### C"
    expect(parseHeadings(md)).toEqual([
      { level: 1, text: "A", line: 1 },
      { level: 2, text: "B", line: 4 },
      { level: 3, text: "C", line: 5 },
    ])
  })

  it("strips a closing hash sequence", () => {
    expect(parseHeadings("## Title ##")).toEqual([{ level: 2, text: "Title", line: 1 }])
  })

  it("allows up to 3 leading spaces but not 4 (code indent)", () => {
    expect(parseHeadings("   # ok")).toEqual([{ level: 1, text: "ok", line: 1 }])
    expect(parseHeadings("    # indented code")).toEqual([])
  })

  it("requires a space after the hashes", () => {
    expect(parseHeadings("#notaheading")).toEqual([])
  })

  it("ignores headings inside fenced code blocks", () => {
    const md = "# Real\n```\n# fake\n## fake2\n```\n## Real2"
    expect(parseHeadings(md)).toEqual([
      { level: 1, text: "Real", line: 1 },
      { level: 2, text: "Real2", line: 6 },
    ])
  })

  it("handles tilde fences and an unclosed fence", () => {
    const md = "~~~\n# hidden\n~~~\n# shown\n```\n# also hidden"
    expect(parseHeadings(md)).toEqual([{ level: 1, text: "shown", line: 4 }])
  })

  it("does not close a fence on a line carrying an info string", () => {
    const md = "```\n# hidden\n``` ts\n# still hidden\n```\n# shown"
    expect(parseHeadings(md)).toEqual([{ level: 1, text: "shown", line: 6 }])
  })

  it("handles CRLF line endings (notes keep original endings)", () => {
    expect(parseHeadings("# A\r\n\r\ntext\r\n## B\r\n")).toEqual([
      { level: 1, text: "A", line: 1 },
      { level: 2, text: "B", line: 4 },
    ])
  })

  it("skips fenced code in CRLF notes", () => {
    expect(parseHeadings("# Real\r\n```\r\n# fake\r\n```\r\n## Real2")).toEqual([
      { level: 1, text: "Real", line: 1 },
      { level: 2, text: "Real2", line: 5 },
    ])
  })

  it("keeps an empty ATX heading as empty text", () => {
    expect(parseHeadings("#\n###   ")).toEqual([
      { level: 1, text: "", line: 1 },
      { level: 3, text: "", line: 2 },
    ])
  })
})

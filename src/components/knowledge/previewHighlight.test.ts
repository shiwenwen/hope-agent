import { describe, expect, it } from "vitest"

import { splitMarkdownForPreviewHighlight } from "./previewHighlight"

describe("splitMarkdownForPreviewHighlight", () => {
  it("highlights the contiguous paragraph block around a line", () => {
    const md = "# Title\n\nFirst line\nSecond line\n\nTail"

    expect(splitMarkdownForPreviewHighlight(md, 4)).toEqual({
      before: "# Title\n",
      highlighted: "First line\nSecond line",
      after: "\nTail",
      startLine: 3,
      endLine: 4,
    })
  })

  it("highlights a heading as a single source block", () => {
    const md = "# Parent\nBody\n\n## Child\nText"

    expect(splitMarkdownForPreviewHighlight(md, 4)?.highlighted).toBe("## Child")
  })

  it("keeps fenced code blocks together and ignores closing fence info strings", () => {
    const md = "Before\n\n```ts\nconst x = 1\n``` still code\nconst y = 2\n```\n\nAfter"

    const split = splitMarkdownForPreviewHighlight(md, 4)

    expect(split?.startLine).toBe(3)
    expect(split?.endLine).toBe(7)
    expect(split?.highlighted).toContain("const y = 2")
  })

  it("returns null for blank or invalid target lines", () => {
    const md = "A\n\nB"

    expect(splitMarkdownForPreviewHighlight(md, 2)).toBeNull()
    expect(splitMarkdownForPreviewHighlight(md, 0)).toBeNull()
  })
})

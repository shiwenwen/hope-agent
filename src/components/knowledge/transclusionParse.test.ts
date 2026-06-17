import { describe, expect, it } from "vitest"

import {
  cleanEmbedRef,
  embedAnchor,
  noteExcerpt,
  parseEmbedSegments,
  stripFrontmatter,
} from "./transclusionParse"

describe("parseEmbedSegments", () => {
  it("splits block embeds out of surrounding markdown", () => {
    const segs = parseEmbedSegments("# Title\n\n![[Other Note]]\n\nmore text")
    expect(segs).toEqual([
      { type: "md", text: "# Title\n" },
      { type: "embed", ref: "Other Note" },
      { type: "md", text: "\nmore text" },
    ])
  })

  it("treats consecutive embeds as separate segments", () => {
    const segs = parseEmbedSegments("![[a]]\n![[b]]")
    expect(segs).toEqual([
      { type: "embed", ref: "a" },
      { type: "embed", ref: "b" },
    ])
  })

  it("allows up to 3 leading spaces on a block embed", () => {
    const segs = parseEmbedSegments("   ![[a]]")
    expect(segs).toEqual([{ type: "embed", ref: "a" }])
  })

  it("does NOT treat a 4-space-indented embed as an embed (indented code)", () => {
    const segs = parseEmbedSegments("para\n\n    ![[a]]\n")
    expect(segs).toEqual([{ type: "md", text: "para\n\n    ![[a]]\n" }])
  })

  it("does NOT treat a tab-indented embed as an embed", () => {
    const segs = parseEmbedSegments("\t![[a]]")
    expect(segs).toEqual([{ type: "md", text: "\t![[a]]" }])
  })

  it("does not treat an embed inside a fenced code block as an embed", () => {
    const md = "```\n![[not an embed]]\n```\n![[real]]"
    const segs = parseEmbedSegments(md)
    expect(segs).toEqual([
      { type: "md", text: "```\n![[not an embed]]\n```" },
      { type: "embed", ref: "real" },
    ])
  })

  it("a 4-space-indented backtick run is NOT a fence (no phantom open)", () => {
    // The indented ``` is literal code, so the embed below still parses.
    const md = "    ```\n    code\n\n![[real]]"
    const segs = parseEmbedSegments(md)
    expect(segs).toEqual([
      { type: "md", text: "    ```\n    code\n" },
      { type: "embed", ref: "real" },
    ])
  })

  it("a shorter inner fence does not close a longer fence", () => {
    // 4-backtick block whose body has a 3-backtick line; embed stays literal.
    const md = "````\n```\n![[inside]]\n````\n![[after]]"
    const segs = parseEmbedSegments(md)
    expect(segs).toEqual([
      { type: "md", text: "````\n```\n![[inside]]\n````" },
      { type: "embed", ref: "after" },
    ])
  })

  it("leaves inline (mid-line) embeds as plain markdown", () => {
    const segs = parseEmbedSegments("see ![[x]] here")
    expect(segs).toEqual([{ type: "md", text: "see ![[x]] here" }])
  })

  it("keeps the raw inner text (anchor/alias) — cleaning is a separate step", () => {
    const segs = parseEmbedSegments("![[folder/note#Heading|Alias]]")
    expect(segs).toEqual([{ type: "embed", ref: "folder/note#Heading|Alias" }])
  })

  it("a tilde fence is not closed by a backtick fence", () => {
    const md = "~~~\n![[inside]]\n```\nstill inside\n~~~\n![[after]]"
    const segs = parseEmbedSegments(md)
    expect(segs).toEqual([
      { type: "md", text: "~~~\n![[inside]]\n```\nstill inside\n~~~" },
      { type: "embed", ref: "after" },
    ])
  })
})

describe("cleanEmbedRef", () => {
  it("drops the alias", () => {
    expect(cleanEmbedRef("Project Plan|See plan")).toBe("Project Plan")
  })
  it("drops the anchor", () => {
    expect(cleanEmbedRef("Project Plan#Risks")).toBe("Project Plan")
  })
  it("drops anchor then alias together", () => {
    expect(cleanEmbedRef("folder/note#Heading|Alias")).toBe("folder/note")
  })
  it("leaves a plain ref untouched", () => {
    expect(cleanEmbedRef("folder/note")).toBe("folder/note")
  })
})

describe("embedAnchor", () => {
  it("returns a heading anchor", () => {
    expect(embedAnchor("Project Plan#Risks")).toBe("Risks")
  })
  it("returns a block anchor with its caret", () => {
    expect(embedAnchor("Note#^p1")).toBe("^p1")
  })
  it("drops the alias before reading the anchor", () => {
    expect(embedAnchor("folder/note#Heading|Alias")).toBe("Heading")
  })
  it("is empty for an anchorless ref", () => {
    expect(embedAnchor("folder/note")).toBe("")
    expect(embedAnchor("Note|Alias")).toBe("")
  })
})

describe("stripFrontmatter", () => {
  it("removes a leading YAML frontmatter block", () => {
    expect(stripFrontmatter("---\ntitle: A\ntags: [x]\n---\n\n# Body")).toBe("# Body")
  })

  it("handles CRLF delimiters", () => {
    expect(stripFrontmatter("---\r\ntitle: A\r\n---\r\nbody")).toBe("body")
  })

  it("leaves content without frontmatter untouched", () => {
    expect(stripFrontmatter("# Body\n\ntext")).toBe("# Body\n\ntext")
  })

  it("does NOT treat an indented/padded --- as a delimiter (matches backend)", () => {
    const md = "  ---  \npara A\n---\npara B"
    expect(stripFrontmatter(md)).toBe(md)
  })

  it("leaves a lone --- (no closing delimiter) intact", () => {
    expect(stripFrontmatter("---\nnot really frontmatter")).toBe("---\nnot really frontmatter")
  })

  it("does not treat a horizontal rule mid-document as frontmatter", () => {
    expect(stripFrontmatter("# Title\n\n---\n\nbody")).toBe("# Title\n\n---\n\nbody")
  })
})

describe("noteExcerpt", () => {
  it("drops frontmatter and a leading heading, returns first paragraph", () => {
    const md = "---\ntitle: A\n---\n\n# Heading\n\nFirst para line one.\nLine two.\n\nSecond para."
    expect(noteExcerpt(md)).toBe("First para line one. Line two.")
  })

  it("collapses whitespace and skips leading blanks", () => {
    expect(noteExcerpt("\n\n   hello    world  \n")).toBe("hello world")
  })

  it("truncates by code points with an ellipsis", () => {
    expect(noteExcerpt("abcdef", 3)).toBe("abc…")
  })

  it("returns empty string for a heading-only note", () => {
    expect(noteExcerpt("# Only a title")).toBe("")
  })
})

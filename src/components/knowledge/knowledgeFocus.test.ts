import { describe, expect, it } from "vitest"

import { resolveKnowledgeFocusReveal } from "./knowledgeFocus"

describe("resolveKnowledgeFocusReveal", () => {
  it("jumps a trailing Obsidian block id to the start of its block", () => {
    const md = "# Project\n\n- Decision\n  rationale ^decision-1\n\nNext"

    expect(resolveKnowledgeFocusReveal(md, { blockId: "decision-1" })).toEqual({ line: 3 })
    expect(resolveKnowledgeFocusReveal(md, { blockId: "^decision-1" })).toEqual({ line: 3 })
  })

  it("attaches a standalone block id line to the previous block", () => {
    const md = "First paragraph\ncontinued\n^p1\n\nSecond paragraph"

    expect(resolveKnowledgeFocusReveal(md, { blockId: "p1" })).toEqual({ line: 1 })
  })

  it("does not let a standalone block id absorb the next paragraph", () => {
    const md = "First paragraph\n^p1\nSecond paragraph ^p2"

    expect(resolveKnowledgeFocusReveal(md, { blockId: "p2" })).toEqual({ line: 3 })
  })

  it("ignores block ids inside fenced code blocks", () => {
    const md = "```\nfake ^target\n```\n\nReal block ^target"

    expect(resolveKnowledgeFocusReveal(md, { blockId: "target" })).toEqual({ line: 5 })
  })

  it("falls back to line and col when the block id is stale", () => {
    expect(resolveKnowledgeFocusReveal("One\nTwo", { blockId: "missing", line: 2, col: 4 })).toEqual(
      { line: 2, col: 4 },
    )
  })

  it("uses headingPath when no precise line is available", () => {
    const md = "# Parent\n\n## Child\nBody\n\n## Other"

    expect(resolveKnowledgeFocusReveal(md, { headingPath: "Parent > Child" })).toEqual({ line: 3 })
  })
})

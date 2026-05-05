import { describe, test, expect } from "vitest"
import { recenterHighlightedSnippet, renderHighlightedSnippet } from "./highlight"
import type { ReactElement } from "react"

// Backend wraps matched runs with STX () / ETX () — never valid
// in user text — so the frontend can build React `<mark>` elements without
// touching innerHTML. These tests pin the parser's behaviour against
// pathological inputs the FTS5 snippet generator can produce in the wild.

function isReactElement(node: unknown): node is ReactElement {
  return typeof node === "object" && node !== null && "type" in (node as object)
}

function asText(node: unknown): string {
  if (typeof node === "string") return node
  if (isReactElement(node)) {
    const props = (node as ReactElement).props as { children?: unknown }
    return typeof props.children === "string" ? props.children : ""
  }
  return ""
}

test("returns empty array on empty input", () => {
  expect(renderHighlightedSnippet("")).toEqual([])
})

test("plain text passes through as a single string node", () => {
  const out = renderHighlightedSnippet("hello world")
  expect(out).toHaveLength(1)
  expect(out[0]).toBe("hello world")
})

test("wraps matched runs in <mark>", () => {
  const out = renderHighlightedSnippet("a hit b")
  expect(out).toHaveLength(3)
  expect(out[0]).toBe("a ")
  const mark = out[1]
  expect(isReactElement(mark)).toBe(true)
  expect((mark as ReactElement).type).toBe("mark")
  expect(asText(mark)).toBe("hit")
  expect(out[2]).toBe(" b")
})

test("handles multiple non-adjacent matches", () => {
  const out = renderHighlightedSnippet("foo mid bar")
  const marks = out.filter(isReactElement)
  expect(marks).toHaveLength(2)
  expect(marks.map(asText)).toEqual(["foo", "bar"])
})

test("handles adjacent matches without dropping characters", () => {
  // FTS5 can emit `<STX>a<ETX><STX>b<ETX>` when adjacent tokens both hit.
  const out = renderHighlightedSnippet("ab")
  const marks = out.filter(isReactElement)
  expect(marks).toHaveLength(2)
  expect(marks.map(asText)).toEqual(["a", "b"])
})

test("unmatched STX is treated as 'in-match until end of string'", () => {
  // Defensive: FTS5 should never emit an unbalanced STX, but the parser
  // must not crash or strip text. Trailing buffer flushes as a `<mark>`.
  const out = renderHighlightedSnippet("ok trailing")
  expect(out).toHaveLength(2)
  expect(out[0]).toBe("ok ")
  expect(asText(out[1])).toBe("trailing")
  expect(isReactElement(out[1])).toBe(true)
})

test("unmatched ETX without prior STX flushes as plain text", () => {
  // Defensive: a stray ETX must not promote the preceding text into a mark.
  const out = renderHighlightedSnippet("plain more")
  expect(out).toHaveLength(2)
  expect(out[0]).toBe("plain")
  expect(out[1]).toBe(" more")
})

test("preserves Unicode (CJK) inside matched run", () => {
  const out = renderHighlightedSnippet("前中文匹配后")
  expect(out).toHaveLength(3)
  expect(asText(out[1])).toBe("中文匹配")
})

test("each <mark> gets a stable React key so adjacent runs don't collide", () => {
  const out = renderHighlightedSnippet("a b")
  const marks = out.filter(isReactElement)
  const keys = marks.map((m) => (m as ReactElement & { key: string }).key)
  expect(new Set(keys).size).toBe(marks.length)
})


describe("recenterHighlightedSnippet", () => {
  test("returns the input untouched when there are no hits", () => {
    expect(recenterHighlightedSnippet("plain text")).toBe("plain text")
    expect(recenterHighlightedSnippet("")).toBe("")
  })

  test("returns the input untouched when the hit is already near the start", () => {
    const s = "ab cd ef\u0002hit\u0003more"
    expect(recenterHighlightedSnippet(s, 24)).toBe(s)
  })

  test("trims a long prefix and prepends an ellipsis when the hit is far", () => {
    const prefix = "x".repeat(200)
    const tail = "\u0002hit\u0003tail"
    const out = recenterHighlightedSnippet(prefix + tail, 24)
    expect(out.startsWith("\u2026")).toBe(true)
    // Prefix preserved should be exactly leadingContext characters before STX.
    const stxIdx = out.indexOf("\u0002")
    expect(stxIdx).toBe(1 + 24) // ellipsis char + 24 prefix chars
    // Hit content survives intact.
    expect(out.includes("hit")).toBe(true)
  })

  test("custom leadingContext shrinks the visible prefix", () => {
    const prefix = "y".repeat(80)
    const tail = "\u0002hit\u0003"
    const out = recenterHighlightedSnippet(prefix + tail, 6)
    expect(out).toBe("\u2026" + "y".repeat(6) + "\u0002hit\u0003")
  })

  test("works on the round-trip with renderHighlightedSnippet", () => {
    // Long prefix that pushes the hit past a 2-line clip — after recenter,
    // the rendered output starts with the ellipsis text node and contains
    // the matched <mark> within the first couple of nodes.
    const long =
      "前缀文字非常非常非常非常非常长以至于超过两行".repeat(4) + "\u0002hit\u0003后缀"
    const recentered = recenterHighlightedSnippet(long, 16)
    const out = renderHighlightedSnippet(recentered)
    // First node is the ellipsis-prefixed leading context.
    expect(typeof out[0]).toBe("string")
    expect((out[0] as string).startsWith("\u2026")).toBe(true)
    // The matched mark must appear in the rendered output.
    const marks = out.filter(isReactElement) as ReactElement[]
    expect(marks.length).toBe(1)
    expect(asText(marks[0])).toBe("hit")
  })
})

// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import {
  __HIGHLIGHT_NAME,
  applyInlineHighlight,
  clearInlineHighlight,
  parseHighlightTerms,
} from "./inlineHighlight"

// jsdom doesn't ship the CSS Custom Highlight API. Stub a minimal but
// faithful surface (`Highlight` constructor + `CSS.highlights` Map-like
// registry) so the production code under test takes the supported path.

interface FakeRegistry {
  store: Map<string, FakeHighlight>
  set(name: string, value: FakeHighlight): void
  delete(name: string): void
  has(name: string): boolean
  get(name: string): FakeHighlight | undefined
}

class FakeHighlight {
  ranges: Range[]
  constructor(...ranges: Range[]) {
    this.ranges = ranges
  }
}

function installHighlightShim(): FakeRegistry {
  const store = new Map<string, FakeHighlight>()
  const registry: FakeRegistry = {
    store,
    set: (n, v) => void store.set(n, v),
    delete: (n) => void store.delete(n),
    has: (n) => store.has(n),
    get: (n) => store.get(n),
  }
  Object.defineProperty(globalThis, "CSS", {
    value: { highlights: registry },
    configurable: true,
    writable: true,
  })
  Object.defineProperty(globalThis, "Highlight", {
    value: FakeHighlight,
    configurable: true,
    writable: true,
  })
  return registry
}

function uninstallHighlightShim() {
  delete (globalThis as { CSS?: unknown }).CSS
  delete (globalThis as { Highlight?: unknown }).Highlight
}

describe("parseHighlightTerms", () => {
  test("returns empty for empty / whitespace input", () => {
    expect(parseHighlightTerms("")).toEqual([])
    expect(parseHighlightTerms(null)).toEqual([])
    expect(parseHighlightTerms(undefined)).toEqual([])
    expect(parseHighlightTerms("   ")).toEqual([])
  })

  test("splits Latin queries on whitespace", () => {
    expect(parseHighlightTerms("hello world")).toEqual(["hello", "world"])
    expect(parseHighlightTerms("  foo   bar  ")).toEqual(["foo", "bar"])
  })

  test("drops noisy single-letter Latin tokens", () => {
    // "a" / "i" / "x" would mark every other character if highlighted.
    expect(parseHighlightTerms("a hello x")).toEqual(["hello"])
  })

  test("keeps single CJK characters", () => {
    expect(parseHighlightTerms("中")).toEqual(["中"])
    expect(parseHighlightTerms("中 文")).toEqual(["中", "文"])
  })

  test("treats CJK phrase without whitespace as one term", () => {
    expect(parseHighlightTerms("中文测试")).toEqual(["中文测试"])
  })
})

describe("applyInlineHighlight", () => {
  let registry: FakeRegistry

  beforeEach(() => {
    registry = installHighlightShim()
  })
  afterEach(() => {
    uninstallHighlightShim()
    document.body.innerHTML = ""
  })

  test("creates a highlight covering each case-insensitive occurrence", () => {
    const root = document.createElement("div")
    root.innerHTML = "<p>Hello World, hello again</p>"
    document.body.appendChild(root)

    applyInlineHighlight(root, ["hello"])

    const hl = registry.get(__HIGHLIGHT_NAME)
    expect(hl).toBeDefined()
    expect(hl?.ranges).toHaveLength(2)
    expect(hl?.ranges[0]?.toString().toLowerCase()).toBe("hello")
    expect(hl?.ranges[1]?.toString().toLowerCase()).toBe("hello")
  })

  test("does nothing when terms is empty", () => {
    const root = document.createElement("div")
    root.textContent = "no terms"
    document.body.appendChild(root)

    // Pre-seed a stale highlight to verify it gets cleared.
    registry.set(__HIGHLIGHT_NAME, new FakeHighlight())
    applyInlineHighlight(root, [])
    expect(registry.has(__HIGHLIGHT_NAME)).toBe(false)
  })

  test("creates no highlight when no occurrences found", () => {
    const root = document.createElement("div")
    root.textContent = "no match here"
    document.body.appendChild(root)

    applyInlineHighlight(root, ["nope"])
    expect(registry.has(__HIGHLIGHT_NAME)).toBe(false)
  })

  test("skips text inside script / style nodes", () => {
    const root = document.createElement("div")
    root.innerHTML =
      "<p>visible</p><script>visible_in_script</script><style>visible_in_style</style>"
    document.body.appendChild(root)

    applyInlineHighlight(root, ["visible"])

    const hl = registry.get(__HIGHLIGHT_NAME)
    expect(hl?.ranges).toHaveLength(1)
    expect(hl?.ranges[0]?.toString()).toBe("visible")
  })

  test("skips text inside aria-hidden subtrees (KaTeX / a11y twins)", () => {
    const root = document.createElement("div")
    root.innerHTML =
      '<p>visible</p><span aria-hidden="true">visible</span>'
    document.body.appendChild(root)

    applyInlineHighlight(root, ["visible"])
    const hl = registry.get(__HIGHLIGHT_NAME)
    expect(hl?.ranges).toHaveLength(1)
  })

  test("collects ranges across multiple terms", () => {
    const root = document.createElement("div")
    root.innerHTML = "<p>foo and bar</p>"
    document.body.appendChild(root)

    applyInlineHighlight(root, ["foo", "bar"])
    const hl = registry.get(__HIGHLIGHT_NAME)
    expect(hl?.ranges).toHaveLength(2)
    const matched = hl?.ranges.map((r) => r.toString()).sort()
    expect(matched).toEqual(["bar", "foo"])
  })

  test("highlights CJK substrings", () => {
    const root = document.createElement("div")
    root.innerHTML = "<p>这是一段中文测试中文消息</p>"
    document.body.appendChild(root)

    applyInlineHighlight(root, ["中文"])
    const hl = registry.get(__HIGHLIGHT_NAME)
    expect(hl?.ranges).toHaveLength(2)
    expect(hl?.ranges.every((r) => r.toString() === "中文")).toBe(true)
  })

  test("clearInlineHighlight drops the entry", () => {
    registry.set(__HIGHLIGHT_NAME, new FakeHighlight())
    clearInlineHighlight()
    expect(registry.has(__HIGHLIGHT_NAME)).toBe(false)
  })

  test("no-op when CSS Highlight API is unavailable", () => {
    uninstallHighlightShim()
    const root = document.createElement("div")
    root.textContent = "anything"
    document.body.appendChild(root)
    // Must not throw and must not blow up the page when running in older
    // engines that don't ship the API.
    expect(() => applyInlineHighlight(root, ["any"])).not.toThrow()
    expect(() => clearInlineHighlight()).not.toThrow()
  })

  test("replaces a prior highlight rather than appending", () => {
    const root = document.createElement("div")
    root.innerHTML = "<p>alpha beta</p>"
    document.body.appendChild(root)

    applyInlineHighlight(root, ["alpha"])
    const first = registry.get(__HIGHLIGHT_NAME)
    expect(first?.ranges).toHaveLength(1)

    applyInlineHighlight(root, ["beta"])
    const second = registry.get(__HIGHLIGHT_NAME)
    expect(second).not.toBe(first)
    expect(second?.ranges).toHaveLength(1)
    expect(second?.ranges[0]?.toString()).toBe("beta")
  })

  test("survives setStart throwing without dropping later ranges", () => {
    const root = document.createElement("div")
    root.innerHTML = "<p>foo bar foo</p>"
    document.body.appendChild(root)

    const realCreate = document.createRange.bind(document)
    let calls = 0
    const spy = vi.spyOn(document, "createRange").mockImplementation(() => {
      calls += 1
      const range = realCreate()
      if (calls === 1) {
        // Simulate a setStart that throws on the first match — the
        // production code's try/catch should swallow it and continue.
        const broken = Object.create(range) as Range
        Object.defineProperty(broken, "setStart", {
          value: () => {
            throw new Error("boom")
          },
        })
        Object.defineProperty(broken, "setEnd", {
          value: () => {},
        })
        return broken
      }
      return range
    })

    applyInlineHighlight(root, ["foo"])
    spy.mockRestore()
    const hl = registry.get(__HIGHLIGHT_NAME)
    // First occurrence dropped due to throw, second still landed.
    expect(hl?.ranges.length).toBe(1)
  })
})

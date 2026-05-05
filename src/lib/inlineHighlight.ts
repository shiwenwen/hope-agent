/**
 * Inline search-hit highlighting for already-rendered message bubbles.
 *
 * Uses the CSS Custom Highlight API (`CSS.highlights` / `Highlight` /
 * `::highlight()`) so we never mutate the rendered DOM — that's a hard
 * requirement here because message bodies are owned by Streamdown / Shiki /
 * KaTeX / Mermaid renderers that re-mount on every prop change. Wrapping
 * text nodes with `<mark>` would be wiped on the next render and would also
 * fight inside KaTeX SVG / Shiki span trees.
 *
 * Highlight ranges live as long as the parent text nodes do; once React
 * re-renders the bubble (e.g. streaming continues, agent name changes) the
 * old text nodes detach and the ranges become inert. `clearInlineHighlight`
 * removes the named entry from the registry so the highlight disappears
 * even if some references happen to outlive the bubble.
 *
 * Backend tokenizer is FTS5 `unicode61` — it splits on Unicode word
 * boundaries (CJK characters tokenize one-per-codepoint). That makes the
 * server FTS hit set a superset of "any term appearing as a substring",
 * so we treat the query as a list of literal substrings and highlight
 * every case-insensitive occurrence inside the target node. Matches the
 * `<mark>` snippet behaviour the sidebar renders for the same query.
 */

const HIGHLIGHT_NAME = "hope-search-hit"

interface HighlightRegistry {
  set(name: string, value: object): void
  delete(name: string): void
  has(name: string): boolean
}

interface HighlightAPI {
  highlights?: HighlightRegistry
}

interface HighlightCtor {
  new (...ranges: Range[]): object
}

function getHighlightApi(): {
  registry: HighlightRegistry
  Highlight: HighlightCtor
} | null {
  if (typeof globalThis === "undefined") return null
  const cssApi = (globalThis as unknown as { CSS?: HighlightAPI }).CSS
  const HighlightImpl = (globalThis as unknown as { Highlight?: HighlightCtor })
    .Highlight
  if (!cssApi?.highlights || !HighlightImpl) return null
  return { registry: cssApi.highlights, Highlight: HighlightImpl }
}

/** Above this codepoint, single-character tokens are meaningful (CJK,
 *  Arabic, Devanagari, …). Below covers Latin / IPA / Greek / Cyrillic
 *  where a 1-char hit would mark every other letter. */
const FIRST_NON_LATIN_CYRILLIC_CODEPOINT = 0x0500

/**
 * Split a raw search query into the literal substrings to highlight. We
 * keep CJK-only queries intact (no whitespace) and split Latin queries on
 * whitespace so multi-word inputs ("foo bar") light up both occurrences
 * independently — same convention as the snippet renderer.
 */
export function parseHighlightTerms(query: string | null | undefined): string[] {
  if (!query) return []
  const trimmed = query.trim()
  if (!trimmed) return []
  // Drop noisy 1-char Latin / IPA / Greek / Cyrillic tokens ("a", "i" would
  // mark every other letter). Keep 1-char tokens above this codepoint —
  // CJK, Arabic, Devanagari, … are meaningful on their own.
  return trimmed
    .split(/\s+/)
    .map((t) => t.trim())
    .filter((t) => t.length > 0)
    .filter((t) => t.length >= 2 || (t.codePointAt(0) ?? 0) >= FIRST_NON_LATIN_CYRILLIC_CODEPOINT)
}

/**
 * Wrap every case-insensitive occurrence of `terms` inside `root` in the
 * named CSS highlight. Replaces any prior highlight registered under the
 * same name. Pass an empty `terms` to act as a no-op (caller may want to
 * call `clearInlineHighlight` instead to remove the existing highlight).
 *
 * Skips text inside `<script>` / `<style>` / KaTeX accessibility nodes
 * (which contain duplicate "alt text" content the user never sees) so we
 * don't draw highlight rectangles in invisible regions.
 */
export function applyInlineHighlight(root: HTMLElement, terms: string[]): void {
  const api = getHighlightApi()
  if (!api) return
  if (terms.length === 0) {
    api.registry.delete(HIGHLIGHT_NAME)
    return
  }

  const lowerTerms = terms.map((t) => t.toLowerCase())
  const ranges: Range[] = []

  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (!node.nodeValue) return NodeFilter.FILTER_REJECT
      const parent = node.parentElement
      if (!parent) return NodeFilter.FILTER_REJECT
      // Skip non-content text nodes — these all carry duplicate or invisible
      // text that should never get highlight rectangles.
      const tag = parent.tagName
      if (tag === "SCRIPT" || tag === "STYLE" || tag === "NOSCRIPT") {
        return NodeFilter.FILTER_REJECT
      }
      // KaTeX renders an off-screen MathML twin for screen readers — the
      // visible glyphs are SVG/spans. Don't highlight the twin.
      if (parent.closest('.katex-mathml, [aria-hidden="true"]')) {
        return NodeFilter.FILTER_REJECT
      }
      return NodeFilter.FILTER_ACCEPT
    },
  })

  let node: Text | null
  while ((node = walker.nextNode() as Text | null)) {
    const text = node.nodeValue
    if (!text) continue
    const haystack = text.toLowerCase()
    for (const term of lowerTerms) {
      if (!term) continue
      let from = 0
      while (from <= haystack.length - term.length) {
        const idx = haystack.indexOf(term, from)
        if (idx === -1) break
        try {
          const range = document.createRange()
          range.setStart(node, idx)
          range.setEnd(node, idx + term.length)
          ranges.push(range)
        } catch {
          // setStart / setEnd can throw if the offset is past the live
          // text length (extremely unlikely race during streaming) — skip
          // and keep walking; one missed range is preferable to aborting.
        }
        from = idx + term.length
      }
    }
  }

  if (ranges.length === 0) {
    api.registry.delete(HIGHLIGHT_NAME)
    return
  }

  const highlight = new api.Highlight(...ranges)
  api.registry.set(HIGHLIGHT_NAME, highlight)
}

export function clearInlineHighlight(): void {
  const api = getHighlightApi()
  if (!api) return
  api.registry.delete(HIGHLIGHT_NAME)
}

/** Exposed for tests. */
export const __HIGHLIGHT_NAME = HIGHLIGHT_NAME

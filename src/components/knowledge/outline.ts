// Parse ATX headings out of a markdown note for the outline navigator (WS9).
// CommonMark-faithful enough for editor navigation: a heading is `#`×1–6 with ≤3
// leading spaces followed by a space (or end of line); fenced code blocks are
// skipped so `# comment` inside ``` isn't mistaken for a heading.

export interface OutlineHeading {
  /** 1–6 */
  level: number
  /** Heading text, trailing closing `#`s stripped. */
  text: string
  /** 1-based source line, for `revealTarget`. */
  line: number
}

const FENCE_RE = /^( {0,3})(`{3,}|~{3,})(.*)$/
const ATX_RE = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?[ \t]*$/

export function parseHeadings(md: string): OutlineHeading[] {
  const out: OutlineHeading[] = []
  // Split on CRLF or LF — notes keep their original line endings (no normalization),
  // and the line-end anchors below don't tolerate a trailing `\r`.
  const lines = md.split(/\r?\n/)
  let fenceMarker: string | null = null // first char of the open fence (` or ~)
  let fenceLen = 0
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const fence = FENCE_RE.exec(line)
    if (fence) {
      const marker = fence[2]
      const ch = marker[0]
      const len = marker.length
      if (fenceMarker === null) {
        // An opening fence may carry an info string; a closing fence may not.
        fenceMarker = ch
        fenceLen = len
        continue
      }
      // Inside a fence: close only on a same-char fence ≥ the opener with no
      // trailing info string.
      if (ch === fenceMarker && len >= fenceLen && fence[3].trim() === "") {
        fenceMarker = null
        fenceLen = 0
      }
      continue
    }
    if (fenceMarker !== null) continue
    const h = ATX_RE.exec(line)
    if (!h) continue
    const text = (h[2] ?? "").replace(/[ \t]+#+$/, "").trim()
    out.push({ level: h[1].length, text, line: i + 1 })
  }
  return out
}

/** One node of the collapsible read-only outline view (Phase 3 G, D8 optional
 *  layer). `body` is the markdown between this heading and the next heading (the
 *  section's own prose); `children` are deeper-level headings nested under it. */
export interface OutlineNode {
  heading: OutlineHeading
  body: string
  children: OutlineNode[]
}

/**
 * Build the heading tree for the outline view: a `preamble` (any text before the
 * first heading) plus a nested tree where each node owns the prose directly under
 * its heading. Purely derived from `parseHeadings` — never rewrites the `.md`
 * (D8 red line). A note with no headings yields the whole body as `preamble`.
 */
export function buildOutline(content: string): { preamble: string; nodes: OutlineNode[] } {
  const headings = parseHeadings(content)
  const lines = content.split(/\r?\n/)
  const firstLine = headings.length ? headings[0].line : lines.length + 1
  const preamble = lines
    .slice(0, firstLine - 1)
    .join("\n")
    .trim()

  const flat: OutlineNode[] = headings.map((h, idx) => {
    const bodyStart = h.line // 0-based index of the line after the heading
    const bodyEnd = idx + 1 < headings.length ? headings[idx + 1].line - 1 : lines.length
    return { heading: h, body: lines.slice(bodyStart, bodyEnd).join("\n").trim(), children: [] }
  })

  const roots: OutlineNode[] = []
  const stack: OutlineNode[] = []
  for (const node of flat) {
    while (stack.length && stack[stack.length - 1].heading.level >= node.heading.level) stack.pop()
    if (stack.length) stack[stack.length - 1].children.push(node)
    else roots.push(node)
    stack.push(node)
  }
  return { preamble, nodes: roots }
}

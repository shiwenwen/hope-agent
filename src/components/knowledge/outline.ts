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

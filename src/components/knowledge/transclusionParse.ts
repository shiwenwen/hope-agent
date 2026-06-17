// Pure helpers for `![[ ]]` transclusion (WS2). Kept free of React / renderer
// imports so they're cheap to unit-test.

export type EmbedSegment = { type: "md"; text: string } | { type: "embed"; ref: string }

// A `![[ref]]` alone on a line (block embed) with at most 3 leading spaces — a
// line indented ≥4 spaces (or a tab) is an indented code block per CommonMark
// and the backend parser, so it must NOT be treated as an embed. Inline embeds
// (mid-paragraph) are also left as text.
const EMBED_LINE_RE = /^ {0,3}!\[\[([^\]\n]+)\]\]\s*$/
// A fenced-code delimiter: ≤3 leading spaces, then ≥3 backticks or tildes.
// Captures the run (for the same-char + length≥open close rule) and the rest.
const FENCE_RE = /^ {0,3}(`{3,}|~{3,})(.*)$/

/**
 * Split note markdown into plain-markdown runs and block-level `![[ref]]`
 * embeds, never treating an embed line inside a fenced code block (CommonMark
 * rules: ≤3-space indent to open, same char + length ≥ opening to close) as an
 * embed.
 */
export function parseEmbedSegments(content: string): EmbedSegment[] {
  const lines = content.split("\n")
  const segments: EmbedSegment[] = []
  let buf: string[] = []
  let inFence = false
  let fenceChar = ""
  let fenceLen = 0

  const flush = () => {
    if (buf.length) {
      segments.push({ type: "md", text: buf.join("\n") })
      buf = []
    }
  }

  for (const line of lines) {
    const fence = line.match(FENCE_RE)
    if (fence) {
      const marker = fence[1]
      const ch = marker[0]
      if (!inFence) {
        inFence = true
        fenceChar = ch
        fenceLen = marker.length
      } else if (ch === fenceChar && marker.length >= fenceLen && fence[2].trim() === "") {
        // Valid closing fence: same char, ≥ opening length, no trailing content.
        inFence = false
        fenceChar = ""
        fenceLen = 0
      }
      buf.push(line)
      continue
    }
    if (!inFence) {
      const m = line.match(EMBED_LINE_RE)
      if (m) {
        flush()
        segments.push({ type: "embed", ref: m[1].trim() })
        continue
      }
    }
    buf.push(line)
  }
  flush()
  return segments
}

/**
 * Clean a `[[ ]]` embed reference to its resolution target: drop the `|alias`
 * then the `#anchor` (mirrors `parser.rs` / `inject.rs`, the canonical pipeline,
 * so transclusion resolves the same notes the link graph does). Phase 2 embeds
 * the whole note even when an anchor is given.
 */
export function cleanEmbedRef(ref: string): string {
  return ref.split("|")[0].split("#")[0].trim()
}

/**
 * The `#anchor` of an embed ref (a `#Heading` or `#^block`), alias dropped first;
 * "" when there is none. Used to scope the transclusion cycle guard so an
 * anchored self-embed (`![[A#^p1]]` inside `A.md`) — a slice of a *different*
 * block — isn't mistaken for whole-note recursion.
 */
export function embedAnchor(ref: string): string {
  const beforeAlias = ref.split("|")[0]
  const hash = beforeAlias.indexOf("#")
  return hash >= 0 ? beforeAlias.slice(hash + 1).trim() : ""
}

/** Drop a leading YAML frontmatter block so embeds show body, not metadata.
 *  Matches the backend (`parser.rs`): the delimiter is exactly `---` on its own
 *  line (no leading/trailing spaces; a trailing CR for CRLF files is allowed). */
export function stripFrontmatter(md: string): string {
  const lines = md.split("\n")
  const isDelim = (l: string | undefined) => l === "---" || l === "---\r"
  if (!isDelim(lines[0])) return md
  for (let i = 1; i < lines.length; i++) {
    if (isDelim(lines[i])) {
      return lines
        .slice(i + 1)
        .join("\n")
        .replace(/^\n+/, "")
    }
  }
  return md // no closing delimiter — not real frontmatter, leave intact
}

/**
 * A short plain-text excerpt for the wikilink hover card (WS9): drop frontmatter,
 * skip leading blank lines / ATX headings, then take the first non-empty block of
 * body text collapsed to a single line, capped at `max` code points (… elided).
 */
export function noteExcerpt(content: string, max = 240): string {
  const body = stripFrontmatter(content)
  const lines = body.split("\n")
  const para: string[] = []
  for (const raw of lines) {
    const line = raw.trim()
    if (para.length === 0) {
      // Skip leading blanks and a leading heading so the excerpt is real prose.
      if (!line || /^#{1,6}\s/.test(line)) continue
      para.push(line)
    } else {
      if (!line) break // first paragraph ended
      para.push(line)
    }
  }
  const text = para.join(" ").replace(/\s+/g, " ").trim()
  const chars = Array.from(text)
  return chars.length > max ? `${chars.slice(0, max).join("")}…` : text
}

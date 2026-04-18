import type { ReactNode } from "react"

/**
 * Render a FTS5 snippet that marks hits with STX (U+0002) / ETX (U+0003)
 * control characters. Returns React elements — never an HTML string — so
 * user-authored content in matched messages cannot escape into the DOM.
 *
 * Backend contract: see [`crates/oc-core/src/session/db.rs`] — the SQL
 * `snippet()` call uses `char(2)`/`char(3)` as start/end delimiters.
 */
export function renderHighlightedSnippet(raw: string): ReactNode[] {
  if (!raw) {
    return []
  }
  const nodes: ReactNode[] = []
  let buffer = ""
  let inMatch = false
  let key = 0
  const flush = () => {
    if (!buffer) return
    if (inMatch) {
      nodes.push(
        <mark
          key={key++}
          className="bg-primary/30 text-foreground rounded px-0.5"
        >
          {buffer}
        </mark>,
      )
    } else {
      nodes.push(buffer)
    }
    buffer = ""
  }
  for (const ch of raw) {
    if (ch === "\u0002") {
      flush()
      inMatch = true
    } else if (ch === "\u0003") {
      flush()
      inMatch = false
    } else {
      buffer += ch
    }
  }
  flush()
  return nodes
}

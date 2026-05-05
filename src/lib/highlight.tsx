import type { ReactNode } from "react"

/**
 * Re-center an FTS5 snippet so the first hit lands inside `leadingContext`
 * characters from the start. Backend `snippet(messages_fts, 0, …, 16)`
 * returns up to 16 tokens around the best match, but on short messages it
 * just returns the entire content — and the line-clamp on the result row
 * (2 lines in the sidebar, 1 line in the in-chat search bar) can hide the
 * actual hit if it sits past the clip boundary. Trimming the prefix is
 * safe because all `\u0002` / `\u0003` markers live at or after `firstHit`,
 * so we never split a mark pair.
 *
 * Returns the original string when there are no hits or the first hit is
 * already near the start.
 */
export function recenterHighlightedSnippet(
  raw: string,
  leadingContext: number = 24,
): string {
  if (!raw) return raw
  const firstHit = raw.indexOf("\u0002")
  if (firstHit < 0 || firstHit <= leadingContext) return raw
  return "…" + raw.slice(firstHit - leadingContext)
}

/**
 * Render a FTS5 snippet that marks hits with STX (U+0002) / ETX (U+0003)
 * control characters. Returns React elements — never an HTML string — so
 * user-authored content in matched messages cannot escape into the DOM.
 *
 * Backend contract: see [`crates/ha-core/src/session/db.rs`] — the SQL
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
      // Yellow-on-dim — high contrast against both light surfaces (search
      // result rows in the sidebar) and the muted snippet text below the
      // session title. `px-1 py-0.5 mx-0.5` give the marker visible breathing
      // room so adjacent hits don't run into each other and the text isn't
      // crushed against the highlight edge. `font-medium` nudges the matched
      // run forward without shifting the baseline.
      nodes.push(
        <mark
          key={key++}
          className="bg-yellow-200/90 text-yellow-950 dark:bg-yellow-400/30 dark:text-yellow-100 rounded px-1 py-0.5 mx-0.5 font-medium"
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

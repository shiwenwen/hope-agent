// Pure helpers for the chat composer's `[[note]]` picker. Mirrors the backend
// grammar (crates/ha-core/src/knowledge/inject.rs scans `\[\[([^\]\n]+)\]\]`)
// so an inserted `[[name]]` round-trips through resolve_inline_injections.

export interface ActiveNoteRef {
  /** Index of the first `[` of the `[[` opener. */
  anchor: number
  /** Current caret position. */
  caret: number
  /** Text typed between `[[` and the caret — the live query. */
  token: string
}

/**
 * Detect an unclosed `[[` governing the caret. Scans backwards from the caret;
 * bails on `]` or newline (token boundary) or a lone `[` (no opener). Returns
 * the opener anchor + the partial token, or null when no `[[` context is active.
 */
export function detectActiveNoteRef(input: string, caret: number): ActiveNoteRef | null {
  if (caret < 2 || caret > input.length) return null
  for (let i = caret - 1; i >= 1; i--) {
    const c = input[i]
    if (c === "]" || c === "\n") return null
    if (c === "[") {
      if (input[i - 1] === "[") {
        return { anchor: i - 1, caret, token: input.slice(i + 1, caret) }
      }
      return null // lone `[` — not a `[[` opener
    }
  }
  return null
}

/** The literal text spliced for a chosen note: `[[inner]]`. */
export function formatNoteInsertion(inner: string): string {
  return `[[${inner}]]`
}

/** Path-form token: rel path minus the markdown extension. */
export function relPathToken(relPath: string): string {
  return relPath.replace(/\.(md|markdown)$/i, "")
}

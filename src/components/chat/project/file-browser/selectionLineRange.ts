import type { CodeSelection } from "./ShikiCodeView"

function lineNumberAt(source: string, offset: number) {
  let line = 1
  for (let i = 0; i < offset; i++) {
    if (source.charCodeAt(i) === 10) line++
  }
  return line
}

/**
 * Rendered Markdown no longer has a one-to-one DOM/source mapping. Preserve the
 * exact selected text, and expose source line numbers only when that literal
 * selection has one unambiguous source match. Office previews have no source
 * text here, so they deliberately use `0 / 0` to represent no source line
 * range rather than inventing a misleading L1-n reference.
 */
export function selectionWithLineRange(text: string, source?: string): CodeSelection {
  if (!source) {
    return {
      text,
      startLine: 0,
      endLine: 0,
    }
  }

  if (text.length > 0) {
    const offset = source.indexOf(text)
    const duplicateOffset = offset >= 0 ? source.indexOf(text, offset + 1) : -1
    if (offset >= 0 && duplicateOffset < 0) {
      return {
        text,
        startLine: lineNumberAt(source, offset),
        endLine: lineNumberAt(source, offset + text.length),
      }
    }
  }

  return {
    text,
    startLine: 0,
    endLine: 0,
  }
}

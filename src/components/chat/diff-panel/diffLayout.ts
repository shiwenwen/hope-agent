import { diffLines, type Change } from "diff"

export interface UnifiedRow {
  type: "added" | "removed" | "context"
  text: string
  oldLineNumber?: number
  newLineNumber?: number
}

export interface SplitRow {
  /** Left column (old). undefined = blank line in split view. */
  left?: { type: "removed" | "context"; text: string; lineNumber: number }
  /** Right column (new). undefined = blank line in split view. */
  right?: { type: "added" | "context"; text: string; lineNumber: number }
}

/**
 * Compute the unified-view list of rows from a before/after pair. Splits the
 * diff blocks back into individual lines so each can be tagged as added /
 * removed / context with the correct line number on each side.
 */
export function buildUnifiedRows(before: string, after: string): UnifiedRow[] {
  const before_ = before ?? ""
  const after_ = after ?? ""
  const blocks: Change[] = diffLines(before_, after_)
  const rows: UnifiedRow[] = []
  let oldLine = 1
  let newLine = 1

  for (const block of blocks) {
    const lines = trimTrailingNewline(block.value).split("\n")
    if (block.added) {
      for (const line of lines) {
        rows.push({ type: "added", text: line, newLineNumber: newLine })
        newLine += 1
      }
    } else if (block.removed) {
      for (const line of lines) {
        rows.push({ type: "removed", text: line, oldLineNumber: oldLine })
        oldLine += 1
      }
    } else {
      for (const line of lines) {
        rows.push({
          type: "context",
          text: line,
          oldLineNumber: oldLine,
          newLineNumber: newLine,
        })
        oldLine += 1
        newLine += 1
      }
    }
  }
  return rows
}

/**
 * Compute the split-view rows. Pairs adjacent removed/added blocks one-to-one
 * so the user sees changed lines side-by-side; remaining lines go on their
 * own column with a blank counterpart.
 */
export function buildSplitRows(before: string, after: string): SplitRow[] {
  const before_ = before ?? ""
  const after_ = after ?? ""
  const blocks: Change[] = diffLines(before_, after_)
  const rows: SplitRow[] = []
  let oldLine = 1
  let newLine = 1
  let i = 0

  while (i < blocks.length) {
    const block = blocks[i]
    if (!block.added && !block.removed) {
      const contextLines = trimTrailingNewline(block.value).split("\n")
      for (const line of contextLines) {
        rows.push({
          left: { type: "context", text: line, lineNumber: oldLine },
          right: { type: "context", text: line, lineNumber: newLine },
        })
        oldLine += 1
        newLine += 1
      }
      i += 1
      continue
    }

    if (block.removed) {
      const removedLines = trimTrailingNewline(block.value).split("\n")
      // Pair with the immediately following added block when present so
      // changed lines render across from each other.
      const next = blocks[i + 1]
      if (next?.added) {
        const addedLines = trimTrailingNewline(next.value).split("\n")
        const pairCount = Math.min(removedLines.length, addedLines.length)
        for (let k = 0; k < pairCount; k++) {
          rows.push({
            left: { type: "removed", text: removedLines[k], lineNumber: oldLine },
            right: { type: "added", text: addedLines[k], lineNumber: newLine },
          })
          oldLine += 1
          newLine += 1
        }
        for (let k = pairCount; k < removedLines.length; k++) {
          rows.push({
            left: { type: "removed", text: removedLines[k], lineNumber: oldLine },
          })
          oldLine += 1
        }
        for (let k = pairCount; k < addedLines.length; k++) {
          rows.push({
            right: { type: "added", text: addedLines[k], lineNumber: newLine },
          })
          newLine += 1
        }
        i += 2
        continue
      }

      for (const line of removedLines) {
        rows.push({ left: { type: "removed", text: line, lineNumber: oldLine } })
        oldLine += 1
      }
      i += 1
      continue
    }

    // Pure added block (no preceding removed)
    const addedLines = trimTrailingNewline(block.value).split("\n")
    for (const line of addedLines) {
      rows.push({ right: { type: "added", text: line, lineNumber: newLine } })
      newLine += 1
    }
    i += 1
  }

  return rows
}

function trimTrailingNewline(value: string): string {
  return value.endsWith("\n") ? value.slice(0, -1) : value
}

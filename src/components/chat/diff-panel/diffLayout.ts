import { diffLines, diffWordsWithSpace, type Change } from "diff"

export interface InlineToken {
  text: string
  changed: boolean
}

export interface UnifiedRow {
  type: "added" | "removed" | "context"
  text: string
  oldLineNumber?: number
  newLineNumber?: number
  hunkIndex?: number
  isHunkStart?: boolean
  inlineTokens?: InlineToken[]
}

export interface SplitCell {
  type: "added" | "removed" | "context"
  text: string
  lineNumber: number
  inlineTokens?: InlineToken[]
}

export interface SplitRow {
  /** Left column (old). undefined = blank line in split view. */
  left?: Extract<SplitCell, { type: "removed" | "context" }> | SplitCell
  /** Right column (new). undefined = blank line in split view. */
  right?: Extract<SplitCell, { type: "added" | "context" }> | SplitCell
  hunkIndex?: number
  isHunkStart?: boolean
}

export interface VisibleRowItem<T> {
  kind: "row"
  row: T
  rowIndex: number
}

export interface FoldItem {
  kind: "fold"
  id: string
  hiddenCount: number
  startIndex: number
  endIndex: number
}

export type DiffViewItem<T> = VisibleRowItem<T> | FoldItem

export interface DiffBuildOptions {
  ignoreWhitespace?: boolean
}

const DEFAULT_CONTEXT_LINES = 4

/**
 * Compute the unified-view list of rows from a before/after pair. Splits the
 * diff blocks back into individual lines so each can be tagged as added /
 * removed / context with the correct line number on each side.
 */
export function buildUnifiedRows(
  before: string,
  after: string,
  options: DiffBuildOptions = {},
): UnifiedRow[] {
  const blocks = buildLineBlocks(before, after, options)
  const rows: UnifiedRow[] = []
  let oldLine = 1
  let newLine = 1
  let i = 0

  while (i < blocks.length) {
    const block = blocks[i]
    if (block.added) {
      const addedLines = splitBlockLines(block.value)
      const hunkIndex = nextHunkIndex(rows)
      addedLines.forEach((line, idx) => {
        rows.push({
          type: "added",
          text: line,
          newLineNumber: newLine,
          hunkIndex,
          isHunkStart: idx === 0,
        })
        newLine += 1
      })
      i += 1
      continue
    }

    if (block.removed) {
      const removedLines = splitBlockLines(block.value)
      const next = blocks[i + 1]
      const addedLines = next?.added ? splitBlockLines(next.value) : []
      const hunkIndex = nextHunkIndex(rows)
      const pairCount = Math.min(removedLines.length, addedLines.length)
      const inlinePairs = buildInlinePairs(removedLines, addedLines)

      removedLines.forEach((line, idx) => {
        rows.push({
          type: "removed",
          text: line,
          oldLineNumber: oldLine,
          hunkIndex,
          isHunkStart: idx === 0,
          inlineTokens: idx < pairCount ? inlinePairs[idx]?.oldTokens : undefined,
        })
        oldLine += 1
      })

      if (next?.added) {
        addedLines.forEach((line, idx) => {
          rows.push({
            type: "added",
            text: line,
            newLineNumber: newLine,
            hunkIndex,
            inlineTokens: idx < pairCount ? inlinePairs[idx]?.newTokens : undefined,
          })
          newLine += 1
        })
        i += 2
      } else {
        i += 1
      }
      continue
    }

    for (const line of splitBlockLines(block.value)) {
      rows.push({
        type: "context",
        text: line,
        oldLineNumber: oldLine,
        newLineNumber: newLine,
      })
      oldLine += 1
      newLine += 1
    }
    i += 1
  }

  return rows
}

/**
 * Compute the split-view rows. Pairs adjacent removed/added blocks one-to-one
 * so the user sees changed lines side-by-side; remaining lines go on their
 * own column with a blank counterpart.
 */
export function buildSplitRows(
  before: string,
  after: string,
  options: DiffBuildOptions = {},
): SplitRow[] {
  const blocks = buildLineBlocks(before, after, options)
  const rows: SplitRow[] = []
  let oldLine = 1
  let newLine = 1
  let i = 0

  while (i < blocks.length) {
    const block = blocks[i]
    if (!block.added && !block.removed) {
      for (const line of splitBlockLines(block.value)) {
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
      const removedLines = splitBlockLines(block.value)
      const next = blocks[i + 1]
      const hunkIndex = nextHunkIndex(rows)
      if (next?.added) {
        const addedLines = splitBlockLines(next.value)
        const pairCount = Math.min(removedLines.length, addedLines.length)
        const inlinePairs = buildInlinePairs(removedLines, addedLines)

        for (let k = 0; k < pairCount; k++) {
          rows.push({
            left: {
              type: "removed",
              text: removedLines[k],
              lineNumber: oldLine,
              inlineTokens: inlinePairs[k]?.oldTokens,
            },
            right: {
              type: "added",
              text: addedLines[k],
              lineNumber: newLine,
              inlineTokens: inlinePairs[k]?.newTokens,
            },
            hunkIndex,
            isHunkStart: k === 0,
          })
          oldLine += 1
          newLine += 1
        }
        for (let k = pairCount; k < removedLines.length; k++) {
          rows.push({
            left: { type: "removed", text: removedLines[k], lineNumber: oldLine },
            hunkIndex,
            isHunkStart: pairCount === 0 && k === 0,
          })
          oldLine += 1
        }
        for (let k = pairCount; k < addedLines.length; k++) {
          rows.push({
            right: { type: "added", text: addedLines[k], lineNumber: newLine },
            hunkIndex,
            isHunkStart: removedLines.length === 0 && k === 0,
          })
          newLine += 1
        }
        i += 2
        continue
      }

      for (let k = 0; k < removedLines.length; k++) {
        rows.push({
          left: { type: "removed", text: removedLines[k], lineNumber: oldLine },
          hunkIndex,
          isHunkStart: k === 0,
        })
        oldLine += 1
      }
      i += 1
      continue
    }

    const hunkIndex = nextHunkIndex(rows)
    const addedLines = splitBlockLines(block.value)
    for (let k = 0; k < addedLines.length; k++) {
      rows.push({
        right: { type: "added", text: addedLines[k], lineNumber: newLine },
        hunkIndex,
        isHunkStart: k === 0,
      })
      newLine += 1
    }
    i += 1
  }

  return rows
}

export function isUnifiedRowChanged(row: UnifiedRow): boolean {
  return row.type === "added" || row.type === "removed"
}

export function isSplitRowChanged(row: SplitRow): boolean {
  return row.left?.type === "removed" || row.right?.type === "added"
}

export function hunkCountForRows(rows: Array<{ hunkIndex?: number }>): number {
  let max = -1
  for (const row of rows) {
    if (typeof row.hunkIndex === "number") max = Math.max(max, row.hunkIndex)
  }
  return max + 1
}

export function firstUnifiedLine(row: UnifiedRow): number | null {
  return row.newLineNumber ?? row.oldLineNumber ?? null
}

export function firstSplitLine(row: SplitRow): number | null {
  return row.right?.lineNumber ?? row.left?.lineNumber ?? null
}

export function buildVisibleRowItems<T>(
  rows: T[],
  options: {
    collapseContext: boolean
    expandedFoldIds: Set<string>
    isChanged: (row: T) => boolean
    contextLines?: number
  },
): DiffViewItem<T>[] {
  const { collapseContext, expandedFoldIds, isChanged } = options
  const contextLines = options.contextLines ?? DEFAULT_CONTEXT_LINES
  if (!collapseContext) {
    return rows.map((row, rowIndex) => ({ kind: "row", row, rowIndex }))
  }

  const changedIndexes: number[] = []
  rows.forEach((row, idx) => {
    if (isChanged(row)) changedIndexes.push(idx)
  })
  if (changedIndexes.length === 0) {
    return rows.map((row, rowIndex) => ({ kind: "row", row, rowIndex }))
  }

  const visible = new Array(rows.length).fill(false)
  for (const idx of changedIndexes) {
    const start = Math.max(0, idx - contextLines)
    const end = Math.min(rows.length - 1, idx + contextLines)
    for (let i = start; i <= end; i++) visible[i] = true
  }

  const items: DiffViewItem<T>[] = []
  let i = 0
  while (i < rows.length) {
    if (visible[i]) {
      items.push({ kind: "row", row: rows[i], rowIndex: i })
      i += 1
      continue
    }

    const start = i
    while (i < rows.length && !visible[i]) i += 1
    const end = i - 1
    const id = `${start}-${end}`
    if (expandedFoldIds.has(id)) {
      for (let idx = start; idx <= end; idx++) {
        items.push({ kind: "row", row: rows[idx], rowIndex: idx })
      }
    } else {
      items.push({ kind: "fold", id, hiddenCount: end - start + 1, startIndex: start, endIndex: end })
    }
  }
  return items
}

function buildLineBlocks(before: string, after: string, options: DiffBuildOptions): Change[] {
  return diffLines(before ?? "", after ?? "", {
    ignoreWhitespace: options.ignoreWhitespace,
    stripTrailingCr: true,
  })
}

function splitBlockLines(value: string): string[] {
  return trimTrailingNewline(value).split("\n")
}

function trimTrailingNewline(value: string): string {
  return value.endsWith("\n") ? value.slice(0, -1) : value
}

function nextHunkIndex(rows: Array<{ hunkIndex?: number }>): number {
  return hunkCountForRows(rows)
}

function buildInlinePairs(
  removedLines: string[],
  addedLines: string[],
): Array<{ oldTokens: InlineToken[]; newTokens: InlineToken[] }> {
  const pairCount = Math.min(removedLines.length, addedLines.length)
  const pairs: Array<{ oldTokens: InlineToken[]; newTokens: InlineToken[] }> = []
  for (let i = 0; i < pairCount; i++) {
    pairs.push(buildInlinePair(removedLines[i], addedLines[i]))
  }
  return pairs
}

function buildInlinePair(oldText: string, newText: string): { oldTokens: InlineToken[]; newTokens: InlineToken[] } {
  const oldTokens: InlineToken[] = []
  const newTokens: InlineToken[] = []
  for (const part of diffWordsWithSpace(oldText, newText)) {
    const token = { text: part.value, changed: !!part.added || !!part.removed }
    if (part.added) {
      newTokens.push(token)
    } else if (part.removed) {
      oldTokens.push(token)
    } else {
      oldTokens.push(token)
      newTokens.push(token)
    }
  }
  return { oldTokens, newTokens }
}

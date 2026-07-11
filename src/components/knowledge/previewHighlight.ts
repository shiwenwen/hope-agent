export interface MarkdownPreviewHighlightSplit {
  before: string
  highlighted: string
  after: string
  startLine: number
  endLine: number
}

const FENCE_RE = /^( {0,3})(`{3,}|~{3,})(.*)$/
const ATX_HEADING_RE = /^ {0,3}#{1,6}(?:[ \t]+.*)?[ \t]*$/

export function splitMarkdownForPreviewHighlight(
  content: string,
  line: number | null | undefined,
): MarkdownPreviewHighlightSplit | null {
  if (!Number.isFinite(line) || !line || line < 1) return null
  const lines = content.split(/\r?\n/)
  if (lines.length === 0) return null
  const target = Math.min(Math.max(Math.floor(line), 1), lines.length)
  const targetIndex = target - 1
  if (!lines[targetIndex]?.trim()) return null

  const fenced = fencedRangeContaining(lines, targetIndex)
  const range = fenced ?? proseRangeContaining(lines, targetIndex)
  if (!range) return null

  return {
    before: lines.slice(0, range.start).join("\n"),
    highlighted: lines.slice(range.start, range.end + 1).join("\n"),
    after: lines.slice(range.end + 1).join("\n"),
    startLine: range.start + 1,
    endLine: range.end + 1,
  }
}

function fencedRangeContaining(
  lines: string[],
  targetIndex: number,
): { start: number; end: number } | null {
  let fenceMarker: string | null = null
  let fenceLen = 0
  let fenceStart = -1

  for (let i = 0; i < lines.length; i++) {
    const fence = FENCE_RE.exec(lines[i])
    if (!fence) continue
    const marker = fence[2]
    const ch = marker[0]
    const len = marker.length
    if (fenceMarker === null) {
      fenceMarker = ch
      fenceLen = len
      fenceStart = i
      continue
    }

    if (ch === fenceMarker && len >= fenceLen && fence[3].trim() === "") {
      const fenceEnd = i
      if (targetIndex >= fenceStart && targetIndex <= fenceEnd) {
        return { start: fenceStart, end: fenceEnd }
      }
      fenceMarker = null
      fenceLen = 0
      fenceStart = -1
    }
  }

  if (fenceMarker !== null && targetIndex >= fenceStart) {
    return { start: fenceStart, end: lines.length - 1 }
  }
  return null
}

function proseRangeContaining(
  lines: string[],
  targetIndex: number,
): { start: number; end: number } | null {
  const targetLine = lines[targetIndex]
  if (ATX_HEADING_RE.test(targetLine)) return { start: targetIndex, end: targetIndex }

  let start = targetIndex
  while (start > 0 && lines[start - 1].trim() && !FENCE_RE.test(lines[start - 1])) {
    if (ATX_HEADING_RE.test(lines[start - 1])) break
    start -= 1
  }

  let end = targetIndex
  while (end + 1 < lines.length && lines[end + 1].trim() && !FENCE_RE.test(lines[end + 1])) {
    if (ATX_HEADING_RE.test(lines[end + 1])) break
    end += 1
  }

  return { start, end }
}

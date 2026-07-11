import { parseHeadings } from "./outline"

export const KNOWLEDGE_FOCUS_EVENT = "hope:knowledge-focus"

const STORAGE_KEY = "hope.knowledge.focus"
const FENCE_RE = /^( {0,3})(`{3,}|~{3,})(.*)$/
const BLOCK_ANCHOR_RE = /(?:^|\s)\^([A-Za-z0-9-]+)\s*$/

export interface KnowledgeFocusTarget {
  kbId: string
  path: string
  line?: number
  col?: number
  headingPath?: string
  blockId?: string
}

export interface KnowledgeFocusRevealRequest {
  line?: number
  col?: number
  headingPath?: string
  blockId?: string
}

export interface KnowledgeFocusRevealTarget {
  line: number
  col?: number
}

function normalizeTarget(value: unknown): KnowledgeFocusTarget | null {
  if (!value || typeof value !== "object") return null
  const raw = value as Record<string, unknown>
  if (typeof raw.kbId !== "string" || raw.kbId.length === 0) return null
  if (typeof raw.path !== "string" || raw.path.length === 0) return null
  const line = typeof raw.line === "number" && Number.isFinite(raw.line) ? raw.line : undefined
  const col = typeof raw.col === "number" && Number.isFinite(raw.col) ? raw.col : undefined
  const headingPath = typeof raw.headingPath === "string" ? raw.headingPath : undefined
  const blockId = typeof raw.blockId === "string" ? raw.blockId : undefined
  return {
    kbId: raw.kbId,
    path: raw.path,
    ...(line ? { line } : {}),
    ...(col != null ? { col } : {}),
    ...(headingPath ? { headingPath } : {}),
    ...(blockId ? { blockId } : {}),
  }
}

function normalizeBlockId(blockId: string | undefined): string | null {
  const trimmed = blockId?.trim().replace(/^\^/, "") ?? ""
  return /^[A-Za-z0-9-]+$/.test(trimmed) ? trimmed : null
}

function normalizeHeadingPath(path: string | undefined): string | null {
  const normalized = path
    ?.split(">")
    .map((part) => part.trim().replace(/\s+/g, " "))
    .filter(Boolean)
    .join(" > ")
  return normalized || null
}

function findBlockLine(content: string, blockId: string): number | null {
  const lines = content.split(/\r?\n/)
  let fenceMarker: string | null = null
  let fenceLen = 0
  let currentBlockStart: number | null = null
  let lastBlockStart: number | null = null

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const lineNo = i + 1
    const fence = FENCE_RE.exec(line)
    if (fence) {
      const marker = fence[2]
      const ch = marker[0]
      const len = marker.length
      if (fenceMarker === null) {
        if (currentBlockStart != null) lastBlockStart = currentBlockStart
        currentBlockStart = null
        fenceMarker = ch
        fenceLen = len
        continue
      }
      if (ch === fenceMarker && len >= fenceLen && fence[3].trim() === "") {
        fenceMarker = null
        fenceLen = 0
      }
      continue
    }
    if (fenceMarker !== null) continue

    const trimmed = line.trim()
    if (!trimmed) {
      if (currentBlockStart != null) lastBlockStart = currentBlockStart
      currentBlockStart = null
      continue
    }

    const anchor = BLOCK_ANCHOR_RE.exec(line)
    const standaloneAnchor = /^\^[A-Za-z0-9-]+\s*$/.test(trimmed)
    if (standaloneAnchor) {
      if (anchor?.[1] === blockId) return currentBlockStart ?? lastBlockStart ?? lineNo
      if (currentBlockStart != null) lastBlockStart = currentBlockStart
      currentBlockStart = null
      continue
    }

    if (currentBlockStart == null) currentBlockStart = lineNo
    if (anchor?.[1] === blockId) return currentBlockStart ?? lineNo
  }

  return null
}

function findHeadingLine(content: string, headingPath: string): number | null {
  const wanted = normalizeHeadingPath(headingPath)
  if (!wanted) return null
  const stack: Array<{ level: number; text: string }> = []
  for (const heading of parseHeadings(content)) {
    while (stack.length && stack[stack.length - 1].level >= heading.level) stack.pop()
    stack.push({ level: heading.level, text: heading.text.trim().replace(/\s+/g, " ") })
    if (stack.map((h) => h.text).filter(Boolean).join(" > ") === wanted) return heading.line
  }
  return null
}

export function resolveKnowledgeFocusReveal(
  content: string,
  request: KnowledgeFocusRevealRequest | null | undefined,
): KnowledgeFocusRevealTarget | null {
  if (!request) return null
  const blockId = normalizeBlockId(request.blockId)
  if (blockId) {
    const line = findBlockLine(content, blockId)
    if (line != null) return { line }
  }

  const line =
    typeof request.line === "number" && Number.isFinite(request.line) && request.line > 0
      ? Math.floor(request.line)
      : null
  if (line != null) {
    const col =
      typeof request.col === "number" && Number.isFinite(request.col) && request.col >= 0
        ? Math.floor(request.col)
        : undefined
    return { line, ...(col != null ? { col } : {}) }
  }

  const headingPath = normalizeHeadingPath(request.headingPath)
  if (headingPath) {
    const headingLine = findHeadingLine(content, headingPath)
    if (headingLine != null) return { line: headingLine }
  }

  return null
}

export function requestKnowledgeFocus(target: KnowledgeFocusTarget): void {
  if (typeof window === "undefined") return
  try {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(target))
  } catch {
    /* sessionStorage may be unavailable; the live event still works. */
  }
  window.dispatchEvent(new CustomEvent(KNOWLEDGE_FOCUS_EVENT, { detail: target }))
}

export function consumePendingKnowledgeFocus(): KnowledgeFocusTarget | null {
  if (typeof window === "undefined") return null
  try {
    const raw = window.sessionStorage.getItem(STORAGE_KEY)
    if (!raw) return null
    window.sessionStorage.removeItem(STORAGE_KEY)
    return normalizeTarget(JSON.parse(raw))
  } catch {
    return null
  }
}

export function subscribeKnowledgeFocus(
  handler: (target: KnowledgeFocusTarget) => void,
): () => void {
  if (typeof window === "undefined") return () => {}
  const listener = (event: Event) => {
    const target = normalizeTarget((event as CustomEvent<unknown>).detail)
    if (target) handler(target)
  }
  window.addEventListener(KNOWLEDGE_FOCUS_EVENT, listener)
  return () => window.removeEventListener(KNOWLEDGE_FOCUS_EVENT, listener)
}

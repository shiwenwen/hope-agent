export const PASTED_TEXT_ATTACHMENT_SOURCE = "pasted_text" as const

export const LONG_PASTE_MIN_CHARS = 4_000
export const LONG_PASTE_MIN_LINES = 30

export interface PastedTextFileMeta {
  source: typeof PASTED_TEXT_ATTACHMENT_SOURCE
  title: string
  charCount: number
  lineCount: number
}

const metaByFile = new WeakMap<File, PastedTextFileMeta>()

export function shouldCreatePastedTextAttachment(text: string): boolean {
  const normalized = normalizePastedText(text)
  if (!normalized.trim()) return false
  return normalized.length >= LONG_PASTE_MIN_CHARS || lineCount(normalized) >= LONG_PASTE_MIN_LINES
}

export function createPastedTextAttachment(text: string): File {
  const normalized = normalizePastedText(text)
  const title = pastedTextTitle(normalized)
  const file = new File([normalized], `${title}.txt`, {
    type: "text/plain",
    lastModified: Date.now(),
  })
  metaByFile.set(file, {
    source: PASTED_TEXT_ATTACHMENT_SOURCE,
    title,
    charCount: normalized.length,
    lineCount: lineCount(normalized),
  })
  return file
}

export function getPastedTextFileMeta(file: File): PastedTextFileMeta | undefined {
  return metaByFile.get(file)
}

function normalizePastedText(text: string): string {
  return text.replace(/\r\n?/g, "\n")
}

function lineCount(text: string): number {
  return text.length === 0 ? 0 : text.split("\n").length
}

function pastedTextTitle(text: string): string {
  const firstLine = text
    .split("\n")
    .map((line) => line.trim().replace(/\s+/g, " "))
    .find(Boolean)

  const sanitized = (firstLine || "pasted-text")
    .replace(/[\\/:*?"<>|]/g, " ")
    .replace(/\s+/g, " ")
    .trim()

  const chars = Array.from(sanitized || "pasted-text")
  const title = chars.length > 48 ? chars.slice(0, 48).join("").trimEnd() : chars.join("")
  if (!title || title === "." || title === "..") return "pasted-text"
  return title
}

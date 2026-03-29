/**
 * Extract valid HTTP(S) URLs from text, deduplicated.
 * Handles both plain text URLs and markdown link syntax [text](url).
 */

const URL_REGEX = /https?:\/\/[^\s<>"')\]]+/gi

const PRIVATE_HOST_PATTERNS = [
  /^localhost$/i,
  /^127\.\d+\.\d+\.\d+$/,
  /^0\.0\.0\.0$/,
  /^10\.\d+\.\d+\.\d+$/,
  /^172\.(1[6-9]|2\d|3[01])\.\d+\.\d+$/,
  /^192\.168\.\d+\.\d+$/,
  /^\[::1\]$/,
]

const SKIP_EXTENSIONS = new Set([
  "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "bmp",
  "mp4", "webm", "mov", "avi", "mp3", "wav", "ogg", "flac",
  "zip", "tar", "gz", "rar", "7z", "pdf", "doc", "docx",
  "xls", "xlsx", "ppt", "pptx", "exe", "dmg", "iso",
])

function isPrivateHost(hostname: string): boolean {
  return PRIVATE_HOST_PATTERNS.some((p) => p.test(hostname))
}

function shouldSkipUrl(url: string): boolean {
  try {
    const parsed = new URL(url)

    if (isPrivateHost(parsed.hostname)) return true

    const path = parsed.pathname.toLowerCase()
    const ext = path.split(".").pop()
    if (ext && SKIP_EXTENSIONS.has(ext)) return true

    return false
  } catch {
    return true
  }
}

export function extractUrls(text: string): string[] {
  const matches = text.match(URL_REGEX)
  if (!matches) return []

  const seen = new Set<string>()
  const result: string[] = []

  for (const raw of matches) {
    // Trim trailing punctuation that's likely not part of the URL
    const url = raw.replace(/[.,;:!?)]+$/, "")
    if (seen.has(url) || shouldSkipUrl(url)) continue
    seen.add(url)
    result.push(url)
  }

  return result
}

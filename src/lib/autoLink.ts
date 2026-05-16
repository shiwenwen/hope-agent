export interface AutoLinkMatch {
  start: number
  end: number
  text: string
  href: string
}

const AUTO_LINK_RE =
  /(?:https?:\/\/|mailto:)[^\s<]+|www\.[^\s<]+|[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi

const TRAILING_PUNCTUATION = new Set([",", ".", "!", "?", ";", ":"])
const BRACKET_PAIRS: Array<[string, string]> = [
  ["(", ")"],
  ["[", "]"],
  ["{", "}"],
  ["<", ">"],
]

function hasBoundaryBefore(text: string, start: number): boolean {
  if (start === 0) return true
  return /[\s([{"'`<]/.test(text[start - 1] || "")
}

function countChar(text: string, char: string): number {
  let count = 0
  for (const ch of text) {
    if (ch === char) count++
  }
  return count
}

function trimmedLinkLength(raw: string): number {
  let end = raw.length

  while (end > 0 && TRAILING_PUNCTUATION.has(raw[end - 1] || "")) {
    end--
  }

  let changed = true
  while (changed && end > 0) {
    changed = false
    const value = raw.slice(0, end)
    for (const [open, close] of BRACKET_PAIRS) {
      if (!value.endsWith(close)) continue
      if (countChar(value, close) > countChar(value, open)) {
        end--
        changed = true
        break
      }
    }
  }

  return end
}

function hrefForAutoLink(text: string): string {
  if (/^www\./i.test(text)) return `https://${text}`
  if (/^[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}$/i.test(text)) return `mailto:${text}`
  return text
}

export function findAutoLinkMatches(text: string): AutoLinkMatch[] {
  const matches: AutoLinkMatch[] = []
  AUTO_LINK_RE.lastIndex = 0

  for (;;) {
    const match = AUTO_LINK_RE.exec(text)
    if (!match) break

    const start = match.index
    if (!hasBoundaryBefore(text, start)) continue

    const raw = match[0]
    const length = trimmedLinkLength(raw)
    if (length <= 0) continue

    const linkText = raw.slice(0, length)
    matches.push({
      start,
      end: start + length,
      text: linkText,
      href: hrefForAutoLink(linkText),
    })
  }

  return matches
}

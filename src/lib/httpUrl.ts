const percentEscapePattern = /%[0-9a-fA-F]{2}/g
const unreservedCharacterPattern = /^[A-Za-z0-9._~-]$/

function normalizePercentEncoding(value: string): string {
  return value.replace(percentEscapePattern, (escape) => {
    const character = String.fromCharCode(Number.parseInt(escape.slice(1), 16))
    return unreservedCharacterPattern.test(character)
      ? character
      : escape.toUpperCase()
  })
}

/** Canonical comparison/storage form for an HTTP transport base URL. */
export function normalizeHttpBaseUrl(value: string): string {
  const trimmed = value.trim()
  if (!trimmed) return ""
  try {
    // URL canonicalizes host casing, default ports, and dot segments. RFC 3986
    // additionally treats escaped unreserved characters as their literal form.
    return normalizePercentEncoding(new URL(trimmed).href).replace(/\/+$/, "")
  } catch {
    // Validation belongs to the caller; keep comparisons deterministic and
    // fail closed for temporarily incomplete form values.
    return trimmed.replace(/\/+$/, "")
  }
}

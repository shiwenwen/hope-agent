/** Canonical comparison/storage form for an HTTP transport base URL. */
export function normalizeHttpBaseUrl(value: string): string {
  const trimmed = value.trim()
  if (!trimmed) return ""
  try {
    // URL canonicalizes host casing, default ports, dot segments, and escapes.
    return new URL(trimmed).href.replace(/\/+$/, "")
  } catch {
    // Validation belongs to the caller; keep comparisons deterministic and
    // fail closed for temporarily incomplete form values.
    return trimmed.replace(/\/+$/, "")
  }
}

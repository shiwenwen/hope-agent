/** Path splitting for the preview-header breadcrumb (see FilePathBreadcrumb). */

export interface FilePathSegment {
  /** Segment text (one path component). */
  label: string
  /** Path from the root through this segment, in the original separator style. */
  path: string
  /** Every segment but the last one addresses a directory. */
  isDir: boolean
}

/**
 * Split a POSIX or Windows path into cumulative segments. The original
 * separator is preserved so a segment path can be handed straight back to a
 * platform API; a Windows drive (`C:`) stays its own leading segment.
 */
export function splitPathSegments(path: string): FilePathSegment[] {
  const separator = path.includes("\\") && !path.includes("/") ? "\\" : "/"
  const trimmed = path.replace(/[\\/]+$/, "")
  const parts = trimmed.split(/[\\/]/).filter(Boolean)
  if (parts.length === 0) return []
  const rootPrefix = /^[\\/]/.test(trimmed) ? separator : ""
  const segments: FilePathSegment[] = []
  for (const [index, part] of parts.entries()) {
    const previous = segments[index - 1]
    segments.push({
      label: part,
      path: previous ? `${previous.path}${separator}${part}` : `${rootPrefix}${part}`,
      isDir: index < parts.length - 1,
    })
  }
  return segments
}

/**
 * URLs and other opaque identifiers must not be rendered as a path. The scheme
 * is required to be at least two characters so a Windows drive (`C:\…`) is
 * still treated as a path.
 */
export function isBreadcrumbablePath(path: string): boolean {
  if (/^[a-z][a-z0-9+.-]+:/i.test(path)) return false
  return splitPathSegments(path).length > 1
}

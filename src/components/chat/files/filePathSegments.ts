/** Path splitting for the preview-header breadcrumb (see FilePathBreadcrumb). */

export interface FilePathSegment {
  /** Segment text (one path component). */
  label: string
  /** Path from the root through this segment, in the original separator style. */
  path: string
  /** Every segment but the last one addresses a directory. */
  isDir: boolean
}

/** Cumulative segments of a POSIX or Windows path, original separator kept. */
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

/** Rejects URLs and opaque schemes; a 2+ char scheme keeps `C:\…` a path. */
export function isBreadcrumbablePath(path: string): boolean {
  if (/^[a-z][a-z0-9+.-]+:/i.test(path)) return false
  return splitPathSegments(path).length > 1
}

/**
 * Last segment of a POSIX or Windows path. Handles trailing separators and
 * mixed `\` / `/` (Windows paths surface in Tauri builds). Falls back to the
 * raw input when the path collapses to nothing.
 */
export function basename(path: string): string {
  const normalized = path.replace(/[\\/]+$/, "")
  const parts = normalized.split(/[\\/]/).filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : normalized || path
}

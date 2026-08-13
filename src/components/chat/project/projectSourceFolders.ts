export interface PromotedProjectSourceFolders {
  workingDir: string
  linkedDirs: string[]
}

export function projectRootFromInstructionsPath(path: string): string | null {
  const trimmed = path.trim().replace(/[\\/]+$/, "")
  const separator = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"))
  if (separator < 0 || trimmed.slice(separator + 1) !== "AGENTS.md") return null
  return trimmed.slice(0, separator) || (trimmed.startsWith("/") ? "/" : null)
}

/** Resolve the primary represented by the current form state. `clearedDefaultDir`
 * is deliberately tri-state: undefined means the persisted effective root is
 * still valid, null means an explicitly-cleared root is resolving its managed
 * default, and a string is that resolved default. */
export function resolveProjectFormPrimaryDir(
  workingDir: string,
  persistedEffectiveDir: string | null,
  clearedDefaultDir?: string | null,
): string | null {
  const explicit = workingDir.trim()
  if (explicit) return explicit
  const fallback = clearedDefaultDir === undefined ? persistedEffectiveDir : clearedDefaultDir
  return fallback?.trim() || null
}

/**
 * Promote a linked source folder while preserving the effective former
 * primary. `previousPrimary` may be the backend-owned default workspace when
 * the Project row has no explicit `workingDir`.
 */
export function promoteProjectSourceFolder(
  nextPrimary: string,
  previousPrimary: string,
  linkedDirs: string[],
): PromotedProjectSourceFolders {
  const nextLinkedDirs = linkedDirs.filter((linked) => linked !== nextPrimary)
  if (
    previousPrimary &&
    previousPrimary !== nextPrimary &&
    !nextLinkedDirs.includes(previousPrimary)
  ) {
    nextLinkedDirs.unshift(previousPrimary)
  }
  return { workingDir: nextPrimary, linkedDirs: nextLinkedDirs }
}

/**
 * Roots shown beside a session's active cwd. Stored linked-root indices stay
 * unchanged; an overridden Project primary is appended as the virtual trailing
 * root understood by the session-scoped project-folder resolver.
 */
export function projectSourceFoldersForSession(
  linkedDirs: string[],
  sessionWorkingDir: string | null,
  projectWorkingDir: string | null,
): string[] {
  if (
    !sessionWorkingDir ||
    !projectWorkingDir ||
    sessionWorkingDir === projectWorkingDir ||
    linkedDirs.includes(projectWorkingDir)
  ) {
    return linkedDirs
  }
  return [...linkedDirs, projectWorkingDir]
}

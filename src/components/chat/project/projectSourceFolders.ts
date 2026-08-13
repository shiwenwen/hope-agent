export interface PromotedProjectSourceFolders {
  workingDir: string
  linkedDirs: string[]
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

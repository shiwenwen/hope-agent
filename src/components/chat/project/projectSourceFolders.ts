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

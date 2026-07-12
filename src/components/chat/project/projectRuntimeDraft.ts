import type { GitBranchInfo, GitInfo } from "@/lib/transport"

export interface ProjectRuntimeDraft {
  requestId: string
  launchMode: "local" | "worktree"
  baseRef: string | null
  baseRefKind: "local" | "remote" | null
  includeLocalChanges: boolean
}

export const createLocalProjectRuntimeDraft = (): ProjectRuntimeDraft => ({
  requestId: "",
  launchMode: "local",
  baseRef: null,
  baseRefKind: null,
  includeLocalChanges: false,
})

export function defaultProjectBranch(info: GitInfo): GitBranchInfo | null {
  return (
    info.branches.find((branch) => branch.isCurrent && branch.kind === "local") ??
    info.branches.find((branch) => branch.kind === "local" && branch.name === "main") ??
    info.branches.find((branch) => branch.kind === "local" && branch.name === "master") ??
    info.branches.find((branch) => branch.kind === "local") ??
    info.branches.find((branch) => branch.kind === "remote") ??
    null
  )
}

export function projectRuntimeDraftForBranch(
  current: ProjectRuntimeDraft,
  branch: GitBranchInfo,
): ProjectRuntimeDraft {
  return {
    ...current,
    baseRef: branch.fullRef,
    baseRefKind: branch.kind,
    includeLocalChanges: branch.kind === "local" && branch.isCurrent,
  }
}

export function projectBranchDisabledForLaunch(
  branch: GitBranchInfo,
  launchMode: ProjectRuntimeDraft["launchMode"],
): boolean {
  return launchMode === "local" && branch.isCheckedOut && !branch.isCurrent
}

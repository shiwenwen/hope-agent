import { describe, expect, it } from "vitest"

import type { GitBranchInfo, GitInfo } from "@/lib/transport"
import {
  createLocalProjectRuntimeDraft,
  defaultProjectBranch,
  projectBranchDisabledForLaunch,
  projectRuntimeDraftForBranch,
} from "./projectRuntimeDraft"

const branch = (overrides: Partial<GitBranchInfo>): GitBranchInfo => ({
  name: "main",
  fullRef: "refs/heads/main",
  kind: "local",
  remote: null,
  isCurrent: false,
  isCheckedOut: false,
  ...overrides,
})

const info = (branches: GitBranchInfo[]): GitInfo => ({
  branch: null,
  branches,
  dirty: {
    stagedFiles: 0,
    unstagedFiles: 0,
    untrackedFiles: 0,
    conflictedFiles: 0,
    changedFiles: 0,
  },
  worktrees: [],
})

describe("project runtime branch defaults", () => {
  it("prefers current local branch, then main/master for detached HEAD", () => {
    const main = branch({})
    const feature = branch({
      name: "feature",
      fullRef: "refs/heads/feature",
      isCurrent: true,
    })
    expect(defaultProjectBranch(info([main, feature]))).toBe(feature)
    expect(defaultProjectBranch(info([main]))).toBe(main)
  })

  it("falls back to the first remote branch when no local branch exists", () => {
    const remote = branch({
      name: "origin/main",
      fullRef: "refs/remotes/origin/main",
      kind: "remote",
      remote: "origin",
    })
    expect(defaultProjectBranch(info([remote]))).toBe(remote)
  })

  it("only carries local changes for the current local branch", () => {
    const current = branch({ isCurrent: true })
    const remote = branch({
      name: "origin/main",
      fullRef: "refs/remotes/origin/main",
      kind: "remote",
      remote: "origin",
    })
    const initial = { ...createLocalProjectRuntimeDraft(), launchMode: "worktree" as const }
    expect(projectRuntimeDraftForBranch(initial, current).includeLocalChanges).toBe(true)
    expect(projectRuntimeDraftForBranch(initial, remote).includeLocalChanges).toBe(false)

    const local = projectRuntimeDraftForBranch(createLocalProjectRuntimeDraft(), current)
    expect(local.launchMode).toBe("local")
    expect(local.baseRef).toBe("refs/heads/main")
  })

  it("blocks local checkout of a branch owned by another worktree", () => {
    const checkedOutElsewhere = branch({ isCheckedOut: true, isCurrent: false })
    expect(projectBranchDisabledForLaunch(checkedOutElsewhere, "local")).toBe(true)
    expect(projectBranchDisabledForLaunch(checkedOutElsewhere, "worktree")).toBe(false)
    expect(
      projectBranchDisabledForLaunch(
        branch({ isCheckedOut: true, isCurrent: true }),
        "local",
      ),
    ).toBe(false)
  })
})

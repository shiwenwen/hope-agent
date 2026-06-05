import { describe, expect, it } from "vitest"
import { resolveWorkspaceEnvironmentStatus } from "./workspaceEnvironment"
import type { WorkspaceEnvironmentSnapshot, WorkspaceGitSnapshot } from "@/lib/transport"

function git(partial: Partial<WorkspaceGitSnapshot> = {}): WorkspaceGitSnapshot {
  return {
    root: "/repo",
    branch: "main",
    detached: false,
    head: "abc123",
    worktrees: [],
    status: {
      changedFiles: 0,
      stagedFiles: 0,
      unstagedFiles: 0,
      untrackedFiles: 0,
      conflictedFiles: 0,
      linesAdded: 0,
      linesRemoved: 0,
      clean: true,
    },
    sync: {
      upstream: "origin/main",
      remote: "https://example.com/repo.git",
      ahead: 0,
      behind: 0,
      state: "upToDate",
    },
    lastCommit: null,
    ...partial,
  }
}

function snapshot(g: WorkspaceGitSnapshot | null, path = "/repo"): WorkspaceEnvironmentSnapshot {
  return {
    workingDir: { path, source: "session", exists: true, name: "repo" },
    git: g,
  }
}

describe("resolveWorkspaceEnvironmentStatus", () => {
  it("handles sessions without a working directory", () => {
    expect(resolveWorkspaceEnvironmentStatus(null).kind).toBe("noWorkingDir")
  })

  it("handles non-git working directories", () => {
    expect(resolveWorkspaceEnvironmentStatus(snapshot(null)).kind).toBe("nonGit")
  })

  it("prioritizes conflicts above other states", () => {
    expect(
      resolveWorkspaceEnvironmentStatus(
        snapshot(
          git({
            status: {
              changedFiles: 2,
              stagedFiles: 0,
              unstagedFiles: 2,
              untrackedFiles: 0,
              conflictedFiles: 1,
              linesAdded: 1,
              linesRemoved: 1,
              clean: false,
            },
            sync: {
              upstream: "origin/main",
              remote: null,
              ahead: 1,
              behind: 1,
              state: "diverged",
            },
          }),
        ),
      ).kind,
    ).toBe("conflicts")
  })

  it("shows dirty before ahead/behind because local changes need attention first", () => {
    expect(
      resolveWorkspaceEnvironmentStatus(
        snapshot(
          git({
            status: {
              changedFiles: 1,
              stagedFiles: 1,
              unstagedFiles: 0,
              untrackedFiles: 0,
              conflictedFiles: 0,
              linesAdded: 5,
              linesRemoved: 0,
              clean: false,
            },
            sync: {
              upstream: "origin/main",
              remote: null,
              ahead: 2,
              behind: 0,
              state: "ahead",
            },
          }),
        ),
      ).kind,
    ).toBe("dirty")
  })

  it("classifies clean sync states", () => {
    expect(resolveWorkspaceEnvironmentStatus(snapshot(git())).kind).toBe("clean")
    expect(resolveWorkspaceEnvironmentStatus(snapshot(git({ sync: { upstream: "origin/main", remote: null, ahead: 2, behind: 0, state: "ahead" } }))).kind).toBe("ahead")
    expect(resolveWorkspaceEnvironmentStatus(snapshot(git({ sync: { upstream: "origin/main", remote: null, ahead: 0, behind: 2, state: "behind" } }))).kind).toBe("behind")
    expect(resolveWorkspaceEnvironmentStatus(snapshot(git({ sync: { upstream: "origin/main", remote: null, ahead: 1, behind: 2, state: "diverged" } }))).kind).toBe("diverged")
  })
})

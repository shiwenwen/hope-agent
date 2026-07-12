// @vitest-environment jsdom

import { act, renderHook } from "@testing-library/react"
import { describe, expect, it } from "vitest"
import type { GitPullRequestReviewComment, SessionGitDiffSnapshot } from "@/lib/transport"
import { useDiffPanel } from "./useDiffPanel"

const snapshot: SessionGitDiffSnapshot = {
  revision: "rev-1",
  scope: "unstaged",
  changes: [
    {
      kind: "file_change",
      path: "src/main.ts",
      action: "edit",
      status: "M",
      linesAdded: 1,
      linesRemoved: 1,
      before: "old\n",
      after: "new\n",
      language: "typescript",
      truncated: false,
      binary: false,
      submodule: false,
      conflicted: false,
      untracked: false,
      hunks: [],
    },
  ],
}

describe("useDiffPanel Git review state", () => {
  it("opens, replaces, and clears a repository diff context", () => {
    const { result } = renderHook(() => useDiffPanel())
    const reviewComment: GitPullRequestReviewComment = {
      threadId: "thread-1",
      commentId: "comment-1",
      author: "reviewer",
      body: "Please keep this fail-closed.",
      path: "src/main.ts",
      line: 2,
      replyCount: 0,
      isResolved: false,
      isOutdated: false,
    }

    act(() => result.current.openGitDiff(snapshot, "session-1", [reviewComment]))
    expect(result.current.showPanel).toBe(true)
    expect(result.current.gitContext).toEqual({
      sessionId: "session-1",
      scope: "unstaged",
      revision: "rev-1",
      reviewComments: [reviewComment],
    })
    expect(result.current.activeChanges[0]?.path).toBe("src/main.ts")

    act(() =>
      result.current.replaceGitDiff({
        revision: "rev-2",
        scope: "staged",
        changes: [],
      }),
    )
    expect(result.current.gitContext?.scope).toBe("staged")
    expect(result.current.gitContext?.revision).toBe("rev-2")
    expect(result.current.gitContext?.reviewComments).toEqual([reviewComment])

    act(() =>
      result.current.openDiff({
        kind: "file_change",
        path: "README.md",
        action: "edit",
        linesAdded: 1,
        linesRemoved: 0,
        before: "",
        after: "hello",
        language: "markdown",
        truncated: false,
      }),
    )
    expect(result.current.gitContext).toBeNull()
  })
})

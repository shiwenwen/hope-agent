// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { GitPullRequestFeedback, SessionGitControlSnapshot } from "@/lib/transport"
import { PullRequestPanel } from "./PullRequestPanel"

const { call, openExternalUrl } = vi.hoisted(() => ({
  call: vi.fn(),
  openExternalUrl: vi.fn(),
}))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call }),
}))
vi.mock("@/lib/openExternalUrl", () => ({ openExternalUrl }))

function feedback(mergeable = "MERGEABLE"): GitPullRequestFeedback {
  return {
    preflight: {
      available: true,
      ghAvailable: true,
      authenticated: true,
      host: "github.com",
      repository: "owner/repo",
      defaultBranch: "main",
      current: {
        number: 42,
        title: "Lifecycle details",
        body: "Keep lifecycle operations safe.",
        url: "https://github.com/owner/repo/pull/42",
        state: "OPEN",
        isDraft: false,
        baseBranch: "main",
        headBranch: "feature",
        additions: 120,
        deletions: 8,
        changedFiles: 5,
        mergeable,
        mergeStateStatus: mergeable === "CONFLICTING" ? "DIRTY" : "CLEAN",
        reviewDecision: "CHANGES_REQUESTED",
        autoMergeEnabled: false,
        reviewers: [{ login: "reviewer", kind: "User" }],
        reviews: [{
          id: "review-1",
          author: "reviewer",
          state: "CHANGES_REQUESTED",
          body: "Please keep deletion fail-closed.",
          submittedAt: "2026-07-12T00:00:00Z",
        }],
      },
    },
    checks: [],
    reviewComments: [{
      threadId: "thread-1",
      commentId: "comment-1",
      author: "reviewer",
      body: "Handle the pending row.",
      path: "src/lifecycle.rs",
      line: 21,
      replyCount: 0,
      isResolved: false,
      isOutdated: false,
    }],
    failedChecks: 0,
    pendingChecks: 0,
    passedChecks: 0,
    unresolvedComments: 1,
    checksTruncated: false,
    commentsTruncated: false,
  }
}

function feedbackWithoutPullRequest(): GitPullRequestFeedback {
  return {
    ...feedback(),
    preflight: {
      available: true,
      ghAvailable: true,
      authenticated: true,
      host: "github.com",
      repository: "owner/repo",
      defaultBranch: "main",
      current: null,
    },
    reviewComments: [],
    unresolvedComments: 0,
  }
}

const snapshot = {
  revision: "rev-1",
} as SessionGitControlSnapshot

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise
  })
  return { promise, resolve }
}

describe("PullRequestPanel", () => {
  beforeEach(() => {
    call.mockReset()
    openExternalUrl.mockReset()
  })
  afterEach(() => {
    vi.useRealTimers()
    cleanup()
  })

  it("keeps polling and refresh requests mutually exclusive", async () => {
    vi.useFakeTimers()
    const pending = deferred<GitPullRequestFeedback>()
    call.mockReturnValue(pending.promise)
    render(<PullRequestPanel sessionId="session-1" onClose={vi.fn()} />)

    await act(async () => {})
    expect(call).toHaveBeenCalledTimes(1)
    act(() => vi.advanceTimersByTime(30_000))
    expect(call).toHaveBeenCalledTimes(1)

    await act(async () => pending.resolve(feedback()))
    expect(screen.getByText("Lifecycle details")).toBeTruthy()
    act(() => vi.advanceTimersByTime(30_000))
    expect(call).toHaveBeenCalledTimes(2)
    vi.useRealTimers()
  })

  it("keeps a created PR actionable while GitHub details are still syncing", async () => {
    call.mockResolvedValue(feedbackWithoutPullRequest())
    const expectedUrl = "https://github.com/owner/repo/pull/42"
    render(
      <PullRequestPanel
        sessionId="session-1"
        expectedUrl={expectedUrl}
        onClose={vi.fn()}
      />,
    )

    expect(screen.getByText("拉取请求已创建，正在同步详情")).toBeTruthy()
    expect(await screen.findByText("拉取请求已创建，但详情尚未同步")).toBeTruthy()
    expect(screen.getByRole("button", { name: "重试" })).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "在 GitHub 打开" }))
    expect(openExternalUrl).toHaveBeenCalledWith(expectedUrl)
  })

  it("marks retained feedback as stale and disables state-dependent actions", async () => {
    call
      .mockResolvedValueOnce(feedback())
      .mockRejectedValueOnce(new Error("refresh failed"))
    render(
      <PullRequestPanel
        sessionId="session-1"
        onClose={vi.fn()}
        onFillInput={vi.fn()}
      />,
    )

    expect(await screen.findByText("Lifecycle details")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "刷新" }))
    expect(await screen.findByText("PR 状态刷新失败，当前数据可能已过期")).toBeTruthy()
    expect(screen.getByText("refresh failed")).toBeTruthy()
    expect(screen.queryByRole("button", { name: "启用自动合并" })).toBeNull()
  })

  it("renders PR details, reviews, and inline comments in the right panel", async () => {
    call.mockResolvedValue(feedback("CONFLICTING"))
    render(
      <PullRequestPanel
        sessionId="session-1"
        onClose={vi.fn()}
        onFillInput={vi.fn()}
      />,
    )

    expect(await screen.findByText("Lifecycle details")).toBeTruthy()
    expect(screen.getAllByText("reviewer").length).toBeGreaterThan(0)
    expect(screen.getByText("Please keep deletion fail-closed.")).toBeTruthy()
    expect(screen.getByText("Handle the pending row.")).toBeTruthy()
    expect(screen.getAllByText("合并冲突").length).toBeGreaterThan(0)
    expect(screen.queryByRole("button", { name: "启用自动合并" })).toBeNull()
  })

  it("enables auto-merge only after explicit confirmation", async () => {
    const prFeedback = feedback()
    call.mockImplementation((command: string) => {
      if (command === "load_session_git_control_cmd") return Promise.resolve(snapshot)
      if (command === "enable_session_git_pr_auto_merge_cmd") {
        return Promise.resolve({
          revision: "rev-1",
          head: "abc123",
          branch: "feature",
          message: "Pull request auto-merge enabled",
          url: "https://github.com/owner/repo/pull/42",
        })
      }
      return Promise.resolve(prFeedback)
    })
    render(
      <PullRequestPanel
        sessionId="session-1"
        onClose={vi.fn()}
        onFillInput={vi.fn()}
      />,
    )

    fireEvent.click(await screen.findByRole("button", { name: "启用自动合并" }))
    expect(screen.getByText(/可能立即合并/)).toBeTruthy()
    expect(call.mock.calls.some(([command]) => command === "enable_session_git_pr_auto_merge_cmd")).toBe(false)
    fireEvent.click(screen.getByRole("button", { name: "确认启用" }))
    await waitFor(() => expect(call).toHaveBeenCalledWith(
      "enable_session_git_pr_auto_merge_cmd",
      {
        sessionId: "session-1",
        input: expect.objectContaining({
          expectedRevision: "rev-1",
          method: "squash",
          confirmAutoMerge: true,
        }),
      },
    ))
  })
})

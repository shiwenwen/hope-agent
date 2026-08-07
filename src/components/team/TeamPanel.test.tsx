// @vitest-environment jsdom

import type { PropsWithChildren } from "react"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

import { TeamPanel } from "./TeamPanel"
import type { ResumeTeamResult } from "./teamTypes"

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  team: {
    teamId: "team-1",
    name: "Research team",
    leadSessionId: "session-1",
    leadAgentId: "ha-main",
    status: "paused",
    createdAt: "2026-08-05T00:00:00.000Z",
    updatedAt: "2026-08-05T00:00:00.000Z",
    config: { maxMembers: 3, autoDissolveOnComplete: false },
  },
  members: [
    {
      memberId: "member-1",
      teamId: "team-1",
      name: "researcher",
      agentId: "ha-main",
      role: "worker",
      status: "paused",
      color: "#2563eb",
      joinedAt: "2026-08-05T00:00:00.000Z",
    },
    {
      memberId: "member-2",
      teamId: "team-1",
      name: "reviewer",
      agentId: "ha-main",
      role: "reviewer",
      status: "paused",
      color: "#16a34a",
      joinedAt: "2026-08-05T00:00:00.000Z",
    },
  ],
  toast: {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
    warning: vi.fn(),
  },
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call: mocks.call }),
}))

vi.mock("sonner", () => ({ toast: mocks.toast }))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallbackOrValues?: string | Record<string, unknown>) => {
      const values = typeof fallbackOrValues === "object" ? fallbackOrValues : undefined
      const template =
        typeof fallbackOrValues === "string"
          ? fallbackOrValues
          : typeof values?.defaultValue === "string"
            ? values.defaultValue
            : key
      return template
        .replace("{{resumed}}", String(values?.resumed ?? ""))
        .replace("{{failed}}", String(values?.failed ?? ""))
    },
  }),
}))

vi.mock("@/components/chat/right-panel/RightPanelShell", () => ({
  RightPanelShell: ({ children }: PropsWithChildren) => <div>{children}</div>,
}))

vi.mock("@/components/ui/tabs", () => ({
  Tabs: ({ children }: PropsWithChildren) => <div>{children}</div>,
  TabsContent: ({ children }: PropsWithChildren) => <div>{children}</div>,
  TabsList: ({ children }: PropsWithChildren) => <div>{children}</div>,
  TabsTrigger: ({ children }: PropsWithChildren) => <button type="button">{children}</button>,
}))

vi.mock("./useTeam", () => ({
  useTeam: () => ({
    team: mocks.team,
    members: mocks.members,
    messages: [],
    tasks: [],
    sendMessage: vi.fn(),
    hasMore: false,
    loadingMore: false,
    loadMoreMessages: vi.fn(),
  }),
}))

vi.mock("./TeamDashboard", () => ({ TeamDashboard: () => null }))
vi.mock("./TeamTaskBoard", () => ({ TeamTaskBoard: () => null }))
vi.mock("./TeamMessageFeed", () => ({ TeamMessageFeed: () => null }))

beforeEach(() => {
  mocks.call.mockReset()
  mocks.members.forEach((member) => {
    member.status = "paused"
  })
  Object.values(mocks.toast).forEach((toast) => toast.mockReset())
})

afterEach(cleanup)

function result(
  disposition: ResumeTeamResult["disposition"],
  resumedMemberCount: number,
  failedMemberCount: number,
): ResumeTeamResult {
  return {
    status:
      disposition === "resumed"
        ? "resumed"
        : disposition === "partial"
          ? "partially_resumed"
          : disposition === "no_op"
            ? "already_complete"
            : "paused",
    teamStatus: disposition === "no_op" || disposition === "refused" ? "paused" : "active",
    disposition,
    teamId: "team-1",
    resumedMemberCount,
    failedMemberCount,
    resumedMembers: [],
    failures: [],
    completedDuringPauseCount: disposition === "no_op" ? 2 : 0,
    completedMembers: [],
    message: "backend diagnostic text",
  }
}

describe("TeamPanel resume feedback", () => {
  test("derives no-resume-needed from persisted completed members on a fresh render", () => {
    mocks.members.forEach((member) => {
      member.status = "completed"
    })

    render(<TeamPanel teamId="team-1" onClose={vi.fn()} />)

    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull()
    const feedback = screen.getByRole("status")
    expect(feedback).toHaveTextContent("No resume needed")
    expect(feedback.className).toContain("text-muted-foreground")
    expect(mocks.call).not.toHaveBeenCalled()
    expect(Object.values(mocks.toast).every((toast) => toast.mock.calls.length === 0)).toBe(true)
  })

  test.each([
    {
      disposition: "resumed" as const,
      resumed: 2,
      failed: 0,
      toast: "success" as const,
      feedback: "2 resumed · 0 failed",
      tone: "text-green-700",
    },
    {
      disposition: "partial" as const,
      resumed: 1,
      failed: 1,
      toast: "warning" as const,
      feedback: "1 resumed · 1 failed",
      tone: "text-amber-700",
    },
    {
      disposition: "refused" as const,
      resumed: 0,
      failed: 2,
      toast: "error" as const,
      feedback: "0 resumed · 2 failed",
      tone: "text-red-700",
    },
    {
      disposition: "no_op" as const,
      resumed: 0,
      failed: 0,
      toast: "info" as const,
      feedback: "No resume needed",
      tone: "text-muted-foreground",
    },
  ])("renders $disposition as a distinct domain result", async (scenario) => {
    mocks.call.mockResolvedValueOnce(
      result(scenario.disposition, scenario.resumed, scenario.failed),
    )
    render(<TeamPanel teamId="team-1" onClose={vi.fn()} />)

    fireEvent.click(screen.getByRole("button", { name: "Resume" }))

    await waitFor(() => expect(mocks.toast[scenario.toast]).toHaveBeenCalledTimes(1))
    expect(mocks.call).toHaveBeenCalledWith("resume_team", { teamId: "team-1" })
    const feedback = screen.getByRole("status")
    expect(feedback).toHaveTextContent(scenario.feedback)
    expect(feedback.className).toContain(scenario.tone)
    expect(feedback.getAttribute("title")).toBeNull()
  })

  test("keeps the resume action busy until the structured result arrives", async () => {
    let resolve!: (value: ResumeTeamResult) => void
    mocks.call.mockReturnValueOnce(
      new Promise<ResumeTeamResult>((done) => {
        resolve = done
      }),
    )
    render(<TeamPanel teamId="team-1" onClose={vi.fn()} />)

    fireEvent.click(screen.getByRole("button", { name: "Resume" }))

    const busyButton = screen.getByRole("button", { name: "Resuming…" })
    expect(busyButton).toBeDisabled()
    resolve(result("resumed", 2, 0))
    await waitFor(() => expect(screen.getByRole("button", { name: "Resume" })).toBeEnabled())
  })

  test("does not carry resume feedback into another team panel", async () => {
    mocks.call.mockResolvedValueOnce(result("partial", 1, 1))
    const { rerender } = render(<TeamPanel teamId="team-1" onClose={vi.fn()} />)

    fireEvent.click(screen.getByRole("button", { name: "Resume" }))
    await screen.findByRole("status")

    rerender(<TeamPanel teamId="team-2" onClose={vi.fn()} />)
    expect(screen.queryByRole("status")).toBeNull()
  })
})

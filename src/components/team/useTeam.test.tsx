// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"

import { useTeam } from "./useTeam"
import type { Team, TeamMessage } from "./teamTypes"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => vi.fn()),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

beforeEach(() => {
  transportMock.call.mockReset()
  transportMock.listen.mockReset()
  transportMock.listen.mockImplementation(() => vi.fn())
})

function team(teamId: string): Team {
  return {
    teamId,
    name: teamId,
    leadSessionId: "session-1",
    leadAgentId: "default",
    status: "active",
    createdAt: "2026-04-26T00:00:00.000Z",
    updatedAt: "2026-04-26T00:00:00.000Z",
    config: {
      maxMembers: 3,
      autoDissolveOnComplete: false,
    },
  }
}

function message(teamId: string, content: string): TeamMessage {
  return {
    messageId: `${teamId}-message`,
    teamId,
    fromMemberId: "lead",
    content,
    messageType: "chat",
    timestamp: "2026-04-26T00:00:00.000Z",
  }
}

function Harness({ teamId }: { teamId: string | null }) {
  const state = useTeam(teamId)
  return (
    <div>
      <div data-testid="loading">{String(state.loading)}</div>
      {state.messages.map((msg) => (
        <div key={msg.messageId}>{msg.content}</div>
      ))}
    </div>
  )
}

function pendingRequest(): Promise<never> {
  return new Promise(() => {
    // Keep the request pending so the test can inspect the immediate switch state.
  })
}

describe("useTeam", () => {
  test("clears loaded data immediately when switching teams", async () => {
    transportMock.call.mockImplementation((command: string, args?: { teamId?: string }) => {
      if (args?.teamId === "team-a") {
        if (command === "get_team") return Promise.resolve(team("team-a"))
        if (command === "get_team_members") return Promise.resolve([])
        if (command === "get_team_messages") {
          return Promise.resolve([[message("team-a", "old team message")], false])
        }
        if (command === "get_team_tasks") return Promise.resolve([])
      }
      if (args?.teamId === "team-b") {
        return pendingRequest()
      }
      return Promise.resolve([])
    })

    const { rerender } = render(<Harness teamId="team-a" />)
    expect(await screen.findByText("old team message")).toBeTruthy()

    rerender(<Harness teamId="team-b" />)

    expect(screen.queryByText("old team message")).toBeNull()
    expect(screen.getByTestId("loading").textContent).toBe("true")
  })
})

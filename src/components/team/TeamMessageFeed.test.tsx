// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"

import { TeamMessageFeed } from "./TeamMessageFeed"
import type { TeamMember, TeamMessage } from "./teamTypes"

const rafSpy = vi.spyOn(window, "requestAnimationFrame").mockImplementation(
  (cb: FrameRequestCallback) => {
    cb(0)
    return 0
  },
)
vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {})

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}))

beforeEach(() => {
  rafSpy.mockClear()
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const members: TeamMember[] = [
  {
    memberId: "lead",
    teamId: "team-a",
    name: "Lead",
    agentId: "default",
    role: "lead",
    status: "idle",
    color: "#2563eb",
    joinedAt: "2026-04-26T00:00:00.000Z",
  },
]

function teamMessage(patch: Partial<TeamMessage>): TeamMessage {
  return {
    messageId: "m1",
    teamId: "team-a",
    fromMemberId: "lead",
    content: "",
    messageType: "chat",
    timestamp: "2026-04-26T00:00:00.000Z",
    ...patch,
  }
}

function makeMessages(count: number, prefix: string, teamId = "team-a"): TeamMessage[] {
  return Array.from({ length: count }, (_, i) =>
    teamMessage({
      messageId: `${teamId}-${i}`,
      teamId,
      content: `${prefix}-${i}`,
      timestamp: `2026-04-26T00:${String(Math.floor(i / 60)).padStart(2, "0")}:${String(
        i % 60,
      ).padStart(2, "0")}.000Z`,
    }),
  )
}

function patchScrollMetrics(
  container: HTMLElement,
  metrics: { scrollHeight: number; clientHeight: number; scrollTop?: number },
) {
  Object.defineProperty(container, "scrollHeight", {
    configurable: true,
    get: () => metrics.scrollHeight,
  })
  Object.defineProperty(container, "clientHeight", {
    configurable: true,
    get: () => metrics.clientHeight,
  })
  if (metrics.scrollTop !== undefined) {
    container.scrollTop = metrics.scrollTop
  }
}

function getScroller(): HTMLElement {
  const el = document.querySelector<HTMLElement>(".overflow-y-auto")
  if (!el) throw new Error("scroll container not found")
  return el
}

describe("TeamMessageFeed", () => {
  test("clears the draft when switching teams", () => {
    const { rerender } = render(
      <TeamMessageFeed
        teamId="team-a"
        messages={[]}
        members={members}
        onSendMessage={vi.fn()}
      />,
    )

    const input = screen.getByPlaceholderText("Message team... (@name for DM)") as HTMLInputElement
    fireEvent.change(input, { target: { value: "draft for team a" } })
    expect(input.value).toBe("draft for team a")

    rerender(
      <TeamMessageFeed
        teamId="team-b"
        messages={[]}
        members={members}
        onSendMessage={vi.fn()}
      />,
    )

    expect(
      (screen.getByPlaceholderText("Message team... (@name for DM)") as HTMLInputElement).value,
    ).toBe("")
  })

  test("resets the rendered window when the loaded message set shrinks", () => {
    const longMessages = makeMessages(231, "long")
    const shortMessages = makeMessages(10, "short")
    const { rerender } = render(
      <TeamMessageFeed
        teamId="team-a"
        messages={longMessages}
        members={members}
        onSendMessage={vi.fn()}
      />,
    )

    const el = getScroller()
    patchScrollMetrics(el, { scrollHeight: 2000, clientHeight: 600, scrollTop: 1400 })
    act(() => {
      fireEvent.scroll(el)
    })

    rerender(
      <TeamMessageFeed
        teamId="team-a"
        messages={shortMessages}
        members={members}
        onSendMessage={vi.fn()}
      />,
    )

    expect(screen.getByText("short-0")).toBeTruthy()
    expect(screen.getByText("short-9")).toBeTruthy()
  })
})

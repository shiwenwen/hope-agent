// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

import type { SessionMeta } from "@/types/chat"
import { dispatchSessionPinChange, sessionWithPinnedState } from "../../sessionPinEvents"
import { useProjectSessions } from "./useProjectSessions"

const transportMock = vi.hoisted(() => ({ call: vi.fn() }))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: { error: vi.fn() },
}))

function session(id: string, projectId: string): SessionMeta {
  return {
    id,
    title: id,
    agentId: "agent-a",
    createdAt: "2026-08-07T00:00:00Z",
    updatedAt: "2026-08-07T00:00:00Z",
    messageCount: 1,
    unreadCount: 0,
    channelUnreadCount: 0,
    hasError: false,
    pendingInteractionCount: 0,
    isCron: false,
    incognito: false,
    projectId,
  }
}

describe("useProjectSessions pin changes", () => {
  beforeEach(() => {
    vi.useFakeTimers()
    transportMock.call.mockReset()
  })

  afterEach(() => {
    cleanup()
    vi.useRealTimers()
    vi.clearAllMocks()
  })

  test("patches only the owning project and refreshes after persistence", async () => {
    const projectSession = session("project-session", "project-a")
    transportMock.call.mockResolvedValueOnce([[projectSession], 1])
    const { result } = renderHook(() =>
      useProjectSessions({
        projectId: "project-a",
        expanded: true,
        changeSignal: "",
        sessionCount: 1,
      }),
    )

    await act(async () => {
      await vi.runOnlyPendingTimersAsync()
    })
    expect(result.current.sessions.map((item) => item.id)).toEqual([projectSession.id])
    expect(result.current.total).toBe(1)

    const callsBeforeUnrelatedEvent = transportMock.call.mock.calls.length
    act(() => {
      dispatchSessionPinChange(
        sessionWithPinnedState(session("other", "project-b"), true),
        "refresh",
      )
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200)
    })
    expect(transportMock.call).toHaveBeenCalledTimes(callsBeforeUnrelatedEvent)

    act(() => {
      dispatchSessionPinChange(sessionWithPinnedState(projectSession, true), "optimistic")
    })
    expect(result.current.sessions).toEqual([])
    expect(result.current.total).toBe(0)
    expect(transportMock.call).toHaveBeenCalledTimes(callsBeforeUnrelatedEvent)

    transportMock.call.mockResolvedValueOnce([[], 0])
    act(() => {
      dispatchSessionPinChange(sessionWithPinnedState(projectSession, true), "refresh")
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(150)
    })
    expect(transportMock.call).toHaveBeenCalledTimes(callsBeforeUnrelatedEvent + 1)
    expect(result.current.sessions).toEqual([])
    expect(result.current.total).toBe(0)
  })
})

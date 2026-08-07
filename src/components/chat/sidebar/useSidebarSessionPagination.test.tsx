// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

import type { SessionMeta } from "@/types/chat"
import { dispatchSessionPinChange, sessionWithPinnedState } from "../sessionPinEvents"
import { useSidebarSessionPagination } from "./useSidebarSessionPagination"

const transportMock = vi.hoisted(() => ({ call: vi.fn() }))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: { error: vi.fn() },
}))

function session(id: string, patch: Partial<SessionMeta> = {}): SessionMeta {
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
    ...patch,
  }
}

describe("useSidebarSessionPagination pin changes", () => {
  beforeEach(() => {
    transportMock.call.mockReset()
  })

  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  test("moves a row between the rendered lists without querying before persistence", async () => {
    const regular = session("regular")
    transportMock.call.mockImplementation((_command: string, args: Record<string, unknown>) => {
      if (args.pinned === true) return Promise.resolve([[], 0])
      if (args.parentSession === false) return Promise.resolve([[regular], 1])
      return Promise.resolve([[], 0])
    })
    const refreshSignal: SessionMeta[] = []
    const { result } = renderHook(() =>
      useSidebarSessionPagination({
        selectedAgentId: null,
        currentSessionId: null,
        enabled: true,
        refreshSignal,
      }),
    )

    await waitFor(() => expect(result.current.sessionsByFilter.session).toHaveLength(1))
    const callsBeforePin = transportMock.call.mock.calls.length

    act(() => {
      dispatchSessionPinChange(sessionWithPinnedState(regular, true), "optimistic")
    })
    expect(result.current.sessionsByFilter.session).toEqual([])
    expect(result.current.pinnedSessions.map((item) => item.id)).toEqual([regular.id])
    expect(transportMock.call).toHaveBeenCalledTimes(callsBeforePin)

    act(() => {
      dispatchSessionPinChange(regular, "rollback")
    })
    expect(result.current.pinnedSessions).toEqual([])
    expect(result.current.sessionsByFilter.session.map((item) => item.id)).toEqual([regular.id])
  })
})

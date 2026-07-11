// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import { TRANSPORT_EVENT_RESYNC_REQUIRED, type Transport } from "@/lib/transport"
import { setTransport } from "@/lib/transport-provider"
import type { AskUserQuestionGroup } from "../ask-user/AskUserQuestionBlock"
import { usePlanMode } from "./usePlanMode"

function questionGroup(overrides: Partial<AskUserQuestionGroup> = {}): AskUserQuestionGroup {
  return {
    requestId: "request-1",
    sessionId: "session-1",
    questions: [
      {
        questionId: "choice",
        text: "Choose",
        options: [{ value: "a", label: "A" }],
        allowCustom: true,
        multiSelect: false,
      },
    ],
    ...overrides,
  }
}

function mockTransport(
  pending: () => Promise<AskUserQuestionGroup | null>,
): { transport: Transport; emit: (name: string, payload: unknown) => void } {
  const listeners = new Map<string, Set<(payload: unknown) => void>>()
  const transport = {
    call: vi.fn((command: string) => {
      if (command === "get_pending_ask_user_group") return pending()
      if (command === "get_plan_mode") return Promise.resolve("off")
      if (command === "get_plan_content") return Promise.resolve(null)
      return Promise.resolve(null)
    }),
    startChat: vi.fn(),
    listen: vi.fn((name: string, handler: (payload: unknown) => void) => {
      const handlers = listeners.get(name) ?? new Set()
      handlers.add(handler)
      listeners.set(name, handlers)
      return () => handlers.delete(handler)
    }),
  } as unknown as Transport
  return {
    transport,
    emit: (name, payload) => {
      for (const handler of listeners.get(name) ?? []) handler(payload)
    },
  }
}

afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe("usePlanMode ask_user timeout reconciliation", () => {
  test("a stale restore response cannot revive a question cleared by timeout", async () => {
    let resolvePending!: (group: AskUserQuestionGroup | null) => void
    const pending = new Promise<AskUserQuestionGroup | null>((resolve) => {
      resolvePending = resolve
    })
    const mock = mockTransport(() => pending)
    setTransport(mock.transport)
    const group = questionGroup()
    const { result } = renderHook(() => usePlanMode("session-1"))

    act(() => mock.emit("ask_user_request", group))
    expect(result.current.pendingQuestionGroup?.requestId).toBe("request-1")

    act(() => {
      mock.emit("ask_user_timed_out", {
        requestId: "request-1",
        sessionId: "session-1",
      })
    })
    expect(result.current.pendingQuestionGroup).toBeNull()

    await act(async () => {
      resolvePending(group)
      await pending
    })
    expect(result.current.pendingQuestionGroup).toBeNull()
  })

  test("a failed newer reconciliation does not discard an older successful response", async () => {
    let resolveFirst!: (group: AskUserQuestionGroup | null) => void
    const first = new Promise<AskUserQuestionGroup | null>((resolve) => {
      resolveFirst = resolve
    })
    let callCount = 0
    const mock = mockTransport(() => {
      callCount += 1
      return callCount === 1 ? first : Promise.reject(new Error("temporary disconnect"))
    })
    setTransport(mock.transport)
    const group = questionGroup()
    const { result } = renderHook(() => usePlanMode("session-1"))

    await act(async () => {
      mock.emit(TRANSPORT_EVENT_RESYNC_REQUIRED, { reason: "connected" })
      await Promise.resolve()
    })
    await act(async () => {
      resolveFirst(group)
      await first
    })

    expect(result.current.pendingQuestionGroup?.requestId).toBe("request-1")
  })

  test("the local deadline clears a card even when the timeout event is lost", async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"))
    const mock = mockTransport(() => Promise.resolve(null))
    setTransport(mock.transport)
    const { result } = renderHook(() => usePlanMode("session-1"))

    await act(async () => {
      await Promise.resolve()
    })
    act(() => {
      mock.emit("ask_user_request", questionGroup({
        timeoutAt: Math.floor(Date.now() / 1000) + 1,
      }))
    })
    expect(result.current.pendingQuestionGroup?.requestId).toBe("request-1")

    await act(async () => {
      vi.advanceTimersByTime(1_000)
      await Promise.resolve()
    })
    expect(result.current.pendingQuestionGroup).toBeNull()
  })
})

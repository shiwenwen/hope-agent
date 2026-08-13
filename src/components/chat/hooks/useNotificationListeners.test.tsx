// @vitest-environment jsdom

import { useEffect, useRef, useState } from "react"
import { act, cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import type { Message } from "@/types/chat"
import { useNotificationListeners } from "./useNotificationListeners"

const transportMock = vi.hoisted(() => {
  const listeners = new Map<string, (payload: unknown) => void>()
  return {
    listeners,
    listen: vi.fn((eventName: string, handler: (payload: unknown) => void) => {
      listeners.set(eventName, handler)
      return () => {
        listeners.delete(eventName)
      }
    }),
    call: vi.fn<(...args: unknown[]) => Promise<unknown>>(() =>
      Promise.reject(new Error("not mocked")),
    ),
  }
})

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
  },
}))

vi.mock("@/lib/notifications", () => ({
  notify: vi.fn(),
}))

afterEach(() => {
  cleanup()
  transportMock.listeners.clear()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

function Harness({
  onMessages,
  consumeParentStreamDeltas,
  currentSessionId = "parent-session",
}: {
  onMessages: (messages: Message[]) => void
  consumeParentStreamDeltas?: boolean
  currentSessionId?: string
}) {
  const [messages, setMessages] = useState<Message[]>([])
  const [loading, setLoading] = useState(false)
  const [, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const currentSessionIdRef = useRef<string | null>("parent-session")
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())

  useEffect(() => {
    currentSessionIdRef.current = currentSessionId
  }, [currentSessionId])

  useNotificationListeners({
    currentSessionIdRef,
    setMessages,
    setLoading,
    loadingSessionsRef,
    setLoadingSessionIds,
    sessionCacheRef,
    reloadSessions: async () => {},
    consumeParentStreamDeltas,
  })

  useEffect(() => {
    onMessages(messages)
  }, [messages, onMessages])

  return <div data-loading={loading ? "true" : "false"} />
}

describe("useNotificationListeners", () => {
  test("does not apply a completed turn reload after the active session changes", async () => {
    let resolveLoad: ((value: [unknown[], number, boolean]) => void) | undefined
    transportMock.call.mockImplementationOnce(
      () =>
        new Promise<[unknown[], number, boolean]>((resolve) => {
          resolveLoad = resolve
        }),
    )

    let latest: Message[] = []
    const onMessages = (messages: Message[]) => {
      latest = messages
    }
    const view = render(<Harness onMessages={onMessages} />)
    const emit = transportMock.listeners.get("session:turn_started")
    expect(emit).toBeTruthy()

    await act(async () => {
      emit?.({ sessionId: "parent-session" })
    })
    view.rerender(
      <Harness currentSessionId="other-session" onMessages={onMessages} />,
    )

    await act(async () => {
      resolveLoad?.([
        [{
          id: 1,
          role: "assistant",
          content: "belongs to the previous session",
          timestamp: "2026-08-13T00:00:00.000Z",
        }],
        1,
        false,
      ])
    })

    expect(latest).toEqual([])
  })

  test("renders parent-agent stream deltas through the shared chat stream handler", async () => {
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0)
      return 1
    })
    vi.stubGlobal("cancelAnimationFrame", () => {})

    let latest: Message[] = []
    render(<Harness onMessages={(messages) => { latest = messages }} />)

    const emit = transportMock.listeners.get("parent_agent_stream")
    expect(emit).toBeTruthy()

    await act(async () => {
      emit?.({ eventType: "started", parentSessionId: "parent-session" })
    })

    await act(async () => {
      emit?.({
        eventType: "delta",
        parentSessionId: "parent-session",
        delta: JSON.stringify({ type: "text_delta", content: "before tool" }),
      })
    })

    expect(latest[0]?.content).toBe("before tool")
    expect(latest[0]?.contentBlocks).toEqual([
      { type: "text", content: "before tool" },
    ])

    await act(async () => {
      emit?.({
        eventType: "delta",
        parentSessionId: "parent-session",
        delta: JSON.stringify({
          type: "tool_call",
          call_id: "call-1",
          name: "read",
          arguments: "{\"path\":\"README.md\"}",
        }),
      })
      emit?.({
        eventType: "delta",
        parentSessionId: "parent-session",
        delta: JSON.stringify({
          type: "tool_result",
          call_id: "call-1",
          result: "ok",
          duration_ms: 5,
        }),
      })
      emit?.({
        eventType: "delta",
        parentSessionId: "parent-session",
        delta: JSON.stringify({ type: "text_delta", content: " after tool" }),
      })
    })

    expect(latest[0]?.content).toBe("before tool after tool")
    expect(latest[0]?.contentBlocks?.map((block) => block.type)).toEqual([
      "text",
      "tool_call",
      "text",
    ])
    expect(latest[0]?.contentBlocks?.[2]).toEqual({
      type: "text",
      content: " after tool",
    })
  })

  test("flushes buffered parent stream text before handling done", async () => {
    const rafCallbacks = new Map<number, FrameRequestCallback>()
    let nextRaf = 1
    const requestFrame = (cb: FrameRequestCallback) => {
      const id = nextRaf++
      rafCallbacks.set(id, cb)
      return id
    }
    const cancelFrame = (id: number) => {
      rafCallbacks.delete(id)
    }
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(requestFrame)
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(cancelFrame)
    vi.stubGlobal("requestAnimationFrame", window.requestAnimationFrame)
    vi.stubGlobal("cancelAnimationFrame", window.cancelAnimationFrame)

    let latest: Message[] = []
    render(<Harness onMessages={(messages) => { latest = messages }} />)

    const emit = transportMock.listeners.get("parent_agent_stream")
    expect(emit).toBeTruthy()

    await act(async () => {
      emit?.({ eventType: "started", parentSessionId: "parent-session" })
    })

    await act(async () => {
      emit?.({
        eventType: "delta",
        parentSessionId: "parent-session",
        delta: JSON.stringify({ type: "text_delta", content: "final chunk" }),
      })
    })

    expect(latest[0]?.content).toBe("")

    await act(async () => {
      emit?.({ eventType: "done", parentSessionId: "parent-session" })
    })

    expect(latest[0]?.content).toBe("final chunk")
    expect(latest[0]?.contentBlocks).toEqual([
      { type: "text", content: "final chunk" },
    ])
    expect(rafCallbacks.size).toBe(0)
  })

  test("can leave parent-agent deltas to the chat stream reattach path", async () => {
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0)
      return 1
    })
    vi.stubGlobal("cancelAnimationFrame", () => {})

    let latest: Message[] = []
    render(
      <Harness
        consumeParentStreamDeltas={false}
        onMessages={(messages) => { latest = messages }}
      />,
    )

    const emit = transportMock.listeners.get("parent_agent_stream")
    expect(emit).toBeTruthy()

    await act(async () => {
      emit?.({ eventType: "started", parentSessionId: "parent-session" })
    })
    expect(latest).toHaveLength(1)
    expect(latest[0]?.role).toBe("assistant")

    await act(async () => {
      emit?.({
        eventType: "delta",
        parentSessionId: "parent-session",
        delta: JSON.stringify({ type: "text_delta", content: "handled elsewhere" }),
      })
    })

    expect(latest[0]?.content).toBe("")
    expect(latest[0]?.contentBlocks).toBeUndefined()
  })
})

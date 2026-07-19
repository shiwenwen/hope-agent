// @vitest-environment jsdom

import { useEffect, useRef, useState } from "react"
import type { MutableRefObject } from "react"
import { act, cleanup, render } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

import type { Message } from "@/types/chat"
import { useChatStreamReattach } from "./useChatStreamReattach"

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, (payload: unknown) => void>()
  const pending = new Map<string, (value: unknown) => void>()
  return {
    listeners,
    pending,
    dbMessages: [] as Message[],
    transport: {
      listen: vi.fn((name: string, handler: (payload: unknown) => void) => {
        listeners.set(name, handler)
        return () => listeners.delete(name)
      }),
      call: vi.fn((name: string) =>
        new Promise((resolve) => {
          pending.set(name, resolve)
        })),
    },
    reload: vi.fn(async (params: {
      sessionId: string
      sessionCacheRef: MutableRefObject<Map<string, Message[]>>
      setMessages: (messages: Message[]) => void
    }) => {
      const next = mocks.dbMessages.map((message) => ({ ...message }))
      params.sessionCacheRef.current.set(params.sessionId, next)
      params.setMessages(next)
      return true
    }),
  }
})

let nextRafId = 1
let rafCallbacks = new Map<number, FrameRequestCallback>()

function flushAnimationFrames(): void {
  const callbacks = [...rafCallbacks.values()]
  rafCallbacks.clear()
  callbacks.forEach((callback) => callback(performance.now()))
}

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => mocks.transport,
}))

vi.mock("@/lib/logger", () => ({
  logger: { warn: vi.fn() },
}))

vi.mock("../chatUtils", async () => {
  const actual = await vi.importActual<typeof import("../chatUtils")>("../chatUtils")
  return { ...actual, reloadAndMergeSessionMessages: mocks.reload }
})

function Harness({ onMessages }: { onMessages: (messages: Message[]) => void }) {
  const [messages, setMessages] = useState<Message[]>([])
  const [, setLoading] = useState(false)
  const [, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const currentSessionIdRef = useRef<string | null>("s1")
  const lastSeqRef = useRef(new Map<string, number>())
  const endedStreamIdsRef = useRef(new Map<string, string>())
  const loadingSessionsRef = useRef(new Set<string>())
  const sessionCacheRef = useRef(new Map<string, Message[]>())

  const updateSessionMessages = (sessionId: string, updater: (prev: Message[]) => Message[]) => {
    setMessages((prev) => {
      const next = updater(prev)
      sessionCacheRef.current.set(sessionId, next)
      return next
    })
  }

  useChatStreamReattach({
    currentSessionId: "s1",
    currentSessionIdRef,
    lastSeqRef,
    endedStreamIdsRef,
    updateSessionMessages,
    setShowCodexAuthExpired: () => {},
    setMessages,
    setLoading,
    loadingSessionsRef,
    setLoadingSessionIds,
    sessionCacheRef,
    reloadSessions: async () => {},
  })

  useEffect(() => onMessages(messages), [messages, onMessages])
  return null
}

beforeEach(() => {
  mocks.listeners.clear()
  mocks.pending.clear()
  mocks.dbMessages = [{ role: "user", content: "question", dbId: 1 }]
  nextRafId = 1
  rafCallbacks = new Map()
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = nextRafId++
    rafCallbacks.set(id, callback)
    return id
  })
  vi.stubGlobal("cancelAnimationFrame", (id: number) => {
    rafCallbacks.delete(id)
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  vi.unstubAllGlobals()
})

describe("useChatStreamReattach durable snapshot handshake", () => {
  test("replays the durable prefix then buffered deltas newer than throughSeq", async () => {
    mocks.dbMessages = [
      { role: "user", content: "question", dbId: 1 },
      {
        role: "assistant",
        content: "AB",
        dbId: 2,
        persistenceRunId: "run-1",
      },
    ]
    let latest: Message[] = []
    render(<Harness onMessages={(messages) => { latest = messages }} />)
    const emit = mocks.listeners.get("chat:stream_delta")
    expect(emit).toBeTruthy()

    await act(async () => {
      emit?.({
        sessionId: "s1",
        streamId: "stream-1",
        seq: 3,
        event: JSON.stringify({ type: "text_delta", content: "C" }),
      })
      mocks.pending.get("get_session_stream_state")?.({
        active: true,
        lastSeq: 3,
        acceptedSeq: 3,
        durableSeq: 3,
        committedSeq: 0,
        persistenceRunId: "run-1",
        streamId: "stream-1",
        turnId: "turn-1",
      })
      mocks.pending.get("get_session_stream_snapshot")?.({
        sessionId: "s1",
        streamId: "stream-1",
        turnId: "turn-1",
        persistenceRunId: "run-1",
        throughSeq: 2,
        durableSeq: 2,
        committedSeq: 0,
        status: "running",
        events: [
          // Adjacent durable token deltas are journal-coalesced; the cursor
          // advances through the inclusive range while content replays once.
          { seq: 2, event: JSON.stringify({ type: "text_delta", content: "AB" }) },
        ],
      })
      await Promise.resolve()
      await Promise.resolve()
      flushAnimationFrames()
    })

    expect(latest.at(-1)?.role).toBe("assistant")
    expect(latest.at(-1)?.content).toBe("ABC")
    expect(latest).toHaveLength(2)

    await act(async () => {
      emit?.({
        sessionId: "s1",
        streamId: "stream-1",
        seq: 2,
        event: JSON.stringify({ type: "text_delta", content: "duplicate" }),
      })
    })
    expect(latest.at(-1)?.content).toBe("ABC")
  })

  test("does not replay a committed journal over canonical DB messages", async () => {
    mocks.dbMessages = [
      { role: "user", content: "question", dbId: 1 },
      { role: "assistant", content: "done", dbId: 2 },
    ]
    let latest: Message[] = []
    render(<Harness onMessages={(messages) => { latest = messages }} />)

    await act(async () => {
      mocks.pending.get("get_session_stream_state")?.({
        active: false,
        lastSeq: 1,
        acceptedSeq: 1,
        durableSeq: 1,
        committedSeq: 1,
        persistenceRunId: "run-1",
        streamId: "stream-1",
        status: "completed",
      })
      mocks.pending.get("get_session_stream_snapshot")?.({
        sessionId: "s1",
        streamId: "stream-1",
        persistenceRunId: "run-1",
        throughSeq: 1,
        durableSeq: 1,
        committedSeq: 1,
        status: "committed",
        events: [
          { seq: 1, event: JSON.stringify({ type: "text_delta", content: "done" }) },
        ],
      })
      await Promise.resolve()
      await Promise.resolve()
      flushAnimationFrames()
    })

    expect(latest.at(-1)?.content).toBe("done")
    expect(latest).toHaveLength(2)
  })

  test("flushes the last durable RAF frame before a pending stream end", async () => {
    let latest: Message[] = []
    render(<Harness onMessages={(messages) => { latest = messages }} />)

    await act(async () => {
      mocks.pending.get("get_session_stream_state")?.({
        active: true,
        lastSeq: 0,
        acceptedSeq: 0,
        durableSeq: 0,
        committedSeq: 0,
        persistenceRunId: "run-pending",
        streamId: "stream-pending",
        turnId: "turn-pending",
      })
      mocks.pending.get("get_session_stream_snapshot")?.({
        sessionId: "s1",
        streamId: "stream-pending",
        turnId: "turn-pending",
        persistenceRunId: "run-pending",
        throughSeq: 0,
        durableSeq: 0,
        committedSeq: 0,
        status: "running",
        events: [],
      })
      await Promise.resolve()
      await Promise.resolve()
    })

    const reloadsBeforeEnd = mocks.reload.mock.calls.length
    await act(async () => {
      mocks.listeners.get("chat:stream_delta")?.({
        sessionId: "s1",
        streamId: "stream-pending",
        seq: 1,
        event: JSON.stringify({ type: "text_delta", content: "durable tail" }),
      })
      // Do not run RAF callbacks: the end handler itself must drain the frame.
      mocks.listeners.get("chat:stream_end")?.({
        sessionId: "s1",
        streamId: "stream-pending",
        turnId: "turn-pending",
        status: "failed",
        finalSeq: 1,
        durableSeq: 1,
        persistenceStatus: "pending",
      })
    })

    expect(latest.at(-1)?.role).toBe("assistant")
    expect(latest.at(-1)?.content).toBe("durable tail")
    expect(mocks.reload).toHaveBeenCalledTimes(reloadsBeforeEnd)
    expect(rafCallbacks.size).toBe(0)
  })

  test("does not let a stale snapshot revive a stream that ended during the handshake", async () => {
    let latest: Message[] = []
    render(<Harness onMessages={(messages) => { latest = messages }} />)

    await act(async () => {
      // Let the staged DB baseline finish, while the state/snapshot calls are
      // intentionally still unresolved.
      await Promise.resolve()
      mocks.listeners.get("chat:stream_delta")?.({
        sessionId: "s1",
        streamId: "stream-race",
        seq: 1,
        event: JSON.stringify({ type: "text_delta", content: "safe tail" }),
      })
      mocks.listeners.get("chat:stream_end")?.({
        sessionId: "s1",
        streamId: "stream-race",
        turnId: "turn-race",
        status: "failed",
        finalSeq: 1,
        durableSeq: 1,
        persistenceStatus: "pending",
      })

      // These responses describe the pre-end state. They must be ignored,
      // including the snapshot's empty live placeholder.
      mocks.pending.get("get_session_stream_state")?.({
        active: true,
        lastSeq: 1,
        acceptedSeq: 1,
        durableSeq: 1,
        committedSeq: 0,
        persistenceRunId: "run-race",
        streamId: "stream-race",
        turnId: "turn-race",
      })
      mocks.pending.get("get_session_stream_snapshot")?.({
        sessionId: "s1",
        streamId: "stream-race",
        turnId: "turn-race",
        persistenceRunId: "run-race",
        throughSeq: 0,
        durableSeq: 0,
        committedSeq: 0,
        status: "running",
        events: [],
      })
      await Promise.resolve()
      await Promise.resolve()
      flushAnimationFrames()
    })

    expect(latest).toHaveLength(2)
    expect(latest[0]?.content).toBe("question")
    expect(latest[1]?.content).toBe("safe tail")
  })
})

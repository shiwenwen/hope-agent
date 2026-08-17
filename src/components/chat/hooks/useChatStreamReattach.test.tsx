// @vitest-environment jsdom

import { useEffect, useRef, useState } from "react"
import type { MutableRefObject } from "react"
import { act, cleanup, render } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

import type { ChatTurnInterruptReason, ChatTurnStatus, Message } from "@/types/chat"
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
      call: vi.fn(
        (name: string) =>
          new Promise((resolve) => {
            pending.set(name, resolve)
          }),
      ),
    },
    reload: vi.fn(
      async (params: {
        sessionId: string
        sessionCacheRef: MutableRefObject<Map<string, Message[]>>
        setMessages: (messages: Message[]) => void
      }) => {
        const next = mocks.dbMessages.map((message) => ({ ...message }))
        params.sessionCacheRef.current.set(params.sessionId, next)
        params.setMessages(next)
        return true
      },
    ),
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

function Harness({
  onMessages,
  onTurnStarted,
  onTurnEnded,
  expectedTurnId,
  initialMessages = [],
}: {
  onMessages: (messages: Message[]) => void
  initialMessages?: Message[]
  onTurnStarted?: (sessionId: string, turnId: string) => void
  onTurnEnded?: (
    sessionId: string,
    status?: ChatTurnStatus | null,
    interruptReason?: ChatTurnInterruptReason | null,
    turnId?: string | null,
  ) => boolean
  expectedTurnId?: string | null
}) {
  const [messages, setMessages] = useState<Message[]>(initialMessages)
  const [, setLoading] = useState(false)
  const [, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const currentSessionIdRef = useRef<string | null>("s1")
  const lastSeqRef = useRef(new Map<string, number>())
  const endedStreamIdsRef = useRef(new Map<string, Set<string>>())
  const loadingSessionsRef = useRef(new Set<string>())
  const sessionCacheRef = useRef(
    new Map<string, Message[]>(initialMessages.length > 0 ? [["s1", initialMessages]] : []),
  )

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
    onTurnStarted,
    onTurnEnded,
    expectedTurnId,
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
  test("fails closed for stream generations outside the expected turn", async () => {
    let latest: Message[] = []
    render(
      <Harness
        expectedTurnId="turn-exact"
        onMessages={(messages) => {
          latest = messages
        }}
      />,
    )
    const emit = mocks.listeners.get("chat:stream_delta")

    await act(async () => {
      emit?.({
        sessionId: "s1",
        streamId: "stream-other",
        seq: 1,
        event: JSON.stringify({ type: "text_delta", content: "wrong" }),
      })
      emit?.({
        sessionId: "s1",
        streamId: "stream-exact",
        seq: 1,
        event: JSON.stringify({ type: "text_delta", content: "right" }),
      })
      mocks.pending.get("get_session_stream_state")?.({
        active: true,
        lastSeq: 1,
        acceptedSeq: 1,
        durableSeq: 0,
        committedSeq: 0,
        streamId: "stream-exact",
        turnId: "turn-exact",
      })
      mocks.pending.get("get_session_stream_snapshot")?.(null)
      await Promise.resolve()
      await Promise.resolve()
      flushAnimationFrames()
    })

    expect(latest.some((message) => String(message.content).includes("wrong"))).toBe(false)
    expect(latest.at(-1)?.content).toBe("right")
  })

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
    render(
      <Harness
        onMessages={(messages) => {
          latest = messages
        }}
      />,
    )
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

  test("replays buffered deltas from a newer stream even below the old throughSeq", async () => {
    let latest: Message[] = []
    render(
      <Harness
        onMessages={(messages) => {
          latest = messages
        }}
      />,
    )
    const emit = mocks.listeners.get("chat:stream_delta")
    expect(emit).toBeTruthy()

    await act(async () => {
      emit?.({
        sessionId: "s1",
        streamId: "stream-new",
        seq: 1,
        event: JSON.stringify({ type: "text_delta", content: "new first token" }),
      })
      mocks.pending.get("get_session_stream_state")?.({
        active: true,
        lastSeq: 7,
        acceptedSeq: 7,
        durableSeq: 7,
        committedSeq: 0,
        persistenceRunId: "run-old",
        streamId: "stream-old",
        turnId: "turn-old",
      })
      mocks.pending.get("get_session_stream_snapshot")?.({
        sessionId: "s1",
        streamId: "stream-old",
        turnId: "turn-old",
        persistenceRunId: "run-old",
        throughSeq: 7,
        durableSeq: 7,
        committedSeq: 0,
        status: "running",
        events: [
          { seq: 7, event: JSON.stringify({ type: "text_delta", content: "old durable; " }) },
        ],
      })
      await Promise.resolve()
      await Promise.resolve()
      flushAnimationFrames()
    })

    expect(latest.at(-1)?.content).toBe("old durable; new first token")
  })

  test("does not let an older active snapshot replace a live newer turn", async () => {
    let activeTurnId: string | null = null
    const endedTurns: string[] = []
    render(
      <Harness
        onMessages={() => {}}
        onTurnStarted={(_sid, turnId) => {
          activeTurnId = turnId
        }}
        onTurnEnded={(_sid, _status, _reason, turnId) => {
          if (turnId && activeTurnId && turnId !== activeTurnId) return false
          if (turnId) endedTurns.push(turnId)
          activeTurnId = null
          return true
        }}
      />,
    )

    await act(async () => {
      mocks.listeners.get("chat:turn_started")?.({ sessionId: "s1", turnId: "turn-new" })
      mocks.pending.get("get_session_stream_state")?.({
        active: true,
        lastSeq: 7,
        acceptedSeq: 7,
        durableSeq: 7,
        committedSeq: 0,
        persistenceRunId: "run-old",
        streamId: "stream-old",
        turnId: "turn-old",
      })
      mocks.pending.get("get_session_stream_snapshot")?.({
        sessionId: "s1",
        streamId: "stream-old",
        turnId: "turn-old",
        persistenceRunId: "run-old",
        throughSeq: 7,
        durableSeq: 7,
        committedSeq: 0,
        status: "running",
        events: [],
      })
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(activeTurnId).toBe("turn-new")

    await act(async () => {
      mocks.listeners.get("chat:stream_end")?.({
        sessionId: "s1",
        streamId: "stream-new",
        turnId: "turn-new",
        status: "completed",
        persistenceStatus: "committed",
      })
    })

    expect(activeTurnId).toBeNull()
    expect(endedTurns).toContain("turn-new")
  })

  test("replays only the live turn stream when a stale snapshot handshake mixes generations", async () => {
    let latest: Message[] = []
    render(
      <Harness
        initialMessages={[
          { role: "user", content: "new question" },
          { role: "assistant", content: "", _clientId: "new-turn-placeholder" },
        ]}
        onMessages={(messages) => {
          latest = messages
        }}
      />,
    )

    await act(async () => {
      const emitDelta = mocks.listeners.get("chat:stream_delta")
      emitDelta?.({
        sessionId: "s1",
        streamId: "stream-old",
        seq: 8,
        event: JSON.stringify({ type: "text_delta", content: "stale before; " }),
      })
      mocks.listeners.get("chat:turn_started")?.({
        sessionId: "s1",
        turnId: "turn-new",
        streamId: "stream-new",
      })
      emitDelta?.({
        sessionId: "s1",
        streamId: "stream-old",
        seq: 9,
        event: JSON.stringify({ type: "text_delta", content: "stale after; " }),
      })
      emitDelta?.({
        sessionId: "s1",
        streamId: "stream-new",
        seq: 1,
        event: JSON.stringify({ type: "text_delta", content: "new reply" }),
      })
      mocks.pending.get("get_session_stream_state")?.({
        active: true,
        lastSeq: 9,
        acceptedSeq: 9,
        durableSeq: 9,
        committedSeq: 0,
        persistenceRunId: "run-old",
        streamId: "stream-old",
        turnId: "turn-old",
      })
      mocks.pending.get("get_session_stream_snapshot")?.({
        sessionId: "s1",
        streamId: "stream-old",
        turnId: "turn-old",
        persistenceRunId: "run-old",
        throughSeq: 9,
        durableSeq: 9,
        committedSeq: 0,
        status: "running",
        events: [],
      })
      await Promise.resolve()
      await Promise.resolve()
      flushAnimationFrames()
    })

    expect(latest.at(-1)?.role).toBe("assistant")
    expect(latest.at(-1)?.content).toBe("new reply")
  })

  test("ignores an older terminal state after a live newer turn starts", async () => {
    let activeTurnId: string | null = null
    render(
      <Harness
        onMessages={() => {}}
        onTurnStarted={(_sid, turnId) => {
          activeTurnId = turnId
        }}
        onTurnEnded={(_sid, _status, _reason, turnId) => {
          if (turnId && activeTurnId && turnId !== activeTurnId) return false
          activeTurnId = null
          return true
        }}
      />,
    )

    await act(async () => {
      mocks.listeners.get("chat:turn_started")?.({ sessionId: "s1", turnId: "turn-new" })
      mocks.pending.get("get_session_stream_state")?.({
        active: false,
        lastSeq: 7,
        acceptedSeq: 7,
        durableSeq: 7,
        committedSeq: 7,
        persistenceRunId: "run-old",
        streamId: "stream-old",
        turnId: null,
        lastTerminalStatus: "interrupted",
      })
      mocks.pending.get("get_session_stream_snapshot")?.(null)
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(activeTurnId).toBe("turn-new")
  })

  test("does not replay a committed journal over canonical DB messages", async () => {
    mocks.dbMessages = [
      { role: "user", content: "question", dbId: 1 },
      { role: "assistant", content: "done", dbId: 2 },
    ]
    let latest: Message[] = []
    render(
      <Harness
        onMessages={(messages) => {
          latest = messages
        }}
      />,
    )

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
        events: [{ seq: 1, event: JSON.stringify({ type: "text_delta", content: "done" }) }],
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
    render(
      <Harness
        onMessages={(messages) => {
          latest = messages
        }}
      />,
    )

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
    render(
      <Harness
        onMessages={(messages) => {
          latest = messages
        }}
      />,
    )

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

  test("ignores a delayed stream end from an older turn", async () => {
    let latest: Message[] = []
    render(
      <Harness
        onMessages={(messages) => {
          latest = messages
        }}
        onTurnEnded={(_sid, _status, _reason, turnId) => turnId === "turn-new"}
      />,
    )

    await act(async () => {
      mocks.pending.get("get_session_stream_state")?.({
        active: true,
        lastSeq: 1,
        acceptedSeq: 1,
        durableSeq: 1,
        committedSeq: 0,
        persistenceRunId: "run-new",
        streamId: "stream-new",
        turnId: "turn-new",
      })
      mocks.pending.get("get_session_stream_snapshot")?.({
        sessionId: "s1",
        streamId: "stream-new",
        turnId: "turn-new",
        persistenceRunId: "run-new",
        throughSeq: 1,
        durableSeq: 1,
        committedSeq: 0,
        status: "running",
        events: [{ seq: 1, event: JSON.stringify({ type: "text_delta", content: "new reply" }) }],
      })
      await Promise.resolve()
      await Promise.resolve()
      flushAnimationFrames()
    })

    const reloadsBeforeEnd = mocks.reload.mock.calls.length
    await act(async () => {
      // A final old delta can already be sitting in its own rAF buffer when
      // the delayed end arrives. The stale terminal must discard that buffer
      // without touching the newer stream.
      mocks.listeners.get("chat:stream_delta")?.({
        sessionId: "s1",
        streamId: "stream-old",
        seq: 2,
        event: JSON.stringify({ type: "text_delta", content: "old buffered tail" }),
      })
      mocks.listeners.get("chat:stream_end")?.({
        sessionId: "s1",
        streamId: "stream-old",
        turnId: "turn-old",
        status: "interrupted",
        persistenceStatus: "committed",
      })
      // Frames that arrive after the old end are quarantined as well.
      mocks.listeners.get("chat:stream_delta")?.({
        sessionId: "s1",
        streamId: "stream-old",
        seq: 3,
        event: JSON.stringify({ type: "text_delta", content: "old late tail" }),
      })
      flushAnimationFrames()
    })

    expect(latest.at(-1)?.content).toBe("new reply")
    expect(mocks.reload).toHaveBeenCalledTimes(reloadsBeforeEnd)
  })
})

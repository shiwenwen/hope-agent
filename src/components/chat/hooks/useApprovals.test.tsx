// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import { TRANSPORT_EVENT_RESYNC_REQUIRED, type Transport } from "@/lib/transport"
import { setTransport } from "@/lib/transport-provider"
import type { ApprovalRequest } from "@/components/chat/ApprovalDialog"
import { useApprovals } from "./useApprovals"

const toastMock = vi.hoisted(() => ({
  error: vi.fn(),
  info: vi.fn(),
}))

vi.mock("sonner", () => ({ toast: toastMock }))

function approval(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    request_id: "approval-1",
    session_id: "session-1",
    command: "rm important-file",
    cwd: "/tmp",
    created_at_ms: Date.now(),
    server_now_ms: Date.now(),
    timeout_secs: 0,
    timeout_action: "deny",
    ...overrides,
  }
}

function mockTransport(options: {
  snapshot: () => Promise<ApprovalRequest[]>
  respond?: () => Promise<unknown>
}): { transport: Transport; emit: (name: string, payload: unknown) => void } {
  const listeners = new Map<string, Set<(payload: unknown) => void>>()
  const transport = {
    call: vi.fn((command: string) => {
      if (command === "list_pending_approvals") return options.snapshot()
      if (command === "respond_to_approval") return options.respond?.() ?? Promise.resolve(null)
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
  vi.clearAllMocks()
})

describe("useApprovals recovery lifecycle", () => {
  test("restores a missed approval from the authoritative snapshot", async () => {
    const request = approval()
    const mock = mockTransport({ snapshot: () => Promise.resolve([request]) })
    setTransport(mock.transport)

    const { result } = renderHook(() => useApprovals("session-1"))
    await act(async () => {
      await Promise.resolve()
    })

    expect(result.current.approvalRequests).toEqual([{ ...request, local_timeout_at_ms: null }])

    await act(async () => {
      mock.emit(TRANSPORT_EVENT_RESYNC_REQUIRED, { reason: "reconnected" })
      await Promise.resolve()
    })
    expect(result.current.approvalRequests).toHaveLength(1)
  })

  test("a stale snapshot cannot revive an approval resolved while it was loading", async () => {
    let resolveSnapshot!: (requests: ApprovalRequest[]) => void
    const snapshot = new Promise<ApprovalRequest[]>((resolve) => {
      resolveSnapshot = resolve
    })
    const mock = mockTransport({ snapshot: () => snapshot })
    setTransport(mock.transport)
    const request = approval()
    const { result } = renderHook(() => useApprovals("session-1"))

    act(() => mock.emit("approval_required", request))
    act(() => {
      mock.emit("approval:resolved", {
        requestId: request.request_id,
        sessionId: request.session_id,
        source: "http",
      })
    })
    expect(result.current.approvalRequests).toEqual([])

    await act(async () => {
      resolveSnapshot([request])
      await snapshot
    })
    expect(result.current.approvalRequests).toEqual([])
  })

  test("merges an older missed approval with a concurrent required event", async () => {
    let resolveSnapshot!: (requests: ApprovalRequest[]) => void
    const snapshot = new Promise<ApprovalRequest[]>((resolve) => {
      resolveSnapshot = resolve
    })
    const mock = mockTransport({ snapshot: () => snapshot })
    setTransport(mock.transport)
    const missed = approval({ request_id: "approval-a", created_at_ms: 1_000 })
    const concurrent = approval({ request_id: "approval-b", created_at_ms: 2_000 })
    const { result } = renderHook(() => useApprovals("session-1"))

    act(() => mock.emit("approval_required", concurrent))
    expect(result.current.approvalRequests.map((request) => request.request_id)).toEqual([
      "approval-b",
    ])

    await act(async () => {
      resolveSnapshot([missed])
      await snapshot
    })
    expect(result.current.approvalRequests.map((request) => request.request_id)).toEqual([
      "approval-a",
      "approval-b",
    ])
  })

  test("keeps a still-pending approval actionable when submit fails", async () => {
    const request = approval()
    const mock = mockTransport({
      snapshot: () => Promise.resolve([request]),
      respond: () => Promise.reject(new Error("network unavailable")),
    })
    setTransport(mock.transport)
    const { result } = renderHook(() => useApprovals("session-1"))
    await act(async () => {
      await Promise.resolve()
    })

    await act(async () => {
      await result.current.handleApprovalResponse(request.request_id, "allow_once")
    })

    expect(result.current.approvalRequests).toEqual([{ ...request, local_timeout_at_ms: null }])
    expect(toastMock.error).toHaveBeenCalled()
  })

  test("reconciles an ambiguous failed response that backend already accepted", async () => {
    const request = approval()
    let pending = [request]
    const mock = mockTransport({
      snapshot: () => Promise.resolve(pending),
      respond: () => {
        pending = []
        return Promise.reject(new Error("response lost"))
      },
    })
    setTransport(mock.transport)
    const { result } = renderHook(() => useApprovals("session-1"))
    await act(async () => {
      await Promise.resolve()
    })

    await act(async () => {
      await result.current.handleApprovalResponse(request.request_id, "deny")
    })
    expect(result.current.approvalRequests).toEqual([])
  })

  test("clears at the absolute deadline even when timeout events are lost", async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"))
    const request = approval({
      created_at_ms: Date.now(),
      server_now_ms: Date.now(),
      timeout_at_ms: Date.now() + 1_000,
      timeout_secs: 1,
    })
    const mock = mockTransport({
      snapshot: () => Promise.resolve([{ ...request, server_now_ms: Date.now() }]),
    })
    setTransport(mock.transport)
    const { result } = renderHook(() => useApprovals("session-1"))
    await act(async () => {
      await Promise.resolve()
    })
    expect(result.current.approvalRequests).toHaveLength(1)

    await act(async () => {
      vi.advanceTimersByTime(1_025)
      await Promise.resolve()
    })
    expect(result.current.approvalRequests).toEqual([])
  })
})

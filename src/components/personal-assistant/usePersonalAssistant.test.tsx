// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { usePersonalAssistant } from "./usePersonalAssistant"

const fixture = vi.hoisted(() => ({
  revision: 1,
  call: vi.fn(),
  listeners: new Map<string, (payload?: unknown) => void>(),
  listen: vi.fn((event: string, callback: (payload?: unknown) => void) => {
    fixture.listeners.set(event, callback)
    return () => fixture.listeners.delete(event)
  }),
}))
vi.mock("@/lib/transport-provider", () => ({
  useTransport: () => fixture,
  useTransportRevision: () => fixture.revision,
  getTransportRevision: () => fixture.revision,
}))
const watch = {
  sessionId: "work-a",
  goalId: "goal-a",
  objective: "Private release",
  state: "blocked",
  needsAttention: true,
  reason: "blocked",
  eventId: 4,
}
const request = {
  id: "request-a",
  targetSessionId: "work-a",
  instruction: "Private instruction",
  status: "running",
}

afterEach(cleanup)
beforeEach(() => {
  fixture.revision = 1
  fixture.listeners.clear()
  fixture.call.mockReset().mockImplementation(async (command: string) => {
    if (command === "list_personal_assistant_requests") return [request]
    if (command === "personal_assistant_inbox") return [watch]
    if (command === "ensure_personal_assistant_home")
      return { id: "private-home", kind: "personal_assistant" }
  })
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" })
})

describe("personal assistant owner projection", () => {
  it("refreshes waiting interactions only for a currently delegated target", async () => {
    const { result } = renderHook(usePersonalAssistant)
    await waitFor(() => expect(result.current.requests).toEqual([request]))
    const before = fixture.call.mock.calls.length
    act(() => fixture.listeners.get("session_pending_interactions_changed")?.({ sessionId: "unrelated" }))
    expect(fixture.call.mock.calls).toHaveLength(before)
    fixture.call.mockImplementation(async command => command === "list_personal_assistant_requests" ? [{ ...request, pendingInteractionCount: 1 }] : [watch])
    act(() => fixture.listeners.get("session_pending_interactions_changed")?.({ sessionId: "work-a" }))
    await waitFor(() => expect(result.current.requests[0].pendingInteractionCount).toBe(1))
  })

  it("removes private rows after a failed read and waits for an explicit verification before retrying", async () => {
    const { result } = renderHook(usePersonalAssistant)
    await waitFor(() => expect(result.current.watches).toEqual([watch]))
    fixture.call.mockRejectedValue(new Error("Owner access revoked"))
    act(() => fixture.listeners.get("personal_assistant:changed")?.())
    await waitFor(() => expect(result.current.error).toContain("Owner access revoked"))
    expect(result.current.requests).toEqual([])
    expect(result.current.watches).toEqual([])
    expect(result.current.authorized).toBe(false)
    const attempts = fixture.call.mock.calls.length
    act(() => fixture.listeners.get("personal_assistant:changed")?.())
    expect(fixture.call.mock.calls).toHaveLength(attempts)
    fixture.call.mockResolvedValue([])
    act(() => fixture.listeners.get(TRANSPORT_EVENT_RESYNC_REQUIRED)?.())
    await waitFor(() => expect(result.current.authorized).toBe(true))
    expect(result.current.error).toBeNull()
  })

  it("discards a previous credential scope's delayed private result", async () => {
    let resolveOld!: (value: unknown) => void
    fixture.call.mockImplementation((command: string) =>
      command === "personal_assistant_inbox"
        ? new Promise((resolve) => {
            resolveOld = resolve
          })
        : Promise.resolve([request]),
    )
    const { result, rerender } = renderHook(usePersonalAssistant)
    fixture.call.mockResolvedValue([])
    fixture.revision = 2
    rerender()
    await waitFor(() => expect(result.current.authorized).toBe(true))
    await act(async () => {
      resolveOld([watch])
    })
    expect(result.current.watches).toEqual([])
    expect(result.current.requests).toEqual([])
  })

  it("refreshes after actual follow and cancellation commands without claiming completion", async () => {
    const { result } = renderHook(usePersonalAssistant)
    await waitFor(() => expect(result.current.authorized).toBe(true))
    await act(async () => result.current.watchGoal("work-a", "goal-a", false))
    expect(fixture.call).toHaveBeenCalledWith("watch_personal_goal", {
      sessionId: "work-a",
      goalId: "goal-a",
    })
    await act(async () => result.current.cancel("request-a"))
    expect(fixture.call).toHaveBeenCalledWith("cancel_personal_assistant_request", {
      id: "request-a",
    })
    expect(result.current.requests[0].status).toBe("running")
    await act(async () => result.current.watchGoal("work-a", "goal-a", true))
    expect(fixture.call).toHaveBeenCalledWith("unwatch_personal_goal", {
      sessionId: "work-a",
      goalId: "goal-a",
    })
  })
})

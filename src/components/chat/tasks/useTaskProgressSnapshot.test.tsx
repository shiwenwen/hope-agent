// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, expect, test, vi } from "vitest"
import type { Message, Task } from "@/types/chat"
import { useTaskProgressSnapshot } from "./useTaskProgressSnapshot"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"

const transport = vi.hoisted(() => ({
  listeners: new Map<string, (raw?: unknown) => void>(),
  call: vi.fn(() => new Promise<unknown>(() => {})),
}))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call: transport.call, listen: (event: string, listener: (raw: unknown) => void) => {
    transport.listeners.set(event, listener)
    return () => { transport.listeners.delete(event) }
  } }),
}))
afterEach(() => {
  cleanup()
  transport.call.mockReset().mockImplementation(() => new Promise<unknown>(() => {}))
})

function task(id: number, sessionId: string): Task {
  return { id, sessionId, content: sessionId, status: "pending", activeForm: null,
    batchId: null, createdAt: "2026-08-01T00:00:00Z", updatedAt: "2026-08-01T00:00:00Z" }
}

test("side progress ignores parent snapshots/events and clears after final deletion", () => {
  const parent = task(1, "main")
  const side = task(2, "side")
  const messages: Message[] = [{ role: "assistant", content: "", toolCalls: [
    { callId: "parent-task", name: "task_create", arguments: "{}", result: JSON.stringify([parent]) },
  ] }]
  const { result, rerender } = renderHook(({ id, rows }) => useTaskProgressSnapshot(id, rows), {
    initialProps: { id: "side", rows: messages },
  })
  expect(result.current).toBeNull()
  act(() => { transport.listeners.get("task_updated")?.({ sessionId: "main", tasks: [parent] }) })
  expect(result.current).toBeNull()
  act(() => { transport.listeners.get("task_updated")?.({ sessionId: "side", tasks: [side] }) })
  expect(result.current?.tasks).toEqual([side])
  const sideMessages: Message[] = [...messages, { role: "assistant", content: "", toolCalls: [
    { callId: "side-task", name: "task_create", arguments: "{}", result: JSON.stringify([side]) },
  ] }]
  rerender({ id: "side", rows: sideMessages })
  act(() => { transport.listeners.get("task_updated")?.({ sessionId: "side", tasks: [] }) })
  expect(result.current?.tasks).toEqual([])
  rerender({ id: "main", rows: messages })
  expect(result.current?.tasks).toEqual([parent])
})

test("reopen and reconnect restore the task ledger instead of stale transcript rows", async () => {
  const side = task(2, "side")
  const messages: Message[] = [{ role: "assistant", content: "", toolCalls: [
    { callId: "side-task", name: "task_create", arguments: "{}", result: JSON.stringify([side]) },
  ] }]
  transport.call.mockResolvedValue([])
  const first = renderHook(() => useTaskProgressSnapshot("side", messages))
  await waitFor(() => expect(first.result.current?.tasks).toEqual([]))
  first.unmount()
  const second = renderHook(() => useTaskProgressSnapshot("side", messages))
  await waitFor(() => expect(second.result.current?.tasks).toEqual([]))
  expect(transport.call).toHaveBeenLastCalledWith("list_session_tasks", { sessionId: "side" })
  transport.call.mockResolvedValue([{ ...side, status: "completed" }])
  act(() => { transport.listeners.get(TRANSPORT_EVENT_RESYNC_REQUIRED)?.() })
  await waitFor(() => expect(second.result.current?.completed).toBe(1))
})

test("late seed results cannot replace live updates or a new session", async () => {
  const pending: Array<(tasks: unknown) => void> = []
  transport.call.mockImplementation(() => new Promise((resolve) => { pending.push(resolve) }))
  const { result, rerender } = renderHook(({ id }) => useTaskProgressSnapshot(id, []), {
    initialProps: { id: "main" },
  })
  const live = { ...task(1, "main"), status: "completed" }
  act(() => { transport.listeners.get("task_updated")?.({ sessionId: "main", tasks: [live] }) })
  await act(async () => { pending[0]([task(1, "main")]) })
  expect(result.current?.completed).toBe(1)
  act(() => { transport.listeners.get(TRANSPORT_EVENT_RESYNC_REQUIRED)?.() })
  rerender({ id: "side" })
  await act(async () => { pending[2]([task(2, "side")]) })
  await act(async () => { pending[1]([task(1, "main")]) })
  expect(result.current?.tasks[0].sessionId).toBe("side")
})

test("deleting the newest task keeps the older remaining ledger authoritative", () => {
  const remaining = task(2, "side")
  const removed = { ...task(3, "side"), updatedAt: "2026-08-02T00:00:00Z" }
  const messages: Message[] = [{ role: "assistant", content: "", toolCalls: [
    { callId: "side-tasks", name: "task_create", arguments: "{}", result: JSON.stringify([remaining, removed]) },
  ] }]
  const { result } = renderHook(() => useTaskProgressSnapshot("side", messages))
  act(() => { transport.listeners.get("task_updated")?.({ sessionId: "side", tasks: [remaining] }) })
  expect(result.current?.tasks).toEqual([remaining])
})

// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, expect, test, vi } from "vitest"
import type { Message, Task } from "@/types/chat"
import { useTaskProgressSnapshot } from "./useTaskProgressSnapshot"

const transport = vi.hoisted(() => ({ listener: null as ((raw: unknown) => void) | null }))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ listen: (_event: string, listener: (raw: unknown) => void) => {
    transport.listener = listener
    return () => { transport.listener = null }
  } }),
}))
afterEach(cleanup)

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
  act(() => { transport.listener?.({ sessionId: "main", tasks: [parent] }) })
  expect(result.current).toBeNull()
  act(() => { transport.listener?.({ sessionId: "side", tasks: [side] }) })
  expect(result.current?.tasks).toEqual([side])
  const sideMessages: Message[] = [...messages, { role: "assistant", content: "", toolCalls: [
    { callId: "side-task", name: "task_create", arguments: "{}", result: JSON.stringify([side]) },
  ] }]
  rerender({ id: "side", rows: sideMessages })
  act(() => { transport.listener?.({ sessionId: "side", tasks: [] }) })
  expect(result.current?.tasks).toEqual([])
  rerender({ id: "main", rows: messages })
  expect(result.current?.tasks).toEqual([parent])
})

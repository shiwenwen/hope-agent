// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, expect, test, vi } from "vitest"

import type { BackgroundJobSnapshot } from "@/types/background-jobs"
import { resolveSideChatSurfaceSessionId } from "../sideChatSurface"
import { useBackgroundJobs } from "./useBackgroundJobs"

const transport = vi.hoisted(() => ({
  call: vi.fn(),
  listeners: new Map<string, (raw: unknown) => void>(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    call: transport.call,
    listen: (event: string, handler: (raw: unknown) => void) => {
      transport.listeners.set(event, handler)
      return () => { transport.listeners.delete(event) }
    },
  }),
}))

afterEach(() => {
  cleanup()
  transport.call.mockReset()
  transport.listeners.clear()
  vi.useRealTimers()
})

function job(sessionId: string): BackgroundJobSnapshot {
  return { jobId: `job-${sessionId}`, sessionId, status: "running", kind: "tool" } as BackgroundJobSnapshot
}

test("active side conversation jobs replace the owner snapshot and restore on close", async () => {
  transport.call.mockImplementation((_command, { sessionId }: { sessionId: string }) =>
    Promise.resolve([job(sessionId)]),
  )
  const { result, rerender } = renderHook(
    ({ open }) => useBackgroundJobs(resolveSideChatSurfaceSessionId("main", "side", open)),
    { initialProps: { open: false } },
  )
  await waitFor(() => expect(result.current.jobs[0]?.sessionId).toBe("main"))
  rerender({ open: true })
  expect(result.current.jobs).toEqual([])
  await waitFor(() => expect(result.current.jobs[0]?.sessionId).toBe("side"))
  expect(result.current.runningCount).toBe(1)
  vi.useFakeTimers()
  const calls = transport.call.mock.calls.length
  act(() => { transport.listeners.get("job:created")?.({ session_id: "main" }) })
  await act(async () => { await vi.advanceTimersByTimeAsync(200) })
  expect(transport.call).toHaveBeenCalledTimes(calls)
  act(() => { transport.listeners.get("job:created")?.({ session_id: "side" }) })
  await act(async () => { await vi.advanceTimersByTimeAsync(200) })
  expect(transport.call).toHaveBeenLastCalledWith("list_background_jobs", { sessionId: "side" })
  expect(transport.call).toHaveBeenCalledTimes(calls + 1)
  vi.useRealTimers()
  rerender({ open: false })
  await waitFor(() => expect(result.current.jobs[0]?.sessionId).toBe("main"))
})

test("a late main-session response cannot replace the side-session jobs", async () => {
  let finishMain!: (rows: BackgroundJobSnapshot[]) => void
  transport.call.mockImplementation((_command, { sessionId }: { sessionId: string }) =>
    sessionId === "main"
      ? new Promise<BackgroundJobSnapshot[]>((resolve) => { finishMain = resolve })
      : Promise.resolve([job(sessionId)]),
  )
  const { result, rerender } = renderHook(({ id }) => useBackgroundJobs(id), {
    initialProps: { id: "main" },
  })
  rerender({ id: "side" })
  await waitFor(() => expect(result.current.jobs[0]?.sessionId).toBe("side"))
  await act(async () => { finishMain([job("main")]) })
  expect(result.current.jobs[0]?.sessionId).toBe("side")
})

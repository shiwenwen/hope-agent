// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, expect, test, vi } from "vitest"
import { isMacControlToolFrameForSession, useMacControlFrame, type MacControlFramePayload } from "./useMacControlFrame"

const transport = vi.hoisted(() => ({
  listener: null as ((raw: unknown) => void) | null,
  call: vi.fn().mockResolvedValue({ frame: null }),
}))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call: transport.call, listen: (_event: string, listener: (raw: unknown) => void) => {
    transport.listener = listener
    return () => { transport.listener = null }
  } }),
}))
afterEach(cleanup)

function frame(sessionId?: string): MacControlFramePayload {
  return { sessionId, snapshotId: "snapshot", actionId: "action", jpegBase64: "jpeg",
    capturedAt: 1, widthPx: 10, heightPx: 10 }
}

test("only the visible owner can open the panel for a tool frame", () => {
  expect(isMacControlToolFrameForSession(frame("side"), "main")).toBe(false)
  expect(isMacControlToolFrameForSession(frame("side"), "other-side")).toBe(false)
  expect(isMacControlToolFrameForSession(frame("side"), "side")).toBe(true)
  expect(isMacControlToolFrameForSession({ mediaId: "shot" }, "side")).toBe(false)
  expect(isMacControlToolFrameForSession({}, "side")).toBe(false)
})

test("session switches clear frames and reject background side events", () => {
  const { result, rerender } = renderHook(({ id }) => useMacControlFrame({
    sessionId: id, pollKey: "test", pollActive: false,
  }), { initialProps: { id: "main" } })
  act(() => { transport.listener?.(frame("side")) })
  expect(result.current.frame).toBeNull()
  act(() => { transport.listener?.(frame("main")) })
  expect(result.current.frame?.sessionId).toBe("main")
  rerender({ id: "side" })
  expect(result.current.frame).toBeNull()
  act(() => { transport.listener?.(frame("main")) })
  expect(result.current.frame).toBeNull()
  act(() => { transport.listener?.(frame("side")) })
  expect(result.current.frame?.sessionId).toBe("side")
  act(() => { transport.listener?.({ ...frame(), path: "unowned-tool.jpg" }) })
  expect(result.current.frame?.sessionId).toBe("side")
  act(() => { transport.listener?.({ ...frame(), actionId: null }) })
  expect(result.current.frame?.actionId).toBeNull()
})

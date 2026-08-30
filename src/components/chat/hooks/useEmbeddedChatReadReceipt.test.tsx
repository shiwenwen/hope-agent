// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

import type { Message } from "@/types/chat"
import { useEmbeddedChatReadReceipt } from "./useEmbeddedChatReadReceipt"

const mocks = vi.hoisted(() => ({
  call: vi.fn(() => Promise.resolve()),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call: mocks.call }),
}))

vi.mock("@/hooks/useReadableSurface", () => ({
  useReadableSurface: (visible: boolean) => visible,
}))

let nextAnimationFrameId = 1
let animationFrames = new Map<number, FrameRequestCallback>()

function flushAnimationFrame(): void {
  const callbacks = [...animationFrames.values()]
  animationFrames.clear()
  callbacks.forEach((callback) => callback(performance.now()))
}

function flushReadReceiptFrames(): void {
  act(() => {
    flushAnimationFrame()
    flushAnimationFrame()
  })
}

beforeEach(() => {
  mocks.call.mockReset().mockResolvedValue(undefined)
  nextAnimationFrameId = 1
  animationFrames = new Map()
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = nextAnimationFrameId++
    animationFrames.set(id, callback)
    return id
  })
  vi.stubGlobal("cancelAnimationFrame", (id: number) => {
    animationFrames.delete(id)
  })
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("useEmbeddedChatReadReceipt", () => {
  test.each([[false, true], [true, false]])(
    "does not acknowledge hidden or scrolled-away transcripts (%s/%s)",
    async (surfaceVisible, tailVisible) => {
      const onRead = vi.fn()
      let finishRead!: () => void
      mocks.call.mockReturnValue(new Promise<void>((resolve) => { finishRead = resolve }))
      const messages: Message[] = [{ role: "assistant", content: "reply", dbId: 52 }]
      const { rerender } = renderHook(({ visible, tail }) =>
        useEmbeddedChatReadReceipt(visible, tail, "side", messages, onRead),
      { initialProps: { visible: surfaceVisible, tail: tailVisible } })
      flushReadReceiptFrames()
      expect(mocks.call).not.toHaveBeenCalled()
      expect(onRead).not.toHaveBeenCalled()
      rerender({ visible: true, tail: true })
      flushReadReceiptFrames()
      expect(onRead).not.toHaveBeenCalled()
      await act(async () => { finishRead() })
      expect(onRead).toHaveBeenCalledWith({ sessionId: "side", throughMessageId: 52 })
    },
  )

  test("does not notify the tray if the durable read fails", async () => {
    const onRead = vi.fn()
    mocks.call.mockRejectedValue(new Error("offline"))
    renderHook(() => useEmbeddedChatReadReceipt(true, true, "side", [
      { role: "assistant", content: "reply", dbId: 52 },
    ], onRead))
    flushReadReceiptFrames()
    await act(async () => { await Promise.resolve() })
    expect(onRead).not.toHaveBeenCalled()
  })

  test("does not attribute the previous transcript to a newly selected session", () => {
    const previousMessages: Message[] = [{ role: "assistant", content: "old reply", dbId: 41 }]
    const targetMessages: Message[] = [{ role: "assistant", content: "new reply", dbId: 52 }]
    const { rerender } = renderHook(
      ({ sessionId, messages }: { sessionId: string; messages: Message[] }) =>
        useEmbeddedChatReadReceipt(true, true, sessionId, messages),
      { initialProps: { sessionId: "session-a", messages: previousMessages } },
    )

    flushReadReceiptFrames()
    expect(mocks.call).toHaveBeenLastCalledWith("mark_session_read_cmd", {
      sessionId: "session-a",
      throughMessageId: 41,
    })
    mocks.call.mockClear()

    rerender({ sessionId: "session-b", messages: previousMessages })
    flushReadReceiptFrames()
    expect(mocks.call).not.toHaveBeenCalled()

    rerender({ sessionId: "session-b", messages: targetMessages })
    flushReadReceiptFrames()
    expect(mocks.call).toHaveBeenCalledWith("mark_session_read_cmd", {
      sessionId: "session-b",
      throughMessageId: 52,
    })
  })
})

// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import { ordinaryExecutionMayNotify, submitAssistantNotification } from "./notifications"
import type { Transport } from "./transport"
const fixture = vi.hoisted(() => ({ revision: 1, call: vi.fn() }))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => fixture,
  getTransportRevision: () => fixture.revision,
}))
beforeEach(() => {
  fixture.revision = 1
  fixture.call.mockReset()
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})
it("permits only an exact ordinary host identity and rejects a late response from another connection", async () => {
  const transport = fixture as unknown as Transport
  fixture.call.mockResolvedValue("ordinary")
  expect(await ordinaryExecutionMayNotify(transport, 1, "owner-turn")).toBe(true)
  fixture.call.mockResolvedValue("assistant")
  expect(await ordinaryExecutionMayNotify(transport, 1, "home-turn")).toBe(false)
  fixture.call.mockResolvedValue("unknown")
  expect(await ordinaryExecutionMayNotify(transport, 1, "deleted-run")).toBe(false)
  fixture.call.mockResolvedValue("ordinary")
  fixture.revision = 2
  expect(await ordinaryExecutionMayNotify(transport, 1, "late-old-root")).toBe(false)
})
it("does not equate constructor return with platform acceptance", async () => {
  vi.stubGlobal(
    "Notification",
    class {
      onshow: (() => void) | null = null
      onerror: (() => void) | null = null
    },
  )
  const submission = submitAssistantNotification("Hope", "Private", vi.fn())
  await vi.advanceTimersByTimeAsync(2000)
  expect(await submission).toBe("unknown")
})
it("records a real platform show event as accepted and a constructor failure as not submitted", async () => {
  let shown: (() => void) | undefined
  vi.stubGlobal(
    "Notification",
    class {
      set onshow(listener: () => void) {
        shown = listener
      }
    },
  )
  const submission = submitAssistantNotification("Hope", "Private", vi.fn())
  shown?.()
  expect(await submission).toBe("accepted")
  vi.stubGlobal(
    "Notification",
    class {
      constructor() {
        throw new Error("Unavailable")
      }
    },
  )
  expect(await submitAssistantNotification("Hope", "Private", vi.fn())).toBe("not_submitted")
})

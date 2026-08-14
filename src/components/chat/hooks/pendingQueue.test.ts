import { describe, expect, test } from "vitest"

import {
  canClaimOwnerlessPendingReplay,
  hasSendableChatPayload,
  nextDispatchablePending,
  shouldApplyPendingQueueSnapshot,
  shouldReplayNextPending,
} from "./pendingQueue"

describe("durable pending queue projection", () => {
  test("never applies a late snapshot to another session", () => {
    expect(shouldApplyPendingQueueSnapshot("session-b", "session-a")).toBe(false)
    expect(shouldApplyPendingQueueSnapshot("session-a", "session-a")).toBe(true)
  })

  test("dispatches only the first actionable FIFO row", () => {
    const items = [
      { id: "saving", sessionId: "s", status: "saving" as const },
      { id: "inserting", sessionId: "s", status: "inserting" as const },
      { id: "first", sessionId: "s", status: "fallback_after_reply" as const },
      { id: "second", sessionId: "s", status: "queued" as const },
    ]
    expect(nextDispatchablePending(items)?.id).toBe("first")
  })

  test("never lets the GUI claim a backend-managed Channel row", () => {
    const items = [
      {
        id: "channel-first",
        sessionId: "s",
        status: "queued" as const,
        managedBy: "channel" as const,
      },
      { id: "desktop-next", sessionId: "s", status: "queued" as const },
    ]
    expect(nextDispatchablePending(items)?.id).toBe("desktop-next")
  })

  test("allows a durable attachment-only row to reach the backend", () => {
    expect(hasSendableChatPayload("", false, false, "queued-request")).toBe(true)
    expect(hasSendableChatPayload("", false, false)).toBe(false)
  })

  test("does not replay after a user Stop from another surface", () => {
    expect(
      shouldReplayNextPending(false, {
        status: "interrupted",
        interruptReason: "user_stop",
      }),
    ).toBe(false)
    expect(
      shouldReplayNextPending(false, {
        status: "cancelling",
        interruptReason: "user_stop",
      }),
    ).toBe(false)
    expect(shouldReplayNextPending(true, { status: "completed" })).toBe(false)
    expect(
      shouldReplayNextPending(false, {
        status: "interrupted",
        interruptReason: "runtime_cancel",
      }),
    ).toBe(true)
  })

  test("claims an ownerless replay only after the current session settles", () => {
    expect(canClaimOwnerlessPendingReplay("s", "s", false, false)).toBe(true)
    expect(canClaimOwnerlessPendingReplay("other", "s", false, false)).toBe(false)
    expect(canClaimOwnerlessPendingReplay("s", "s", true, false)).toBe(false)
    expect(canClaimOwnerlessPendingReplay("s", "s", false, true)).toBe(false)
    expect(
      canClaimOwnerlessPendingReplay("s", "s", false, false, {
        status: "interrupted",
        interruptReason: "user_stop",
      }),
    ).toBe(false)
  })
})

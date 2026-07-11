import { describe, expect, test } from "vitest"

import {
  hasSendableChatPayload,
  nextDispatchablePending,
  shouldApplyPendingQueueSnapshot,
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

  test("allows a durable attachment-only row to reach the backend", () => {
    expect(hasSendableChatPayload("", false, false, "queued-request")).toBe(true)
    expect(hasSendableChatPayload("", false, false)).toBe(false)
  })
})

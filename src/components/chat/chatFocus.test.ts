// @vitest-environment jsdom

import { describe, expect, test, vi } from "vitest"

import { chatFocusTargetForPetNavigation, requestChatFocus, subscribeChatFocus } from "./chatFocus"

describe("chat focus", () => {
  test("maps a side Pet target through its owning source session", () => {
    expect(
      chatFocusTargetForPetNavigation({
        kind: "side",
        sessionId: "side-1",
        sourceSessionId: "source-1",
      }),
    ).toEqual({ sessionId: "source-1", sideSessionId: "side-1" })
  })

  test("preserves a side session when focus is relayed through the window event", () => {
    const handler = vi.fn()
    const unsubscribe = subscribeChatFocus(handler)

    requestChatFocus({ sessionId: "source-1", sideSessionId: "side-1" })

    expect(handler).toHaveBeenCalledWith({ sessionId: "source-1", sideSessionId: "side-1" })
    unsubscribe()
  })
})

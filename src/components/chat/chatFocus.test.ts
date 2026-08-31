// @vitest-environment jsdom

import { beforeEach, describe, expect, test, vi } from "vitest"
import { emitTo, listen } from "@tauri-apps/api/event"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { toast } from "sonner"
import { isTauriMode } from "@/lib/transport"
import { focusMainWindow } from "@/lib/spaceWindow"

import {
  CHAT_FOCUS_EVENT,
  chatFocusTargetForPetNavigation,
  requestChatFocus,
  subscribeChatFocus,
} from "./chatFocus"

vi.mock("@tauri-apps/api/event", () => ({ emitTo: vi.fn(), listen: vi.fn() }))
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: vi.fn() }))
vi.mock("@/lib/transport", () => ({ isTauriMode: vi.fn() }))
vi.mock("@/lib/spaceWindow", () => ({ focusMainWindow: vi.fn() }))
vi.mock("@/lib/logger", () => ({ logger: { error: vi.fn() } }))
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }))
vi.mock("i18next", () => ({ t: (key: string) => key }))

beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(isTauriMode).mockReturnValue(false)
  vi.mocked(getCurrentWindow).mockReturnValue({ label: "main" } as ReturnType<
    typeof getCurrentWindow
  >)
  vi.mocked(emitTo).mockResolvedValue(undefined)
  vi.mocked(listen).mockResolvedValue(vi.fn())
  vi.mocked(focusMainWindow).mockResolvedValue(undefined)
})

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
    expect(emitTo).not.toHaveBeenCalled()
    expect(listen).not.toHaveBeenCalled()
    unsubscribe()
  })

  test.each(["quickchat", "knowledge-space-window"])(
    "relays navigation from %s to the main window without dispatching locally",
    async (label) => {
      vi.mocked(isTauriMode).mockReturnValue(true)
      vi.mocked(getCurrentWindow).mockReturnValue({ label } as ReturnType<typeof getCurrentWindow>)
      const localHandler = vi.fn()
      window.addEventListener(CHAT_FOCUS_EVENT, localHandler)
      const target = {
        sessionId: "source-1",
        sideSessionId: "side-1",
        targetMessageId: 42,
        controlTarget: { kind: "goal", itemId: "goal-1" },
      }

      requestChatFocus(target)

      expect(emitTo).toHaveBeenCalledWith("main", CHAT_FOCUS_EVENT, target)
      await vi.waitFor(() => expect(focusMainWindow).toHaveBeenCalledOnce())
      expect(localHandler).not.toHaveBeenCalled()
      window.removeEventListener(CHAT_FOCUS_EVENT, localHandler)
    },
  )

  test("keeps main-window navigation synchronous and delivers it only once", async () => {
    vi.mocked(isTauriMode).mockReturnValue(true)
    vi.mocked(getCurrentWindow).mockReturnValue({ label: "main" } as ReturnType<
      typeof getCurrentWindow
    >)
    const handler = vi.fn()
    const stop = vi.fn()
    vi.mocked(listen).mockResolvedValue(stop)
    const unsubscribe = subscribeChatFocus(handler)

    requestChatFocus({ sessionId: "target", targetMessageId: 42 })

    expect(handler).toHaveBeenCalledExactlyOnceWith({ sessionId: "target", targetMessageId: 42 })
    expect(emitTo).not.toHaveBeenCalled()
    await Promise.resolve()
    unsubscribe()
    expect(stop).toHaveBeenCalledOnce()
  })

  test("validates native payloads and ignores events after cleanup, including late registration", async () => {
    vi.mocked(isTauriMode).mockReturnValue(true)
    let resolveListen!: (stop: () => void) => void
    vi.mocked(listen).mockReturnValue(
      new Promise((resolve) => {
        resolveListen = resolve
      }),
    )
    const handler = vi.fn()
    const unsubscribe = subscribeChatFocus(handler)
    const nativeHandler = vi.mocked(listen).mock.calls[0][1]
    const receive = (payload: unknown) => nativeHandler({ event: CHAT_FOCUS_EVENT, id: 1, payload })

    receive({ sessionId: "source-1", sideSessionId: "side-1", targetMessageId: 42 })
    receive({ sessionId: "" })
    expect(handler).toHaveBeenCalledExactlyOnceWith({
      sessionId: "source-1",
      sideSessionId: "side-1",
      targetMessageId: 42,
    })

    unsubscribe()
    receive({ sessionId: "target" })
    requestChatFocus({ sessionId: "target" })
    const stop = vi.fn()
    resolveListen(stop)
    await Promise.resolve()
    expect(stop).toHaveBeenCalledOnce()
    expect(handler).toHaveBeenCalledOnce()
  })

  test("reports a failed cross-window dispatch instead of silently dropping the click", async () => {
    vi.mocked(isTauriMode).mockReturnValue(true)
    vi.mocked(getCurrentWindow).mockReturnValue({ label: "quickchat" } as ReturnType<
      typeof getCurrentWindow
    >)
    vi.mocked(emitTo).mockRejectedValue(new Error("window unavailable"))

    requestChatFocus({ sessionId: "target" })

    await vi.waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith("chat.openSourceConversationFailed"),
    )
    expect(focusMainWindow).not.toHaveBeenCalled()
  })
})

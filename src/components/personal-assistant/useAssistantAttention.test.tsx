// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import { useAssistantAttention } from "./useAssistantAttention"

const fixture = vi.hoisted(() => ({
  revision: 1,
  focused: true,
  permission: true,
  focusListener: null as ((value: boolean) => void) | null,
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
  submit: vi
    .fn<(title: string, body: string, onOpen: () => void) => Promise<"unknown">>()
    .mockResolvedValue("unknown"),
}))
vi.mock("@/lib/transport-provider", () => ({
  useTransport: () => fixture,
  useTransportRevision: () => fixture.revision,
  getTransportRevision: () => fixture.revision,
}))
vi.mock("@/lib/notifications", () => ({
  hasAssistantNotificationPermission: async () => fixture.permission,
  isAppWindowFocused: () => fixture.focused,
  subscribeAppWindowFocus: (listener: (value: boolean) => void) => {
    fixture.focusListener = listener
    return () => {
      fixture.focusListener = null
    }
  },
  submitAssistantNotification: (title: string, body: string, onOpen: () => void) =>
    fixture.submit(title, body, onOpen),
}))
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }))
beforeEach(() => {
  fixture.revision = 1
  fixture.focused = true
  fixture.permission = true
  fixture.submit.mockClear()
  fixture.call.mockReset().mockImplementation(async (command: string) => {
    if (command === "get_notification_config") return { enabled: true, showChatContent: false }
    if (command === "claim_personal_assistant_desktop_notice")
      return { id: "notice-1", title: "Hope", body: "PRIVATE_BODY" }
  })
})
afterEach(cleanup)

it("reports Home presence separately from viewed state and uses one host claim when backgrounded", async () => {
  renderHook(() => useAssistantAttention(true, true, vi.fn()))
  await waitFor(() =>
    expect(fixture.call).toHaveBeenCalledWith(
      "set_personal_assistant_presence",
      expect.objectContaining({ active: true }),
    ),
  )
  expect(fixture.call.mock.calls.some(([command]) => command.includes("view"))).toBe(false)
  expect(fixture.call.mock.calls.some(([command]) => command.includes("claim"))).toBe(false)
  act(() => {
    fixture.focused = false
    fixture.focusListener?.(false)
  })
  await waitFor(() => expect(fixture.submit).toHaveBeenCalledTimes(1))
  expect(fixture.submit.mock.calls[0][1]).toBe("personalAssistant.noticePolicy.privateNotice")
  await waitFor(() =>
    expect(fixture.call).toHaveBeenCalledWith(
      "finish_personal_assistant_desktop_notice",
      expect.objectContaining({ outcome: "unknown" }),
    ),
  )
})
it("does not claim a notification or ask for permission when OS permission is absent", async () => {
  fixture.focused = false
  fixture.permission = false
  renderHook(() => useAssistantAttention(true, false, vi.fn()))
  await waitFor(() => expect(fixture.call).toHaveBeenCalledWith("get_notification_config"))
  await act(async () => {
    await Promise.resolve()
  })
  expect(fixture.submit).not.toHaveBeenCalled()
  expect(fixture.call.mock.calls.some(([command]) => command.includes("claim"))).toBe(false)
})
it("drops a late private claim after a connection change and settles only on the captured transport", async () => {
  fixture.focused = false
  let finish: ((value: unknown) => void) | undefined
  fixture.call.mockImplementation(async (command: string) => {
    if (command === "get_notification_config") return { enabled: true }
    if (command === "claim_personal_assistant_desktop_notice")
      return new Promise((resolve) => {
        finish = resolve
      })
  })
  renderHook(() => useAssistantAttention(true, false, vi.fn()))
  await waitFor(() => expect(finish).toBeDefined())
  await act(async () => {
    fixture.revision++
    finish?.({ id: "old", title: "Hope", body: "PRIVATE_OLD_ROOT" })
    await Promise.resolve()
  })
  expect(fixture.submit).not.toHaveBeenCalled()
  await waitFor(() =>
    expect(fixture.call).toHaveBeenCalledWith(
      "finish_personal_assistant_desktop_notice",
      expect.objectContaining({ id: "old", outcome: "not_submitted" }),
    ),
  )
})

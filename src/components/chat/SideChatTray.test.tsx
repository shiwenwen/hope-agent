// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import type { SessionMeta } from "@/types/chat"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { SideChatTray } from "./SideChatTray"

const transportMock = vi.hoisted(() => {
  const listeners = new Map<string, (payload: unknown) => void>()
  return {
    listeners,
    call: vi.fn<(...args: unknown[]) => Promise<unknown>>(() =>
      Promise.resolve({ active: false }),
    ),
    listen: vi.fn((eventName: string, handler: (payload: unknown) => void) => {
      listeners.set(eventName, handler)
      return () => listeners.delete(eventName)
    }),
  }
})

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

const sideChat = {
  id: "side-1",
  title: "Side task",
  agentId: "ha-main",
  createdAt: "2026-08-30T00:00:00.000Z",
  updatedAt: "2026-08-30T00:00:00.000Z",
  messageCount: 0,
  unreadCount: 0,
  channelUnreadCount: 0,
  hasError: false,
  pendingInteractionCount: 0,
  isCron: false,
  kind: "side",
} as SessionMeta

afterEach(() => {
  cleanup()
  transportMock.listeners.clear()
  vi.clearAllMocks()
  transportMock.call.mockReset().mockResolvedValue({ active: false })
})

describe("SideChatTray", () => {
  test.each(["completed", "failed"])("keeps %s unread in an open panel until a real receipt", async (status) => {
    const state = { active: false, lastTerminalStatus: status, turnId: "turn-1", lastTerminalRead: false }
    transportMock.call.mockResolvedValue(state)
    const props = { chats: [sideChat], activeId: "side-1", panelOpen: true, creating: false,
      onCreate: vi.fn(), onSelect: vi.fn(), onClosePanel: vi.fn() }
    const { rerender } = render(<SideChatTray {...props} />)
    const label = `Side task · common.statusValues.${status}`
    await waitFor(() => expect(screen.getByRole("button", { name: label })).toBeTruthy())
    await act(async () => {
      transportMock.listeners.get("chat:stream_end")?.({ sessionId: "side-1", status, turnId: "turn-1" })
    })
    fireEvent.click(screen.getByRole("button", { name: label }))
    expect(screen.getByRole("button", { name: label })).toBeTruthy()
    // A receipt for an earlier rendered row must not acknowledge a newer result.
    rerender(<SideChatTray {...props} readReceipt={{ sessionId: "side-1", throughMessageId: 10 }} />)
    await act(async () => { await Promise.resolve() })
    expect(screen.getByRole("button", { name: label })).toBeTruthy()
    transportMock.call.mockResolvedValue({ ...state, lastTerminalRead: true })
    rerender(<SideChatTray {...props} readReceipt={{ sessionId: "side-1", throughMessageId: 20 }} />)
    await waitFor(() => expect(screen.getByRole("button", { name: "Side task" })).toBeTruthy())
  })

  test.each(["completed", "failed"])("does not restore durably read %s badges on remount", async (status) => {
    transportMock.call.mockResolvedValue({
      active: false, lastTerminalStatus: status, turnId: "read-turn", lastTerminalRead: true,
    })
    const props = {
      chats: [sideChat], activeId: "side-1", panelOpen: false, creating: false,
      onCreate: vi.fn(), onSelect: vi.fn(), onClosePanel: vi.fn(),
    }
    const first = render(<SideChatTray {...props} />)
    await act(async () => { await Promise.resolve() })
    expect(screen.getByRole("button", { name: "Side task" })).toBeTruthy()
    first.unmount()
    render(<SideChatTray {...props} />)
    await act(async () => { await Promise.resolve() })
    expect(screen.getByRole("button", { name: "Side task" })).toBeTruthy()
    transportMock.call.mockResolvedValue({
      active: false, lastTerminalStatus: status, turnId: "unread-turn", lastTerminalRead: false,
    })
    await act(async () => {
      transportMock.listeners.get(TRANSPORT_EVENT_RESYNC_REQUIRED)?.({ reason: "reconnected" })
    })
    expect(screen.getByRole("button", { name: `Side task · common.statusValues.${status}` })).toBeTruthy()
  })

  test.each([
    ["completed", "completed"],
    ["failed", "failed"],
  ])("restores an inactive %s turn after reload", async (lastTerminalStatus, label) => {
    transportMock.call.mockResolvedValueOnce({
      active: false,
      lastTerminalStatus,
      turnId: "turn-1",
    })
    render(
      <SideChatTray
        chats={[sideChat]}
        activeId="side-1"
        panelOpen={false}
        creating={false}
        onCreate={vi.fn()}
        onSelect={vi.fn()}
        onClosePanel={vi.fn()}
      />,
    )
    await waitFor(() => {
      expect(screen.getByRole("button", {
        name: `Side task · common.statusValues.${label}`,
      })).toBeTruthy()
    })
  })

  test("recovers missed terminal events on resync without repeating acknowledged results", async () => {
    transportMock.call.mockResolvedValueOnce({ active: true, turnId: "turn-1" })
    render(
      <SideChatTray
        chats={[sideChat]}
        activeId="side-1"
        panelOpen={false}
        creating={false}
        onCreate={vi.fn()}
        onSelect={vi.fn()}
        onClosePanel={vi.fn()}
      />,
    )
    await waitFor(() => {
      expect(screen.getByRole("button", {
        name: "Side task · common.statusValues.running",
      })).toBeTruthy()
    })

    transportMock.call.mockResolvedValue({
      active: false,
      lastTerminalStatus: "completed",
      turnId: "turn-1",
    })
    await act(async () => {
      transportMock.listeners.get(TRANSPORT_EVENT_RESYNC_REQUIRED)?.({ reason: "reconnected" })
    })
    fireEvent.click(screen.getByRole("button", {
      name: "Side task · common.statusValues.completed",
    }))
    expect(screen.getByRole("button", { name: "Side task · common.statusValues.completed" })).toBeTruthy()
    transportMock.call.mockResolvedValue({
      active: false, lastTerminalStatus: "completed", turnId: "turn-1", lastTerminalRead: true,
    })
    await act(async () => {
      transportMock.listeners.get(TRANSPORT_EVENT_RESYNC_REQUIRED)?.({ reason: "reconnected" })
    })
    expect(screen.getByRole("button", { name: "Side task" })).toBeTruthy()

    transportMock.call.mockResolvedValue({
      active: false,
      lastTerminalStatus: "failed",
      turnId: "turn-2",
    })
    await act(async () => {
      transportMock.listeners.get(TRANSPORT_EVENT_RESYNC_REQUIRED)?.({ reason: "reconnected" })
    })
    expect(screen.getByRole("button", {
      name: "Side task · common.statusValues.failed",
    })).toBeTruthy()
  })

  test("does not let an old terminal snapshot overwrite a newer live turn", async () => {
    let resolveSnapshot!: (value: unknown) => void
    transportMock.call.mockReturnValueOnce(new Promise((resolve) => { resolveSnapshot = resolve }))
    render(
      <SideChatTray
        chats={[sideChat]}
        activeId="side-1"
        panelOpen={false}
        creating={false}
        onCreate={vi.fn()}
        onSelect={vi.fn()}
        onClosePanel={vi.fn()}
      />,
    )
    await act(async () => {
      transportMock.listeners.get("chat:turn_started")?.({ sessionId: "side-1", turnId: "turn-2" })
      resolveSnapshot({ active: false, lastTerminalStatus: "failed", turnId: "turn-1" })
    })
    expect(screen.getByRole("button", {
      name: "Side task · common.statusValues.running",
    })).toBeTruthy()
  })

  test("retains running, completed, and failed state while the panel is closed", async () => {
    transportMock.call.mockResolvedValueOnce({ active: true })
    const onSelect = vi.fn()
    render(
      <SideChatTray
        chats={[sideChat]}
        activeId="side-1"
        panelOpen={false}
        creating={false}
        onCreate={vi.fn()}
        onSelect={onSelect}
        onClosePanel={vi.fn()}
      />,
    )

    await waitFor(() => {
      expect(
        screen.getByRole("button", {
          name: "Side task · common.statusValues.running",
        }),
      ).toBeTruthy()
    })

    transportMock.call.mockResolvedValue({ active: false, lastTerminalStatus: "completed" })
    await act(async () => {
      transportMock.listeners.get("chat:stream_end")?.({
        sessionId: "side-1",
        status: "completed",
      })
    })
    const completedButton = screen.getByRole("button", {
      name: "Side task · common.statusValues.completed",
    })
    fireEvent.click(completedButton)
    expect(onSelect).toHaveBeenCalledWith("side-1")
    expect(screen.getByRole("button", { name: "Side task · common.statusValues.completed" })).toBeTruthy()

    transportMock.call.mockResolvedValue({ active: false, lastTerminalStatus: "failed" })
    await act(async () => {
      transportMock.listeners.get("chat:turn_started")?.({ sessionId: "side-1" })
      transportMock.listeners.get("chat:stream_end")?.({
        sessionId: "side-1",
        status: "failed",
      })
    })
    expect(
      screen.getByRole("button", {
        name: "Side task · common.statusValues.failed",
      }),
    ).toBeTruthy()
  })
})

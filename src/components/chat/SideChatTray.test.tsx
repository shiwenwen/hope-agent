// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import type { SessionMeta } from "@/types/chat"
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
})

describe("SideChatTray", () => {
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
    expect(screen.getByRole("button", { name: "Side task" })).toBeTruthy()

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

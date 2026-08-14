// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { useEffect } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"
import CronConversationsPanel from "./CronConversationsPanel"

const calls: Array<{ command: string; args?: unknown }> = []
const listeners = new Map<string, (raw: unknown) => void>()

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    call: async (command: string, args?: unknown) => {
      calls.push({ command, args })
      if (command === "cron_run_timeline") {
        return [
          {
            runLogId: 1,
            sessionId: "cron-session-1",
            turnId: "turn-1",
            targetMessageId: 101,
            jobId: "job-1",
            jobName: "Daily summary",
            status: "success",
            startedAt: "2026-07-15T00:00:00Z",
            unreadCount: 1,
          },
          {
            runLogId: 2,
            sessionId: "cron-session-1",
            turnId: "turn-2",
            targetMessageId: 202,
            jobId: "job-2",
            jobName: "Weekly review",
            status: "success",
            startedAt: "2026-07-14T00:00:00Z",
            unreadCount: 1,
          },
        ]
      }
      if (command === "list_agents") return []
      if (command === "cron_unread_total") return 0
      if (command === "mark_session_read_cmd") return null
      return null
    },
    listen: (event: string, callback: (raw: unknown) => void) => {
      listeners.set(event, callback)
      return () => listeners.delete(event)
    },
  }),
}))

vi.mock("./CronSessionViewer", () => ({
  default: function MockCronSessionViewer({
    runLogId,
    sessionId,
    targetMessageId,
    expectedTurnId,
    onLoaded,
  }: {
    runLogId: number
    sessionId: string
    targetMessageId?: number | null
    expectedTurnId?: string | null
    onLoaded?: (runLogId: number) => void
  }) {
    useEffect(() => onLoaded?.(runLogId), [onLoaded, runLogId])
    return (
      <div
        data-testid="cron-viewer"
        data-run-log-id={runLogId}
        data-target-message-id={targetMessageId}
        data-expected-turn-id={expectedTurnId}
      >
        {sessionId}
      </div>
    )
  },
}))

afterEach(() => {
  cleanup()
  calls.length = 0
  listeners.clear()
  vi.clearAllMocks()
})

describe("CronConversationsPanel unread behavior", () => {
  it("does not clear the auto-selected newest run until the user explicitly clicks it", async () => {
    render(<CronConversationsPanel />)

    await screen.findByTestId("cron-viewer")
    expect(calls.some((call) => call.command === "mark_session_read_cmd")).toBe(false)

    const title = screen.getByText("Daily summary")
    fireEvent.click(title.closest("button") as HTMLButtonElement)

    await waitFor(() => {
      expect(
        calls.some(
          (call) =>
            call.command === "mark_session_read_cmd" &&
            (call.args as { throughMessageId?: number })?.throughMessageId === 101,
        ),
      ).toBe(true)
    })
  })

  it("waits for a newly selected transcript to load before clearing it", async () => {
    render(<CronConversationsPanel />)

    await screen.findByTestId("cron-viewer")
    expect(calls.some((call) => call.command === "mark_session_read_cmd")).toBe(false)

    fireEvent.click(screen.getByText("Weekly review").closest("button") as HTMLButtonElement)

    await waitFor(() => {
      expect(
        calls.some(
          (call) =>
            call.command === "mark_session_read_cmd" &&
            (call.args as { sessionId?: string; throughMessageId?: number })?.sessionId ===
              "cron-session-1" &&
            (call.args as { throughMessageId?: number })?.throughMessageId === 202,
        ),
      ).toBe(true)
    })
    expect(screen.getByTestId("cron-viewer").dataset.runLogId).toBe("2")
    expect(screen.getByTestId("cron-viewer").dataset.expectedTurnId).toBe("turn-2")
  })
})

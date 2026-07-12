// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import ApprovalDialog, { type ApprovalRequest } from "./ApprovalDialog"

function request(id: string, timeoutAtMs: number, timeoutSecs: number): ApprovalRequest {
  return {
    request_id: id,
    session_id: "session-1",
    command: `command-${id}`,
    cwd: "/tmp",
    created_at_ms: Date.now(),
    server_now_ms: Date.now(),
    timeout_at_ms: timeoutAtMs,
    local_timeout_at_ms: timeoutAtMs,
    timeout_secs: timeoutSecs,
    timeout_action: "deny",
  }
}

afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe("ApprovalDialog authoritative lifecycle", () => {
  test("uses each queued request's absolute deadline instead of restarting the timeout", () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"))
    const first = request("first", Date.now() + 60_000, 60)
    const second = request("second", Date.now() + 10_000, 10)
    const { rerender } = render(<ApprovalDialog requests={[first, second]} onRespond={vi.fn()} />)

    expect(screen.getByText("1:00")).toBeTruthy()
    rerender(<ApprovalDialog requests={[second]} onRespond={vi.fn()} />)
    expect(screen.getByText("10s")).toBeTruthy()
  })

  test("prevents duplicate responses while a request is submitting", async () => {
    let resolve!: () => void
    const pending = new Promise<void>((done) => {
      resolve = done
    })
    const onRespond = vi.fn(() => pending)
    render(
      <ApprovalDialog
        requests={[request("first", Date.now() + 60_000, 60)]}
        onRespond={onRespond}
      />,
    )

    const allowOnce = screen.getByRole("button", { name: "approval.allowOnce" })
    fireEvent.click(allowOnce)
    fireEvent.click(allowOnce)
    expect(onRespond).toHaveBeenCalledTimes(1)
    expect((allowOnce as HTMLButtonElement).disabled).toBe(true)

    await act(async () => {
      resolve()
      await pending
    })
    expect((allowOnce as HTMLButtonElement).disabled).toBe(false)
  })
})

// @vitest-environment jsdom

import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import "@testing-library/jest-dom/vitest"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  listeners: new Map<string, (payload: unknown) => void>(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    call: mocks.call,
    listen: (event: string, listener: (payload: unknown) => void) => {
      mocks.listeners.set(event, listener)
      return () => mocks.listeners.delete(event)
    },
  }),
}))

import { usePetActivity } from "./usePetActivity"

function Harness() {
  const { snapshot } = usePetActivity()
  return <span>{snapshot.revision}</span>
}

beforeEach(() => {
  mocks.call.mockReset()
  mocks.listeners.clear()
  mocks.call.mockResolvedValue({
    revision: 1,
    generatedAt: "2026-07-24T00:00:00Z",
    stale: false,
    dominant: null,
    activities: [],
    total: 0,
    truncated: false,
  })
})

afterEach(cleanup)

describe("usePetActivity", () => {
  test("refreshes the Pet snapshot as soon as an async session title is updated", async () => {
    render(<Harness />)

    await waitFor(() => expect(mocks.call).toHaveBeenCalledOnce())
    expect(screen.getByText("1")).toBeInTheDocument()

    act(() => {
      mocks.listeners.get("session:title_updated")?.({
        sessionId: "session-1",
        title: "Concise generated title",
      })
    })

    await waitFor(() => expect(mocks.call).toHaveBeenCalledTimes(2))
  })
})

// @vitest-environment jsdom

import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import "@testing-library/jest-dom/vitest"
import { StrictMode } from "react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import type { PetActivity } from "@/types/pet"

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  listeners: new Map<string, (payload: unknown) => void>(),
  warn: vi.fn(),
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

vi.mock("@/lib/logger", () => ({
  logger: { warn: mocks.warn },
}))

import { petStreamPreviewLine, usePetStreamPreviews } from "./usePetStreamPreviews"

const runningActivity: PetActivity = {
  activityId: "session-1",
  status: "running",
  title: "Live task",
  titleKind: "session",
  updatedAt: "2026-07-24T00:00:00Z",
  boundary: 10,
  target: { kind: "regular", sessionId: "session-1" },
}

function Harness({ activities }: { activities: PetActivity[] }) {
  const previews = usePetStreamPreviews(activities)
  return <span>{previews.get("session-1") ?? "empty"}</span>
}

beforeEach(() => {
  mocks.call.mockReset()
  mocks.listeners.clear()
  mocks.warn.mockReset()
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    return window.setTimeout(() => callback(performance.now()), 0)
  })
  vi.stubGlobal("cancelAnimationFrame", (id: number) => window.clearTimeout(id))
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("usePetStreamPreviews", () => {
  test("restores the durable prefix before applying buffered and live deltas", async () => {
    let resolveSnapshot: (value: unknown) => void = () => undefined
    mocks.call.mockReturnValue(
      new Promise((resolve) => {
        resolveSnapshot = resolve
      }),
    )
    render(<Harness activities={[runningActivity]} />)

    await waitFor(() => expect(mocks.call).toHaveBeenCalledOnce())
    act(() => {
      mocks.listeners.get("chat:stream_delta")?.({
        sessionId: "session-1",
        streamId: "stream-1",
        seq: 2,
        event: JSON.stringify({ type: "text_delta", content: " world" }),
      })
    })
    expect(screen.getByText("empty")).toBeInTheDocument()

    await act(async () => {
      resolveSnapshot({
        streamId: "stream-1",
        throughSeq: 1,
        status: "running",
        events: [{ seq: 1, event: JSON.stringify({ type: "text_delta", content: "Hello" }) }],
      })
      await Promise.resolve()
    })
    expect(await screen.findByText("Hello world")).toBeInTheDocument()

    act(() => {
      mocks.listeners.get("chat:stream_delta")?.({
        sessionId: "session-1",
        streamId: "stream-1",
        seq: 3,
        event: JSON.stringify({ type: "text_delta", content: "!" }),
      })
    })
    expect(await screen.findByText("Hello world!")).toBeInTheDocument()
  })

  test("does not subscribe an incognito activity to textual snapshots", async () => {
    render(<Harness activities={[{ ...runningActivity, titleKind: "incognito", title: null }]} />)
    await act(async () => Promise.resolve())
    expect(mocks.call).not.toHaveBeenCalled()
    expect(screen.getByText("empty")).toBeInTheDocument()
  })

  test("restarts the snapshot handshake after a StrictMode effect remount", async () => {
    mocks.call.mockResolvedValue({
      streamId: "stream-strict",
      throughSeq: 1,
      status: "running",
      events: [{ seq: 1, event: JSON.stringify({ type: "text_delta", content: "Restored" }) }],
    })
    render(
      <StrictMode>
        <Harness activities={[runningActivity]} />
      </StrictMode>,
    )

    expect(await screen.findByText("Restored")).toBeInTheDocument()
    expect(mocks.call.mock.calls.length).toBeGreaterThanOrEqual(2)
  })

  test("keeps the changing tail of a long single-line preview", () => {
    const value = petStreamPreviewLine(`old ${"a".repeat(260)} latest`)
    expect(value.startsWith("…")).toBe(true)
    expect(value.endsWith("latest")).toBe(true)
    expect(Array.from(value)).toHaveLength(240)
  })

  test("removes incomplete Markdown markers from a streaming preview", () => {
    expect(petStreamPreviewLine("## Plan\n\n**Streaming `PetBubble`\n- next step")).toBe(
      "Plan Streaming PetBubble next step",
    )
  })
})

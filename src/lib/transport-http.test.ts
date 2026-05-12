import { afterEach, beforeEach, expect, test, vi } from "vitest"

import { HttpTransport } from "./transport-http"

const fetchMock = vi.fn()

beforeEach(() => {
  fetchMock.mockReset()
  vi.stubGlobal("fetch", fetchMock)
})

afterEach(() => {
  vi.unstubAllGlobals()
})

test("HttpTransport.startChat only bridges session_created and not late turn_started", async () => {
  const transport = new HttpTransport("http://localhost:8420")
  const events: string[] = []

  fetchMock.mockResolvedValue(
    new Response(
      JSON.stringify({
        sessionId: "session-123",
        response: "assistant reply",
        turnId: "turn-456",
      }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      },
    ),
  )

  const response = await transport.startChat(
    {
      message: "hello",
      attachments: [],
      sessionId: null,
    },
    (event) => events.push(event),
  )

  expect(response).toBe("assistant reply")
  expect(events).toEqual([
    JSON.stringify({
      type: "session_created",
      session_id: "session-123",
    }),
  ])
})

import { afterEach, beforeEach, expect, test, vi } from "vitest"

const fetchMock = vi.fn()

beforeEach(() => {
  vi.resetModules()
  fetchMock.mockReset()
  vi.stubEnv("VITE_SERVER_URL", "https://agent.example/")
  vi.stubGlobal("window", { location: { origin: "https://ui.example" } })
  vi.stubGlobal("fetch", fetchMock)
})

afterEach(() => {
  vi.unstubAllEnvs()
  vi.unstubAllGlobals()
})

test("cross-origin web authentication validates with Bearer and keeps no browser session", async () => {
  fetchMock.mockResolvedValue(
    new Response(
      JSON.stringify({
        authRequired: true,
        resourceTicket: "resource-ticket",
        eventTicket: "event-ticket",
        expiresInSecs: 900,
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
  )
  const provider = await import("./transport-provider")

  await expect(provider.authenticateWebOwnerToken("owner-secret")).resolves.toBe(true)
  expect(fetchMock).toHaveBeenCalledWith(
    "https://agent.example/api/auth/transport-tickets",
    {
      method: "POST",
      headers: { Authorization: "Bearer owner-secret" },
    },
  )
  expect(
    fetchMock.mock.calls.some(([url]) => String(url).endsWith("/api/auth/session")),
  ).toBe(false)
})

test("cross-origin web authentication distinguishes a rejected token from an outage", async () => {
  fetchMock.mockResolvedValueOnce(new Response("unauthorized", { status: 401 }))
  const provider = await import("./transport-provider")

  await expect(provider.authenticateWebOwnerToken("wrong-secret")).resolves.toBe(false)

  fetchMock.mockRejectedValueOnce(new TypeError("network unavailable"))
  await expect(provider.authenticateWebOwnerToken("owner-secret")).rejects.toThrow(
    "network unavailable",
  )
})

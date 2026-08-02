// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, test, vi } from "vitest"

import { AuthGate } from "./AuthGate"

const { authenticateWebOwnerToken, clearStoredApiKey } = vi.hoisted(() => ({
  authenticateWebOwnerToken: vi.fn(),
  clearStoredApiKey: vi.fn(),
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("@/lib/transport", () => ({
  isTauriMode: () => false,
}))

vi.mock("@/lib/transport-provider", () => ({
  authenticateWebOwnerToken,
  configuredHttpBase: () => "https://agent.example",
  isConfiguredHttpBaseSameOrigin: () => false,
}))

vi.mock("@/lib/api-key-storage", () => ({
  AUTH_REQUIRED_EVENT: "ha:auth-required",
  clearStoredApiKey,
  getStoredApiKey: () => null,
}))

beforeEach(() => {
  authenticateWebOwnerToken.mockReset()
  clearStoredApiKey.mockReset()
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(
    new Response(JSON.stringify({ authRequired: true, authenticated: false }), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  ))
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

test("uses the configured API origin and the remote Bearer authentication flow", async () => {
  authenticateWebOwnerToken.mockResolvedValue(true)
  render(<AuthGate><div>authenticated app</div></AuthGate>)

  await screen.findByLabelText("auth.tokenLabel")
  expect(fetch).toHaveBeenCalledWith("https://agent.example/api/auth/status", {
    credentials: "omit",
    cache: "no-store",
  })

  fireEvent.change(screen.getByLabelText("auth.tokenLabel"), {
    target: { value: "remote-owner-token" },
  })
  fireEvent.click(screen.getByRole("button", { name: "auth.tokenContinue" }))

  await waitFor(() => {
    expect(authenticateWebOwnerToken).toHaveBeenCalledWith("remote-owner-token")
  })
  expect(await screen.findByText("authenticated app")).toBeTruthy()
  expect(clearStoredApiKey).toHaveBeenCalled()
})

// @vitest-environment jsdom

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, it, vi } from "vitest"

import { OpenClawImportPanel } from "./OpenClawImportPanel"

const transportMock = vi.hoisted(() => ({
  call: vi.fn<(name: string) => Promise<unknown>>(),
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    warn: vi.fn(),
    error: vi.fn(),
  },
}))

beforeEach(() => {
  transportMock.call.mockReset()
})

afterEach(() => {
  cleanup()
})

it("handles a missing OpenClaw installation from Settings", async () => {
  transportMock.call.mockResolvedValueOnce(undefined)

  render(<OpenClawImportPanel onSkip={() => {}} onImported={() => {}} />)

  await waitFor(() => {
    expect(screen.getByText("onboarding.importOpenClaw.notFound")).toBeTruthy()
  })
  expect(screen.getByRole("button", { name: "onboarding.importOpenClaw.continue" })).toBeTruthy()
})

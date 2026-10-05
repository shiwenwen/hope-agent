// @vitest-environment jsdom

import { afterEach, expect, test, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"

import { SummaryStep } from "./SummaryStep"
import type { OnboardingStepKey } from "../types"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) =>
      key === "settings.webSearchProviderKeyless" ? "Free multi-engine search" : key,
  }),
}))

const transportMock = vi.hoisted(() => ({ call: vi.fn() }))
vi.mock("@/lib/transport-provider", () => ({ getTransport: () => transportMock }))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

test("shows default keyless search when optional search setup is skipped", async () => {
  transportMock.call.mockImplementation(async (command: string) => {
    if (command === "list_local_ips") return []
    if (command === "get_web_search_config") {
      return {
        providers: [
          { id: "keyless", enabled: true },
          { id: "duck-duck-go", enabled: true },
        ],
      }
    }
    return null
  })
  render(
    <SummaryStep
      draft={{ serverMode: "local" }}
      skipped={new Set<OnboardingStepKey>(["search-provider"])}
    />,
  )
  expect(await screen.findByText("Free multi-engine search")).toBeTruthy()
  expect(screen.queryByText("settings.webSearchProviderDDG")).toBeNull()
})

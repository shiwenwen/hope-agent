// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"

import { SummaryStep } from "./SummaryStep"

import type { OnboardingStepKey } from "../types"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => {
      if (key === "settings.webSearchProviderDDG") return "DuckDuckGo"
      if (key === "onboarding.summary.skipped") return "Skipped"
      return key
    },
  }),
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("SummaryStep", () => {
  test("shows saved search provider even when the optional step was skipped", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "list_local_ips") return []
      if (command === "get_web_search_config") {
        return {
          providers: [{ id: "duck-duck-go", enabled: true }],
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

    expect(await screen.findByText("DuckDuckGo")).toBeTruthy()
  })
})

// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import AcpControlPanel from "./AcpControlPanel"

const transportMock = vi.hoisted(() => ({ call: vi.fn() }))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
}))

const config = {
  enabled: true,
  backends: [
    {
      id: "custom-acp",
      name: "Custom ACP",
      binary: "acp-adapter",
      acpArgs: ["--config", "/Users/me/My Project/config.json"],
      protocol: "v1",
      distribution: {
        source: "custom",
        package: "custom-acp",
        version: null,
        platformFiles: [],
        authMethod: "inherited_environment",
      },
      enabled: true,
      defaultModel: null,
      env: {},
    },
  ],
  maxConcurrentSessions: 5,
  defaultTimeoutSecs: 0,
  runtimeTtlSecs: 1800,
  autoDiscover: true,
}

beforeEach(() => {
  transportMock.call.mockImplementation((method: string) => {
    if (method === "acp_get_config") return Promise.resolve(structuredClone(config))
    if (method === "acp_list_backends") return Promise.resolve([])
    if (method === "acp_set_config") return Promise.resolve(undefined)
    return Promise.reject(new Error(`unexpected method: ${method}`))
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("AcpControlPanel launch arguments", () => {
  it("preserves an argument containing whitespace when another argument is edited", async () => {
    render(<AcpControlPanel />)
    const flagInput = await screen.findByLabelText("Launch arguments 1")

    fireEvent.change(flagInput, { target: { value: "--settings" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => {
      const saveCall = transportMock.call.mock.calls.find(([method]) => method === "acp_set_config")
      expect(saveCall?.[1].config.backends[0].acpArgs).toEqual([
        "--settings",
        "/Users/me/My Project/config.json",
      ])
    })
  })
})

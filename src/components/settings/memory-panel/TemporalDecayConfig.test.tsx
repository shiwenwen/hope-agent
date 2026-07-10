// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import TemporalDecayConfig from "./TemporalDecayConfig"

const tMock = vi.hoisted(() => {
  return (
    key: string,
    options?:
      | string
      | ({
          defaultValue?: string
        } & Record<string, unknown>),
  ) => {
    let text =
      typeof options === "string"
        ? options
        : typeof options?.defaultValue === "string"
          ? options.defaultValue
          : key

    if (typeof options === "object") {
      for (const [name, value] of Object.entries(options)) {
        text = text.replaceAll(`{{${name}}}`, String(value))
      }
    }

    return text
  }
})

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: tMock,
  }),
}))

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
  },
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
    warn: vi.fn(),
  },
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

describe("TemporalDecayConfig", () => {
  it("shows redacted load failure detail and lets the user retry", async () => {
    let loadCalls = 0
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_temporal_decay_config") {
        loadCalls += 1
        if (loadCalls === 1) throw new Error("read failed token=decay-secret")
        return { enabled: true, halfLifeDays: 14 }
      }
      return null
    })

    render(<TemporalDecayConfig />)

    expect(await screen.findByText("Failed to load memory search tuning")).toBeTruthy()
    expect(screen.getByText("Details: read failed token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/decay-secret/)).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Retry" }))

    await waitFor(() => {
      expect(screen.queryByText("Failed to load memory search tuning")).toBeNull()
    })
    expect(loadCalls).toBe(2)
    expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("true")
  })
})

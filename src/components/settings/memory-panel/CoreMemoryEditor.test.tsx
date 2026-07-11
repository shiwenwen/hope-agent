// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import CoreMemoryEditor from "./CoreMemoryEditor"

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
  },
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("CoreMemoryEditor", () => {
  it("shows redacted load failure detail without rendering an empty editor, then retries", async () => {
    let loadCalls = 0
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_global_memory_md") {
        loadCalls += 1
        if (loadCalls === 1) throw new Error("core read failed token=core-secret")
        return "Always explain tradeoffs."
      }
      return null
    })

    render(<CoreMemoryEditor scope="global" />)

    expect(await screen.findByText("Failed to load global core memory")).toBeTruthy()
    expect(screen.getByText("Details: core read failed token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/core-secret/)).toBeNull()
    expect(screen.queryByPlaceholderText("settings.coreMemoryPlaceholder")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Retry" }))

    await waitFor(() => {
      expect(screen.queryByText("Failed to load global core memory")).toBeNull()
    })
    expect(loadCalls).toBe(2)
    expect(
      (screen.getByPlaceholderText("settings.coreMemoryPlaceholder") as HTMLTextAreaElement).value,
    ).toBe("Always explain tradeoffs.")
  })
})

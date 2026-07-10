// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { ComponentProps } from "react"

import HybridSearchConfigSection from "./HybridSearchConfig"

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

describe("HybridSearchConfigSection", () => {
  function renderSection() {
    const data = {
      embeddingConfig: { providerType: "openai-compatible", apiModel: "text-embedding-3-small" },
      dedupConfig: { thresholdHigh: 0.02, thresholdMerge: 0.012 },
      setDedupConfig: vi.fn(),
      dedupExpanded: false,
      setDedupExpanded: vi.fn(),
    }

    type SectionData = ComponentProps<typeof HybridSearchConfigSection>["data"]
    render(<HybridSearchConfigSection data={data as unknown as SectionData} />)
  }

  it("shows redacted advanced config load failure detail and lets the user retry", async () => {
    let hybridLoadCalls = 0
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_hybrid_search_config") {
        hybridLoadCalls += 1
        if (hybridLoadCalls === 1) throw new Error("hybrid failed token=hybrid-secret")
        return { vectorWeight: 0.7, textWeight: 0.3, rrfK: 80 }
      }
      if (command === "get_mmr_config") return { enabled: true, lambda: 0.6 }
      if (command === "get_embedding_cache_config") return { enabled: true, maxEntries: 5000 }
      if (command === "get_multimodal_config") {
        return { enabled: false, modalities: ["image", "audio"], maxFileBytes: 10 * 1024 * 1024 }
      }
      if (command === "get_memory_selection_config") {
        return { enabled: false, threshold: 8, maxSelected: 5 }
      }
      return null
    })

    renderSection()

    expect(await screen.findByText("Failed to load memory search tuning")).toBeTruthy()
    expect(screen.getByText("Details: hybrid failed token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/hybrid-secret/)).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Retry" }))

    await waitFor(() => {
      expect(screen.queryByText("Failed to load memory search tuning")).toBeNull()
    })
    expect(hybridLoadCalls).toBe(2)
  })
})

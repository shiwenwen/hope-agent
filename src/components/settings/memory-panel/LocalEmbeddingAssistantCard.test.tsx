// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import { TooltipProvider } from "@/components/ui/tooltip"
import LocalEmbeddingAssistantCard from "./LocalEmbeddingAssistantCard"

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

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
    warn: vi.fn(),
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

describe("LocalEmbeddingAssistantCard", () => {
  function renderCard() {
    return render(
      <TooltipProvider>
        <LocalEmbeddingAssistantCard onActivated={vi.fn()} />
      </TooltipProvider>,
    )
  }

  it("shows a refresh failure instead of an endless detecting state before models load", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "local_embedding_list_models") {
        throw new Error("detect failed token=ollama-secret")
      }
      if (command === "local_llm_detect_ollama") {
        return { phase: "not-installed", baseUrl: "http://localhost:11434", installScriptSupported: false }
      }
      if (command === "memory_embedding_get") {
        return { selection: { enabled: false }, currentModel: null }
      }
      return null
    })

    renderCard()

    expect(await screen.findByText(/Failed to refresh local embedding assistant/)).toBeTruthy()
    expect(screen.getByText(/Details: detect failed token=\[redacted\]/)).toBeTruthy()
    expect(screen.queryByText("settings.localEmbedding.detecting")).toBeNull()
    expect(screen.queryByText(/ollama-secret/)).toBeNull()
  })

  it("clears a refresh failure after retry succeeds", async () => {
    let calls = 0
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "local_embedding_list_models") {
        calls += 1
        if (calls === 1) throw new Error("detect failed token=retry-secret")
        return [
          {
            id: "nomic-embed-text",
            displayName: "Nomic Embed Text",
            sizeMb: 274,
            dimensions: 768,
            contextWindow: 8192,
            languages: ["en"],
            recommended: true,
            installed: false,
          },
        ]
      }
      if (command === "local_llm_detect_ollama") {
        return { phase: "running", baseUrl: "http://localhost:11434", installScriptSupported: true }
      }
      if (command === "memory_embedding_get") {
        return { selection: { enabled: false }, currentModel: null }
      }
      return null
    })

    renderCard()

    expect(await screen.findByText(/Failed to refresh local embedding assistant/)).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: /Retry/i }))

    await waitFor(() => {
      expect(screen.queryByText(/Failed to refresh local embedding assistant/)).toBeNull()
    })
    expect(screen.getByText("Nomic Embed Text")).toBeTruthy()
    expect(screen.queryByText(/retry-secret/)).toBeNull()
  })
})

// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"

import KnowledgePanel from "./KnowledgePanel"

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
  useTranslation: () => ({ t: tMock }),
}))

vi.mock("@/components/ui/model-chain-editor", () => ({
  ModelChainEditor: ({
    value,
    onChange,
    availableModels,
    inheritLabel,
  }: {
    value: {
      primary: { providerId: string; modelId: string }
      fallbacks: { providerId: string; modelId: string }[]
    } | null
    onChange: (
      next: {
        primary: { providerId: string; modelId: string }
        fallbacks: { providerId: string; modelId: string }[]
      } | null,
    ) => void
    availableModels?: Array<{
      providerId: string
      providerName: string
      modelId: string
      modelName: string
    }> | null
    inheritLabel: string
  }) => (
    <select
      aria-label={inheritLabel}
      value={value ? `${value.primary.providerId}::${value.primary.modelId}` : ""}
      onChange={(event) => {
        const selected = (availableModels ?? []).find(
          (model) => `${model.providerId}::${model.modelId}` === event.target.value,
        )
        onChange(
          selected
            ? {
                primary: { providerId: selected.providerId, modelId: selected.modelId },
                fallbacks: [],
              }
            : null,
        )
      }}
    >
      <option value="">{inheritLabel}</option>
      {(availableModels ?? []).map((model) => (
        <option
          key={`${model.providerId}::${model.modelId}`}
          value={`${model.providerId}::${model.modelId}`}
        >
          {model.providerName} / {model.modelName}
        </option>
      ))}
    </select>
  ),
}))

const toastMock = vi.hoisted(() => ({
  error: vi.fn(),
  success: vi.fn(),
  message: vi.fn(),
}))

vi.mock("sonner", () => ({
  toast: toastMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
    warn: vi.fn(),
  },
}))

vi.mock("@/hooks/useReembedJob", () => ({
  useReembedJob: () => ({ job: null, dismiss: vi.fn() }),
}))

vi.mock("./memory-panel/EmbeddingActivationDialog", () => ({
  default: () => null,
}))

vi.mock("./KnowledgeMaintenanceSection", () => ({
  default: () => <div data-testid="knowledge-maintenance-section" />,
}))

vi.mock("./SpriteSection", () => ({
  default: () => <div data-testid="sprite-section" />,
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
  useTransport: () => transportMock,
}))

if (!HTMLElement.prototype.hasPointerCapture) {
  Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
    configurable: true,
    value: () => false,
  })
}

if (!HTMLElement.prototype.setPointerCapture) {
  Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
    configurable: true,
    value: () => undefined,
  })
}

if (!HTMLElement.prototype.releasePointerCapture) {
  Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", {
    configurable: true,
    value: () => undefined,
  })
}

if (!HTMLElement.prototype.scrollIntoView) {
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: () => undefined,
  })
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function defaultKnowledgePanelCommandResponse(command: string) {
  if (command === "embedding_model_config_list") return []
  if (command === "knowledge_embedding_get_cmd") {
    return { selection: { enabled: false }, currentModel: null, needsReembed: false }
  }
  if (command === "knowledge_compile_config_get_cmd") return { modelOverride: null }
  if (command === "get_available_models") return []
  if (command === "kb_passive_recall_config_get_cmd") {
    return { enabled: false, topN: 3, maxChars: 800, cacheTtlSecs: 300, showSnippet: false }
  }
  if (command === "knowledge_media_retention_config_get_cmd") {
    return {
      enabled: false,
      maxTotalBytes: 1024 * 1024 * 1024,
      maxSourceBytes: 100 * 1024 * 1024,
      thumbnailMaxEdgePx: 512,
      pruneWhenOverQuota: true,
    }
  }
  if (command === "knowledge_chunk_get_cmd") return { maxChars: 1200, overlapChars: 120 }
  if (command === "knowledge_search_config_get_cmd") {
    return {
      textWeight: 0.4,
      vectorWeight: 0.6,
      rrfK: 60,
      mmrLambda: 0.7,
      candidateMultiplier: 4,
    }
  }
  return null
}

describe("KnowledgePanel", () => {
  it("shows knowledge embedding load failures instead of a silent off state", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "embedding_model_config_list") {
        throw new Error("config store locked token=knowledge-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    expect(await screen.findByText("Failed to load knowledge vector search settings")).toBeTruthy()
    expect(
      screen.getByText("Details: config store locked token=[redacted]"),
    ).toBeTruthy()
    expect(screen.queryByText(/knowledge-secret/)).toBeNull()
  })

  it("shows source-to-note model list load failures instead of a silent empty list", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "knowledge_compile_config_get_cmd") return { modelOverride: null }
      if (command === "get_available_models") throw new Error("models token=model-list-secret")
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    expect(await screen.findByText("Failed to load source-to-note agent list")).toBeTruthy()
    expect(screen.getByText("Details: models token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/model-list-secret/)).toBeNull()
  })

  it("shows passive related notes load failures instead of hiding the section", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "kb_passive_recall_config_get_cmd") {
        throw new Error("passive load token=passive-load-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    expect(await screen.findByText("Failed to load passive related notes setting")).toBeTruthy()
    expect(screen.getByText("Details: passive load token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/passive-load-secret/)).toBeNull()
  })

  it("rolls back the passive related notes toggle when saving fails", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "kb_passive_recall_config_get_cmd") {
        return { enabled: false, topN: 3, maxChars: 800, cacheTtlSecs: 300, showSnippet: false }
      }
      if (command === "kb_passive_recall_config_set_cmd") {
        throw new Error("toggle token=passive-toggle-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    expect(await screen.findByText("Passive related notes")).toBeTruthy()
    const passiveToggle = screen.getAllByRole("switch")[1]
    expect(passiveToggle.getAttribute("aria-checked")).toBe("false")
    fireEvent.click(passiveToggle)

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("Failed to update passive related notes toggle", {
        description: "Details: toggle token=[redacted]",
      }),
    )
    expect(passiveToggle.getAttribute("aria-checked")).toBe("false")
    expect(screen.queryByText(/passive-toggle-secret/)).toBeNull()
  })

  it("shows knowledge search ranking load failures instead of silent empty controls", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "knowledge_search_config_get_cmd") {
        throw new Error("search load token=search-load-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    fireEvent.click(await screen.findByRole("button", { name: /Advanced · search ranking/i }))
    expect(await screen.findByText("Failed to load knowledge search ranking settings")).toBeTruthy()
    expect(screen.getByText("Details: search load token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/search-load-secret/)).toBeNull()
  })

  it("keeps the search draft when the shared reset request fails", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "reset_settings_section") {
        throw new Error("restore token=search-restore-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    const searchRankingToggle = await screen.findByRole("button", {
      name: /Advanced · search ranking/i,
    })
    fireEvent.click(searchRankingToggle)
    fireEvent.click(
      await within(searchRankingToggle.parentElement as HTMLElement).findByRole("button", {
        name: /恢复此区域|Restore this section/i,
      }),
    )
    fireEvent.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "common.restoreDefaults",
      }),
    )

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith(
        "settings.resetDefaultsFailed",
        {
          description: "restore token=search-restore-secret",
        },
      ),
    )
    expect(screen.getByRole("alertdialog")).toBeTruthy()
  })

  it("shows original media retention load failures instead of hiding the section", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "knowledge_media_retention_config_get_cmd") {
        throw new Error("media load token=media-load-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    const mediaTitle = await screen.findByText("Original media retention")
    const mediaSection = mediaTitle.closest(".rounded-lg") as HTMLElement
    expect(
      await within(mediaSection).findByText("Failed to load original media retention settings"),
    ).toBeTruthy()
    expect(within(mediaSection).getByText("Details: media load token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/media-load-secret/)).toBeNull()
  })

  it("rolls back the original media retention toggle when saving fails", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "knowledge_media_retention_config_get_cmd") {
        return {
          enabled: false,
          maxTotalBytes: 1024 * 1024 * 1024,
          maxSourceBytes: 100 * 1024 * 1024,
          thumbnailMaxEdgePx: 512,
          pruneWhenOverQuota: true,
        }
      }
      if (command === "knowledge_media_retention_config_set_cmd") {
        throw new Error("media toggle token=media-toggle-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    // The panel renders the same title in its loading placeholder, which has
    // no switch, so `findByText` resolves before the config arrives. Re-query
    // until the loaded section (a different element) is on screen.
    const mediaToggle = await waitFor(() => {
      const section = screen
        .getByText("Original media retention")
        .closest(".rounded-lg") as HTMLElement
      return within(section).getByRole("switch")
    })
    expect(mediaToggle.getAttribute("aria-checked")).toBe("false")
    fireEvent.click(mediaToggle)

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith(
        "Failed to update original media retention toggle",
        {
          description: "Details: media toggle token=[redacted]",
        },
      ),
    )
    expect(mediaToggle.getAttribute("aria-checked")).toBe("false")
    expect(screen.queryByText(/media-toggle-secret/)).toBeNull()
  })

  it("shows knowledge chunking load failures instead of blank inputs", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "knowledge_chunk_get_cmd") {
        throw new Error("chunk load token=chunk-load-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    fireEvent.click(await screen.findByRole("button", { name: /Advanced · chunking/i }))
    expect(await screen.findByText("Failed to load knowledge chunking settings")).toBeTruthy()
    expect(screen.getByText("Details: chunk load token=[redacted]")).toBeTruthy()
    expect(screen.queryByText("Chunk size (chars)")).toBeNull()
    expect(screen.queryByText(/chunk-load-secret/)).toBeNull()
  })

  it("shows knowledge chunking save failures with redacted detail", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "knowledge_chunk_set_cmd") {
        throw new Error("chunk save token=chunk-save-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    fireEvent.click(await screen.findByRole("button", { name: /Advanced · chunking/i }))
    expect(await screen.findByText("Chunk size (chars)")).toBeTruthy()
    fireEvent.change(screen.getByLabelText("Chunk size (chars)"), {
      target: { value: "1500" },
    })
    fireEvent.click(screen.getByRole("button", { name: /^Save$/i }))

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("Failed to save knowledge chunking settings", {
        description: "Details: chunk save token=[redacted]",
      }),
    )
    expect(screen.queryByText(/chunk-save-secret/)).toBeNull()
  })

  it("rolls back the source-to-note model selection when saving fails", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "knowledge_compile_config_get_cmd") {
        return { modelOverride: null }
      }
      if (command === "get_available_models") {
        return [
          {
            providerId: "provider-1",
            providerName: "Provider One",
            apiType: "openai_chat",
            modelId: "model-1",
            modelName: "Model One",
            inputTypes: ["text"],
            contextWindow: 128000,
            maxTokens: 8192,
            reasoning: false,
          },
          {
            providerId: "provider-2",
            providerName: "Provider Two",
            apiType: "openai_chat",
            modelId: "model-2",
            modelName: "Model Two",
            inputTypes: ["text"],
            contextWindow: 128000,
            maxTokens: 8192,
            reasoning: false,
          },
        ]
      }
      if (command === "knowledge_compile_config_set_cmd") {
        throw new Error("save token=compile-secret")
      }
      return defaultKnowledgePanelCommandResponse(command)
    })

    render(<KnowledgePanel />)

    const compileSelector = await screen.findByRole("combobox", {
      name: "settings.knowledgeCompile.agentDefault",
    })
    fireEvent.change(compileSelector, { target: { value: "provider-2::model-2" } })

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("Failed to save source-to-note agent setting", {
        description: "Details: save token=[redacted]",
      }),
    )
    expect(compileSelector).toHaveProperty("value", "")
    expect(screen.queryByText(/compile-secret/)).toBeNull()
  })
})

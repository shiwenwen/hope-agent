// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, test, vi } from "vitest"
import ProviderSetup from "./index"
import type { ModelConfig, ProviderTemplate } from "./types"

const transportMock = vi.hoisted(() => ({ call: vi.fn() }))

vi.mock("@/lib/transport-provider", () => ({ getTransport: () => transportMock }))
vi.mock("@/lib/logger", () => ({ logger: { warn: vi.fn() } }))
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

const model: ModelConfig = {
  id: "new-model",
  name: "New model",
  inputTypes: ["text"],
  contextWindow: 128_000,
  maxTokens: 8_192,
  reasoning: false,
  costInput: null,
  costOutput: null,
}

vi.mock("./TemplateGrid", () => ({
  TemplateGrid: ({
    onSelectTemplate,
    onStartCustom,
  }: {
    onSelectTemplate: (template: ProviderTemplate) => void
    onStartCustom: () => void
  }) => (
    <>
      <button
        onClick={() =>
          onSelectTemplate({
            key: "new-provider",
            name: "New provider",
            description: "",
            icon: "",
            apiType: "openai-chat",
            baseUrl: "https://provider.example.test",
            apiKeyPlaceholder: "",
            requiresApiKey: false,
            models: [model],
          })
        }
      >
        template
      </button>
      <button onClick={onStartCustom}>custom</button>
    </>
  ),
}))

vi.mock("./TemplateConfig", () => ({
  TemplateConfig: ({ onSave }: { onSave: () => void }) => <button onClick={onSave}>save</button>,
}))

vi.mock("./CustomWizard", () => ({
  CustomWizard: ({
    setModels,
    onSave,
  }: {
    setModels: (models: ModelConfig[]) => void
    onSave: () => void
  }) => (
    <>
      <button onClick={() => setModels([model])}>add model</button>
      <button onClick={onSave}>save</button>
    </>
  ),
}))

beforeEach(() => {
  transportMock.call.mockReset()
  transportMock.call.mockImplementation(async (command: string) => {
    if (command === "get_providers") {
      return [{ id: "existing-provider", models: [{ id: "existing-model" }] }]
    }
    if (command === "add_provider") return { id: "new-provider", models: [model] }
    return null
  })
})

afterEach(cleanup)

test.each(["template", "custom"])(
  "adding a %s provider leaves default selection to the atomic backend write",
  async (mode) => {
    const onComplete = vi.fn()
    render(<ProviderSetup onComplete={onComplete} onCodexAuth={vi.fn()} />)
    fireEvent.click(screen.getByRole("button", { name: mode }))
    if (mode === "custom") fireEvent.click(screen.getByRole("button", { name: "add model" }))
    fireEvent.click(screen.getByRole("button", { name: "save" }))

    await waitFor(() => expect(onComplete).toHaveBeenCalledOnce())
    expect(transportMock.call).toHaveBeenCalledWith(
      "add_provider",
      expect.objectContaining({ config: expect.objectContaining({ models: [model] }) }),
    )
    expect(
      transportMock.call.mock.calls.filter(([command]) => command === "set_active_model"),
    ).toHaveLength(0)
    // Only the initial chooser load reads the provider list. Saving does not
    // guess an added Provider's identity from the last item of another read.
    expect(
      transportMock.call.mock.calls.filter(([command]) => command === "get_providers"),
    ).toHaveLength(1)
  },
)

// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, test, vi } from "vitest"
import ProviderEditPage from "./ProviderEditPage"
import type { ModelConfig, ProviderConfig } from "./provider-setup/types"

const transport = vi.hoisted(() => ({ call: vi.fn() }))

vi.mock("@/lib/transport-provider", () => ({ getTransport: () => transport }))
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }))
vi.mock("@/components/common/ProviderIcon", () => ({ default: () => null }))
vi.mock("@/components/settings/provider-setup", () => ({
  SortableModelEditor: ({
    model,
    onTest,
  }: {
    model: ModelConfig
    onTest?: (modelId: string) => Promise<string>
  }) => <button onClick={() => void onTest?.(model.id)}>test draft model</button>,
}))

beforeEach(() => {
  transport.call.mockReset()
  transport.call.mockResolvedValue(JSON.stringify({ success: true, message: "synthetic result" }))
})
afterEach(cleanup)

test.each(["wrkspc_Draft", ""])(
  "connection, model and save requests share the unsaved workspace binding %j",
  async (workspaceId) => {
    const provider: ProviderConfig = {
      id: "provider",
      name: "Anthropic",
      apiType: "anthropic",
      baseUrl: "https://api.anthropic.com",
      apiKey: "synthetic-legacy-key",
      authProfiles: [
        { id: "one", label: "one", apiKey: "synthetic-profile-key", enabled: true },
        { id: "two", label: "two", apiKey: "synthetic-other-key", enabled: false },
      ],
      models: [
        {
          id: "claude-fable-5-1",
          name: "Fable",
          inputTypes: ["text"],
          contextWindow: 1_000_000,
          maxTokens: 128_000,
          reasoning: true,
          costInput: null,
          costOutput: null,
        },
      ],
      enabled: true,
      userAgent: "test-agent",
      thinkingStyle: "anthropic",
      currency: "USD",
      allowPrivateNetwork: false,
    }
    const onSave = vi.fn()
    render(<ProviderEditPage provider={provider} onSave={onSave} onCancel={vi.fn()} />)
    fireEvent.click(screen.getAllByRole("switch", { name: "authProfiles.multiWorkspace" })[0])
    fireEvent.change(screen.getByRole("textbox", { name: "authProfiles.workspaceId" }), {
      target: { value: workspaceId },
    })

    const draft = {
      ...provider,
      authProfiles: [
        { ...provider.authProfiles[0], anthropicWorkspaceId: workspaceId },
        provider.authProfiles[1],
      ],
    }
    fireEvent.click(screen.getByRole("button", { name: "provider.testConnection" }))
    await waitFor(() =>
      expect(transport.call).toHaveBeenCalledWith("test_provider", { config: draft }),
    )

    transport.call.mockResolvedValueOnce({ models: [], truncated: false })
    fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
    await screen.findByText("provider.discovery.empty")
    expect(transport.call).toHaveBeenCalledWith("discover_provider_models", { config: draft })
    fireEvent.click(screen.getByRole("button", { name: "common.cancel" }))

    fireEvent.click(screen.getByRole("button", { name: /model.modelList/ }))
    fireEvent.click(screen.getByRole("button", { name: "test draft model" }))
    await waitFor(() =>
      expect(transport.call).toHaveBeenCalledWith("test_model", {
        config: draft,
        modelId: "claude-fable-5-1",
      }),
    )

    fireEvent.click(screen.getByRole("button", { name: "common.save" }))
    await waitFor(() =>
      expect(transport.call).toHaveBeenCalledWith("update_provider", { config: draft }),
    )
    expect(onSave).toHaveBeenCalledOnce()
    expect(provider.authProfiles[0].anthropicWorkspaceId).toBeUndefined()
  },
)

test("discovered models append only after confirmation and persist with the existing model unchanged", async () => {
  const model: ModelConfig = {
    id: "existing",
    name: "Saved",
    inputTypes: ["text", "image"],
    contextWindow: 32000,
    maxTokens: 8000,
    reasoning: true,
    costInput: 2,
    costOutput: 3,
  }
  const provider: ProviderConfig = {
    id: "custom",
    name: "Custom",
    apiType: "openai-chat",
    baseUrl: "https://synthetic.example/v1",
    apiKey: "masked...key",
    authProfiles: [],
    models: [model],
    enabled: true,
    thinkingStyle: "none",
    userAgent: "synthetic",
  }
  const before = structuredClone(provider)
  transport.call.mockImplementation(async (command: string) =>
    command === "discover_provider_models"
      ? {
          models: [
            { id: "existing", name: "Remote overwrite" },
            { id: "new", name: "New" },
          ],
          truncated: false,
        }
      : null,
  )
  const saved = vi.fn()
  render(<ProviderEditPage provider={provider} onSave={saved} onCancel={vi.fn()} />)
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
  fireEvent.click(await screen.findByRole("switch", { name: "provider.discovery.select" }))
  expect(screen.queryByText("Remote overwrite")).toBeNull()
  fireEvent.change(screen.getByRole("spinbutton", { name: "provider.discovery.context" }), {
    target: { value: "16000" },
  })
  fireEvent.change(screen.getByRole("spinbutton", { name: "provider.discovery.output" }), {
    target: { value: "2000" },
  })
  fireEvent.click(screen.getByRole("switch", { name: "provider.discovery.confirm" }))
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.add" }))
  expect(
    transport.call.mock.calls.filter(([command]) => command === "update_provider"),
  ).toHaveLength(0)
  fireEvent.click(screen.getByRole("button", { name: "common.save" }))
  await waitFor(() => expect(saved).toHaveBeenCalledOnce())
  expect(transport.call).toHaveBeenCalledWith("update_provider", {
    config: {
      ...provider,
      currency: "USD",
      allowPrivateNetwork: false,
      models: [
        model,
        {
          id: "new",
          name: "New",
          inputTypes: ["text"],
          contextWindow: 16000,
          maxTokens: 2000,
          reasoning: false,
          costInput: null,
          costOutput: null,
        },
      ],
    },
  })
  expect(provider).toEqual(before)
  expect(
    transport.call.mock.calls.filter(([command]) => command === "set_active_model"),
  ).toHaveLength(0)
})

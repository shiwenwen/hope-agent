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

// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import ExternalMemoryProviderCredentials from "./ExternalMemoryProviderCredentials"
import type {
  ExternalMemoryProviderConfig,
  ExternalMemoryProviderCredentialStatus,
} from "./types"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: string | ({ defaultValue?: string } & Record<string, unknown>)) => {
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
    },
  }),
}))

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}))

vi.mock("@/lib/logger", () => ({
  logger: { error: vi.fn() },
}))

const transportMock = vi.hoisted(() => ({ call: vi.fn() }))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

const provider: ExternalMemoryProviderConfig = {
  id: "mem0-main",
  kind: "mem0",
  displayName: "Mem0",
  enabled: true,
  syncPolicy: "manual",
  endpointConfigured: false,
  lastSyncAt: null,
  lastError: null,
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("ExternalMemoryProviderCredentials", () => {
  test("stores a Mem0 connection without reading the API key back", async () => {
    const emptyStatus: ExternalMemoryProviderCredentialStatus = {
      providerId: provider.id,
      configured: false,
      endpointConfigured: false,
      apiKeyConfigured: false,
    }
    const savedStatus: ExternalMemoryProviderCredentialStatus = {
      providerId: provider.id,
      configured: true,
      endpointConfigured: true,
      apiKeyConfigured: true,
      endpointOrigin: "https://api.mem0.ai",
      subjectId: "alice",
      protocol: "platform_v3",
      source: "file",
    }
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_external_memory_provider_credential_status") return emptyStatus
      if (command === "save_external_memory_provider_credentials") return savedStatus
      return null
    })
    const onStatusChanged = vi.fn()

    render(
      <ExternalMemoryProviderCredentials
        provider={provider}
        configDirty={false}
        onStatusChanged={onStatusChanged}
      />,
    )

    await waitFor(() =>
      expect(transportMock.call).toHaveBeenCalledWith(
        "get_external_memory_provider_credential_status",
        { providerId: "mem0-main" },
      ),
    )
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /Save connection/i }).hasAttribute("disabled"),
      ).toBe(false),
    )
    fireEvent.change(screen.getByDisplayValue("hope-agent-user"), {
      target: { value: "alice" },
    })
    fireEvent.change(
      screen.getByPlaceholderText("Required for Mem0 Platform; optional for local OSS"),
      { target: { value: "m0-secret" } },
    )
    fireEvent.click(screen.getByRole("button", { name: /Save connection/i }))

    await waitFor(() =>
      expect(transportMock.call).toHaveBeenCalledWith(
        "save_external_memory_provider_credentials",
        {
          providerId: "mem0-main",
          credentials: {
            providerId: "mem0-main",
            endpoint: "https://api.mem0.ai",
            subjectId: "alice",
            protocol: "auto",
            apiKey: "m0-secret",
          },
        },
      ),
    )
    expect(await screen.findByText("Configured via file · https://api.mem0.ai")).toBeTruthy()
    expect(screen.queryByDisplayValue("m0-secret")).toBeNull()
    expect(onStatusChanged).toHaveBeenLastCalledWith("mem0-main", savedStatus)
  })

  test("does not touch credentials while provider config has unsaved changes", () => {
    render(
      <ExternalMemoryProviderCredentials
        provider={provider}
        configDirty
        onStatusChanged={vi.fn()}
      />,
    )

    expect(transportMock.call).not.toHaveBeenCalled()
    expect(screen.getByRole("button", { name: /Save connection/i }).hasAttribute("disabled")).toBe(
      true,
    )
    expect(
      screen.getByText("Save the provider configuration before changing its connection."),
    ).toBeTruthy()
  })
})

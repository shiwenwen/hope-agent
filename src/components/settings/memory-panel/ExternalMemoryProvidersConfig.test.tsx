// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import ExternalMemoryProvidersConfigPanel from "./ExternalMemoryProvidersConfig"
import type {
  ExternalMemoryProviderPreflightReport,
  ExternalMemoryProviderSyncReport,
  ExternalMemoryProvidersConfig as ExternalMemoryProvidersConfigValue,
} from "./types"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (
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
    },
  }),
}))

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
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

describe("ExternalMemoryProvidersConfig", () => {
  test("renders planned adapter state and blocked privacy copy from loaded config", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    })
    const config: ExternalMemoryProvidersConfigValue = {
      enabled: true,
      providers: [
        {
          id: "zep-main",
          kind: "zep",
          displayName: "Zep main",
          enabled: true,
          syncPolicy: "manual",
          endpointConfigured: true,
          lastSyncAt: null,
          lastError: "provider failed token=provider-secret",
        },
      ],
    }
    const preflight: ExternalMemoryProviderPreflightReport = {
      generatedAt: "2026-07-08T00:00:00.000Z",
      globalEnabled: true,
      dryRunOnly: true,
      localMemoryTotal: 12,
      localMemoryWithEmbedding: 8,
      statsUnavailable: true,
      statsError: "stats unavailable token=stats-secret",
      runnableProviderCount: 0,
      blockedProviderCount: 1,
      providers: [
        {
          id: "zep-main",
          kind: "zep",
          displayName: "Zep main",
          action: "blocked",
          dryRunOnly: true,
          plannedDataFlow: "manual",
          runtimeDataFlow: "none",
          plannedSendsQueryContext: true,
          plannedSendsLocalMemory: true,
          plannedImportsExternalMemory: true,
          runtimeSendsQueryContext: false,
          runtimeSendsLocalMemory: false,
          runtimeImportsExternalMemory: false,
          localMemoryCandidateCount: 12,
          health: {
            id: "zep-main",
            kind: "zep",
            displayName: "Zep main",
            enabled: true,
            syncPolicy: "manual",
            status: "warning",
            endpointConfigured: true,
            lastSyncAt: null,
            lastError: "provider failed token=provider-secret",
            runtimeDataFlow: "none",
            runtimeSyncEnabled: false,
            syncBlocked: true,
            syncBlockReasons: ["adapter_unavailable"],
            capabilities: {
              adapterAvailable: false,
              requiresEndpoint: true,
              supportsManual: true,
              supportsPull: true,
              supportsPush: true,
              supportsBidirectional: true,
            },
          },
        },
      ],
    }
    const syncReport: ExternalMemoryProviderSyncReport = {
      generatedAt: "2026-07-08T00:01:00.000Z",
      globalEnabled: true,
      externalIoPerformed: false,
      localMemoryTotal: 12,
      localMemoryWithEmbedding: 8,
      statsUnavailable: false,
      statsError: null,
      runnableProviderCount: 0,
      blockedProviderCount: 1,
      executedProviderCount: 0,
      succeededProviderCount: 0,
      failedProviderCount: 0,
      providers: [
        {
          id: "zep-main",
          kind: "zep",
          displayName: "Zep main",
          status: "blocked",
          externalIoPerformed: false,
          preflight: preflight.providers[0],
          importedMemoryCount: 0,
          exportedMemoryCount: 0,
          updatedMemoryCount: 0,
          skippedMemoryCount: 0,
          error: null,
        },
      ],
    }

    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_external_memory_providers_config") return config
      if (command === "get_external_memory_providers_preflight") return preflight
      if (command === "run_external_memory_provider_sync") return syncReport
      return null
    })

    render(<ExternalMemoryProvidersConfigPanel />)

    fireEvent.click(screen.getByRole("button", { name: /External providers/i }))

    expect(await screen.findByDisplayValue("Zep main")).toBeTruthy()
    expect(screen.getByText("Adapter pending")).toBeTruthy()
    expect(screen.getByText("Dry run: 0 would sync / 1 blocked · 12 local memories")).toBeTruthy()
    expect(
      screen.getByText("Local memory stats could not be loaded; candidate counts may be incomplete."),
    ).toBeTruthy()
    expect(screen.getByText("Details: stats unavailable token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/stats-secret/)).toBeNull()
    expect(screen.getByText("Last error: provider failed token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/provider-secret/)).toBeNull()
    expect(
      screen.getByText("Preflight: blocked · planned manual → runtime none · local candidates 12"),
    ).toBeTruthy()
    expect(
      screen.getByText(
        "No provider data will move until setup, policy, and adapter readiness all pass.",
      ),
    ).toBeTruthy()
    expect(
      screen.getByText("Runtime adapter not shipped yet; this provider is configuration-only."),
    ).toBeTruthy()
    expect(screen.getByText("Blocked by")).toBeTruthy()
    expect(screen.getByText("Adapter unavailable")).toBeTruthy()
    expect(
      screen.getByText("Supported: Manual, Pull only, Push only, Bidirectional"),
    ).toBeTruthy()
    fireEvent.click(screen.getAllByRole("button", { name: /Copy report/i })[0])
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1))
    expect(writeText.mock.calls[0]?.[0]).toContain("# External Memory Provider Sync Preflight")
    expect(writeText.mock.calls[0]?.[0]).toContain("- Action: blocked")
    expect(writeText.mock.calls[0]?.[0]).toContain("- Runtime data flow: none")
    expect(writeText.mock.calls[0]?.[0]).toContain("- Block reasons: adapter_unavailable")
    expect(writeText.mock.calls[0]?.[0]).toContain("- Local memory stats unavailable: yes")
    expect(writeText.mock.calls[0]?.[0]).toContain(
      "- Local memory stats error: stats unavailable token=[redacted]",
    )
    expect(writeText.mock.calls[0]?.[0]).toContain(
      "- Last error: provider failed token=[redacted]",
    )
    expect(writeText.mock.calls[0]?.[0]).not.toContain("stats-secret")
    expect(writeText.mock.calls[0]?.[0]).not.toContain("provider-secret")
    fireEvent.click(screen.getByRole("button", { name: /Run sync/i }))
    await waitFor(() =>
      expect(screen.getByText("Last run: 0 executed / 1 blocked · external IO no")).toBeTruthy(),
    )
    expect(
      screen.getByText(
        "No external IO was performed. Providers were blocked, disabled, or had no changed records.",
      ),
    ).toBeTruthy()
    expect(
      screen.getByText("Sync report: blocked · external IO no · imported 0 / exported 0 / updated 0"),
    ).toBeTruthy()
    fireEvent.click(screen.getAllByRole("button", { name: /Copy report/i })[1])
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(2))
    expect(writeText.mock.calls[1]?.[0]).toContain("# External Memory Provider Sync Report")
    expect(writeText.mock.calls[1]?.[0]).toContain("- External IO performed: no")
    expect(writeText.mock.calls[1]?.[0]).toContain("- Status: blocked")
    expect(writeText.mock.calls[1]?.[0]).toContain("Blocked or unavailable adapters fail closed")
    expect(transportMock.call).toHaveBeenCalledWith("get_external_memory_providers_config")
    expect(transportMock.call).toHaveBeenCalledWith("get_external_memory_providers_preflight")
    expect(transportMock.call).toHaveBeenCalledWith("run_external_memory_provider_sync")
  })
})

import { describe, expect, it } from "vitest"
import {
  externalMemoryProviderKindCapabilities,
  externalMemoryProviderOverview,
  externalMemoryProviderNeedsEndpointSetup,
  externalMemoryProviderPrivacySummary,
  externalMemoryProviderReadiness,
  externalMemoryProviderSyncBlockReasons,
  externalMemoryProviderSyncBlockSummary,
  externalMemoryProviderSupportedSyncPolicies,
  formatExternalMemoryProviderPreflightDiagnostics,
  formatExternalMemoryProviderSyncDiagnostics,
  isExternalMemoryProviderSyncActive,
} from "./externalMemoryProviderReadiness"
import type {
  ExternalMemoryProviderConfig,
  ExternalMemoryProviderHealth,
  ExternalMemoryProviderKind,
  ExternalMemoryProviderPreflightReport,
  ExternalMemoryProviderSyncReport,
} from "./types"

const EXTERNAL_MEMORY_PROVIDER_KINDS: ExternalMemoryProviderKind[] = [
  "mem0",
  "zep",
  "supermemory",
  "honcho",
  "hindsight",
  "open_viking",
  "custom",
]

const UNAVAILABLE_CAPABILITIES = {
  adapterAvailable: false,
  requiresEndpoint: true,
  supportsManual: true,
  supportsPull: true,
  supportsPush: true,
  supportsBidirectional: true,
}

function provider(
  patch: Partial<ExternalMemoryProviderConfig> = {},
): ExternalMemoryProviderConfig {
  return {
    id: "mem0-main",
    kind: "mem0",
    displayName: "Mem0",
    enabled: true,
    syncPolicy: "manual",
    endpointConfigured: false,
    lastSyncAt: null,
    lastError: null,
    ...patch,
  }
}

function healthProvider(
  patch: Partial<ExternalMemoryProviderHealth> = {},
): ExternalMemoryProviderHealth {
  return {
    id: "mem0-main",
    kind: "mem0",
    displayName: "Mem0",
    enabled: true,
    syncPolicy: "manual",
    endpointConfigured: true,
    status: "ok",
    capabilities: {
      adapterAvailable: false,
      requiresEndpoint: true,
      supportsManual: true,
      supportsPull: true,
      supportsPush: true,
      supportsBidirectional: true,
    },
    lastSyncAt: null,
    lastError: null,
    ...patch,
  }
}

describe("external memory provider readiness", () => {
  it("ships a runtime adapter for every registered provider kind", () => {
    for (const kind of EXTERNAL_MEMORY_PROVIDER_KINDS) {
      expect(externalMemoryProviderKindCapabilities(kind)).toEqual({
        adapterAvailable: true,
        requiresEndpoint: true,
        supportsManual: true,
        supportsPull: true,
        supportsPush: true,
        supportsBidirectional: true,
      })
    }
  })

  it("returns provider kind capabilities as a caller-safe copy", () => {
    const capabilities = externalMemoryProviderKindCapabilities("mem0")
    capabilities.supportsPush = false

    expect(externalMemoryProviderKindCapabilities("mem0").supportsPush).toBe(true)
  })

  it("treats sync as active only when global and provider switches allow it", () => {
    expect(isExternalMemoryProviderSyncActive(true, provider())).toBe(true)
    expect(isExternalMemoryProviderSyncActive(false, provider())).toBe(false)
    expect(isExternalMemoryProviderSyncActive(true, provider({ enabled: false }))).toBe(false)
    expect(isExternalMemoryProviderSyncActive(true, provider({ syncPolicy: "off" }))).toBe(false)
  })

  it("requires endpoint setup only for active providers without an endpoint", () => {
    expect(externalMemoryProviderNeedsEndpointSetup(true, provider())).toBe(true)
    expect(
      externalMemoryProviderNeedsEndpointSetup(true, provider({ endpointConfigured: true })),
    ).toBe(false)
    expect(externalMemoryProviderNeedsEndpointSetup(false, provider())).toBe(false)
    expect(
      externalMemoryProviderNeedsEndpointSetup(true, provider({ syncPolicy: "off" })),
    ).toBe(false)
  })

  it("summarizes provider readiness from global sync, provider switch, policy, and endpoint", () => {
    expect(externalMemoryProviderReadiness(false, provider({ endpointConfigured: true }))).toBe(
      "off",
    )
    expect(
      externalMemoryProviderReadiness(true, provider({ enabled: false, endpointConfigured: true })),
    ).toBe("off")
    expect(
      externalMemoryProviderReadiness(
        true,
        provider({ syncPolicy: "off", endpointConfigured: true }),
      ),
    ).toBe("off")
    expect(externalMemoryProviderReadiness(true, provider())).toBe("needs_setup")
    expect(externalMemoryProviderReadiness(true, provider({ endpointConfigured: true }))).toBe(
      "ready",
    )
  })

  it("surfaces unsupported policy in per-provider readiness", () => {
    expect(
      externalMemoryProviderReadiness(true, {
        ...provider({
          endpointConfigured: true,
          syncPolicy: "push_only",
        }),
        capabilities: {
          adapterAvailable: true,
          requiresEndpoint: true,
          supportsManual: true,
          supportsPull: true,
          supportsPush: false,
          supportsBidirectional: false,
        },
      }),
    ).toBe("unsupported_policy")

    expect(
      externalMemoryProviderReadiness(true, {
        ...provider({
          endpointConfigured: true,
          syncPolicy: "manual",
        }),
        policySupported: false,
      }),
    ).toBe("unsupported_policy")
  })

  it("reports ready only when a concrete adapter is available", () => {
    expect(
      externalMemoryProviderOverview(true, [
        healthProvider({
          capabilities: {
            adapterAvailable: true,
            requiresEndpoint: true,
            supportsManual: true,
            supportsPull: true,
            supportsPush: true,
            supportsBidirectional: true,
          },
        }),
      ]),
    ).toEqual({
      state: "ready",
      needsSetupCount: 0,
      unsupportedPolicyCount: 0,
      adapterPendingCount: 0,
      errorCount: 0,
      readyCount: 1,
    })
  })

  it("flags unsupported sync policy before reporting adapter readiness", () => {
    expect(
      externalMemoryProviderOverview(true, [
        healthProvider({
          syncPolicy: "push_only",
          capabilities: {
            adapterAvailable: true,
            requiresEndpoint: true,
            supportsManual: true,
            supportsPull: true,
            supportsPush: false,
            supportsBidirectional: false,
          },
        }),
      ]),
    ).toEqual({
      state: "unsupported_policy",
      needsSetupCount: 0,
      unsupportedPolicyCount: 1,
      adapterPendingCount: 0,
      errorCount: 0,
      readyCount: 0,
    })
  })

  it("projects supported sync policies from adapter capabilities", () => {
    expect(
      externalMemoryProviderSupportedSyncPolicies({
        kind: "mem0",
        capabilities: {
          adapterAvailable: true,
          requiresEndpoint: true,
          supportsManual: true,
          supportsPull: true,
          supportsPush: false,
          supportsBidirectional: false,
        },
      }),
    ).toEqual(["manual", "pull_only"])
  })

  it("explains external provider privacy/data flow only when runtime sync can really run", () => {
    expect(externalMemoryProviderPrivacySummary(false, provider({ endpointConfigured: true }))).toEqual(
      {
        policyDataFlow: "none",
        runtimeDataFlow: "none",
        runtimeSyncEnabled: false,
        syncBlocked: false,
        sendsQueryContext: false,
        sendsLocalMemory: false,
        importsExternalMemory: false,
        requiresExplicitAction: false,
        automaticSync: false,
      },
    )

    expect(
      externalMemoryProviderPrivacySummary(
        true,
        {
          ...provider({ kind: "zep", endpointConfigured: true }),
          capabilities: UNAVAILABLE_CAPABILITIES,
        },
      ),
    ).toMatchObject({
      policyDataFlow: "manual",
      runtimeDataFlow: "none",
      runtimeSyncEnabled: false,
      syncBlocked: true,
      sendsQueryContext: false,
      sendsLocalMemory: false,
      importsExternalMemory: false,
    })

    expect(
      externalMemoryProviderPrivacySummary(true, {
        ...provider({
          endpointConfigured: true,
          syncPolicy: "pull_only",
        }),
        capabilities: {
          adapterAvailable: true,
          requiresEndpoint: true,
          supportsManual: true,
          supportsPull: true,
          supportsPush: true,
          supportsBidirectional: true,
        },
      }),
    ).toMatchObject({
      policyDataFlow: "pull_only",
      runtimeDataFlow: "pull_only",
      runtimeSyncEnabled: true,
      syncBlocked: false,
      sendsQueryContext: true,
      sendsLocalMemory: false,
      importsExternalMemory: true,
      requiresExplicitAction: false,
      automaticSync: true,
    })

    expect(
      externalMemoryProviderPrivacySummary(true, {
        ...provider({
          endpointConfigured: true,
          syncPolicy: "push_only",
        }),
        capabilities: {
          adapterAvailable: true,
          requiresEndpoint: true,
          supportsManual: true,
          supportsPull: true,
          supportsPush: true,
          supportsBidirectional: true,
        },
      }),
    ).toMatchObject({
      runtimeDataFlow: "push_only",
      sendsQueryContext: false,
      sendsLocalMemory: true,
      importsExternalMemory: false,
      automaticSync: true,
    })
  })

  it("prefers backend health data-flow projection when present", () => {
    expect(
      externalMemoryProviderPrivacySummary(true, {
        ...provider({
          endpointConfigured: true,
          syncPolicy: "pull_only",
        }),
        policyDataFlow: "pull_only",
        runtimeDataFlow: "none",
        runtimeSyncEnabled: false,
        syncBlocked: true,
        sendsQueryContext: false,
        sendsLocalMemory: false,
        importsExternalMemory: false,
        requiresExplicitAction: false,
        automaticSync: false,
      }),
    ).toEqual({
      policyDataFlow: "pull_only",
      runtimeDataFlow: "none",
      runtimeSyncEnabled: false,
      syncBlocked: true,
      sendsQueryContext: false,
      sendsLocalMemory: false,
      importsExternalMemory: false,
      requiresExplicitAction: false,
      automaticSync: false,
    })
  })

  it("reports stable sync block reasons and prefers backend reasons when present", () => {
    expect(
      externalMemoryProviderSyncBlockReasons(
        false,
        {
          ...provider({
            kind: "zep",
            endpointConfigured: false,
            lastError: "boom",
          }),
          capabilities: UNAVAILABLE_CAPABILITIES,
        },
      ),
    ).toEqual(["global_disabled", "endpoint_missing", "adapter_unavailable", "last_error"])

    expect(
      externalMemoryProviderSyncBlockReasons(true, {
        ...provider({
          endpointConfigured: true,
          syncPolicy: "push_only",
        }),
        capabilities: {
          adapterAvailable: true,
          requiresEndpoint: true,
          supportsManual: true,
          supportsPull: true,
          supportsPush: false,
          supportsBidirectional: false,
        },
      }),
    ).toEqual(["policy_unsupported"])

    expect(
      externalMemoryProviderSyncBlockReasons(true, {
        ...provider({ endpointConfigured: true }),
        syncBlockReasons: ["adapter_unavailable"],
      }),
    ).toEqual(["adapter_unavailable"])
  })

  it("summarizes sync blockers for overview chips", () => {
    expect(
      externalMemoryProviderSyncBlockSummary(true, [
        {
          ...provider({
            id: "mem0-1",
            kind: "zep",
            endpointConfigured: false,
            lastError: "boom",
          }),
          capabilities: UNAVAILABLE_CAPABILITIES,
        },
        {
          ...provider({
            id: "zep-1",
            endpointConfigured: true,
            lastError: null,
          }),
          syncBlockReasons: ["adapter_unavailable", "adapter_unavailable"],
        },
        provider({
          id: "custom-1",
          enabled: false,
          syncPolicy: "off",
          endpointConfigured: true,
          lastError: null,
        }),
      ]),
    ).toEqual({
      blockedProviderCount: 3,
      reasonCounts: {
        provider_disabled: 1,
        policy_off: 1,
        endpoint_missing: 1,
        adapter_unavailable: 2,
        last_error: 1,
      },
      topReasons: [
        "provider_disabled",
        "policy_off",
        "endpoint_missing",
        "adapter_unavailable",
        "last_error",
      ],
    })
  })

  it("summarizes overview state with local-first warning priority", () => {
    expect(externalMemoryProviderOverview(false, [healthProvider({ lastError: "boom" })])).toEqual({
      state: "off",
      needsSetupCount: 0,
      unsupportedPolicyCount: 0,
      adapterPendingCount: 0,
      errorCount: 0,
      readyCount: 0,
    })

    expect(
      externalMemoryProviderOverview(true, [
        healthProvider({ endpointConfigured: false }),
        healthProvider({ id: "zep-main", kind: "zep", lastError: "boom" }),
        healthProvider({
          id: "custom-main",
          kind: "custom",
          capabilities: {
            adapterAvailable: true,
            requiresEndpoint: true,
            supportsManual: true,
            supportsPull: true,
            supportsPush: true,
            supportsBidirectional: true,
          },
        }),
      ]),
    ).toEqual({
      state: "needs_setup",
      needsSetupCount: 1,
      unsupportedPolicyCount: 0,
      adapterPendingCount: 1,
      errorCount: 0,
      readyCount: 1,
    })

    expect(
      externalMemoryProviderOverview(true, [
        healthProvider({
          lastError: "boom",
          capabilities: {
            adapterAvailable: true,
            requiresEndpoint: true,
            supportsManual: true,
            supportsPull: true,
            supportsPush: true,
            supportsBidirectional: true,
          },
        }),
        healthProvider({ id: "custom-main", kind: "custom", enabled: false }),
      ]),
    ).toEqual({
      state: "error",
      needsSetupCount: 0,
      unsupportedPolicyCount: 0,
      adapterPendingCount: 0,
      errorCount: 1,
      readyCount: 0,
    })

    expect(externalMemoryProviderOverview(true, [healthProvider()])).toEqual({
      state: "adapter_pending",
      needsSetupCount: 0,
      unsupportedPolicyCount: 0,
      adapterPendingCount: 1,
      errorCount: 0,
      readyCount: 0,
    })

    expect(
      externalMemoryProviderOverview(true, [healthProvider({ syncPolicy: "off" })]),
    ).toEqual({
      state: "no_active",
      needsSetupCount: 0,
      unsupportedPolicyCount: 0,
      adapterPendingCount: 0,
      errorCount: 0,
      readyCount: 0,
    })
  })

  it("formats external provider preflight diagnostics with safety notes", () => {
    const report: ExternalMemoryProviderPreflightReport = {
      generatedAt: "2026-07-08T10:00:00.000Z",
      globalEnabled: true,
      dryRunOnly: true,
      localMemoryTotal: 42,
      localMemoryWithEmbedding: 31,
      statsUnavailable: true,
      statsError: "stats failed token=stats-secret",
      runnableProviderCount: 0,
      blockedProviderCount: 1,
      providers: [
        {
          id: "mem0-main?token=id-secret",
          kind: "mem0",
          displayName: "Mem0 api_key=display-secret",
          action: "blocked",
          dryRunOnly: true,
          health: healthProvider({
            id: "mem0-main?token=id-secret",
            displayName: "Mem0 api_key=display-secret",
            endpointConfigured: false,
            lastError:
              "request failed https://api.example.test/sync?token=tok-secret Authorization: Bearer bearer-secret api_key=sk-live-secret sk-testsecret123456",
          }),
          plannedDataFlow: "manual",
          runtimeDataFlow: "none",
          plannedSendsQueryContext: true,
          plannedSendsLocalMemory: true,
          plannedImportsExternalMemory: true,
          runtimeSendsQueryContext: false,
          runtimeSendsLocalMemory: false,
          runtimeImportsExternalMemory: false,
          localMemoryCandidateCount: 42,
        },
      ],
    }

    const markdown = formatExternalMemoryProviderPreflightDiagnostics(report)

    expect(markdown).toContain("# External Memory Provider Sync Preflight")
    expect(markdown).toContain("- Dry run only: yes")
    expect(markdown).toContain("- Global sync enabled: yes")
    expect(markdown).toContain("- Local memories: 42")
    expect(markdown).toContain("- Local memories with embedding: 31")
    expect(markdown).toContain("- Local memory stats unavailable: yes")
    expect(markdown).toContain("- Local memory stats error: stats failed token=[redacted]")
    expect(markdown).toContain("### Mem0 api_key=[redacted] (mem0, mem0-main?token=[redacted])")
    expect(markdown).toContain("- Action: blocked")
    expect(markdown).toContain("- Planned data flow: manual")
    expect(markdown).toContain("- Runtime data flow: none")
    expect(markdown).toContain("- Planned sends query context: yes")
    expect(markdown).toContain("- Runtime sends local memory: no")
    expect(markdown).toContain(
      "- Block reasons: endpoint_missing|adapter_unavailable|last_error",
    )
    expect(markdown).toContain("token=[redacted]")
    expect(markdown).toContain("Authorization: Bearer [redacted]")
    expect(markdown).toContain("api_key=[redacted]")
    expect(markdown).toContain("sk-[redacted]")
    expect(markdown).not.toContain("tok-secret")
    expect(markdown).not.toContain("bearer-secret")
    expect(markdown).not.toContain("display-secret")
    expect(markdown).not.toContain("id-secret")
    expect(markdown).not.toContain("sk-live-secret")
    expect(markdown).not.toContain("sk-testsecret123456")
    expect(markdown).not.toContain("stats-secret")
    expect(markdown).toContain("## Safety Notes")
    expect(markdown).toContain("does not call external provider APIs")
    expect(markdown).toContain("Local SQLite memory remains the source of truth")
    expect(markdown).toContain("local candidate counts are incomplete")
  })

  it("formats empty external provider preflight diagnostics", () => {
    const markdown = formatExternalMemoryProviderPreflightDiagnostics({
      generatedAt: "2026-07-08T10:00:00.000Z",
      globalEnabled: false,
      dryRunOnly: true,
      localMemoryTotal: 0,
      localMemoryWithEmbedding: 0,
      runnableProviderCount: 0,
      blockedProviderCount: 0,
      providers: [],
    })

    expect(markdown).toContain("## Providers\n\n- None")
    expect(markdown).toContain("- Global sync enabled: no")
  })

  it("formats external provider sync diagnostics without implying hidden external IO", () => {
    const report: ExternalMemoryProviderSyncReport = {
      generatedAt: "2026-07-08T10:05:00.000Z",
      globalEnabled: true,
      externalIoPerformed: false,
      localMemoryTotal: 42,
      localMemoryWithEmbedding: 31,
      statsUnavailable: false,
      statsError: null,
      runnableProviderCount: 1,
      blockedProviderCount: 1,
      executedProviderCount: 0,
      succeededProviderCount: 0,
      failedProviderCount: 0,
      providers: [
        {
          id: "mem0-main?token=id-secret",
          kind: "mem0",
          displayName: "Mem0 api_key=display-secret",
          status: "no_runtime_adapter",
          externalIoPerformed: false,
          importedMemoryCount: 0,
          exportedMemoryCount: 0,
          updatedMemoryCount: 0,
          skippedMemoryCount: 0,
          error: "adapter missing token=runtime-secret",
          preflight: {
            id: "mem0-main?token=id-secret",
            kind: "mem0",
            displayName: "Mem0 api_key=display-secret",
            action: "would_sync",
            dryRunOnly: true,
            health: healthProvider({
              id: "mem0-main?token=id-secret",
              displayName: "Mem0 api_key=display-secret",
              capabilities: {
                adapterAvailable: true,
                requiresEndpoint: true,
                supportsManual: true,
                supportsPull: true,
                supportsPush: true,
                supportsBidirectional: true,
              },
            }),
            plannedDataFlow: "manual",
            runtimeDataFlow: "manual",
            plannedSendsQueryContext: true,
            plannedSendsLocalMemory: true,
            plannedImportsExternalMemory: true,
            runtimeSendsQueryContext: true,
            runtimeSendsLocalMemory: true,
            runtimeImportsExternalMemory: true,
            localMemoryCandidateCount: 42,
          },
        },
      ],
    }

    const markdown = formatExternalMemoryProviderSyncDiagnostics(report)

    expect(markdown).toContain("# External Memory Provider Sync Report")
    expect(markdown).toContain("- External IO performed: no")
    expect(markdown).toContain("- Providers runnable by preflight: 1")
    expect(markdown).toContain("- Executed providers: 0")
    expect(markdown).toContain("- Status: no_runtime_adapter")
    expect(markdown).toContain("- Preflight action: would_sync")
    expect(markdown).toContain("- Imported memories: 0")
    expect(markdown).toContain("- Error: adapter missing token=[redacted]")
    expect(markdown).toContain("no runtime adapter owns execution yet")
    expect(markdown).not.toContain("runtime-secret")
    expect(markdown).not.toContain("display-secret")
    expect(markdown).not.toContain("id-secret")
  })
})

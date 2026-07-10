import type {
  ExternalMemoryProviderCapabilities,
  ExternalMemoryProviderConfig,
  ExternalMemoryProviderDataFlow,
  ExternalMemoryProviderHealth,
  ExternalMemoryProviderKind,
  ExternalMemoryProviderPreflightReport,
  ExternalMemoryProviderSyncReport,
  ExternalMemoryProviderSyncBlockReason,
  ExternalMemorySyncPolicy,
} from "./types"
import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type ExternalMemoryProviderReadiness =
  | "off"
  | "needs_setup"
  | "unsupported_policy"
  | "adapter_pending"
  | "ready"
export type ExternalMemoryProviderOverviewState =
  | "off"
  | "needs_setup"
  | "unsupported_policy"
  | "adapter_pending"
  | "error"
  | "ready"
  | "no_active"

export interface ExternalMemoryProviderOverview {
  state: ExternalMemoryProviderOverviewState
  needsSetupCount: number
  unsupportedPolicyCount: number
  adapterPendingCount: number
  errorCount: number
  readyCount: number
}

export interface ExternalMemoryProviderPrivacySummary {
  policyDataFlow: ExternalMemoryProviderDataFlow
  runtimeDataFlow: ExternalMemoryProviderDataFlow
  runtimeSyncEnabled: boolean
  syncBlocked: boolean
  sendsQueryContext: boolean
  sendsLocalMemory: boolean
  importsExternalMemory: boolean
  requiresExplicitAction: boolean
  automaticSync: boolean
}

export interface ExternalMemoryProviderSyncBlockSummary {
  blockedProviderCount: number
  reasonCounts: Partial<Record<ExternalMemoryProviderSyncBlockReason, number>>
  topReasons: ExternalMemoryProviderSyncBlockReason[]
}

const PREFLIGHT_DIAGNOSTIC_MAX_CHARS = 420

function yesNo(value: boolean): string {
  return value ? "yes" : "no"
}

function optional(value: string | number | null | undefined): string {
  if (value === null || value === undefined || value === "") return "-"
  return String(value)
}

export function externalMemoryProviderDiagnosticText(
  value: string,
  maxChars = PREFLIGHT_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

function optionalDiagnostic(value: string | null | undefined): string {
  if (value === null || value === undefined || value.trim() === "") return "-"
  return externalMemoryProviderDiagnosticText(value)
}

export const DEFAULT_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES: ExternalMemoryProviderCapabilities = {
  adapterAvailable: false,
  requiresEndpoint: true,
  supportsManual: false,
  supportsPull: false,
  supportsPush: false,
  supportsBidirectional: false,
}

const SHIPPED_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES: ExternalMemoryProviderCapabilities = {
  adapterAvailable: true,
  requiresEndpoint: true,
  supportsManual: true,
  supportsPull: true,
  supportsPush: true,
  supportsBidirectional: true,
}

export const EXTERNAL_MEMORY_PROVIDER_KIND_CAPABILITIES: Readonly<Record<
  ExternalMemoryProviderKind,
  Readonly<ExternalMemoryProviderCapabilities>
>> = {
  mem0: { ...SHIPPED_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES },
  zep: { ...SHIPPED_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES },
  supermemory: { ...SHIPPED_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES },
  honcho: { ...SHIPPED_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES },
  hindsight: { ...SHIPPED_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES },
  open_viking: { ...SHIPPED_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES },
  custom: { ...SHIPPED_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES },
}

export function externalMemoryProviderKindCapabilities(
  kind: ExternalMemoryProviderKind,
): ExternalMemoryProviderCapabilities {
  return {
    ...(EXTERNAL_MEMORY_PROVIDER_KIND_CAPABILITIES[kind] ??
      DEFAULT_EXTERNAL_MEMORY_PROVIDER_CAPABILITIES),
  }
}

export function externalMemoryProviderCapabilities(
  provider:
    | Pick<ExternalMemoryProviderConfig, "kind">
    | Pick<ExternalMemoryProviderHealth, "kind" | "capabilities">,
): ExternalMemoryProviderCapabilities {
  const base = externalMemoryProviderKindCapabilities(provider.kind)
  if (!("capabilities" in provider) || !provider.capabilities) return base
  return { ...base, ...provider.capabilities }
}

type ExternalMemoryProviderPolicySupportInput = Pick<
  ExternalMemoryProviderConfig,
  "kind" | "syncPolicy"
> &
  Partial<Pick<ExternalMemoryProviderHealth, "capabilities" | "policySupported">>

export function externalMemoryProviderPolicySupported(
  provider: ExternalMemoryProviderPolicySupportInput,
): boolean {
  if (typeof provider.policySupported === "boolean") return provider.policySupported
  const capabilities = externalMemoryProviderCapabilities(provider)
  if (provider.syncPolicy === "off") return true
  if (provider.syncPolicy === "manual") return capabilities.supportsManual
  if (provider.syncPolicy === "pull_only") return capabilities.supportsPull
  if (provider.syncPolicy === "push_only") return capabilities.supportsPush
  if (provider.syncPolicy === "bidirectional") return capabilities.supportsBidirectional
  return false
}

const SYNC_POLICY_CAPABILITY_ORDER: ExternalMemorySyncPolicy[] = [
  "manual",
  "pull_only",
  "push_only",
  "bidirectional",
]

export const EXTERNAL_MEMORY_PROVIDER_SYNC_BLOCK_REASON_ORDER: ExternalMemoryProviderSyncBlockReason[] =
  [
    "global_disabled",
    "provider_disabled",
    "policy_off",
    "endpoint_missing",
    "policy_unsupported",
    "adapter_unavailable",
    "last_error",
  ]

export function externalMemoryProviderSupportedSyncPolicies(
  provider:
    | Pick<ExternalMemoryProviderConfig, "kind">
    | Pick<ExternalMemoryProviderHealth, "kind" | "capabilities">,
): ExternalMemorySyncPolicy[] {
  return SYNC_POLICY_CAPABILITY_ORDER.filter((syncPolicy) =>
    externalMemoryProviderPolicySupported({ ...provider, syncPolicy }),
  )
}

export function isExternalMemoryProviderSyncActive(
  globalEnabled: boolean,
  provider: Pick<ExternalMemoryProviderConfig, "enabled" | "syncPolicy">,
): boolean {
  return globalEnabled && provider.enabled && provider.syncPolicy !== "off"
}

export function externalMemoryProviderReadiness(
  globalEnabled: boolean,
  provider: Pick<
    ExternalMemoryProviderConfig,
    "kind" | "enabled" | "syncPolicy" | "endpointConfigured"
  > &
    Partial<Pick<ExternalMemoryProviderHealth, "capabilities" | "policySupported">>,
): ExternalMemoryProviderReadiness {
  if (!isExternalMemoryProviderSyncActive(globalEnabled, provider)) return "off"
  const capabilities = externalMemoryProviderCapabilities(provider)
  if (capabilities.requiresEndpoint && !provider.endpointConfigured) return "needs_setup"
  if (!externalMemoryProviderPolicySupported(provider)) return "unsupported_policy"
  return capabilities.adapterAvailable ? "ready" : "adapter_pending"
}

function syncPolicyDataFlow(syncPolicy: ExternalMemorySyncPolicy): ExternalMemoryProviderDataFlow {
  return syncPolicy === "off" ? "none" : syncPolicy
}

function policySendsQueryContext(syncPolicy: ExternalMemorySyncPolicy): boolean {
  return syncPolicy === "manual" || syncPolicy === "pull_only" || syncPolicy === "bidirectional"
}

function policySendsLocalMemory(syncPolicy: ExternalMemorySyncPolicy): boolean {
  return syncPolicy === "manual" || syncPolicy === "push_only" || syncPolicy === "bidirectional"
}

function policyImportsExternalMemory(syncPolicy: ExternalMemorySyncPolicy): boolean {
  return syncPolicy === "manual" || syncPolicy === "pull_only" || syncPolicy === "bidirectional"
}

export function externalMemoryProviderPrivacySummary(
  globalEnabled: boolean,
  provider: Pick<
    ExternalMemoryProviderConfig,
    "kind" | "enabled" | "syncPolicy" | "endpointConfigured"
  > &
    Partial<
      Pick<
        ExternalMemoryProviderHealth,
        | "capabilities"
        | "policySupported"
        | "policyDataFlow"
        | "runtimeDataFlow"
        | "runtimeSyncEnabled"
        | "syncBlocked"
        | "sendsQueryContext"
        | "sendsLocalMemory"
        | "importsExternalMemory"
        | "requiresExplicitAction"
        | "automaticSync"
      >
    >,
): ExternalMemoryProviderPrivacySummary {
  const syncActive = isExternalMemoryProviderSyncActive(globalEnabled, provider)
  const readiness = externalMemoryProviderReadiness(globalEnabled, provider)
  const runtimeSyncEnabled = readiness === "ready"
  const policyDataFlow = syncActive ? syncPolicyDataFlow(provider.syncPolicy) : "none"
  const runtimeDataFlow = runtimeSyncEnabled ? policyDataFlow : "none"
  const computed: ExternalMemoryProviderPrivacySummary = {
    policyDataFlow,
    runtimeDataFlow,
    runtimeSyncEnabled,
    syncBlocked: syncActive && !runtimeSyncEnabled,
    sendsQueryContext: runtimeSyncEnabled && policySendsQueryContext(provider.syncPolicy),
    sendsLocalMemory: runtimeSyncEnabled && policySendsLocalMemory(provider.syncPolicy),
    importsExternalMemory: runtimeSyncEnabled && policyImportsExternalMemory(provider.syncPolicy),
    requiresExplicitAction: runtimeSyncEnabled && provider.syncPolicy === "manual",
    automaticSync:
      runtimeSyncEnabled &&
      (provider.syncPolicy === "pull_only" ||
        provider.syncPolicy === "push_only" ||
        provider.syncPolicy === "bidirectional"),
  }

  return {
    policyDataFlow: provider.policyDataFlow ?? computed.policyDataFlow,
    runtimeDataFlow: provider.runtimeDataFlow ?? computed.runtimeDataFlow,
    runtimeSyncEnabled: provider.runtimeSyncEnabled ?? computed.runtimeSyncEnabled,
    syncBlocked: provider.syncBlocked ?? computed.syncBlocked,
    sendsQueryContext: provider.sendsQueryContext ?? computed.sendsQueryContext,
    sendsLocalMemory: provider.sendsLocalMemory ?? computed.sendsLocalMemory,
    importsExternalMemory: provider.importsExternalMemory ?? computed.importsExternalMemory,
    requiresExplicitAction: provider.requiresExplicitAction ?? computed.requiresExplicitAction,
    automaticSync: provider.automaticSync ?? computed.automaticSync,
  }
}

export function externalMemoryProviderSyncBlockReasons(
  globalEnabled: boolean,
  provider: Pick<
    ExternalMemoryProviderConfig,
    "kind" | "enabled" | "syncPolicy" | "endpointConfigured" | "lastError"
  > &
    Partial<
      Pick<
        ExternalMemoryProviderHealth,
        "capabilities" | "policySupported" | "syncBlockReasons"
      >
    >,
): ExternalMemoryProviderSyncBlockReason[] {
  if (provider.syncBlockReasons) return provider.syncBlockReasons

  const reasons: ExternalMemoryProviderSyncBlockReason[] = []
  if (!globalEnabled) reasons.push("global_disabled")
  if (!provider.enabled) reasons.push("provider_disabled")
  if (provider.syncPolicy === "off") reasons.push("policy_off")
  if (provider.syncPolicy !== "off") {
    const capabilities = externalMemoryProviderCapabilities(provider)
    if (capabilities.requiresEndpoint && !provider.endpointConfigured) {
      reasons.push("endpoint_missing")
    }
    if (!externalMemoryProviderPolicySupported(provider)) {
      reasons.push("policy_unsupported")
    }
    if (!capabilities.adapterAvailable) {
      reasons.push("adapter_unavailable")
    }
    if (provider.lastError) {
      reasons.push("last_error")
    }
  }
  return reasons
}

export function externalMemoryProviderSyncBlockSummary(
  globalEnabled: boolean,
  providers: (Pick<
    ExternalMemoryProviderConfig,
    "kind" | "enabled" | "syncPolicy" | "endpointConfigured" | "lastError"
  > &
    Partial<
      Pick<
        ExternalMemoryProviderHealth,
        "capabilities" | "policySupported" | "syncBlockReasons"
      >
    >)[],
): ExternalMemoryProviderSyncBlockSummary {
  const reasonCounts: Partial<Record<ExternalMemoryProviderSyncBlockReason, number>> = {}
  let blockedProviderCount = 0

  for (const provider of providers) {
    const uniqueReasons = new Set(externalMemoryProviderSyncBlockReasons(globalEnabled, provider))
    if (uniqueReasons.size === 0) continue
    blockedProviderCount += 1
    for (const reason of uniqueReasons) {
      reasonCounts[reason] = (reasonCounts[reason] ?? 0) + 1
    }
  }

  return {
    blockedProviderCount,
    reasonCounts,
    topReasons: EXTERNAL_MEMORY_PROVIDER_SYNC_BLOCK_REASON_ORDER.filter(
      (reason) => (reasonCounts[reason] ?? 0) > 0,
    ),
  }
}

export function externalMemoryProviderNeedsEndpointSetup(
  globalEnabled: boolean,
  provider: Pick<
    ExternalMemoryProviderConfig,
    "kind" | "enabled" | "syncPolicy" | "endpointConfigured"
  > &
    Partial<Pick<ExternalMemoryProviderHealth, "capabilities" | "policySupported">>,
): boolean {
  return externalMemoryProviderReadiness(globalEnabled, provider) === "needs_setup"
}

export function externalMemoryProviderOverview(
  globalEnabled: boolean,
  providers: Pick<
    ExternalMemoryProviderHealth,
    | "kind"
    | "enabled"
    | "syncPolicy"
    | "endpointConfigured"
    | "capabilities"
    | "policySupported"
    | "lastError"
  >[],
): ExternalMemoryProviderOverview {
  if (!globalEnabled) {
    return {
      state: "off",
      needsSetupCount: 0,
      unsupportedPolicyCount: 0,
      adapterPendingCount: 0,
      errorCount: 0,
      readyCount: 0,
    }
  }

  let needsSetupCount = 0
  let unsupportedPolicyCount = 0
  let adapterPendingCount = 0
  let errorCount = 0
  let readyCount = 0

  for (const provider of providers) {
    if (!provider.enabled || provider.syncPolicy === "off") continue
    const capabilities = externalMemoryProviderCapabilities(provider)
    if (capabilities.requiresEndpoint && !provider.endpointConfigured) {
      needsSetupCount += 1
    } else if (!externalMemoryProviderPolicySupported(provider)) {
      unsupportedPolicyCount += 1
    } else if (!capabilities.adapterAvailable) {
      adapterPendingCount += 1
    } else if (provider.lastError) {
      errorCount += 1
    } else {
      readyCount += 1
    }
  }

  return {
    state:
      needsSetupCount > 0
        ? "needs_setup"
        : unsupportedPolicyCount > 0
          ? "unsupported_policy"
          : adapterPendingCount > 0
            ? "adapter_pending"
            : errorCount > 0
              ? "error"
              : readyCount > 0
                ? "ready"
                : "no_active",
    needsSetupCount,
    unsupportedPolicyCount,
    adapterPendingCount,
    errorCount,
    readyCount,
  }
}

export function formatExternalMemoryProviderPreflightDiagnostics(
  report: ExternalMemoryProviderPreflightReport,
): string {
  const lines: string[] = [
    "# External Memory Provider Sync Preflight",
    "",
    `- Generated at: ${report.generatedAt}`,
    `- Dry run only: ${yesNo(report.dryRunOnly)}`,
    `- Global sync enabled: ${yesNo(report.globalEnabled)}`,
    `- Local memories: ${report.localMemoryTotal}`,
    `- Local memories with embedding: ${report.localMemoryWithEmbedding}`,
    `- Local memory stats unavailable: ${yesNo(report.statsUnavailable ?? false)}`,
    `- Local memory stats error: ${optionalDiagnostic(report.statsError)}`,
    `- Providers that would sync: ${report.runnableProviderCount}`,
    `- Blocked providers: ${report.blockedProviderCount}`,
    "",
    "## Providers",
    "",
  ]

  if (report.providers.length === 0) {
    lines.push("- None")
  } else {
    for (const provider of report.providers) {
      const health = provider.health
      const reasons = externalMemoryProviderSyncBlockReasons(report.globalEnabled, health)
      const capabilities = externalMemoryProviderCapabilities(health)
      lines.push(
        `### ${optionalDiagnostic(provider.displayName)} (${provider.kind}, ${optionalDiagnostic(provider.id)})`,
        "",
        `- Action: ${provider.action}`,
        `- Dry run only: ${yesNo(provider.dryRunOnly)}`,
        `- Health status: ${health.status}`,
        `- Enabled: ${yesNo(health.enabled)}`,
        `- Sync policy: ${health.syncPolicy}`,
        `- Endpoint configured: ${yesNo(health.endpointConfigured)}`,
        `- Adapter available: ${yesNo(capabilities.adapterAvailable)}`,
        `- Policy supported: ${yesNo(externalMemoryProviderPolicySupported(health))}`,
        `- Planned data flow: ${provider.plannedDataFlow}`,
        `- Runtime data flow: ${provider.runtimeDataFlow}`,
        `- Planned sends query context: ${yesNo(provider.plannedSendsQueryContext)}`,
        `- Planned sends local memory: ${yesNo(provider.plannedSendsLocalMemory)}`,
        `- Planned imports external memory: ${yesNo(provider.plannedImportsExternalMemory)}`,
        `- Runtime sends query context: ${yesNo(provider.runtimeSendsQueryContext)}`,
        `- Runtime sends local memory: ${yesNo(provider.runtimeSendsLocalMemory)}`,
        `- Runtime imports external memory: ${yesNo(provider.runtimeImportsExternalMemory)}`,
        `- Local memory candidates: ${provider.localMemoryCandidateCount}`,
        `- Block reasons: ${reasons.join("|") || "-"}`,
        `- Last sync: ${optional(health.lastSyncAt)}`,
        `- Last error: ${optionalDiagnostic(health.lastError)}`,
        "",
      )
    }
  }

  lines.push(
    "",
    "## Safety Notes",
    "",
    "- This report is generated from an owner-only dry run payload.",
    "- The preflight does not call external provider APIs, read provider data, write provider data, or change local memory.",
    "- Local SQLite memory remains the source of truth; external providers are additive.",
    "- If local memory stats are unavailable, local candidate counts are incomplete and should not be treated as zero memories.",
    "- Runtime data flow stays `none` while setup, policy, or adapter readiness is blocked.",
  )

  return lines.join("\n")
}

export function formatExternalMemoryProviderSyncDiagnostics(
  report: ExternalMemoryProviderSyncReport,
): string {
  const lines: string[] = [
    "# External Memory Provider Sync Report",
    "",
    `- Generated at: ${report.generatedAt}`,
    `- Global sync enabled: ${yesNo(report.globalEnabled)}`,
    `- External IO performed: ${yesNo(report.externalIoPerformed)}`,
    `- Local memories: ${report.localMemoryTotal}`,
    `- Local memories with embedding: ${report.localMemoryWithEmbedding}`,
    `- Local memory stats unavailable: ${yesNo(report.statsUnavailable ?? false)}`,
    `- Local memory stats error: ${optionalDiagnostic(report.statsError)}`,
    `- Providers runnable by preflight: ${report.runnableProviderCount}`,
    `- Blocked providers: ${report.blockedProviderCount}`,
    `- Executed providers: ${report.executedProviderCount}`,
    `- Succeeded providers: ${report.succeededProviderCount}`,
    `- Failed providers: ${report.failedProviderCount}`,
    "",
    "## Providers",
    "",
  ]

  if (report.providers.length === 0) {
    lines.push("- None")
  } else {
    for (const provider of report.providers) {
      const preflight = provider.preflight
      const health = preflight.health
      const reasons = externalMemoryProviderSyncBlockReasons(report.globalEnabled, health)
      lines.push(
        `### ${optionalDiagnostic(provider.displayName)} (${provider.kind}, ${optionalDiagnostic(provider.id)})`,
        "",
        `- Status: ${provider.status}`,
        `- External IO performed: ${yesNo(provider.externalIoPerformed)}`,
        `- Preflight action: ${preflight.action}`,
        `- Planned data flow: ${preflight.plannedDataFlow}`,
        `- Runtime data flow: ${preflight.runtimeDataFlow}`,
        `- Local memory candidates: ${preflight.localMemoryCandidateCount}`,
        `- Imported memories: ${provider.importedMemoryCount}`,
        `- Exported memories: ${provider.exportedMemoryCount}`,
        `- Updated memories: ${provider.updatedMemoryCount}`,
        `- Skipped memories: ${provider.skippedMemoryCount}`,
        `- Block reasons: ${reasons.join("|") || "-"}`,
        `- Error: ${optionalDiagnostic(provider.error)}`,
        "",
      )
    }
  }

  lines.push(
    "",
    "## Safety Notes",
    "",
    "- Blocked or unavailable adapters fail closed; configuration alone never implies external IO.",
    "- A `no_runtime_adapter` result means preflight would allow sync, but no runtime adapter owns execution yet.",
    "- Local SQLite memory remains the source of truth; future external providers must stay additive.",
    "- If external IO is `no`, imported/exported/updated counts are informational zeros, not evidence of provider state.",
  )

  return lines.join("\n")
}

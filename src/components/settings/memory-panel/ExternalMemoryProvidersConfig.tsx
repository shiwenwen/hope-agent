import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import {
  AlertTriangle,
  Check,
  ChevronRight,
  Clock3,
  Cloud,
  Copy,
  Loader2,
  Plus,
  Save,
  ShieldCheck,
  Trash2,
} from "lucide-react"
import type {
  ExternalMemoryProviderConfig,
  ExternalMemoryProviderCredentialStatus,
  ExternalMemoryProviderDataFlow,
  ExternalMemoryProviderKind,
  ExternalMemoryProviderPreflightReport,
  ExternalMemoryProviderSyncReport,
  ExternalMemoryProvidersConfig as ExternalMemoryProvidersConfigValue,
  ExternalMemorySyncPolicy,
} from "./types"
import {
  externalMemoryProviderCapabilities,
  externalMemoryProviderPolicySupported,
  externalMemoryProviderPrivacySummary,
  externalMemoryProviderReadiness as getExternalMemoryProviderReadiness,
  externalMemoryProviderNeedsEndpointSetup,
  externalMemoryProviderSyncBlockReasons,
  externalMemoryProviderSupportedSyncPolicies,
  externalMemoryProviderDiagnosticText,
  formatExternalMemoryProviderPreflightDiagnostics,
  formatExternalMemoryProviderSyncDiagnostics,
  isExternalMemoryProviderSyncActive,
} from "./externalMemoryProviderReadiness"
import { externalMemoryProviderSyncBlockReasonLabel } from "./externalMemoryProviderLabels"
import { externalMemoryProviderOperationErrorToast } from "./externalMemoryProviderOperationFeedback"
import ExternalMemoryProviderCredentials from "./ExternalMemoryProviderCredentials"

const PROVIDER_KINDS: ExternalMemoryProviderKind[] = [
  "mem0",
  "zep",
  "supermemory",
  "honcho",
  "hindsight",
  "open_viking",
  "custom",
]

const SYNC_POLICIES: ExternalMemorySyncPolicy[] = [
  "off",
  "manual",
  "pull_only",
  "push_only",
  "bidirectional",
]

const EMPTY_CONFIG: ExternalMemoryProvidersConfigValue = {
  enabled: false,
  providers: [],
}

function providerKindLabel(kind: ExternalMemoryProviderKind): string {
  if (kind === "open_viking") return "OpenViking"
  if (kind === "mem0") return "Mem0"
  return kind
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ")
}

function providerId(kind: ExternalMemoryProviderKind, index: number): string {
  return `${kind.replace(/_/g, "-")}-${index + 1}`
}

function createDraftProvider(existingCount: number): ExternalMemoryProviderConfig {
  const kind: ExternalMemoryProviderKind = "custom"
  return {
    id: providerId(kind, existingCount),
    kind,
    displayName: providerKindLabel(kind),
    enabled: false,
    syncPolicy: "off",
    endpointConfigured: false,
    lastSyncAt: null,
    lastError: null,
  }
}

function configsEqual(
  left: ExternalMemoryProvidersConfigValue,
  right: ExternalMemoryProvidersConfigValue,
): boolean {
  return JSON.stringify(left) === JSON.stringify(right)
}

function formatProviderTimestamp(value?: string | null): string | null {
  if (!value) return null
  const parsed = Date.parse(value)
  if (!Number.isFinite(parsed)) return null
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(parsed))
}

function dataFlowLabel(flow: ExternalMemoryProviderDataFlow): string {
  return flow.replace(/_/g, " ")
}

export default function ExternalMemoryProvidersConfig() {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [config, setConfig] = useState<ExternalMemoryProvidersConfigValue>(EMPTY_CONFIG)
  const [original, setOriginal] = useState<ExternalMemoryProvidersConfigValue>(EMPTY_CONFIG)
  const [preflight, setPreflight] = useState<ExternalMemoryProviderPreflightReport | null>(null)
  const [preflightError, setPreflightError] = useState<string | null>(null)
  const [preflightLoading, setPreflightLoading] = useState(false)
  const [syncReport, setSyncReport] = useState<ExternalMemoryProviderSyncReport | null>(null)
  const [syncError, setSyncError] = useState<string | null>(null)
  const [syncLoading, setSyncLoading] = useState(false)
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const refreshPreflight = useCallback(async () => {
    setPreflightLoading(true)
    try {
      const report = await getTransport().call<ExternalMemoryProviderPreflightReport>(
        "get_external_memory_providers_preflight",
      )
      setPreflight(report ?? null)
      setPreflightError(null)
    } catch (e) {
      logger.error(
        "settings",
        "ExternalMemoryProvidersConfig::preflight",
        "Failed to load external memory provider preflight",
        e,
      )
      const failure = externalMemoryProviderOperationErrorToast("preflight", t, e)
      setPreflight(null)
      setPreflightError(
        failure.description ? `${failure.title}\n${failure.description}` : failure.title,
      )
    } finally {
      setPreflightLoading(false)
    }
  }, [t])

  const copyPreflightDiagnostics = useCallback(async () => {
    if (!preflight) return
    try {
      await navigator.clipboard.writeText(
        formatExternalMemoryProviderPreflightDiagnostics(preflight),
      )
      toast.success(t("common.copied", "Copied"))
    } catch (e) {
      logger.warn(
        "settings",
        "ExternalMemoryProvidersConfig::copyPreflightDiagnostics",
        "Failed to copy external memory provider preflight diagnostics",
        e,
      )
      const failureToast = externalMemoryProviderOperationErrorToast(
        "copyPreflightDiagnostics",
        t,
        e,
      )
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }, [preflight, t])

  const runSyncCheck = useCallback(async () => {
    setSyncLoading(true)
    try {
      const report = await getTransport().call<ExternalMemoryProviderSyncReport>(
        "run_external_memory_provider_sync",
      )
      setSyncReport(report ?? null)
      setSyncError(null)
    } catch (e) {
      logger.error(
        "settings",
        "ExternalMemoryProvidersConfig::sync",
        "Failed to run external memory provider sync check",
        e,
      )
      const failure = externalMemoryProviderOperationErrorToast("sync", t, e)
      setSyncReport(null)
      setSyncError(failure.description ? `${failure.title}\n${failure.description}` : failure.title)
    } finally {
      setSyncLoading(false)
    }
  }, [t])

  const copySyncDiagnostics = useCallback(async () => {
    if (!syncReport) return
    try {
      await navigator.clipboard.writeText(formatExternalMemoryProviderSyncDiagnostics(syncReport))
      toast.success(t("common.copied", "Copied"))
    } catch (e) {
      logger.warn(
        "settings",
        "ExternalMemoryProvidersConfig::copySyncDiagnostics",
        "Failed to copy external memory provider sync diagnostics",
        e,
      )
      const failureToast = externalMemoryProviderOperationErrorToast("copySyncDiagnostics", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }, [syncReport, t])

  const load = useCallback(async () => {
    try {
      const next = await getTransport().call<ExternalMemoryProvidersConfigValue>(
        "get_external_memory_providers_config",
      )
      setConfig(next ?? EMPTY_CONFIG)
      setOriginal(next ?? EMPTY_CONFIG)
      await refreshPreflight()
    } catch (e) {
      logger.error(
        "settings",
        "ExternalMemoryProvidersConfig::load",
        "Failed to load external memory providers",
        e,
      )
      const failureToast = externalMemoryProviderOperationErrorToast("load", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setLoaded(true)
    }
  }, [refreshPreflight, t])

  useEffect(() => {
    void load()
  }, [load])

  const dirty = useMemo(() => loaded && !configsEqual(config, original), [config, loaded, original])
  const preflightByProviderId = useMemo(
    () => new Map(preflight?.providers.map((provider) => [provider.id, provider]) ?? []),
    [preflight],
  )
  const syncResultByProviderId = useMemo(
    () => new Map(syncReport?.providers.map((provider) => [provider.id, provider]) ?? []),
    [syncReport],
  )
  const activeCount = config.providers.filter((provider) =>
    isExternalMemoryProviderSyncActive(config.enabled, provider),
  ).length
  const needsSetupCount = config.providers.filter(
    (provider) => externalMemoryProviderNeedsEndpointSetup(config.enabled, provider),
  ).length
  const syncPolicyLabels = useMemo<Record<ExternalMemorySyncPolicy, string>>(
    () => ({
      off: t("settings.memoryExternalPolicyOff", "Off"),
      manual: t("settings.memoryExternalPolicyManual", "Manual"),
      pull_only: t("settings.memoryExternalPolicyPullOnly", "Pull only"),
      push_only: t("settings.memoryExternalPolicyPushOnly", "Push only"),
      bidirectional: t("settings.memoryExternalPolicyBidirectional", "Bidirectional"),
    }),
    [t],
  )
  const syncPolicyDescriptions = useMemo<Record<ExternalMemorySyncPolicy, string>>(
    () => ({
      off: t("settings.memoryExternalPolicyOffDesc", "Disabled for this provider."),
      manual: t(
        "settings.memoryExternalPolicyManualDesc",
        "Sync only when you explicitly start it.",
      ),
      pull_only: t(
        "settings.memoryExternalPolicyPullOnlyDesc",
        "Read external memory into Hope Agent's review queue only.",
      ),
      push_only: t(
        "settings.memoryExternalPolicyPushOnlyDesc",
        "Send selected Hope Agent memory outward only.",
      ),
      bidirectional: t(
        "settings.memoryExternalPolicyBidirectionalDesc",
        "Reconcile both directions after explicit setup.",
      ),
    }),
    [t],
  )

  const patchProvider = (id: string, patch: Partial<ExternalMemoryProviderConfig>) => {
    setConfig((prev) => ({
      ...prev,
      providers: prev.providers.map((provider) =>
        provider.id === id ? { ...provider, ...patch } : provider,
      ),
    }))
  }

  const handleCredentialStatusChanged = useCallback(
    (providerId: string, status: ExternalMemoryProviderCredentialStatus | null) => {
      const endpointConfigured = status?.endpointConfigured ?? false
      const apply = (value: ExternalMemoryProvidersConfigValue) => ({
        ...value,
        providers: value.providers.map((provider) =>
          provider.id === providerId ? { ...provider, endpointConfigured } : provider,
        ),
      })
      setConfig(apply)
      setOriginal(apply)
      void refreshPreflight()
    },
    [refreshPreflight],
  )

  const handleSave = async () => {
    setSaving(true)
    try {
      await getTransport().call("save_external_memory_providers_config", { config })
      const saved = await getTransport().call<ExternalMemoryProvidersConfigValue>(
        "get_external_memory_providers_config",
      )
      setConfig(saved ?? EMPTY_CONFIG)
      setOriginal(saved ?? EMPTY_CONFIG)
      await refreshPreflight()
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error(
        "settings",
        "ExternalMemoryProvidersConfig::save",
        "Failed to save external memory providers",
        e,
      )
      setSaveStatus("failed")
      const failureToast = externalMemoryProviderOperationErrorToast("save", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="mt-6 pt-4 border-t border-border/50">
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setExpanded(!expanded)}
        className="h-auto -ml-2 gap-1 px-2 py-1 text-sm font-medium text-muted-foreground hover:bg-transparent hover:text-foreground"
      >
        <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-90")} />
        <Cloud className="h-3.5 w-3.5 mr-0.5" />
        {t("settings.memoryExternalProviders", "External providers")}
      </Button>

      {expanded && (
        <div className="mt-3 space-y-4">
          <p className="text-xs text-muted-foreground">
            {t(
              "settings.memoryExternalProvidersDesc",
              "Optional additive memory providers. Local memory remains the source of truth; connections are disabled until you explicitly configure and enable them.",
            )}
          </p>

          <div className="flex items-center justify-between gap-3 rounded-md border bg-background/60 px-3 py-2">
            <div>
              <div className="text-xs font-medium">
                {t("settings.memoryExternalProvidersGlobal", "Enable provider sync")}
              </div>
              <div className="text-[11px] text-muted-foreground">
                {needsSetupCount > 0
                  ? t("settings.memoryExternalProvidersNeedsSetupSummary", {
                      defaultValue:
                        "{{active}} active / {{total}} configured · {{count}} need setup",
                      active: activeCount,
                      total: config.providers.length,
                      count: needsSetupCount,
                    })
                  : t("settings.memoryExternalProvidersActiveSummary", {
                      defaultValue: "{{active}} active / {{total}} configured",
                      active: activeCount,
                      total: config.providers.length,
                    })}
              </div>
            </div>
            <Switch
              checked={config.enabled}
              onCheckedChange={(enabled) => setConfig((prev) => ({ ...prev, enabled }))}
            />
          </div>

          <div className="rounded-md border bg-muted/25 px-3 py-2 text-xs">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="min-w-0">
                <div className="font-medium">
                  {t("settings.memoryExternalProviderPreflightTitle", "Sync preflight")}
                </div>
                <div className="text-[11px] text-muted-foreground">
                  {preflight
                    ? t("settings.memoryExternalProviderPreflightSummary", {
                        defaultValue:
                          "Dry run: {{ready}} would sync / {{blocked}} blocked · {{total}} local memories",
                        ready: preflight.runnableProviderCount,
                        blocked: preflight.blockedProviderCount,
                        total: preflight.localMemoryTotal,
                      })
                    : preflightLoading
                      ? t("settings.memoryExternalProviderPreflightLoading", "Loading preflight…")
                      : t(
                          "settings.memoryExternalProviderPreflightUnavailable",
                          "Preflight unavailable.",
                        )}
                  {dirty && (
                    <span className="ml-1">
                      {t(
                        "settings.memoryExternalProviderPreflightSaveHint",
                        "Save changes to refresh the active plan.",
                      )}
                    </span>
                  )}
                </div>
              </div>
              <div className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 gap-1.5"
                  disabled={!preflight}
                  onClick={() => void copyPreflightDiagnostics()}
                >
                  <Copy className="h-3.5 w-3.5" />
                  {t("settings.memoryExternalProviderPreflightCopy", "Copy report")}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 gap-1.5"
                  disabled={preflightLoading}
                  onClick={() => void refreshPreflight()}
                >
                  {preflightLoading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Clock3 className="h-3.5 w-3.5" />
                  )}
                  {t("settings.memoryExternalProviderPreflightRefresh", "Refresh")}
                </Button>
              </div>
            </div>
            {preflightError && (
              <div className="mt-2 whitespace-pre-line rounded bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
                {preflightError}
              </div>
            )}
            {preflight?.statsUnavailable && (
              <div className="mt-2 rounded border border-amber-500/20 bg-amber-500/5 px-2 py-1.5 text-[11px] text-amber-700 dark:text-amber-300">
                <div>
                  {t(
                    "settings.memoryExternalProviderPreflightStatsUnavailable",
                    "Local memory stats could not be loaded; candidate counts may be incomplete.",
                  )}
                </div>
                {preflight.statsError && (
                  <div className="mt-0.5 text-muted-foreground">
                    {t("settings.memoryExternalProviderPreflightStatsError", {
                      defaultValue: "Details: {{error}}",
                      error: externalMemoryProviderDiagnosticText(preflight.statsError),
                    })}
                  </div>
                )}
              </div>
            )}
          </div>

          <div className="rounded-md border bg-muted/20 px-3 py-2 text-xs">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="min-w-0">
                <div className="font-medium">
                  {t("settings.memoryExternalProviderSyncReportTitle", "Sync report")}
                </div>
                <div className="text-[11px] text-muted-foreground">
                  {syncReport
                    ? t("settings.memoryExternalProviderSyncReportSummary", {
                        defaultValue:
                          "Last run: {{executed}} executed / {{blocked}} blocked · external IO {{io}}",
                        executed: syncReport.executedProviderCount,
                        blocked: syncReport.blockedProviderCount,
                        io: syncReport.externalIoPerformed
                          ? t("common.yes", "yes")
                          : t("common.no", "no"),
                      })
                    : syncLoading
                      ? t("settings.memoryExternalProviderSyncReportLoading", "Running check…")
                      : t(
                          "settings.memoryExternalProviderSyncReportEmpty",
                          "Run a sync check to produce an execution report.",
                        )}
                  {dirty && (
                    <span className="ml-1">
                      {t(
                        "settings.memoryExternalProviderSyncReportSaveHint",
                        "Save changes before running against the active config.",
                      )}
                    </span>
                  )}
                </div>
              </div>
              <div className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 gap-1.5"
                  disabled={!syncReport}
                  onClick={() => void copySyncDiagnostics()}
                >
                  <Copy className="h-3.5 w-3.5" />
                  {t("settings.memoryExternalProviderSyncReportCopy", "Copy report")}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 gap-1.5"
                  disabled={syncLoading || dirty}
                  onClick={() => void runSyncCheck()}
                >
                  {syncLoading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <ShieldCheck className="h-3.5 w-3.5" />
                  )}
                  {t("settings.memoryExternalProviderSyncReportRun", "Run sync")}
                </Button>
              </div>
            </div>
            {syncError && (
              <div className="mt-2 whitespace-pre-line rounded bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
                {syncError}
              </div>
            )}
            {syncReport && !syncReport.externalIoPerformed && (
              <div className="mt-2 rounded border border-sky-500/20 bg-sky-500/5 px-2 py-1.5 text-[11px] text-sky-700 dark:text-sky-300">
                {t(
                  "settings.memoryExternalProviderSyncReportNoIo",
                  "No external IO was performed. Providers were blocked, disabled, or had no changed records.",
                )}
              </div>
            )}
          </div>

          {config.providers.length === 0 ? (
            <div className="rounded-md border border-dashed px-3 py-4 text-center text-xs text-muted-foreground">
              {t("settings.memoryExternalProvidersEmpty", "No external providers configured.")}
            </div>
          ) : (
            <div className="space-y-3">
              {config.providers.map((provider) => {
                const providerPreflight = preflightByProviderId.get(provider.id)
                const runtimeProvider = providerPreflight?.health
                  ? {
                      ...provider,
                      capabilities: providerPreflight.health.capabilities,
                      policySupported: providerPreflight.health.policySupported,
                    }
                  : provider
                const readiness = getExternalMemoryProviderReadiness(
                  config.enabled,
                  runtimeProvider,
                )
                const capabilities = externalMemoryProviderCapabilities(runtimeProvider)
                const supportedPolicies = externalMemoryProviderSupportedSyncPolicies(runtimeProvider)
                const needsEndpointSetup = readiness === "needs_setup"
                const policySupported = externalMemoryProviderPolicySupported(runtimeProvider)
                const privacySummary = externalMemoryProviderPrivacySummary(
                  config.enabled,
                  runtimeProvider,
                )
                const syncBlockReasons = externalMemoryProviderSyncBlockReasons(
                  config.enabled,
                  runtimeProvider,
                )
                const providerSyncResult = syncResultByProviderId.get(provider.id)
                const readinessLabel = (() => {
                  switch (readiness) {
                    case "ready":
                      return t("settings.memoryExternalProviderReady", "Ready")
                    case "unsupported_policy":
                      return t(
                        "settings.memoryExternalProviderUnsupportedPolicy",
                        "Unsupported policy",
                      )
                    case "adapter_pending":
                      return t("settings.memoryExternalProviderAdapterPending", "Adapter pending")
                    case "needs_setup":
                      return t("settings.memoryExternalProviderNeedsSetupBadge", "Needs setup")
                    default:
                      return t("settings.memoryExternalProviderOff", "Off")
                  }
                })()
                const safetyCopy = (() => {
                  switch (readiness) {
                    case "ready":
                      return t(
                        "settings.memoryExternalProviderLocalFirstReady",
                        "Provider adapter is available for additive sync; local memory remains the source of truth.",
                      )
                    case "adapter_pending":
                      return t(
                        "settings.memoryExternalProviderLocalFirstAdapterPending",
                        "Endpoint metadata is configured, but this runtime adapter is not available yet. No sync will run.",
                      )
                    case "unsupported_policy":
                      return t(
                        "settings.memoryExternalProviderLocalFirstUnsupportedPolicy",
                        "The selected sync policy is not supported by this provider adapter. No sync will run.",
                      )
                    case "needs_setup":
                      return t(
                        "settings.memoryExternalProviderLocalFirstNeedsSetup",
                        "Sync cannot run yet. Local memory remains available.",
                      )
                    default:
                      return t(
                        "settings.memoryExternalProviderLocalFirstOff",
                        "Provider sync is not active; no memory data leaves Hope Agent through this provider.",
                      )
                  }
                })()
                const privacyCopy = (() => {
                  if (!isExternalMemoryProviderSyncActive(config.enabled, provider)) {
                    return t(
                      "settings.memoryExternalProviderPrivacyOff",
                      "No provider data will move while this provider or global sync is off.",
                    )
                  }
                  if (privacySummary.syncBlocked) {
                    return t(
                      "settings.memoryExternalProviderPrivacyBlocked",
                      "No provider data will move until setup, policy, and adapter readiness all pass.",
                    )
                  }
                  switch (provider.syncPolicy) {
                    case "manual":
                      return t(
                        "settings.memoryExternalProviderPrivacyManual",
                        "Manual sync may send selected memory or retrieval context only after an explicit action.",
                      )
                    case "pull_only":
                      return t(
                        "settings.memoryExternalProviderPrivacyPullOnly",
                        "Pull sync may send retrieval context to the provider and import external memories; it does not push local memory records.",
                      )
                    case "push_only":
                      return t(
                        "settings.memoryExternalProviderPrivacyPushOnly",
                        "Push sync may send selected local memory updates outward; it does not import external memories.",
                      )
                    case "bidirectional":
                      return t(
                        "settings.memoryExternalProviderPrivacyBidirectional",
                        "Bidirectional sync may send retrieval context and selected memory updates, and may import external memories.",
                      )
                    default:
                      return t(
                        "settings.memoryExternalProviderPrivacyOff",
                        "No provider data will move while this provider or global sync is off.",
                      )
                  }
                })()
                const lastSyncLabel =
                  formatProviderTimestamp(provider.lastSyncAt) ??
                  t("settings.memoryExternalProviderNeverSynced", "Never")

                return (
                  <div key={provider.id} className="rounded-md border bg-background/60 p-3">
                    <div className="flex items-start justify-between gap-3">
                      <div className="grid min-w-0 flex-1 gap-3 md:grid-cols-[minmax(0,1fr)_150px_150px]">
                        <div className="space-y-1">
                          <label className="text-[11px] text-muted-foreground">
                            {t("settings.memoryExternalProviderName", "Name")}
                          </label>
                          <Input
                            value={provider.displayName}
                            onChange={(event) =>
                              patchProvider(provider.id, { displayName: event.target.value })
                            }
                            className="h-8 text-xs"
                          />
                        </div>
                        <div className="space-y-1">
                          <label className="text-[11px] text-muted-foreground">
                            {t("settings.memoryExternalProviderKind", "Provider")}
                          </label>
                          <Select
                            value={provider.kind}
                            onValueChange={(kind) =>
                              patchProvider(provider.id, {
                                kind: kind as ExternalMemoryProviderKind,
                              })
                            }
                          >
                            <SelectTrigger className="h-8 text-xs">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              {PROVIDER_KINDS.map((kind) => (
                                <SelectItem key={kind} value={kind}>
                                  {providerKindLabel(kind)}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </div>
                        <div className="space-y-1">
                          <label className="text-[11px] text-muted-foreground">
                            {t("settings.memoryExternalProviderPolicy", "Sync policy")}
                          </label>
                          <Select
                            value={provider.syncPolicy}
                            onValueChange={(syncPolicy) =>
                              patchProvider(provider.id, {
                                syncPolicy: syncPolicy as ExternalMemorySyncPolicy,
                              })
                            }
                          >
                            <SelectTrigger className="h-8 text-xs">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              {SYNC_POLICIES.map((policy) => (
                                <SelectItem
                                  key={policy}
                                  value={policy}
                                  disabled={
                                    !externalMemoryProviderPolicySupported({
                                      kind: provider.kind,
                                      syncPolicy: policy,
                                    })
                                  }
                                >
                                  {syncPolicyLabels[policy]}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                          <div className="text-[11px] leading-snug text-muted-foreground">
                            {syncPolicyDescriptions[provider.syncPolicy]}
                          </div>
                          <div className="text-[10px] leading-snug text-muted-foreground">
                            {t("settings.memoryExternalProviderSupportedPolicies", {
                              defaultValue: "Supported: {{policies}}",
                              policies:
                                supportedPolicies
                                  .map((policy) => syncPolicyLabels[policy])
                                  .join(", ") || t("common.none", "None"),
                            })}
                          </div>
                        </div>
                      </div>
                      <div className="flex shrink-0 items-center gap-2">
                        <span
                          className={cn(
                            "inline-flex h-6 items-center rounded-md px-2 text-[11px] font-medium",
                            readiness === "ready" &&
                              "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
                            readiness === "needs_setup" &&
                              "bg-amber-500/10 text-amber-700 dark:text-amber-300",
                            readiness === "unsupported_policy" &&
                              "bg-rose-500/10 text-rose-700 dark:text-rose-300",
                            readiness === "adapter_pending" &&
                              "bg-sky-500/10 text-sky-700 dark:text-sky-300",
                            readiness === "off" && "bg-muted text-muted-foreground",
                          )}
                        >
                          {readinessLabel}
                        </span>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8 text-muted-foreground hover:text-destructive"
                          onClick={() =>
                            setConfig((prev) => ({
                              ...prev,
                              providers: prev.providers.filter((item) => item.id !== provider.id),
                            }))
                          }
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </div>

                    <div className="mt-3 flex flex-wrap items-center gap-4 text-xs">
                      <label className="flex items-center gap-2">
                        <Switch
                          checked={provider.enabled}
                          onCheckedChange={(enabled) => patchProvider(provider.id, { enabled })}
                        />
                        {t("settings.memoryExternalProviderEnabled", "Provider enabled")}
                      </label>
                      {!capabilities.adapterAvailable && (
                        <label className="flex items-center gap-2">
                          <Switch
                            checked={provider.endpointConfigured}
                            onCheckedChange={(endpointConfigured) =>
                              patchProvider(provider.id, { endpointConfigured })
                            }
                          />
                          {t(
                            "settings.memoryExternalProviderEndpointConfigured",
                            "Endpoint configured outside Hope Agent",
                          )}
                        </label>
                      )}
                      {needsEndpointSetup && (
                        <span className="inline-flex items-center gap-1 text-amber-600 dark:text-amber-300">
                          <AlertTriangle className="h-3.5 w-3.5" />
                          {t(
                            "settings.memoryExternalProviderNeedsSetup",
                            "Finish endpoint setup before sync can run.",
                          )}
                        </span>
                      )}
                      {!policySupported && (
                        <span className="inline-flex items-center gap-1 text-rose-600 dark:text-rose-300">
                          <AlertTriangle className="h-3.5 w-3.5" />
                          {t(
                            "settings.memoryExternalProviderUnsupportedPolicyDesc",
                            "Choose a supported sync policy before sync can run.",
                          )}
                        </span>
                      )}
                      {provider.lastError && (
                        <span className="line-clamp-2 max-w-full break-words text-amber-600 dark:text-amber-300">
                          {t("settings.memoryExternalProviderError", {
                            defaultValue: "Last error: {{error}}",
                            error: externalMemoryProviderDiagnosticText(provider.lastError),
                          })}
                        </span>
                      )}
                    </div>

                    {capabilities.adapterAvailable && (
                      <ExternalMemoryProviderCredentials
                        provider={provider}
                        configDirty={dirty}
                        onStatusChanged={handleCredentialStatusChanged}
                      />
                    )}

                    <div className="mt-3 grid gap-1.5 rounded-md bg-muted/35 px-3 py-2 text-[11px] text-muted-foreground">
                      <div className="flex items-start gap-1.5">
                        <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-emerald-600 dark:text-emerald-300" />
                        <span>{safetyCopy}</span>
                      </div>
                      <div className="flex items-start gap-1.5">
                        <Cloud className="mt-0.5 h-3.5 w-3.5 shrink-0 text-sky-600 dark:text-sky-300" />
                        <span>{privacyCopy}</span>
                      </div>
                      <div className="flex items-start gap-1.5">
                        <Cloud className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                        <span>
                          {capabilities.adapterAvailable
                            ? t(
                                "settings.memoryExternalProviderAdapterAvailable",
                                "Runtime adapter available.",
                              )
                            : t(
                                "settings.memoryExternalProviderAdapterUnavailable",
                                "Runtime adapter not shipped yet; this provider is configuration-only.",
                              )}
                        </span>
                      </div>
                      {providerPreflight && (
                        <div className="flex items-start gap-1.5">
                          <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-violet-600 dark:text-violet-300" />
                          <span>
                            {t("settings.memoryExternalProviderPreflightPlan", {
                              defaultValue:
                                "Preflight: {{action}} · planned {{planned}} → runtime {{runtime}} · local candidates {{count}}",
                              action: providerPreflight.action.replace(/_/g, " "),
                              planned: dataFlowLabel(providerPreflight.plannedDataFlow),
                              runtime: dataFlowLabel(providerPreflight.runtimeDataFlow),
                              count: providerPreflight.localMemoryCandidateCount,
                            })}
                          </span>
                        </div>
                      )}
                      {providerSyncResult && (
                        <div className="flex items-start gap-1.5">
                          <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-sky-600 dark:text-sky-300" />
                          <span>
                            {t("settings.memoryExternalProviderSyncResult", {
                              defaultValue:
                                "Sync report: {{status}} · external IO {{io}} · imported {{imported}} / exported {{exported}} / updated {{updated}}",
                              status: providerSyncResult.status.replace(/_/g, " "),
                              io: providerSyncResult.externalIoPerformed
                                ? t("common.yes", "yes")
                                : t("common.no", "no"),
                              imported: providerSyncResult.importedMemoryCount,
                              exported: providerSyncResult.exportedMemoryCount,
                              updated: providerSyncResult.updatedMemoryCount,
                            })}
                            {providerSyncResult.error
                              ? t("settings.memoryExternalProviderSyncResultError", {
                                  defaultValue: " · {{error}}",
                                  error: externalMemoryProviderDiagnosticText(
                                    providerSyncResult.error,
                                  ),
                                })
                              : null}
                          </span>
                        </div>
                      )}
                      {syncBlockReasons.length > 0 && (
                        <div className="flex flex-wrap items-center gap-1.5">
                          <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-300" />
                          <span>{t("settings.memoryExternalProviderBlockedBy", "Blocked by")}</span>
                          {syncBlockReasons.map((reason) => (
                            <span
                              key={reason}
                              className="rounded bg-background/80 px-1.5 py-0.5 font-mono text-[10px]"
                            >
                              {externalMemoryProviderSyncBlockReasonLabel(reason, t)}
                            </span>
                          ))}
                        </div>
                      )}
                      <div className="flex items-center gap-1.5">
                        <Clock3 className="h-3.5 w-3.5 shrink-0" />
                        <span>
                          {t("settings.memoryExternalProviderLastSync", {
                            defaultValue: "Last sync: {{value}}",
                            value: lastSyncLabel,
                          })}
                        </span>
                      </div>
                    </div>
                  </div>
                )
              })}
            </div>
          )}

          <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="gap-1.5"
              onClick={() =>
                setConfig((prev) => ({
                  ...prev,
                  providers: [...prev.providers, createDraftProvider(prev.providers.length)],
                }))
              }
            >
              <Plus className="h-3.5 w-3.5" />
              {t("settings.memoryExternalProviderAdd", "Add provider")}
            </Button>
            <Button
              onClick={() => void handleSave()}
              disabled={saving || !dirty}
              variant={
                saveStatus === "saved"
                  ? "outline"
                  : saveStatus === "failed"
                    ? "destructive"
                    : "default"
              }
              size="sm"
              className="gap-1.5"
            >
              {saving ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : saveStatus === "saved" ? (
                <Check className="h-3.5 w-3.5 text-green-600" />
              ) : (
                <Save className="h-3.5 w-3.5" />
              )}
              {saveStatus === "saved"
                ? t("common.saved")
                : saveStatus === "failed"
                  ? t("common.retry")
                  : t("common.save")}
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

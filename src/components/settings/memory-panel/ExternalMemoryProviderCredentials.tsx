import { useCallback, useEffect, useRef, useState } from "react"
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
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { KeyRound, Link2, Loader2, Save, Trash2 } from "lucide-react"
import type {
  ExternalMemoryProviderConfig,
  ExternalMemoryProviderCredentialInput,
  ExternalMemoryProviderCredentialStatus,
  ExternalMemoryProviderKind,
} from "./types"
import { externalMemoryProviderOperationErrorToast } from "./externalMemoryProviderOperationFeedback"

interface Props {
  provider: ExternalMemoryProviderConfig
  configDirty: boolean
  onStatusChanged: (
    providerId: string,
    status: ExternalMemoryProviderCredentialStatus | null,
  ) => void
}

interface CredentialDraft {
  endpoint: string
  apiKey: string
  subjectId: string
  protocol: string
}

interface ProviderConnectionPreset {
  endpoint: string
  subjectId: string
  subjectScopeKey: string
  subjectScope: string
  protocols: Array<{ value: string; label: string }>
  apiKeyHint: string
}

const PROVIDER_CONNECTION_PRESETS: Record<ExternalMemoryProviderKind, ProviderConnectionPreset> = {
  mem0: {
    endpoint: "https://api.mem0.ai",
    subjectId: "hope-agent-user",
    subjectScopeKey: "settings.memoryExternalProviderScopeMem0",
    subjectScope: "user ID",
    protocols: [
      { value: "platform_v3", label: "Mem0 Platform v3" },
      { value: "oss", label: "Mem0 OSS" },
    ],
    apiKeyHint: "Required for Mem0 Platform; optional for local OSS",
  },
  zep: {
    endpoint: "http://localhost:8000",
    subjectId: "hope-agent-user",
    subjectScopeKey: "settings.memoryExternalProviderScopeZep",
    subjectScope: "Graphiti group ID",
    protocols: [{ value: "graphiti_http", label: "Graphiti HTTP sidecar" }],
    apiKeyHint: "Optional bearer token for a protected Graphiti sidecar",
  },
  supermemory: {
    endpoint: "https://api.supermemory.ai",
    subjectId: "hope-agent-user",
    subjectScopeKey: "settings.memoryExternalProviderScopeSupermemory",
    subjectScope: "container tag",
    protocols: [
      { value: "cloud", label: "Supermemory Cloud" },
      { value: "self_hosted", label: "Supermemory self-hosted" },
    ],
    apiKeyHint: "Required for Supermemory Cloud and self-hosted endpoints",
  },
  honcho: {
    endpoint: "https://api.honcho.dev",
    subjectId: "hope-agent",
    subjectScopeKey: "settings.memoryExternalProviderScopeHoncho",
    subjectScope: "workspace ID",
    protocols: [
      { value: "v3", label: "Honcho API v3" },
      { value: "self_hosted", label: "Honcho self-hosted v3" },
    ],
    apiKeyHint: "Required for Honcho Cloud; optional when self-hosted auth is disabled",
  },
  hindsight: {
    endpoint: "https://api.hindsight.vectorize.io",
    subjectId: "hope-agent",
    subjectScopeKey: "settings.memoryExternalProviderScopeHindsight",
    subjectScope: "memory bank ID",
    protocols: [
      { value: "v1", label: "Hindsight API v1" },
      { value: "self_hosted", label: "Hindsight self-hosted v1" },
    ],
    apiKeyHint: "Required for Hindsight Cloud; optional for an unprotected local server",
  },
  open_viking: {
    endpoint: "http://localhost:1933",
    subjectId: "hope-agent",
    subjectScopeKey: "settings.memoryExternalProviderScopeOpenViking",
    subjectScope: "sync session prefix",
    protocols: [{ value: "v1", label: "OpenViking REST v1" }],
    apiKeyHint: "Use a tenant user API key; local development mode may omit it",
  },
  custom: {
    endpoint: "https://memory.example.com",
    subjectId: "hope-agent-user",
    subjectScopeKey: "settings.memoryExternalProviderScopeCustom",
    subjectScope: "subject ID",
    protocols: [{ value: "hope_sync_v1", label: "Hope Sync v1" }],
    apiKeyHint: "Optional bearer token for your Hope Sync v1 sidecar",
  },
}

function defaultDraft(provider: ExternalMemoryProviderConfig): CredentialDraft {
  const preset = PROVIDER_CONNECTION_PRESETS[provider.kind]
  return {
    endpoint: preset.endpoint,
    apiKey: "",
    subjectId: preset.subjectId,
    protocol: "auto",
  }
}

export default function ExternalMemoryProviderCredentials({
  provider,
  configDirty,
  onStatusChanged,
}: Props) {
  const { t } = useTranslation()
  const tRef = useRef(t)
  const onStatusChangedRef = useRef(onStatusChanged)
  tRef.current = t
  onStatusChangedRef.current = onStatusChanged
  const [status, setStatus] = useState<ExternalMemoryProviderCredentialStatus | null>(null)
  const [draft, setDraft] = useState<CredentialDraft>(() => defaultDraft(provider))
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const applyStatus = useCallback(
    (next: ExternalMemoryProviderCredentialStatus | null) => {
      setStatus(next)
      onStatusChangedRef.current(provider.id, next)
      if (next?.configured) {
        setDraft((current) => ({
          ...current,
          endpoint: "",
          apiKey: "",
          subjectId: next.subjectId || current.subjectId,
          protocol: next.protocol || current.protocol,
        }))
      }
    },
    [provider.id],
  )

  const loadStatus = useCallback(async () => {
    if (configDirty) return
    setLoading(true)
    try {
      const next = await getTransport().call<ExternalMemoryProviderCredentialStatus>(
        "get_external_memory_provider_credential_status",
        { providerId: provider.id },
      )
      applyStatus(next ?? null)
      setError(null)
    } catch (cause) {
      logger.error(
        "settings",
        "ExternalMemoryProviderCredentials::load",
        "Failed to load external memory provider credentials status",
        cause,
      )
      const failure = externalMemoryProviderOperationErrorToast(
        "loadCredentials",
        tRef.current,
        cause,
      )
      setError(failure.description ? `${failure.title}\n${failure.description}` : failure.title)
    } finally {
      setLoading(false)
    }
  }, [applyStatus, configDirty, provider.id])

  useEffect(() => {
    void loadStatus()
  }, [loadStatus])

  const save = async () => {
    const credentials: ExternalMemoryProviderCredentialInput = {
      providerId: provider.id,
      endpoint: draft.endpoint.trim(),
      subjectId: draft.subjectId.trim(),
      protocol: draft.protocol,
      ...(draft.apiKey.trim() ? { apiKey: draft.apiKey.trim() } : {}),
    }
    setSaving(true)
    try {
      const next = await getTransport().call<ExternalMemoryProviderCredentialStatus>(
        "save_external_memory_provider_credentials",
        { providerId: provider.id, credentials },
      )
      applyStatus(next ?? null)
      setError(null)
      toast.success(t("settings.memoryExternalProviderConnectionSaved", "Connection saved"))
    } catch (cause) {
      logger.error(
        "settings",
        "ExternalMemoryProviderCredentials::save",
        "Failed to save external memory provider credentials",
        cause,
      )
      const failure = externalMemoryProviderOperationErrorToast("saveCredentials", t, cause)
      setError(failure.description ? `${failure.title}\n${failure.description}` : failure.title)
    } finally {
      setSaving(false)
    }
  }

  const clear = async () => {
    setClearing(true)
    try {
      await getTransport().call("clear_external_memory_provider_credentials", {
        providerId: provider.id,
      })
      setDraft(defaultDraft(provider))
      applyStatus(null)
      setError(null)
      toast.success(t("settings.memoryExternalProviderConnectionCleared", "Connection cleared"))
    } catch (cause) {
      logger.error(
        "settings",
        "ExternalMemoryProviderCredentials::clear",
        "Failed to clear external memory provider credentials",
        cause,
      )
      const failure = externalMemoryProviderOperationErrorToast("clearCredentials", t, cause)
      setError(failure.description ? `${failure.title}\n${failure.description}` : failure.title)
    } finally {
      setClearing(false)
    }
  }

  const firstSetup = !status?.configured
  const connectionPreset = PROVIDER_CONNECTION_PRESETS[provider.kind]
  const saveDisabled =
    saving ||
    loading ||
    configDirty ||
    !draft.subjectId.trim() ||
    (firstSetup && !draft.endpoint.trim())

  return (
    <div className="mt-3 border-t border-border/50 pt-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <div>
          <div className="flex items-center gap-1.5 text-xs font-medium">
            <Link2 className="h-3.5 w-3.5" />
            {t("settings.memoryExternalProviderConnection", "Provider connection")}
          </div>
          <div className="mt-0.5 text-[11px] text-muted-foreground">
            {status?.configured
              ? t("settings.memoryExternalProviderConnectionReady", {
                  defaultValue: "Configured via {{source}} · {{origin}}",
                  source: status.source || "secure file",
                  origin: status.endpointOrigin || "endpoint",
                })
              : t(
                  "settings.memoryExternalProviderConnectionEmpty",
                  "Credentials are stored separately with restricted file permissions.",
                )}
          </div>
        </div>
        {loading && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />}
      </div>

      <div className="grid gap-2 md:grid-cols-2">
        <div className="space-y-1 md:col-span-2">
          <label className="text-[11px] text-muted-foreground">
            {t("settings.memoryExternalProviderEndpoint", "Endpoint")}
          </label>
          <Input
            value={draft.endpoint}
            onChange={(event) => setDraft((current) => ({ ...current, endpoint: event.target.value }))}
            placeholder={
              status?.endpointOrigin
                ? t("settings.memoryExternalProviderEndpointKeep", {
                    defaultValue: "Stored: {{origin}} · leave blank to keep",
                    origin: status.endpointOrigin,
                  })
                : defaultDraft(provider).endpoint
            }
            className="h-8 font-mono text-xs"
            autoComplete="url"
          />
        </div>
        <div className="space-y-1">
          <label className="text-[11px] text-muted-foreground">
            {t("settings.memoryExternalProviderSubjectId", "Remote scope ID")}
          </label>
          <Input
            value={draft.subjectId}
            onChange={(event) => setDraft((current) => ({ ...current, subjectId: event.target.value }))}
            className="h-8 font-mono text-xs"
            autoComplete="off"
          />
          <div className="text-[10px] text-muted-foreground">
            {t("settings.memoryExternalProviderSubjectHint", {
              defaultValue: "{{scope}} used to isolate this connection.",
              scope: t(connectionPreset.subjectScopeKey, connectionPreset.subjectScope),
            })}
          </div>
        </div>
        <div className="space-y-1">
          <label className="text-[11px] text-muted-foreground">
            {t("settings.memoryExternalProviderProtocol", "Protocol")}
          </label>
          <Select
            value={draft.protocol}
            onValueChange={(protocol) => setDraft((current) => ({ ...current, protocol }))}
          >
            <SelectTrigger className="h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="auto">{t("common.auto", "Auto")}</SelectItem>
              {connectionPreset.protocols.map((protocol) => (
                <SelectItem key={protocol.value} value={protocol.value}>
                  {t(
                    `settings.memoryExternalProviderProtocolLabels.${provider.kind}.${protocol.value}`,
                    protocol.label,
                  )}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1 md:col-span-2">
          <label className="text-[11px] text-muted-foreground">
            {t("settings.memoryExternalProviderApiKey", "API key")}
          </label>
          <div className="relative">
            <KeyRound className="pointer-events-none absolute left-2.5 top-2 h-3.5 w-3.5 text-muted-foreground" />
            <Input
              type="password"
              value={draft.apiKey}
              onChange={(event) => setDraft((current) => ({ ...current, apiKey: event.target.value }))}
              placeholder={
                status?.apiKeyConfigured
                  ? t("settings.memoryExternalProviderApiKeyKeep", "Stored · leave blank to keep")
                  : provider.kind === "mem0"
                    ? t("settings.memoryExternalProviderApiKeyOptional", connectionPreset.apiKeyHint)
                    : provider.kind === "supermemory"
                      ? t("settings.memoryExternalProviderApiKeyRequired", connectionPreset.apiKeyHint)
                      : t(
                          "settings.memoryExternalProviderApiKeyGeneric",
                          connectionPreset.apiKeyHint,
                        )
              }
              className="h-8 pl-8 font-mono text-xs"
              autoComplete="new-password"
            />
          </div>
        </div>
      </div>

      {configDirty && (
        <div className="mt-2 text-[11px] text-amber-600 dark:text-amber-300">
          {t(
            "settings.memoryExternalProviderConnectionSaveConfigFirst",
            "Save the provider configuration before changing its connection.",
          )}
        </div>
      )}
      {error && (
        <div className="mt-2 whitespace-pre-line rounded bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      )}

      <div className="mt-2 flex flex-wrap justify-end gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-7 gap-1.5"
          disabled={clearing || saving || configDirty || !status?.configured}
          onClick={() => void clear()}
        >
          {clearing ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Trash2 className="h-3.5 w-3.5" />
          )}
          {t("settings.memoryExternalProviderConnectionClear", "Clear connection")}
        </Button>
        <Button
          type="button"
          size="sm"
          className="h-7 gap-1.5"
          disabled={saveDisabled}
          onClick={() => void save()}
        >
          {saving ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Save className="h-3.5 w-3.5" />
          )}
          {t("settings.memoryExternalProviderConnectionSave", "Save connection")}
        </Button>
      </div>
    </div>
  )
}

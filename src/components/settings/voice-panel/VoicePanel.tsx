import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import type { TFunction } from "i18next"
import { Check, Loader2, Mic, Plus, Server, Trash2, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { IconTip } from "@/components/ui/tooltip"
import { SecretInput } from "@/components/ui/secret-input"
import ProviderIcon from "@/components/common/ProviderIcon"

function Card({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={cn("rounded-lg border bg-card text-card-foreground shadow-sm", className)}>
      {children}
    </div>
  )
}
function CardHeader({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn("p-4 pb-2", className)}>{children}</div>
}
function CardTitle({ children, className }: { children: React.ReactNode; className?: string }) {
  return <h3 className={cn("text-base font-semibold leading-none", className)}>{children}</h3>
}
function CardContent({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn("p-4 pt-2", className)}>{children}</div>
}
function Badge({
  children,
  variant = "default",
  className,
}: {
  children: React.ReactNode
  variant?: "default" | "outline" | "secondary"
  className?: string
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-medium",
        variant === "outline" && "border border-border bg-background",
        variant === "secondary" && "bg-secondary text-secondary-foreground",
        variant === "default" && "bg-primary text-primary-foreground",
        className,
      )}
    >
      {children}
    </span>
  )
}
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
// Shared kind type / batch-vs-streaming sets from src/lib/stt.ts so the
// active-model picker, the kind dropdown, and the desktop voice hook agree
// on the dispatch rules.
import {
  BATCH_CAPABLE_KINDS,
  STREAMING_KINDS,
  unwrapActiveSttModel,
  type ActiveSttModel,
  type SttProviderKind,
} from "@/lib/stt"

interface SttModelConfig {
  id: string
  name: string
  supportsStreaming?: boolean
  languages?: string[]
  costPerMinute?: number
  supportsTimestamps?: boolean
  supportsDiarization?: boolean
}

interface SttProviderConfig {
  id: string
  name: string
  kind: SttProviderKind
  baseUrl: string
  apiKey?: string
  authProfiles?: unknown[]
  models: SttModelConfig[]
  enabled: boolean
  allowPrivateNetwork?: boolean
  extra?: Record<string, string>
}

interface KnownLocalSttBackend {
  key: string
  name: string
  kind: SttProviderKind
  baseUrl: string
  hosts: string[]
  port: number
  knownModels: SttModelConfig[]
  installHintEn: string
  installHintZh: string
  installUrl: string
}

import { STT_PRESETS, findPreset, presetSlugFromProvider, type SttKindPreset } from "./presets"

function presetModels(preset: SttKindPreset): SttModelConfig[] {
  return preset.defaultModels.map((model) => ({
    id: model.id,
    name: model.name,
  }))
}

function presetModelDisplayName(
  preset: SttKindPreset | undefined,
  model: SttModelConfig,
  t: TFunction,
): string {
  const seeded = preset?.defaultModels.find((candidate) => candidate.id === model.id)
  if (seeded?.nameKey) return t(seeded.nameKey)
  return model.name || model.id
}

function modelsMatchPresetDefaults(models: SttModelConfig[], preset: SttKindPreset): boolean {
  if (models.length !== preset.defaultModels.length) return false
  return models.every((model, index) => {
    const seeded = preset.defaultModels[index]
    if (model.id !== seeded.id) return false
    // A nameKey marks a protocol sentinel rather than a user-named model.
    // Match it by its stable wire ID so a value saved by an older localized
    // build is still recognized after the UI language changes.
    return seeded.nameKey != null || (model.name || model.id) === seeded.name
  })
}

function canonicalizePresetModelNames(
  preset: SttKindPreset | undefined,
  models: SttModelConfig[],
): SttModelConfig[] {
  if (!preset) return models
  return models.map((model) => {
    const seeded = preset.defaultModels.find(
      (candidate) => candidate.id === model.id && candidate.nameKey,
    )
    return seeded ? { ...model, name: seeded.name } : model
  })
}

/** Shared label markup for the API-type dropdown — same row appears in
 * the SelectTrigger value and every SelectItem, so factor it out. */
function renderPresetLabel(p: SttKindPreset | undefined, fallback: string) {
  if (!p) return fallback
  return (
    <span className="flex items-center gap-2">
      <ProviderIcon providerKey={p.iconKey} size={16} color className="shrink-0" />
      <span className="truncate">
        {p.chineseName ? `${p.chineseName} · ${p.brand}` : p.brand}
        {p.protocol ? ` (${p.protocol})` : ""}
      </span>
    </span>
  )
}

// Dead block — replaced by ./presets. Below kept until next edit removes it.

interface ExtraField {
  key: string
  label: string
  labelKey?: string
  hintKey?: string
  type: "text" | "password"
  required: boolean
}

/// Required and optional `extra` fields per provider kind. Used to
/// surface the right inputs in the edit dialog so users can fill the
/// per-vendor credentials (`app_id`, `api_secret`, `app_key`, etc) that
/// the backend will require at WS handshake time.
const KIND_EXTRA_SCHEMA: Record<SttProviderKind, ExtraField[]> = {
  "openai-transcriptions": [],
  "openai-compatible": [],
  "openai-chat-completions-asr": [],
  "elevenlabs-stt": [],
  "xai-stt": [],
  "deepgram-ws": [],
  "assemblyai-ws": [],
  "azure-ws": [
    {
      key: "region",
      label: "Region",
      labelKey: "voice.settings.extraFields.region",
      type: "text",
      required: true,
      hintKey: "voice.settings.extraFields.regionHint",
    },
  ],
  "xunfei-ws": [
    {
      key: "api_secret",
      label: "APISecret",
      type: "password",
      required: true,
      hintKey: "voice.settings.extraFields.xunfeiApiSecretHint",
    },
    {
      key: "app_id",
      label: "APPID",
      type: "text",
      required: true,
      hintKey: "voice.settings.extraFields.xunfeiAppIdHint",
    },
    {
      key: "accent",
      label: "Accent",
      labelKey: "voice.settings.extraFields.accent",
      type: "text",
      required: false,
      hintKey: "voice.settings.extraFields.accentHint",
    },
  ],
  "volcengine-ws": [
    {
      key: "app_key",
      label: "APP ID",
      type: "text",
      required: true,
      hintKey: "voice.settings.extraFields.volcengineAppIdHint",
    },
    {
      key: "resource_id",
      label: "ResourceId",
      type: "text",
      required: false,
      hintKey: "voice.settings.extraFields.volcengineResourceIdHint",
    },
  ],
}

/**
 * Per-kind hint for the API Key input — surfaces what the upstream
 * console actually calls this secret (Access Token, Subscription Key,
 * APIKey, …) so users don't paste the wrong field.
 */
const KIND_API_KEY_HINT: Partial<Record<SttProviderKind, string>> = {
  "azure-ws": "voice.settings.apiKeyHints.azure",
  "volcengine-ws": "voice.settings.apiKeyHints.volcengine",
  "xunfei-ws": "voice.settings.apiKeyHints.xunfei",
}


const blankProvider = (): SttProviderConfig => ({
  id: "",
  name: "",
  kind: "openai-transcriptions",
  baseUrl: "https://api.openai.com",
  apiKey: "",
  authProfiles: [],
  models: [],
  enabled: true,
  allowPrivateNetwork: false,
  extra: {},
})

// ── Component ─────────────────────────────────────────────────────

export default function VoicePanel() {
  const { t, i18n } = useTranslation()

  const [providers, setProviders] = useState<SttProviderConfig[]>([])
  const [activeModel, setActiveModel] = useState<ActiveSttModel | null>(null)
  const [imFallback, setImFallback] = useState<ActiveSttModel | null>(null)
  const [backends, setBackends] = useState<KnownLocalSttBackend[]>([])
  const [probes, setProbes] = useState<Record<string, boolean | null>>({})
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [dialogProvider, setDialogProvider] = useState<SttProviderConfig | null>(null)

  const refresh = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const transport = getTransport()
      const [list, active, im, cat] = await Promise.all([
        transport.call<SttProviderConfig[]>("get_stt_providers", {}),
        transport.call<unknown>("get_active_stt_model", {}),
        transport.call<unknown>("get_im_fallback_stt_model", {}),
        transport.call<KnownLocalSttBackend[]>("list_known_local_stt_backends", {}),
      ])
      setProviders(list ?? [])
      setActiveModel(unwrapActiveSttModel(active, "activeModel"))
      setImFallback(unwrapActiveSttModel(im, "imFallbackModel"))
      setBackends(cat ?? [])
    } catch (e) {
      setError(String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const probeBackend = useCallback(async (key: string) => {
    setProbes((p) => ({ ...p, [key]: null }))
    try {
      const alive = await getTransport().call<boolean | { alive?: boolean }>(
        "probe_local_stt_backend",
        { key },
      )
      const ok =
        typeof alive === "object" && alive !== null && "alive" in alive
          ? Boolean((alive as { alive?: boolean }).alive)
          : Boolean(alive)
      setProbes((p) => ({ ...p, [key]: ok }))
    } catch {
      setProbes((p) => ({ ...p, [key]: false }))
    }
  }, [])

  const upsertLocal = useCallback(
    async (backend: KnownLocalSttBackend, model: SttModelConfig, activate: boolean) => {
      try {
        await getTransport().call("upsert_known_local_stt_provider_cmd", {
          backendKey: backend.key,
          provider: {
            id: "",
            name: backend.name,
            kind: backend.kind,
            baseUrl: backend.baseUrl,
            apiKey: "",
            authProfiles: [],
            models: [],
            enabled: true,
            allowPrivateNetwork: true,
            extra: {},
          },
          model,
          activate,
        })
        await refresh()
      } catch (e) {
        setError(String(e))
      }
    },
    [refresh],
  )

  const deleteProvider = useCallback(
    async (id: string) => {
      try {
        await getTransport().call("delete_stt_provider", { providerId: id })
        await refresh()
      } catch (e) {
        setError(String(e))
      }
    },
    [refresh],
  )

  const setActive = useCallback(
    async (selection: ActiveSttModel | null) => {
      try {
        if (selection) {
          await getTransport().call("set_active_stt_model", {
            providerId: selection.providerId,
            modelId: selection.modelId,
          })
        } else {
          await getTransport().call("clear_active_stt_model", {})
        }
        await refresh()
      } catch (e) {
        setError(String(e))
      }
    },
    [refresh],
  )

  const setIm = useCallback(
    async (selection: ActiveSttModel | null) => {
      try {
        await getTransport().call("set_im_fallback_stt_model", { selection })
        await refresh()
      } catch (e) {
        setError(String(e))
      }
    },
    [refresh],
  )

  const allAvailable = useMemo(() => {
    const out: {
      providerId: string
      modelId: string
      label: string
      streaming: boolean
    }[] = []
    for (const p of providers) {
      if (!p.enabled) continue
      // Either path is now wired in `useVoiceInput`: batch via
      // `stt_transcribe_blob`, streaming via `stt_start_session` +
      // PCM16 worklet. Surface both — `streaming: true` lets the
      // selector tag the realtime providers in the dropdown label.
      if (!BATCH_CAPABLE_KINDS.has(p.kind) && !STREAMING_KINDS.has(p.kind)) continue
      const streaming = STREAMING_KINDS.has(p.kind)
      for (const m of p.models) {
        const preset = findPreset(presetSlugFromProvider(p.kind, p.baseUrl))
        out.push({
          providerId: p.id,
          modelId: m.id,
          label: `${p.name} · ${presetModelDisplayName(preset, m, t)}${streaming ? " · streaming" : ""}`,
          streaming,
        })
      }
    }
    return out
  }, [providers, t])

  const installHint = useCallback(
    (b: KnownLocalSttBackend) =>
      i18n.language && i18n.language.startsWith("zh") ? b.installHintZh : b.installHintEn,
    [i18n.language],
  )

  if (loading) {
    return (
      <div className="flex-1 overflow-y-auto p-6 flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("voice.processing")}
      </div>
    )
  }

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-6">
      <header className="flex items-center gap-2">
        <Mic className="h-5 w-5" />
        <div>
          <h2 className="text-lg font-semibold">{t("voice.settings.title")}</h2>
          <p className="text-xs text-muted-foreground">
            {t("voice.settings.subtitle")}
          </p>
        </div>
      </header>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
        </div>
      )}

      {/* Active model picker */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-medium">
            {t("voice.settings.activeModel")}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-2">
            <Label>{t("voice.settings.activeModelLabel")}</Label>
            <Select
              value={activeModel ? `${activeModel.providerId}::${activeModel.modelId}` : "__none__"}
              onValueChange={(v) => {
                if (v === "__none__") {
                  void setActive(null)
                  return
                }
                const [providerId, modelId] = v.split("::")
                void setActive({ providerId, modelId })
              }}
            >
              <SelectTrigger>
                <SelectValue placeholder={t("voice.settings.noModel")} />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">{t("voice.settings.noModel")}</SelectItem>
                {allAvailable.map((m) => (
                  <SelectItem key={`${m.providerId}::${m.modelId}`} value={`${m.providerId}::${m.modelId}`}>
                    {m.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {t("voice.settings.batchOnlyHint")}
            </p>
          </div>
          <div className="space-y-2">
            <Label>{t("voice.settings.imFallback")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("voice.settings.imFallbackHint")}
            </p>
            <Select
              value={imFallback ? `${imFallback.providerId}::${imFallback.modelId}` : "__none__"}
              onValueChange={(v) => {
                if (v === "__none__") {
                  void setIm(null)
                  return
                }
                const [providerId, modelId] = v.split("::")
                void setIm({ providerId, modelId })
              }}
            >
              <SelectTrigger>
                <SelectValue placeholder={t("voice.settings.imFallbackEmpty")} />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">{t("voice.settings.imFallbackEmpty")}</SelectItem>
                {/* IM auto-transcription is a batch operation (one audio blob
                    per message). Streaming-only providers reject this path
                    server-side, so don't surface them in the picker. */}
                {allAvailable
                  .filter((m) => !m.streaming)
                  .map((m) => (
                    <SelectItem key={`im-${m.providerId}::${m.modelId}`} value={`${m.providerId}::${m.modelId}`}>
                      {m.label}
                    </SelectItem>
                  ))}
              </SelectContent>
            </Select>
          </div>
        </CardContent>
      </Card>

      {/* Cloud + custom providers list */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between gap-2">
          <CardTitle className="text-sm font-medium">
            {t("voice.settings.providers")}
          </CardTitle>
          <Button
            size="sm"
            variant="outline"
            onClick={() => setDialogProvider(blankProvider())}
          >
            <Plus className="h-3.5 w-3.5 mr-1" />
            {t("voice.settings.addProvider")}
          </Button>
        </CardHeader>
        <CardContent className="space-y-3">
          {providers.length === 0 && (
            <p className="text-xs text-muted-foreground">
              {t("voice.settings.noProviders")}
            </p>
          )}
          {providers.map((p) => {
            const preset = findPreset(presetSlugFromProvider(p.kind, p.baseUrl))
            return (
            <div
              key={p.id}
              className="flex items-start justify-between gap-3 rounded-md border p-3"
            >
              <div className="min-w-0 flex items-start gap-3">
                <ProviderIcon
                  providerKey={preset?.iconKey}
                  providerName={preset?.brand ?? p.name}
                  size={24}
                  color
                  className="shrink-0 mt-0.5"
                />
                <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-sm truncate">{p.name}</span>
                  <Badge variant="outline" className="text-[10px]">
                    {p.kind}
                  </Badge>
                  {!p.enabled && (
                    <Badge variant="secondary" className="text-[10px]">
                      {t("voice.settings.disabled")}
                    </Badge>
                  )}
                </div>
                <p className="text-xs text-muted-foreground truncate">{p.baseUrl}</p>
                {p.models.length > 0 && (
                  <p className="text-xs text-muted-foreground mt-1">
                    {p.models.map((m) => m.id).join(" · ")}
                  </p>
                )}
                </div>
              </div>
              <div className="flex items-center gap-1 shrink-0">
                <Button size="sm" variant="ghost" onClick={() => setDialogProvider(p)}>
                  {t("common.edit")}
                </Button>
                <IconTip label={t("common.delete")}>
                  <Button
                    size="icon"
                    variant="ghost"
                    className="h-7 w-7 text-destructive"
                    onClick={() => void deleteProvider(p.id)}
                    aria-label={t("common.delete")}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </IconTip>
              </div>
            </div>
            )
          })}
        </CardContent>
      </Card>

      {/* Local backends */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-medium">
            {t("voice.settings.localBackends")}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {backends.map((b) => (
            <div key={b.key} className="rounded-md border p-3">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <Server className="h-4 w-4" />
                  <span className="font-medium text-sm">{b.name}</span>
                  <code className="text-xs text-muted-foreground">{b.baseUrl}</code>
                </div>
                <div className="flex items-center gap-2">
                  {probes[b.key] === true && (
                    <Badge variant="outline" className="text-emerald-600 border-emerald-300">
                      <Check className="h-3 w-3 mr-1" />
                      {t("voice.settings.backendAlive")}
                    </Badge>
                  )}
                  {probes[b.key] === false && (
                    <Badge variant="outline" className="text-muted-foreground">
                      <X className="h-3 w-3 mr-1" />
                      {t("voice.settings.backendOffline")}
                    </Badge>
                  )}
                  <Button size="sm" variant="outline" onClick={() => void probeBackend(b.key)}>
                    {probes[b.key] === null ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      t("voice.settings.probe")
                    )}
                  </Button>
                </div>
              </div>
              <p className="text-xs text-muted-foreground mt-2">{installHint(b)}</p>
              <div className="mt-2 flex flex-wrap gap-2">
                {b.knownModels.map((m) => (
                  <Button
                    key={m.id}
                    size="sm"
                    variant="secondary"
                    className="text-xs"
                    onClick={() => void upsertLocal(b, m, true)}
                  >
                    + {m.id}
                  </Button>
                ))}
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      {dialogProvider && (
        <ProviderDialog
          provider={dialogProvider}
          onClose={() => setDialogProvider(null)}
          onSaved={() => {
            setDialogProvider(null)
            void refresh()
          }}
        />
      )}
    </div>
  )
}

// ── Provider add / edit dialog ────────────────────────────────────

function ProviderDialog({
  provider,
  onClose,
  onSaved,
}: {
  provider: SttProviderConfig
  onClose: () => void
  onSaved: () => void
}) {
  const { t } = useTranslation()
  const isNew = !provider.id

  const [name, setName] = useState(provider.name)
  const [presetSlug, setPresetSlug] = useState<string>(() =>
    presetSlugFromProvider(provider.kind, provider.baseUrl),
  )
  const kind: SttProviderKind = findPreset(presetSlug)?.kind ?? provider.kind
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl)
  const [apiKey, setApiKey] = useState(provider.apiKey ?? "")
  const [enabled, setEnabled] = useState(provider.enabled)
  const [allowPrivate, setAllowPrivate] = useState(provider.allowPrivateNetwork ?? false)
  const [extraValues, setExtraValues] = useState<Record<string, string>>(
    () => ({ ...(provider.extra ?? {}) }),
  )
  const [models, setModels] = useState<SttModelConfig[]>(() => {
    // For brand-new providers whose preset has seed models (iFlytek,
    // Volcengine, DashScope), seed the list so the activation flow has
    // something to pick. Existing providers keep their saved models.
    if (!provider.id && provider.models.length === 0) {
      const preset = findPreset(presetSlugFromProvider(provider.kind, provider.baseUrl))
      if (preset && preset.defaultModels.length > 0) {
        return presetModels(preset)
      }
    }
    return provider.models.map((m) => ({ ...m }))
  })
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const extraSchema = useMemo(() => KIND_EXTRA_SCHEMA[kind] ?? [], [kind])

  const onPresetChange = useCallback(
    (nextSlug: string) => {
      const next = findPreset(nextSlug)
      const prev = findPreset(presetSlug)
      if (!next) return
      setPresetSlug(nextSlug)
      // Prefill the default baseUrl on preset switch when the user
      // hasn't customised one (or is still on the previous default).
      const prevDefault = prev?.defaultBaseUrl ?? ""
      if (!baseUrl.trim() || baseUrl.trim() === prevDefault) {
        setBaseUrl(next.defaultBaseUrl)
      }
      // Models that are still the previous preset's defaults are leftover
      // UI noise — swap them for the new preset's defaults so users
      // don't see a wrong-vendor list of seeded entries. Models the user
      // typed or edited (not matching the previous defaults) survive.
      setModels((current) => {
        const isResidual = prev != null && modelsMatchPresetDefaults(current, prev)
        if (isResidual) {
          return presetModels(next)
        }
        if (current.length === 0 && next.defaultModels.length > 0) {
          return presetModels(next)
        }
        return current
      })
    },
    [baseUrl, presetSlug],
  )

  const save = useCallback(async () => {
    setSaving(true)
    setError(null)
    try {
      // Provider name is the only cross-vendor required field on the
      // top-level form — without it the row in the list looks empty.
      if (!name.trim()) {
        setError(t("voice.settings.errProviderNameRequired"))
        setSaving(false)
        return
      }
      // Mismatched schemes silently fail at connect time, so block them
      // up front. Transport + requirements come from the preset registry.
      const preset = findPreset(presetSlug)
      const trimmedBase = baseUrl.trim()
      if (trimmedBase) {
        const lower = trimmedBase.toLowerCase()
        if (preset?.transport === "ws") {
          if (!lower.startsWith("ws://") && !lower.startsWith("wss://")) {
            setError(t("voice.settings.errBaseUrlMustBeWs"))
            setSaving(false)
            return
          }
        } else if (!lower.startsWith("http://") && !lower.startsWith("https://")) {
          setError(t("voice.settings.errBaseUrlMustBeHttp"))
          setSaving(false)
          return
        }
      } else if (preset?.requiresBaseUrl) {
        setError(t("voice.settings.errBaseUrlRequired"))
        setSaving(false)
        return
      }
      // Local OpenAI-compatible servers bound to private networks may
      // legitimately have no API key. When editing an existing provider,
      // an empty input means "keep the stored key" (the backend merge
      // logic preserves redacted ones).
      const apiKeyTrim = apiKey.trim()
      const requiresApiKey = !(kind === "openai-compatible" && allowPrivate)
      if (isNew && requiresApiKey && !apiKeyTrim) {
        setError(t("voice.settings.errApiKeyRequired"))
        setSaving(false)
        return
      }
      // Required `extra` fields per kind — surface vendor-specific
      // missing credentials in the dialog rather than a cryptic backend
      // error on first stream attempt.
      for (const field of extraSchema) {
        if (field.required && !extraValues[field.key]?.trim()) {
          setError(
            t("voice.settings.errExtraFieldRequired", {
              field: field.labelKey ? t(field.labelKey) : field.label,
            }),
          )
          setSaving(false)
          return
        }
      }
      const trimmedModels: SttModelConfig[] = canonicalizePresetModelNames(
        preset,
        models
          .map((m) => ({ ...m, id: m.id.trim(), name: (m.name || "").trim() }))
          .filter((m) => m.id)
          .map((m) => ({ ...m, name: m.name || m.id })),
      )
      // Strip empty extra values so they don't override redacted-but-set
      // values on round-trip and don't get sent as `""`.
      const trimmedExtra: Record<string, string> = {}
      for (const [k, v] of Object.entries(extraValues)) {
        if (v.trim()) trimmedExtra[k] = v.trim()
      }
      const payload: SttProviderConfig = {
        ...provider,
        name: name.trim(),
        kind,
        baseUrl: baseUrl.trim(),
        apiKey,
        models: trimmedModels,
        enabled,
        allowPrivateNetwork: allowPrivate,
        extra: trimmedExtra,
      }
      if (isNew) {
        await getTransport().call("add_stt_provider", { provider: payload })
      } else {
        // `providerId` is consumed by HTTP path templating; Tauri command
        // takes only `provider` and ignores the extra arg.
        await getTransport().call("update_stt_provider", {
          providerId: payload.id,
          provider: payload,
        })
      }
      onSaved()
    } catch (e) {
      setError(String(e))
    } finally {
      setSaving(false)
    }
  }, [
    allowPrivate,
    apiKey,
    baseUrl,
    enabled,
    extraSchema,
    extraValues,
    isNew,
    kind,
    models,
    name,
    onSaved,
    presetSlug,
    provider,
    t,
  ])

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className={cn("max-w-2xl")}>
        <DialogHeader>
          <DialogTitle>
            {isNew ? t("voice.settings.addProvider") : t("voice.settings.editProvider")}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label>{t("voice.settings.providerName")}</Label>
            <Input value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <div className="space-y-1.5">
            <Label>{t("voice.settings.providerKind")}</Label>
            <Select value={presetSlug} onValueChange={onPresetChange}>
              <SelectTrigger>
                <SelectValue>{renderPresetLabel(findPreset(presetSlug), presetSlug)}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                {STT_PRESETS.map((p) => (
                  <SelectItem key={p.slug} value={p.slug}>
                    {renderPresetLabel(p, p.slug)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label>{t("voice.settings.baseUrl")}</Label>
            <Input value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} />
          </div>
          <div className="space-y-1.5">
            <Label>
              {t("voice.settings.apiKey")}
              {KIND_API_KEY_HINT[kind] && (
                <span className="ml-1.5 text-xs text-muted-foreground font-normal">
                  · {t(KIND_API_KEY_HINT[kind])}
                </span>
              )}
            </Label>
            <SecretInput
              value={apiKey}
              onChange={setApiKey}
              placeholder={isNew ? "" : t("voice.settings.apiKeyMasked")}
            />
          </div>
          {extraSchema.map((field) => (
            <div key={field.key} className="space-y-1.5">
              <Label>
                {field.labelKey ? t(field.labelKey) : field.label}
                {field.required && <span className="text-destructive ml-0.5">*</span>}
              </Label>
              {field.type === "password" ? (
                <SecretInput
                  value={extraValues[field.key] ?? ""}
                  onChange={(next) =>
                    setExtraValues((prev) => ({ ...prev, [field.key]: next }))
                  }
                />
              ) : (
                <Input
                  type="text"
                  value={extraValues[field.key] ?? ""}
                  onChange={(e) =>
                    setExtraValues((prev) => ({ ...prev, [field.key]: e.target.value }))
                  }
                />
              )}
              {field.hintKey && (
                <p className="text-xs text-muted-foreground">{t(field.hintKey)}</p>
              )}
            </div>
          ))}
          <div className="space-y-1.5">
            <Label>{t("model.modelList")}</Label>
            <div className="space-y-2">
              {models.map((m, i) => (
                <div key={i} className="flex items-center gap-2">
                  <Input
                    value={m.id}
                    placeholder={t("model.modelId")}
                    onChange={(e) =>
                      setModels((prev) =>
                        prev.map((row, j) => (j === i ? { ...row, id: e.target.value } : row)),
                      )
                    }
                    className="flex-1 font-mono text-xs h-8"
                  />
                  <Input
                    value={presetModelDisplayName(findPreset(presetSlug), m, t)}
                    placeholder={t("model.displayName")}
                    readOnly={Boolean(
                      findPreset(presetSlug)?.defaultModels.some(
                        (candidate) => candidate.id === m.id && candidate.nameKey,
                      ),
                    )}
                    onChange={(e) =>
                      setModels((prev) =>
                        prev.map((row, j) => (j === i ? { ...row, name: e.target.value } : row)),
                      )
                    }
                    className="flex-1 text-xs h-8"
                  />
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-8 w-8 text-muted-foreground hover:text-destructive shrink-0"
                    onClick={() => setModels((prev) => prev.filter((_, j) => j !== i))}
                    aria-label={t("common.delete")}
                  >
                    <X className="h-3.5 w-3.5" />
                  </Button>
                </div>
              ))}
            </div>
            <Button
              variant="outline"
              size="sm"
              className="w-full"
              onClick={() => setModels((prev) => [...prev, { id: "", name: "" }])}
            >
              <Plus className="h-3.5 w-3.5 mr-1" />
              {t("model.addModel")}
            </Button>
          </div>
          <div className="flex items-center justify-between">
            <Label>{t("voice.settings.enabled")}</Label>
            <Switch checked={enabled} onCheckedChange={setEnabled} />
          </div>
          <div className="flex items-center justify-between">
            <div>
              <Label>{t("voice.settings.allowPrivate")}</Label>
              <p className="text-xs text-muted-foreground">
                {t("voice.settings.allowPrivateHint")}
              </p>
            </div>
            <Switch checked={allowPrivate} onCheckedChange={setAllowPrivate} />
          </div>
          {error && <p className="text-xs text-destructive">{error}</p>}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button onClick={() => void save()} disabled={saving || !name.trim()}>
            {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

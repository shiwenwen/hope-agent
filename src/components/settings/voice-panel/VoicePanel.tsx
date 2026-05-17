import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
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

// ── Types mirrored from ha-core ──────────────────────────────────

type SttProviderKind =
  | "openai-transcriptions"
  | "openai-compatible"
  | "deepgram-ws"
  | "assemblyai-ws"
  | "azure-ws"
  | "volcengine-ws"
  | "xunfei-ws"

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

interface ActiveSttModel {
  providerId: string
  modelId: string
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

const KIND_OPTIONS: { value: SttProviderKind; label: string }[] = [
  { value: "openai-transcriptions", label: "OpenAI Audio Transcriptions" },
  { value: "openai-compatible", label: "OpenAI-compatible" },
  { value: "deepgram-ws", label: "Deepgram (WS)" },
  { value: "assemblyai-ws", label: "AssemblyAI (WS)" },
  { value: "azure-ws", label: "Azure Speech (WS)" },
]

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
        transport.call<ActiveSttModel | { activeModel?: ActiveSttModel } | null>(
          "get_active_stt_model",
          {},
        ),
        transport.call<ActiveSttModel | { imFallbackModel?: ActiveSttModel } | null>(
          "get_im_fallback_stt_model",
          {},
        ),
        transport.call<KnownLocalSttBackend[]>("list_known_local_stt_backends", {}),
      ])
      setProviders(list ?? [])
      // Tauri returns plain Option<T>; HTTP returns `{ activeModel: ... }`.
      const normActive =
        active && typeof active === "object" && "activeModel" in active
          ? (active as { activeModel?: ActiveSttModel | null }).activeModel ?? null
          : (active as ActiveSttModel | null)
      const normIm =
        im && typeof im === "object" && "imFallbackModel" in im
          ? (im as { imFallbackModel?: ActiveSttModel | null }).imFallbackModel ?? null
          : (im as ActiveSttModel | null)
      setActiveModel(normActive ?? null)
      setImFallback(normIm ?? null)
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
    const out: { providerId: string; modelId: string; label: string }[] = []
    for (const p of providers) {
      if (!p.enabled) continue
      for (const m of p.models) {
        out.push({
          providerId: p.id,
          modelId: m.id,
          label: `${p.name} · ${m.name || m.id}`,
        })
      }
    }
    return out
  }, [providers])

  const installHint = useCallback(
    (b: KnownLocalSttBackend) =>
      i18n.language && i18n.language.startsWith("zh") ? b.installHintZh : b.installHintEn,
    [i18n.language],
  )

  if (loading) {
    return (
      <div className="p-6 flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("voice.processing")}
      </div>
    )
  }

  return (
    <div className="space-y-6 p-4 max-w-4xl">
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
                {allAvailable.map((m) => (
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
          {providers.map((p) => (
            <div
              key={p.id}
              className="flex items-start justify-between gap-3 rounded-md border p-3"
            >
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
          ))}
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
  const [kind, setKind] = useState<SttProviderKind>(provider.kind)
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl)
  const [apiKey, setApiKey] = useState(provider.apiKey ?? "")
  const [enabled, setEnabled] = useState(provider.enabled)
  const [allowPrivate, setAllowPrivate] = useState(provider.allowPrivateNetwork ?? false)
  const [modelsText, setModelsText] = useState(
    provider.models.map((m) => `${m.id}\t${m.name || m.id}`).join("\n"),
  )
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const save = useCallback(async () => {
    setSaving(true)
    setError(null)
    try {
      const models: SttModelConfig[] = modelsText
        .split("\n")
        .map((row) => row.trim())
        .filter(Boolean)
        .map((row) => {
          const [id, ...rest] = row.split("\t")
          return { id: id.trim(), name: rest.join("\t").trim() || id.trim() }
        })
      const payload: SttProviderConfig = {
        ...provider,
        name: name.trim(),
        kind,
        baseUrl: baseUrl.trim(),
        apiKey,
        models,
        enabled,
        allowPrivateNetwork: allowPrivate,
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
  }, [allowPrivate, apiKey, baseUrl, enabled, isNew, kind, modelsText, name, onSaved, provider])

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
            <Select value={kind} onValueChange={(v) => setKind(v as SttProviderKind)}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {KIND_OPTIONS.map((k) => (
                  <SelectItem key={k.value} value={k.value}>
                    {k.label}
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
            <Label>{t("voice.settings.apiKey")}</Label>
            <Input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={isNew ? "" : t("voice.settings.apiKeyMasked")}
            />
          </div>
          <div className="space-y-1.5">
            <Label>{t("voice.settings.models")}</Label>
            <textarea
              value={modelsText}
              onChange={(e) => setModelsText(e.target.value)}
              rows={5}
              className="w-full font-mono text-xs rounded-md border px-2 py-1.5 bg-background"
              placeholder={t("voice.settings.modelsPlaceholder")}
            />
            <p className="text-xs text-muted-foreground">
              {t("voice.settings.modelsHint")}
            </p>
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

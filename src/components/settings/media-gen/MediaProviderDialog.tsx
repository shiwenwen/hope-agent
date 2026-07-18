// Add / edit dialog for one media-generation provider. Create flow is
// two-step: template card grid (all built-in vendors; duplicates of an
// existing kind are allowed) → credential form; edit mode jumps straight
// to the form. Mirrors the STT VoicePanel ProviderDialog structure.

import { useCallback, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import type { TFunction } from "i18next"
import { ArrowLeft, Loader2, Plus, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { NumberInput } from "@/components/ui/number-input"
import { RadioPills } from "@/components/ui/radio-pills"
import { SecretInput } from "@/components/ui/secret-input"
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
import ProviderIcon from "@/components/common/ProviderIcon"
import TestResultDisplay, {
  parseTestResult,
  type TestResult,
} from "@/components/settings/TestResultDisplay"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"

import type {
  MediaAudioKind,
  MediaModelConfig,
  MediaProviderConfig,
  MediaProviderTemplate,
  MediaVendorKind,
  MediaVoiceOption,
} from "./types"
import { VENDOR_DISPLAY_NAME, VENDOR_ICON_KEY } from "./vendor"

// ── Constants ─────────────────────────────────────────────────────

/** Vendors whose speech models accept a configurable default voice. */
const VOICE_VENDORS: ReadonlySet<MediaVendorKind> = new Set([
  "elevenlabs",
  "openai",
  "openai-compatible",
])

const AUDIO_KINDS: MediaAudioKind[] = ["speech", "music", "sfx"]

const AUDIO_KIND_LABEL_KEY: Record<MediaAudioKind, string> = {
  speech: "settings.mediaModels.kindSpeech",
  music: "settings.mediaModels.kindMusic",
  sfx: "settings.mediaModels.kindSfx",
}

// ── Helpers ───────────────────────────────────────────────────────

function cloneModels(models: MediaModelConfig[]): MediaModelConfig[] {
  return JSON.parse(JSON.stringify(models)) as MediaModelConfig[]
}

function blankFromTemplate(tpl: MediaProviderTemplate): MediaProviderConfig {
  return {
    id: "",
    name: tpl.name,
    kind: tpl.kind,
    baseUrl: null,
    apiKey: "",
    enabled: true,
    models: cloneModels(tpl.models),
    defaultVoice: null,
    allowPrivateNetwork: false,
    extra: {},
  }
}

/** Capability summary chips for one model row. */
function capSummary(m: MediaModelConfig, t: TFunction): string[] {
  const chips: string[] = []
  if (m.modality === "image") {
    const img = m.image
    if (!img) return chips
    if (img.sizes.length > 0) chips.push(`${t("settings.mediaModels.capSizes")} ×${img.sizes.length}`)
    else if (img.supportsSize) chips.push(t("settings.mediaModels.capSizes"))
    if (img.supportsAspectRatio) chips.push(t("settings.mediaModels.capAspectRatio"))
    if (img.supportsResolution) chips.push(t("settings.mediaModels.capResolution"))
    if (img.edit) chips.push(t("settings.mediaModels.capEdit"))
    if (img.supportsMask) chips.push(t("settings.mediaModels.capMask"))
    if (img.maxN > 1) chips.push(`N ≤ ${img.maxN}`)
  } else if (m.modality === "audio") {
    const kinds = m.audio?.kinds ?? []
    if (kinds.length === 0) chips.push(t("settings.mediaModels.kindAll"))
    else for (const k of kinds) chips.push(t(AUDIO_KIND_LABEL_KEY[k]))
    if (m.audio?.supportsDuration) chips.push(t("settings.mediaModels.capDuration"))
    if (m.audio?.needsVoice) chips.push(t("settings.mediaModels.capVoice"))
  }
  return chips
}

function ModalityBadge({ modality }: { modality: MediaModelConfig["modality"] }) {
  const { t } = useTranslation()
  if (modality !== "image" && modality !== "audio") return null
  return (
    <span className="inline-flex shrink-0 items-center rounded-full bg-secondary px-2 py-0.5 text-[10px] font-medium text-secondary-foreground">
      {modality === "image"
        ? t("settings.mediaModels.modalityImage")
        : t("settings.mediaModels.modalityAudio")}
    </span>
  )
}

// ── Custom model editor ───────────────────────────────────────────

function CustomModelEditor({
  existingIds,
  onAdd,
  onCancel,
}: {
  existingIds: string[]
  onAdd: (model: MediaModelConfig) => void
  onCancel: () => void
}) {
  const { t } = useTranslation()
  const [id, setId] = useState("")
  const [name, setName] = useState("")
  const [modality, setModality] = useState<"image" | "audio">("image")
  // Image caps
  const [supportsSize, setSupportsSize] = useState(true)
  const [supportsAspectRatio, setSupportsAspectRatio] = useState(false)
  const [supportsResolution, setSupportsResolution] = useState(false)
  const [supportsMask, setSupportsMask] = useState(false)
  const [maxN, setMaxN] = useState(1)
  // Audio caps
  const [kinds, setKinds] = useState<MediaAudioKind[]>(["speech"])
  const [supportsDuration, setSupportsDuration] = useState(false)
  const [needsVoice, setNeedsVoice] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const toggleKind = (k: MediaAudioKind) => {
    setKinds((prev) => (prev.includes(k) ? prev.filter((x) => x !== k) : [...prev, k]))
  }

  const add = () => {
    const trimmed = id.trim()
    if (!trimmed) {
      setError(t("settings.mediaModels.errModelIdRequired"))
      return
    }
    if (existingIds.includes(trimmed)) {
      setError(t("settings.mediaModels.errModelIdDuplicate"))
      return
    }
    onAdd({
      id: trimmed,
      name: name.trim() || trimmed,
      modality,
      image:
        modality === "image"
          ? {
              maxN: Math.min(Math.max(Math.round(maxN) || 1, 1), 10),
              supportsSize,
              supportsAspectRatio,
              supportsResolution,
              sizes: [],
              aspectRatios: [],
              resolutions: [],
              supportsMask,
              edit: null,
            }
          : null,
      audio:
        modality === "audio"
          ? {
              kinds,
              supportsDuration,
              needsVoice,
              defaultVoice: null,
              minDurationSecs: null,
              maxDurationSecs: null,
            }
          : null,
      extra: {},
    })
  }

  const capRow = (label: string, checked: boolean, onChange: (v: boolean) => void) => (
    <div className="flex items-center justify-between gap-2">
      <span className="text-xs text-muted-foreground">{label}</span>
      <Switch checked={checked} onCheckedChange={onChange} />
    </div>
  )

  return (
    <div className="space-y-3 rounded-lg border border-border/60 bg-secondary/20 p-3">
      <div className="grid grid-cols-2 gap-2">
        <div className="space-y-1.5">
          <Label className="text-xs">{t("model.modelId")}</Label>
          <Input
            value={id}
            onChange={(e) => setId(e.target.value)}
            className="h-8 font-mono text-xs"
          />
        </div>
        <div className="space-y-1.5">
          <Label className="text-xs">{t("model.displayName")}</Label>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="h-8 text-xs"
          />
        </div>
      </div>
      <div className="space-y-1.5">
        <Label className="text-xs">{t("settings.mediaModels.modality")}</Label>
        <RadioPills
          value={modality}
          onChange={setModality}
          variant="strong"
          layout="wrap"
          options={[
            { value: "image", label: t("settings.mediaModels.modalityImage") },
            { value: "audio", label: t("settings.mediaModels.modalityAudio") },
          ]}
          ariaLabel={t("settings.mediaModels.modality")}
        />
      </div>
      {modality === "image" ? (
        <div className="space-y-2">
          {capRow(t("settings.mediaModels.supportsSize"), supportsSize, setSupportsSize)}
          {capRow(
            t("settings.mediaModels.supportsAspectRatio"),
            supportsAspectRatio,
            setSupportsAspectRatio,
          )}
          {capRow(
            t("settings.mediaModels.supportsResolution"),
            supportsResolution,
            setSupportsResolution,
          )}
          {capRow(t("settings.mediaModels.supportsMask"), supportsMask, setSupportsMask)}
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs text-muted-foreground">
              {t("settings.mediaModels.maxN")}
            </span>
            <NumberInput
              value={maxN}
              min={1}
              max={10}
              onChange={(e) => setMaxN(Number(e.target.value) || 1)}
              className="h-8 w-24 text-xs"
            />
          </div>
        </div>
      ) : (
        <div className="space-y-2">
          <div className="space-y-1.5">
            <Label className="text-xs">{t("settings.mediaModels.audioKinds")}</Label>
            <div className="flex flex-wrap gap-1.5" role="group">
              {AUDIO_KINDS.map((k) => {
                const active = kinds.includes(k)
                return (
                  <button
                    key={k}
                    type="button"
                    role="checkbox"
                    aria-checked={active}
                    onClick={() => toggleKind(k)}
                    className={cn(
                      "inline-flex items-center rounded-md px-2 py-1.5 text-xs transition-colors",
                      active
                        ? "bg-secondary/70 text-foreground"
                        : "bg-secondary/40 text-muted-foreground hover:bg-secondary/60 hover:text-foreground",
                    )}
                  >
                    {t(AUDIO_KIND_LABEL_KEY[k])}
                  </button>
                )
              })}
            </div>
          </div>
          {capRow(
            t("settings.mediaModels.supportsDuration"),
            supportsDuration,
            setSupportsDuration,
          )}
          {capRow(t("settings.mediaModels.needsVoice"), needsVoice, setNeedsVoice)}
        </div>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onCancel}>
          {t("common.cancel")}
        </Button>
        <Button size="sm" onClick={add}>
          {t("common.add")}
        </Button>
      </div>
    </div>
  )
}

// ── Model list ────────────────────────────────────────────────────

function MediaModelList({
  models,
  onChange,
}: {
  models: MediaModelConfig[]
  onChange: (next: MediaModelConfig[]) => void
}) {
  const { t } = useTranslation()
  const [adding, setAdding] = useState(false)

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <Label>{t("settings.mediaModels.models")}</Label>
        <span className="text-[10px] text-muted-foreground">
          {t("settings.mediaModels.presetReadonly")}
        </span>
      </div>
      <div className="space-y-1.5">
        {models.map((m) => (
          <div
            key={m.id}
            className="rounded-md border border-border/60 px-3 py-2"
          >
            <div className="flex items-center gap-2">
              <span className="truncate font-mono text-xs text-foreground">{m.id}</span>
              {m.name && m.name !== m.id && (
                <span className="truncate text-xs text-muted-foreground">{m.name}</span>
              )}
              <ModalityBadge modality={m.modality} />
              <Button
                variant="ghost"
                size="icon"
                className="ml-auto h-6 w-6 shrink-0 text-muted-foreground hover:text-destructive"
                onClick={() => onChange(models.filter((x) => x.id !== m.id))}
                aria-label={t("common.delete")}
              >
                <X className="h-3.5 w-3.5" />
              </Button>
            </div>
            {capSummary(m, t).length > 0 && (
              <div className="mt-1 flex flex-wrap gap-1">
                {capSummary(m, t).map((chip, i) => (
                  <span
                    key={i}
                    className="rounded-md border border-border/50 bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground"
                  >
                    {chip}
                  </span>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>
      {adding ? (
        <CustomModelEditor
          existingIds={models.map((m) => m.id)}
          onAdd={(model) => {
            onChange([...models, model])
            setAdding(false)
          }}
          onCancel={() => setAdding(false)}
        />
      ) : (
        <Button variant="outline" size="sm" className="w-full" onClick={() => setAdding(true)}>
          <Plus className="mr-1 h-3.5 w-3.5" />
          {t("settings.mediaModels.addCustomModel")}
        </Button>
      )}
    </div>
  )
}

// ── Dialog ────────────────────────────────────────────────────────

export default function MediaProviderDialog({
  templates,
  provider,
  onClose,
  onSubmit,
}: {
  templates: MediaProviderTemplate[]
  /** null = create (starts at the template grid). */
  provider: MediaProviderConfig | null
  onClose: () => void
  onSubmit: (provider: MediaProviderConfig, isNew: boolean) => Promise<void>
}) {
  const { t } = useTranslation()
  const isNew = provider === null

  const [draft, setDraft] = useState<MediaProviderConfig | null>(() =>
    provider
      ? { ...provider, models: cloneModels(provider.models), extra: { ...provider.extra } }
      : null,
  )
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<TestResult | null>(null)
  const [voices, setVoices] = useState<MediaVoiceOption[] | null>(null)
  const [voicesLoading, setVoicesLoading] = useState(false)
  const [voicesError, setVoicesError] = useState<string | null>(null)

  const template = useMemo(
    () => (draft ? templates.find((tpl) => tpl.kind === draft.kind) ?? null : null),
    [draft, templates],
  )

  const set = useCallback((patch: Partial<MediaProviderConfig>) => {
    setDraft((d) => (d ? { ...d, ...patch } : d))
  }, [])

  const chooseTemplate = (tpl: MediaProviderTemplate) => {
    setDraft(blankFromTemplate(tpl))
    setError(null)
    setTestResult(null)
  }

  const runTest = useCallback(async () => {
    if (!draft) return
    setTesting(true)
    setTestResult(null)
    try {
      // Saved provider with untouched credentials → probe by id (works on
      // HTTP where the loaded key is masked). Otherwise probe the draft.
      const credsUntouched =
        provider != null &&
        draft.apiKey === provider.apiKey &&
        (draft.baseUrl ?? "") === (provider.baseUrl ?? "")
      const args = credsUntouched
        ? { providerId: provider.id }
        : {
            kind: draft.kind,
            apiKey: draft.apiKey,
            baseUrl: draft.baseUrl?.trim() || undefined,
          }
      // Tauri returns the probe JSON as a string; HTTP returns it parsed.
      const raw = await getTransport().call<unknown>("test_media_provider", args)
      const msg = typeof raw === "string" ? raw : JSON.stringify(raw)
      setTestResult(parseTestResult(msg, false))
    } catch (e) {
      setTestResult(parseTestResult(String(e), true))
    } finally {
      setTesting(false)
    }
  }, [draft, provider])

  const fetchVoices = useCallback(async () => {
    if (!provider?.id) return
    setVoicesLoading(true)
    setVoicesError(null)
    try {
      const list = await getTransport().call<MediaVoiceOption[]>("list_media_voices", {
        providerId: provider.id,
        limit: 200,
      })
      setVoices(list ?? [])
    } catch (e) {
      setVoices(null)
      setVoicesError(String(e))
    } finally {
      setVoicesLoading(false)
    }
  }, [provider])

  const save = useCallback(async () => {
    if (!draft) return
    setError(null)
    if (!draft.name.trim()) {
      setError(t("settings.mediaModels.errNameRequired"))
      return
    }
    const baseUrlTrim = draft.baseUrl?.trim() ?? ""
    if (draft.kind === "openai-compatible" && !baseUrlTrim) {
      setError(t("settings.mediaModels.errBaseUrlRequired"))
      return
    }
    const requiresApiKey = template?.requiresApiKey ?? draft.kind !== "openai-compatible"
    if (isNew && requiresApiKey && !draft.apiKey.trim()) {
      setError(t("settings.mediaModels.errApiKeyRequired"))
      return
    }
    setSaving(true)
    try {
      await onSubmit(
        {
          ...draft,
          name: draft.name.trim(),
          baseUrl: baseUrlTrim || null,
          defaultVoice: draft.defaultVoice?.trim() || null,
        },
        isNew,
      )
      onClose()
    } catch (e) {
      setError(String(e))
    } finally {
      setSaving(false)
    }
  }, [draft, isNew, onClose, onSubmit, t, template])

  const showVoice = draft != null && VOICE_VENDORS.has(draft.kind)
  const inTemplateStep = isNew && draft === null

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {isNew && draft !== null && (
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6"
                onClick={() => {
                  setDraft(null)
                  setError(null)
                  setTestResult(null)
                }}
                aria-label={t("common.back")}
              >
                <ArrowLeft className="h-4 w-4" />
              </Button>
            )}
            {inTemplateStep
              ? t("settings.mediaModels.chooseTemplate")
              : isNew
                ? t("settings.mediaModels.addProvider")
                : t("settings.mediaModels.editProvider")}
          </DialogTitle>
        </DialogHeader>

        {inTemplateStep ? (
          <div className="grid max-h-[65vh] grid-cols-2 gap-2 overflow-y-auto sm:grid-cols-3">
            {templates.map((tpl) => (
              <button
                key={tpl.key}
                type="button"
                onClick={() => chooseTemplate(tpl)}
                className="flex flex-col items-center gap-2 rounded-lg border border-border bg-card p-4 transition-colors hover:bg-secondary/40"
              >
                <ProviderIcon providerKey={VENDOR_ICON_KEY[tpl.kind]} size={28} color />
                <span className="text-center text-xs font-medium text-foreground">
                  {tpl.key === "openai-compatible"
                    ? t("settings.mediaModels.customOpenAiCompatible")
                    : tpl.name}
                </span>
              </button>
            ))}
          </div>
        ) : draft ? (
          <>
            <div className="max-h-[62vh] space-y-3 overflow-y-auto pr-1">
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <ProviderIcon providerKey={VENDOR_ICON_KEY[draft.kind]} size={16} color />
                <span>{VENDOR_DISPLAY_NAME[draft.kind]}</span>
              </div>
              <div className="space-y-1.5">
                <Label>{t("settings.mediaModels.providerName")}</Label>
                <Input value={draft.name} onChange={(e) => set({ name: e.target.value })} />
              </div>
              <div className="space-y-1.5">
                <Label>{t("settings.mediaModels.baseUrl")}</Label>
                <Input
                  value={draft.baseUrl ?? ""}
                  placeholder={template?.baseUrl || undefined}
                  onChange={(e) => set({ baseUrl: e.target.value })}
                />
              </div>
              <div className="space-y-1.5">
                <Label>{t("settings.mediaModels.apiKey")}</Label>
                <SecretInput value={draft.apiKey} onChange={(next) => set({ apiKey: next })} />
              </div>
              <div className="flex items-center justify-between">
                <Label>{t("settings.mediaModels.enabled")}</Label>
                <Switch
                  checked={draft.enabled}
                  onCheckedChange={(enabled) => set({ enabled })}
                />
              </div>
              {draft.kind === "openai-compatible" && (
                <div className="flex items-center justify-between gap-3">
                  <div>
                    <Label>{t("settings.mediaModels.allowPrivate")}</Label>
                    <p className="text-xs text-muted-foreground">
                      {t("settings.mediaModels.allowPrivateHint")}
                    </p>
                  </div>
                  <Switch
                    checked={draft.allowPrivateNetwork}
                    onCheckedChange={(allowPrivateNetwork) => set({ allowPrivateNetwork })}
                  />
                </div>
              )}
              {showVoice && (
                <div className="space-y-1.5">
                  <Label>{t("settings.mediaModels.defaultVoice")}</Label>
                  <div className="flex items-center gap-2">
                    <Input
                      value={draft.defaultVoice ?? ""}
                      onChange={(e) => set({ defaultVoice: e.target.value })}
                      className="flex-1"
                    />
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={isNew || voicesLoading}
                      onClick={() => void fetchVoices()}
                    >
                      {voicesLoading ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        t("settings.mediaModels.fetchVoices")
                      )}
                    </Button>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {isNew
                      ? t("settings.mediaModels.fetchVoicesSaveFirst")
                      : t("settings.mediaModels.defaultVoiceHint")}
                  </p>
                  {voicesError && <p className="text-xs text-destructive">{voicesError}</p>}
                  {voices != null &&
                    (voices.length === 0 ? (
                      <p className="text-xs text-muted-foreground">
                        {t("settings.mediaModels.voicesEmpty")}
                      </p>
                    ) : (
                      <Select
                        value={
                          voices.some((v) => v.voiceId === draft.defaultVoice)
                            ? (draft.defaultVoice as string)
                            : undefined
                        }
                        onValueChange={(v) => set({ defaultVoice: v })}
                      >
                        <SelectTrigger>
                          <SelectValue placeholder={t("settings.mediaModels.selectVoice")} />
                        </SelectTrigger>
                        <SelectContent>
                          {voices.map((v) => (
                            <SelectItem key={v.voiceId} value={v.voiceId}>
                              {v.name}
                              {v.category ? ` · ${v.category}` : ""}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    ))}
                </div>
              )}
              <MediaModelList models={draft.models} onChange={(models) => set({ models })} />
              {testResult && <TestResultDisplay result={testResult} />}
              {error && <p className="text-xs text-destructive">{error}</p>}
            </div>
            <DialogFooter className="items-center sm:justify-between">
              <Button
                variant="outline"
                size="sm"
                disabled={testing}
                onClick={() => void runTest()}
              >
                {testing ? (
                  <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                ) : null}
                {t("provider.testConnection")}
              </Button>
              <div className="flex items-center gap-2">
                <Button variant="ghost" onClick={onClose} disabled={saving}>
                  {t("common.cancel")}
                </Button>
                <Button onClick={() => void save()} disabled={saving || !draft.name.trim()}>
                  {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : t("common.save")}
                </Button>
              </div>
            </DialogFooter>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  )
}

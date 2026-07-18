// Settings → Tool Settings → Media Generation: merged replacement for the old
// imageGenerate/audioGenerate tabs. Providers themselves are managed in
// Settings → Model Configuration → Media Generation Models; this panel only edits the
// per-function default chains (image/speech/music/sfx) and tool defaults.

import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, Boxes, Check, Loader2, Settings2 } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { NumberInput } from "@/components/ui/number-input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ModelChainEditor } from "@/components/ui/model-chain-editor"
import type { AvailableModel } from "@/components/ui/model-selector"
import {
  AUDIO_DURATION_OPTIONS,
  MEDIA_FUNCTION_KEYS,
  VALID_ASPECT_RATIOS,
  VALID_RESOLUTIONS,
  firstAutoCandidate,
  openMediaModelSettings,
  providerUsable,
  type MediaFunctionKey,
  type MediaGenConfigView,
  type MediaModelChain,
  type MediaProviderConfig,
} from "./media-gen/types"
import { useAvailableMediaModels, useMediaGenData } from "./media-gen/useMediaGenData"

const SIZE_OPTIONS = ["1024x1024", "1024x1536", "1536x1024", "1024x1792", "1792x1024"]
const TIMEOUT_MIN = 30
const TIMEOUT_MAX = 900
/** Radix Select forbids empty-string item values; sentinel for "unset". */
const NONE_VALUE = "__none__"

interface DraftState {
  chains: Record<MediaFunctionKey, MediaModelChain | null>
  imageDefaults: {
    enabled: boolean
    timeoutSeconds: number
    defaultSize: string
    defaultAspectRatio: string | null
    defaultResolution: string | null
  }
  audioDefaults: {
    enabled: boolean
    timeoutSeconds: number
    defaultDurationSecs: number | null
  }
}

/** Normalized draft (`undefined` optionals folded to `null`) so dirty
 *  comparison and select bindings are stable. */
function draftFromConfig(cfg: MediaGenConfigView): DraftState {
  return {
    chains: {
      image: cfg.chains.image ?? null,
      speech: cfg.chains.speech ?? null,
      music: cfg.chains.music ?? null,
      sfx: cfg.chains.sfx ?? null,
    },
    imageDefaults: {
      enabled: cfg.imageDefaults.enabled,
      timeoutSeconds: cfg.imageDefaults.timeoutSeconds,
      defaultSize: cfg.imageDefaults.defaultSize,
      defaultAspectRatio: cfg.imageDefaults.defaultAspectRatio ?? null,
      defaultResolution: cfg.imageDefaults.defaultResolution ?? null,
    },
    audioDefaults: {
      enabled: cfg.audioDefaults.enabled,
      timeoutSeconds: cfg.audioDefaults.timeoutSeconds,
      defaultDurationSecs: cfg.audioDefaults.defaultDurationSecs ?? null,
    },
  }
}

function clampTimeout(n: number): number {
  const rounded = Math.round(n)
  if (!Number.isFinite(rounded)) return TIMEOUT_MIN
  return Math.min(TIMEOUT_MAX, Math.max(TIMEOUT_MIN, rounded))
}

/** One "default chain" field: label + ModelChainEditor + auto-resolution hint
 *  (only shown while inheriting — mirrors backend provider-order fallback). */
function ChainField({
  label,
  fn,
  value,
  models,
  providers,
  onChange,
}: {
  label: string
  fn: MediaFunctionKey
  value: MediaModelChain | null
  models: AvailableModel[]
  providers: MediaProviderConfig[]
  onChange: (next: MediaModelChain | null) => void
}) {
  const { t } = useTranslation()
  const auto = value ? null : firstAutoCandidate(providers, fn)
  return (
    <div className="space-y-1.5">
      <span className="text-xs text-muted-foreground">{label}</span>
      <ModelChainEditor
        value={value}
        onChange={onChange}
        availableModels={models}
        inheritLabel={t("settings.mediaGenerate.followProviderOrder", "跟随服务商顺序（自动）")}
      />
      {!value &&
        (auto ? (
          <p className="text-xs text-muted-foreground">
            {t("settings.mediaGenerate.autoWillUse", {
              defaultValue: "当前将使用：{{name}}",
              name: `${auto.provider.name} / ${auto.model.name || auto.model.id}`,
            })}
          </p>
        ) : (
          <p className="flex items-center gap-1 text-xs text-amber-600 dark:text-amber-500">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            {t("settings.mediaGenerate.noAvailableModel", "无可用模型")}
          </p>
        ))}
    </div>
  )
}

/** Select bound to a nullable string value via the NONE sentinel. */
function NullableSelect({
  value,
  options,
  noneLabel,
  onChange,
}: {
  value: string | null
  options: string[]
  noneLabel: string
  onChange: (next: string | null) => void
}) {
  return (
    <Select
      value={value ?? NONE_VALUE}
      onValueChange={(v) => onChange(v === NONE_VALUE ? null : v)}
    >
      <SelectTrigger>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={NONE_VALUE}>{noneLabel}</SelectItem>
        {options.map((opt) => (
          <SelectItem key={opt} value={opt}>
            {opt}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

export default function MediaGeneratePanel() {
  const { t } = useTranslation()
  const { config, loading, reload } = useMediaGenData()
  const [draft, setDraft] = useState<DraftState | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  // Re-sync the draft from server truth on (re)load; while the user edits,
  // `config` is stable so this never clobbers in-progress changes.
  useEffect(() => {
    if (config) setDraft(draftFromConfig(config))
  }, [config])

  const imageModels = useAvailableMediaModels(config, "image")
  const speechModels = useAvailableMediaModels(config, "speech")
  const musicModels = useAvailableMediaModels(config, "music")
  const sfxModels = useAvailableMediaModels(config, "sfx")

  const providers = useMemo(() => config?.providers ?? [], [config])
  const hasUsableProvider = useMemo(() => providers.some(providerUsable), [providers])

  const baseline = useMemo(
    () => (config ? JSON.stringify(draftFromConfig(config)) : ""),
    [config],
  )
  const isDirty = draft != null && JSON.stringify(draft) !== baseline

  const setChain = (fn: MediaFunctionKey, chain: MediaModelChain | null) =>
    setDraft((d) => (d ? { ...d, chains: { ...d.chains, [fn]: chain } } : d))
  const patchImage = (patch: Partial<DraftState["imageDefaults"]>) =>
    setDraft((d) => (d ? { ...d, imageDefaults: { ...d.imageDefaults, ...patch } } : d))
  const patchAudio = (patch: Partial<DraftState["audioDefaults"]>) =>
    setDraft((d) => (d ? { ...d, audioDefaults: { ...d.audioDefaults, ...patch } } : d))

  const save = async () => {
    if (!config || !draft) return
    setSaving(true)
    try {
      const transport = getTransport()
      const base = draftFromConfig(config)
      // Chains save per-function; only send the ones that actually changed.
      for (const fn of MEDIA_FUNCTION_KEYS) {
        if (JSON.stringify(draft.chains[fn]) !== JSON.stringify(base.chains[fn])) {
          await transport.call("set_media_default_chain", {
            function: fn,
            chain: draft.chains[fn],
          })
        }
      }
      await transport.call("update_media_gen_defaults", {
        imageDefaults: {
          ...draft.imageDefaults,
          timeoutSeconds: clampTimeout(draft.imageDefaults.timeoutSeconds),
        },
        audioDefaults: {
          ...draft.audioDefaults,
          timeoutSeconds: clampTimeout(draft.audioDefaults.timeoutSeconds),
        },
      })
      await reload()
      setSaveStatus("saved")
    } catch (e) {
      logger.error("settings", "MediaGeneratePanel", `save failed: ${e}`)
      setSaveStatus("failed")
    } finally {
      setSaving(false)
      setTimeout(() => setSaveStatus("idle"), 2000)
    }
  }

  if (loading || !config || !draft) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6">
        <div className="space-y-8">
          {/* Header + provider entry */}
          <div className="space-y-3">
            <p className="text-xs text-muted-foreground">
              {t(
                "settings.mediaGenerate.desc",
                "为图像与音频生成配置默认模型链和生成参数；服务商与模型在「媒体生成模型」设置中管理。",
              )}
            </p>
            {!hasUsableProvider ? (
              <div className="rounded-lg border border-border bg-muted/30 px-6 py-8 flex flex-col items-center gap-2 text-center">
                <Boxes className="h-6 w-6 text-muted-foreground/60" />
                <p className="text-sm font-medium">
                  {t("settings.mediaGenerate.noProviderTitle", "尚未配置媒体生成服务商")}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t(
                    "settings.mediaGenerate.noProviderDesc",
                    "需要先添加媒体生成服务商，媒体生成工具才能工作。",
                  )}
                </p>
                <Button size="sm" className="mt-2" onClick={openMediaModelSettings}>
                  {t("settings.mediaGenerate.goConfigure", "去配置媒体生成模型")}
                </Button>
              </div>
            ) : (
              <Button
                variant="ghost"
                size="sm"
                className="h-auto w-fit px-0 py-0 text-xs text-muted-foreground hover:text-foreground hover:bg-transparent"
                onClick={openMediaModelSettings}
              >
                <Settings2 className="h-3.5 w-3.5" />
                {t("settings.mediaGenerate.manageProviders", "管理媒体生成服务商")}
              </Button>
            )}
          </div>

          {/* Image generation */}
          <section className="space-y-4">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.mediaGenerate.imageSection", "图像生成")}
              </h3>
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground">
                  {t("settings.mediaGenerate.enableTool", "启用")}
                </span>
                <Switch
                  checked={draft.imageDefaults.enabled}
                  onCheckedChange={(v) => patchImage({ enabled: v })}
                />
              </div>
            </div>

            <ChainField
              label={t("settings.mediaGenerate.imageChain", "默认模型链")}
              fn="image"
              value={draft.chains.image}
              models={imageModels}
              providers={providers}
              onChange={(chain) => setChain("image", chain)}
            />

            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-1.5">
                <span className="text-xs text-muted-foreground">
                  {t("settings.mediaGenerate.defaultSize", "默认尺寸")}
                </span>
                <Select
                  value={draft.imageDefaults.defaultSize}
                  onValueChange={(v) => patchImage({ defaultSize: v })}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {SIZE_OPTIONS.map((size) => (
                      <SelectItem key={size} value={size}>
                        {size}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-1.5">
                <span className="text-xs text-muted-foreground">
                  {t("settings.mediaGenerate.defaultAspectRatio", "默认宽高比")}
                </span>
                <NullableSelect
                  value={draft.imageDefaults.defaultAspectRatio}
                  options={VALID_ASPECT_RATIOS}
                  noneLabel={t("settings.mediaGenerate.notSpecified", "不指定")}
                  onChange={(v) => patchImage({ defaultAspectRatio: v })}
                />
              </div>

              <div className="space-y-1.5">
                <span className="text-xs text-muted-foreground">
                  {t("settings.mediaGenerate.defaultResolution", "默认分辨率")}
                </span>
                <NullableSelect
                  value={draft.imageDefaults.defaultResolution}
                  options={VALID_RESOLUTIONS}
                  noneLabel={t("settings.mediaGenerate.notSpecified", "不指定")}
                  onChange={(v) => patchImage({ defaultResolution: v })}
                />
              </div>

              <div className="space-y-1.5">
                <span className="text-xs text-muted-foreground">
                  {t("settings.mediaGenerate.timeout", "请求超时（秒）")}
                </span>
                <NumberInput
                  min={TIMEOUT_MIN}
                  max={TIMEOUT_MAX}
                  value={draft.imageDefaults.timeoutSeconds}
                  onChange={(e) => {
                    const num = parseInt(e.target.value, 10)
                    if (!isNaN(num)) patchImage({ timeoutSeconds: num })
                  }}
                />
              </div>
            </div>
          </section>

          {/* Audio generation */}
          <section className="space-y-4">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.mediaGenerate.audioSection", "音频生成")}
              </h3>
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground">
                  {t("settings.mediaGenerate.enableTool", "启用")}
                </span>
                <Switch
                  checked={draft.audioDefaults.enabled}
                  onCheckedChange={(v) => patchAudio({ enabled: v })}
                />
              </div>
            </div>

            <ChainField
              label={t("settings.mediaGenerate.speechChain", "语音模型链")}
              fn="speech"
              value={draft.chains.speech}
              models={speechModels}
              providers={providers}
              onChange={(chain) => setChain("speech", chain)}
            />
            <ChainField
              label={t("settings.mediaGenerate.musicChain", "音乐模型链")}
              fn="music"
              value={draft.chains.music}
              models={musicModels}
              providers={providers}
              onChange={(chain) => setChain("music", chain)}
            />
            <ChainField
              label={t("settings.mediaGenerate.sfxChain", "音效模型链")}
              fn="sfx"
              value={draft.chains.sfx}
              models={sfxModels}
              providers={providers}
              onChange={(chain) => setChain("sfx", chain)}
            />

            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-1.5">
                <span className="text-xs text-muted-foreground">
                  {t("settings.mediaGenerate.defaultDuration", "默认时长")}
                </span>
                <Select
                  value={
                    draft.audioDefaults.defaultDurationSecs == null
                      ? NONE_VALUE
                      : String(draft.audioDefaults.defaultDurationSecs)
                  }
                  onValueChange={(v) =>
                    patchAudio({
                      defaultDurationSecs: v === NONE_VALUE ? null : parseInt(v, 10),
                    })
                  }
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NONE_VALUE}>
                      {t("settings.mediaGenerate.notSpecified", "不指定")}
                    </SelectItem>
                    {AUDIO_DURATION_OPTIONS.map((secs) => (
                      <SelectItem key={secs} value={String(secs)}>
                        {t("settings.mediaGenerate.durationOption", {
                          defaultValue: "{{seconds}} 秒",
                          seconds: secs,
                        })}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-1.5">
                <span className="text-xs text-muted-foreground">
                  {t("settings.mediaGenerate.timeout", "请求超时（秒）")}
                </span>
                <NumberInput
                  min={TIMEOUT_MIN}
                  max={TIMEOUT_MAX}
                  value={draft.audioDefaults.timeoutSeconds}
                  onChange={(e) => {
                    const num = parseInt(e.target.value, 10)
                    if (!isNaN(num)) patchAudio({ timeoutSeconds: num })
                  }}
                />
              </div>
            </div>
          </section>
        </div>
      </div>

      {/* Save — fixed bottom */}
      <div className="shrink-0 flex justify-end px-6 py-3 border-t border-border/30">
        <Button
          onClick={save}
          disabled={(!isDirty && saveStatus === "idle") || saving}
          className={cn(
            saveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
            saveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
          )}
        >
          {saving ? (
            <span className="flex items-center gap-1.5">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.saving")}
            </span>
          ) : saveStatus === "saved" ? (
            <span className="flex items-center gap-1.5">
              <Check className="h-3.5 w-3.5" />
              {t("common.saved")}
            </span>
          ) : saveStatus === "failed" ? (
            t("common.saveFailed")
          ) : (
            t("common.save")
          )}
        </Button>
      </div>
    </div>
  )
}

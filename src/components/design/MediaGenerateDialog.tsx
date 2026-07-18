/**
 * 图 / 音生成对话框（从 DesignView 内嵌 prompt 框抽出并扩展）：
 * 打开时拉 `get_media_gen_overview`（sanitized，无凭据）驱动参数区 —— image 走宽高比 /
 * 分辨率，audio 走 kind（语音/音乐/音效）/ 音色 / 时长；对应功能无可用模型时渲染
 * 空态引导（去配置媒体生成模型 / 重新检查）。参数以 camelCase 直传 `CreateArtifactInput`。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Image as ImageIcon, ImageOff, Loader2, Music, VolumeX } from "lucide-react"
import { toast } from "sonner"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { RadioPills } from "@/components/ui/radio-pills"
import { IconTip } from "@/components/ui/tooltip"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  MediaAudioKind,
  MediaFunctionKey,
  MediaGenOverview,
  MediaVoiceOption,
} from "@/components/settings/media-gen/types"
import {
  AUDIO_DURATION_OPTIONS,
  VALID_RESOLUTIONS,
  openMediaModelSettings,
} from "@/components/settings/media-gen/types"
import { fetchMediaGenOverview } from "@/components/settings/media-gen/useMediaGenData"

/** 确认回调载荷：可选参数只在用户显式选择（非「自动 / 默认」）时携带。 */
export interface MediaGeneratePayload {
  prompt: string
  aspectRatio?: string
  imageSize?: string
  imageResolution?: string
  audioKind?: MediaAudioKind
  audioVoice?: string
  audioDurationSecs?: number
}

interface Props {
  open: boolean
  kind: "image" | "audio"
  onClose: () => void
  onConfirm: (payload: MediaGeneratePayload) => void | Promise<void>
  busy?: boolean
}

/** 无 caps 时的常用宽高比兜底（VALID_ASPECT_RATIOS 的常用 6 个）。 */
const COMMON_ASPECT_RATIOS = ["1:1", "4:3", "3:4", "16:9", "9:16", "21:9"]

const AUTO = "__auto__"
const DEFAULT_VOICE = "__default__"
const CUSTOM_VOICE = "__custom__"
const DEFAULT_DURATION = "__default__"

const AUDIO_KINDS: MediaAudioKind[] = ["speech", "music", "sfx"]

export function MediaGenerateDialog({ open, kind, onClose, onConfirm, busy = false }: Props) {
  const { t } = useTranslation()

  const [overview, setOverview] = useState<MediaGenOverview | null>(null)
  const [overviewLoading, setOverviewLoading] = useState(false)
  const [prompt, setPrompt] = useState("")
  // image 参数
  const [aspect, setAspect] = useState(AUTO)
  const [resolution, setResolution] = useState(AUTO)
  // audio 参数
  const [audioKind, setAudioKind] = useState<MediaAudioKind>("speech")
  const [voice, setVoice] = useState(DEFAULT_VOICE)
  const [customVoiceMode, setCustomVoiceMode] = useState(false)
  const [customVoice, setCustomVoice] = useState("")
  const [duration, setDuration] = useState(DEFAULT_DURATION)
  // 音色目录：null = 未拉取（Select 打开时懒拉一次）
  const [voices, setVoices] = useState<MediaVoiceOption[] | null>(null)
  const [voicesLoading, setVoicesLoading] = useState(false)

  // 每次 open 拉一次 overview；「重新检查」复用。fetch 期间关闭 / 重开用代际计数丢弃陈旧结果。
  const loadGen = useRef(0)
  const loadOverview = useCallback(async (audioTarget?: "audio") => {
    const gen = ++loadGen.current
    setOverviewLoading(true)
    const ov = await fetchMediaGenOverview()
    if (gen !== loadGen.current) return
    setOverview(ov)
    setOverviewLoading(false)
    setVoices(null) // 候选可能已变（重新检查场景），废弃已拉音色
    setVoicesLoading(false)
    // 默认语音；语音无可用模型时落到首个可用的音频 kind。
    if (ov && audioTarget === "audio" && !ov.speech.available) {
      const fallback = (["music", "sfx"] as const).find((k) => ov[k].available)
      if (fallback) setAudioKind(fallback)
    }
  }, [])

  useEffect(() => {
    if (!open) {
      loadGen.current++ // 丢弃在途 fetch
      return
    }
    setOverview(null)
    setPrompt("")
    setAspect(AUTO)
    setResolution(AUTO)
    setAudioKind("speech")
    setVoice(DEFAULT_VOICE)
    setCustomVoiceMode(false)
    setCustomVoice("")
    setDuration(DEFAULT_DURATION)
    setVoices(null)
    setVoicesLoading(false)
    void loadOverview(kind === "audio" ? "audio" : undefined)
  }, [open, kind, loadOverview])

  // 当前功能位（image 或所选音频 kind）的首候选 = 「将使用」的模型。
  const fnKey: MediaFunctionKey = kind === "image" ? "image" : audioKind
  const fn = overview ? overview[fnKey] : null
  const cand = fn?.candidates[0] ?? null

  const audioAllUnavailable =
    overview != null && AUDIO_KINDS.every((k) => !overview[k].available)
  const showEmpty =
    overview != null && (kind === "image" ? !overview.image.available : audioAllUnavailable)

  const aspectChoices = useMemo(
    () =>
      cand?.image?.aspectRatios?.length ? cand.image.aspectRatios : COMMON_ASPECT_RATIOS,
    [cand],
  )
  const resolutionChoices = useMemo(
    () => (cand?.image?.resolutions?.length ? cand.image.resolutions : VALID_RESOLUTIONS),
    [cand],
  )
  const needsVoice = kind === "audio" && audioKind === "speech" && !!cand?.audio?.needsVoice
  const supportsDuration = kind === "audio" && !!cand?.audio?.supportsDuration
  // Only offer durations within the model's declared range (SFX ≈ 0.5–30s,
  // music ≈ 10–300s); out-of-range picks would be silently clamped.
  const durationChoices = useMemo(() => {
    const min = cand?.audio?.minDurationSecs ?? null
    const max = cand?.audio?.maxDurationSecs ?? null
    return AUDIO_DURATION_OPTIONS.filter(
      (s) => (min == null || s >= min) && (max == null || s <= max),
    )
  }, [cand])

  // prompt 以 [music] / [sfx] 开头 → 自动切 kind（不删文本；目标 kind 不可用则不切）。
  const handlePromptChange = useCallback(
    (v: string) => {
      setPrompt(v)
      if (kind !== "audio") return
      const lead = v.trimStart().toLowerCase()
      const target: MediaAudioKind | null = lead.startsWith("[music]")
        ? "music"
        : lead.startsWith("[sfx]")
          ? "sfx"
          : null
      if (target && target !== audioKind && (!overview || overview[target].available)) {
        setAudioKind(target)
      }
    },
    [kind, audioKind, overview],
  )

  const loadVoices = useCallback(async () => {
    if (!cand) return
    setVoicesLoading(true)
    try {
      const list = await getTransport().call<MediaVoiceOption[]>("list_media_voices", {
        providerId: cand.providerId,
      })
      setVoices(list)
    } catch (e) {
      logger.error("design", "MediaGenerateDialog", "load voices failed", e)
      toast.error(t("design.gen.voicesLoadFailed", "音色列表拉取失败"))
      setVoices([])
    } finally {
      setVoicesLoading(false)
    }
  }, [cand, t])

  const handleConfirm = useCallback(() => {
    const p = prompt.trim()
    if (!p) return
    const payload: MediaGeneratePayload = { prompt: p }
    if (kind === "image") {
      // Only carry a parameter the resolved model actually supports —
      // otherwise the backend capability check skips the sole candidate and
      // the whole generation fails.
      if (cand?.image?.supportsAspectRatio && aspect !== AUTO) {
        payload.aspectRatio = aspect
      }
      if (cand?.image?.supportsResolution && resolution !== AUTO) {
        payload.imageResolution = resolution
      }
    } else {
      payload.audioKind = audioKind
      if (needsVoice) {
        const v = customVoiceMode ? customVoice.trim() : voice !== DEFAULT_VOICE ? voice : ""
        if (v) payload.audioVoice = v
      }
      if (supportsDuration && duration !== DEFAULT_DURATION) {
        payload.audioDurationSecs = Number(duration)
      }
    }
    void onConfirm(payload)
  }, [
    prompt,
    kind,
    aspect,
    resolution,
    cand,
    audioKind,
    needsVoice,
    customVoiceMode,
    customVoice,
    voice,
    supportsDuration,
    duration,
    onConfirm,
  ])

  const audioKindLabel = useCallback(
    (k: MediaAudioKind) =>
      k === "speech"
        ? t("design.gen.kindSpeech", "语音")
        : k === "music"
          ? t("design.gen.kindMusic", "音乐")
          : t("design.gen.kindSfx", "音效"),
    [t],
  )

  const willUseLine = cand ? (
    <p className="text-xs text-muted-foreground">
      {t("design.gen.willUse", "将使用：{{provider}} / {{model}}", {
        provider: cand.providerName,
        model: cand.modelName || cand.modelId,
      })}
    </p>
  ) : null

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {kind === "audio" ? (
              <Music className="h-4 w-4" />
            ) : (
              <ImageIcon className="h-4 w-4" />
            )}
            {kind === "audio"
              ? t("design.newAudio", "生成音频")
              : t("design.newImage", "生成图像")}
          </DialogTitle>
        </DialogHeader>

        {overviewLoading ? (
          <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("design.gen.loading", "正在检查可用模型…")}
          </div>
        ) : showEmpty ? (
          /* 空态引导：对应功能无任何可用模型 → 指去设置页配置。 */
          <div className="flex flex-col items-center gap-3 py-8 text-center">
            {kind === "image" ? (
              <ImageOff className="h-8 w-8 text-muted-foreground/60" />
            ) : (
              <VolumeX className="h-8 w-8 text-muted-foreground/60" />
            )}
            <div className="space-y-1">
              <p className="text-sm font-medium">
                {kind === "image"
                  ? t("design.gen.imageUnavailableTitle", "未配置图像服务商")
                  : t("design.gen.audioUnavailableTitle", "未配置音频服务商")}
              </p>
              <p className="text-xs text-muted-foreground">
                {kind === "image"
                  ? t(
                      "design.gen.imageUnavailableDesc",
                      "配置图像服务商后即可在设计空间生成图片。",
                    )
                  : t(
                      "design.gen.audioUnavailableDesc",
                      "配置语音、音乐或音效模型后即可生成音频。",
                    )}
              </p>
            </div>
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                onClick={() => {
                  openMediaModelSettings()
                  onClose()
                }}
              >
                {t("design.gen.goConfigure", "去配置媒体生成模型")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void loadOverview(kind === "audio" ? "audio" : undefined)}
              >
                {t("design.gen.recheck", "重新检查")}
              </Button>
            </div>
          </div>
        ) : (
          <>
            {kind === "audio" && (
              <div className="space-y-1.5">
                <span className="text-sm text-muted-foreground">
                  {t("design.gen.audioKind", "类型")}
                </span>
                <RadioPills<MediaAudioKind>
                  value={audioKind}
                  onChange={setAudioKind}
                  variant="strong"
                  layout="wrap"
                  itemClassName="h-7 px-3 text-xs"
                  ariaLabel={t("design.gen.audioKind", "类型")}
                  options={AUDIO_KINDS.map((k) => {
                    const unavailable = overview != null && !overview[k].available
                    return {
                      value: k,
                      disabled: unavailable,
                      label: (
                        <IconTip
                          label={
                            unavailable
                              ? t("design.gen.kindUnavailable", "该类型暂无可用模型")
                              : undefined
                          }
                        >
                          <span>{audioKindLabel(k)}</span>
                        </IconTip>
                      ),
                    }
                  })}
                />
              </div>
            )}

            <Textarea
              autoFocus
              value={prompt}
              onChange={(e) => handlePromptChange(e.target.value)}
              rows={3}
              placeholder={
                kind === "audio"
                  ? t(
                      "design.audioPromptPlaceholder",
                      "旁白文本，或音乐/音效描述（可加 [music] / [sfx] 前缀）…",
                    )
                  : t("design.imagePromptPlaceholder", "描述你想要的图像…")
              }
              className="resize-none"
            />

            {kind === "image" &&
              (cand?.image?.supportsAspectRatio || cand?.image?.supportsResolution) && (
              <div className="space-y-3">
                {cand?.image?.supportsAspectRatio && (
                  <div className="space-y-1.5">
                    <span className="text-sm text-muted-foreground">
                      {t("design.gen.aspectRatio", "宽高比")}
                    </span>
                    <RadioPills
                      value={aspect}
                      onChange={setAspect}
                      variant="strong"
                      layout="wrap"
                      itemClassName="h-7 px-3 text-xs"
                      ariaLabel={t("design.gen.aspectRatio", "宽高比")}
                      options={[
                        { value: AUTO, label: t("design.gen.auto", "自动") },
                        ...aspectChoices.map((r) => ({ value: r, label: r })),
                      ]}
                    />
                  </div>
                )}
                {cand?.image?.supportsResolution && (
                  <div className="space-y-1.5">
                    <span className="text-sm text-muted-foreground">
                      {t("design.gen.resolution", "分辨率")}
                    </span>
                    <Select value={resolution} onValueChange={setResolution}>
                      <SelectTrigger className="h-8 w-40 text-sm">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={AUTO}>{t("design.gen.auto", "自动")}</SelectItem>
                        {resolutionChoices.map((r) => (
                          <SelectItem key={r} value={r}>
                            {r}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                )}
              </div>
            )}

            {kind === "audio" && (needsVoice || supportsDuration) && (
              <div className="space-y-3">
                {needsVoice && (
                  <div className="space-y-1.5">
                    <span className="text-sm text-muted-foreground">
                      {t("design.gen.voice", "音色")}
                    </span>
                    {customVoiceMode ? (
                      <div className="flex items-center gap-2">
                        <Input
                          autoFocus
                          value={customVoice}
                          onChange={(e) => setCustomVoice(e.target.value)}
                          placeholder={t("design.gen.voiceCustomPh", "输入音色 ID…")}
                          className="h-8 text-sm"
                        />
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-8 shrink-0"
                          onClick={() => {
                            setCustomVoiceMode(false)
                            setCustomVoice("")
                          }}
                        >
                          {t("design.gen.voiceBackToList", "返回列表")}
                        </Button>
                      </div>
                    ) : (
                      <Select
                        value={voice}
                        onValueChange={(v) => {
                          if (v === CUSTOM_VOICE) setCustomVoiceMode(true)
                          else setVoice(v)
                        }}
                        onOpenChange={(o) => {
                          // 懒拉取：下拉首开时才请求音色目录（仅支持列目录的 provider）。
                          if (o && voices === null && !voicesLoading && cand?.supportsVoiceListing) {
                            void loadVoices()
                          }
                        }}
                      >
                        <SelectTrigger className="h-8 w-56 text-sm">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value={DEFAULT_VOICE}>
                            {t("design.gen.voiceDefault", "默认")}
                          </SelectItem>
                          {voicesLoading ? (
                            <div className="flex items-center gap-2 px-2 py-1.5 text-xs text-muted-foreground">
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                              {t("design.gen.voicesLoading", "拉取音色…")}
                            </div>
                          ) : (
                            (voices ?? []).map((v) => (
                              <SelectItem key={v.voiceId} value={v.voiceId}>
                                {v.name}
                              </SelectItem>
                            ))
                          )}
                          <SelectItem value={CUSTOM_VOICE}>
                            {t("design.gen.voiceCustom", "自定义…")}
                          </SelectItem>
                        </SelectContent>
                      </Select>
                    )}
                  </div>
                )}
                {supportsDuration && (
                  <div className="space-y-1.5">
                    <span className="text-sm text-muted-foreground">
                      {t("design.gen.duration", "时长")}
                    </span>
                    <Select value={duration} onValueChange={setDuration}>
                      <SelectTrigger className="h-8 w-40 text-sm">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={DEFAULT_DURATION}>
                          {t("design.gen.durationDefault", "默认")}
                        </SelectItem>
                        {durationChoices.map((s) => (
                          <SelectItem key={s} value={String(s)}>
                            {t("design.gen.durationSecs", "{{n}} 秒", { n: s })}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                )}
              </div>
            )}

            {willUseLine}

            <DialogFooter>
              <Button variant="ghost" onClick={onClose}>
                {t("common.cancel", "取消")}
              </Button>
              <Button onClick={handleConfirm} disabled={busy || !prompt.trim()}>
                {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                {t("design.generate", "生成")}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

export default MediaGenerateDialog

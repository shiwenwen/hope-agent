import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Check, Loader2, Info, RefreshCw } from "lucide-react"

interface AudioGenProviderEntry {
  id: string
  enabled: boolean
  apiKey: string | null
  baseUrl: string | null
  model: string | null
  voice: string | null
}

interface AudioGenConfig {
  providers: AudioGenProviderEntry[]
  timeoutSeconds: number
}

const DEFAULT_CONFIG: AudioGenConfig = {
  providers: [
    { id: "openai", enabled: false, apiKey: null, baseUrl: null, model: null, voice: null },
    { id: "elevenlabs", enabled: false, apiKey: null, baseUrl: null, model: null, voice: null },
  ],
  timeoutSeconds: 120,
}

const PROVIDER_META: Record<
  string,
  { name: string; caps: string; defaultModel: string; baseUrl: string; voiceHint: string }
> = {
  openai: {
    name: "OpenAI",
    caps: "TTS",
    defaultModel: "gpt-4o-mini-tts",
    baseUrl: "https://api.openai.com",
    voiceHint: "alloy",
  },
  elevenlabs: {
    name: "ElevenLabs",
    caps: "TTS + Music",
    defaultModel: "eleven_multilingual_v2",
    baseUrl: "https://api.elevenlabs.io",
    voiceHint: "21m00Tcm4TlvDq8ikWAM",
  },
}

export default function AudioGeneratePanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<AudioGenConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  // B8-1：ElevenLabs 语音实时列表（点按拉取，需已填 key）。
  const [voices, setVoices] = useState<{ voiceId: string; name: string; category?: string }[]>([])
  const [voicesLoading, setVoicesLoading] = useState(false)
  // B8-1：策展音频模型目录（后端单一真相源），按 provider 呈现已知模型预设提示。
  const [catalog, setCatalog] = useState<
    { id: string; label: string; provider: string; kind: string; default: boolean }[]
  >([])
  useEffect(() => {
    let cancelled = false
    void getTransport()
      .call<typeof catalog>("get_audio_model_catalog_cmd")
      .then((c) => {
        if (!cancelled) setCatalog(c ?? [])
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const isDirty = JSON.stringify(config) !== savedSnapshot

  const fetchVoices = async () => {
    setVoicesLoading(true)
    try {
      const list = await getTransport().call<
        { voiceId: string; name: string; category?: string }[]
      >("list_elevenlabs_voices_cmd", { limit: 100 })
      setVoices(list ?? [])
    } catch (e) {
      logger.error("settings", "AudioGeneratePanel", "fetch voices failed", e)
    } finally {
      setVoicesLoading(false)
    }
  }

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<AudioGenConfig>("get_audio_generate_config")
      .then((cfg) => {
        if (!cancelled) {
          setConfig(cfg)
          setSavedSnapshot(JSON.stringify(cfg))
        }
      })
      .catch((e) => logger.error("settings", "AudioGeneratePanel", `load failed: ${e}`))
    return () => {
      cancelled = true
    }
  }, [])

  const save = async () => {
    setSaving(true)
    try {
      await getTransport().call("save_audio_generate_config", { config })
      setSavedSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "AudioGeneratePanel", `save failed: ${e}`)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const updateProvider = (index: number, updates: Partial<AudioGenProviderEntry>) => {
    setConfig((prev) => {
      const providers = [...prev.providers]
      providers[index] = { ...providers[index], ...updates }
      return { ...prev, providers }
    })
  }

  const hasAnyConfigured = config.providers.some((p) => p.enabled && p.apiKey?.trim())

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6">
        <div className="space-y-6">
          <p className="text-xs text-muted-foreground">
            {t(
              "settings.audioGenerateDesc",
              "配置音频合成 Provider（BYOK），用于设计空间「音频」形态：语音旁白 / 音乐 / 音效。按顺序 failover。",
            )}
          </p>

          {!hasAnyConfigured && (
            <div className="flex items-start gap-2 rounded-md bg-muted/50 p-3">
              <Info className="h-4 w-4 mt-0.5 text-muted-foreground shrink-0" />
              <p className="text-xs text-muted-foreground">
                {t("settings.audioGenNoProvider", "尚未配置音频 Provider — 启用其一并填入 API Key。")}
              </p>
            </div>
          )}

          <div className="space-y-4">
            {config.providers.map((provider, index) => {
              const meta = PROVIDER_META[provider.id] ?? {
                name: provider.id,
                caps: "",
                defaultModel: "",
                baseUrl: "",
                voiceHint: "",
              }
              return (
                <div
                  key={provider.id}
                  className={cn(
                    "rounded-lg border p-4 space-y-3 transition-colors",
                    provider.enabled ? "border-primary/30 bg-primary/5" : "border-border",
                  )}
                >
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <span className="flex h-5 w-5 items-center justify-center rounded-full bg-muted text-[10px] font-medium text-muted-foreground">
                        {index + 1}
                      </span>
                      <span className="text-sm font-medium">{meta.name}</span>
                      <span className="text-[10px] text-muted-foreground rounded bg-muted px-1.5 py-0.5">
                        {meta.caps}
                      </span>
                    </div>
                    <Switch
                      checked={provider.enabled}
                      onCheckedChange={(v) => updateProvider(index, { enabled: v })}
                    />
                  </div>

                  {provider.enabled && (
                    <div className="space-y-3 pt-1">
                      <div className="space-y-1.5">
                        <span className="text-xs text-muted-foreground">
                          {t("settings.audioGenApiKey", "API Key")}
                        </span>
                        <Input
                          type="password"
                          value={provider.apiKey ?? ""}
                          placeholder={provider.id === "elevenlabs" ? "xi-..." : "sk-..."}
                          onChange={(e) => updateProvider(index, { apiKey: e.target.value || null })}
                        />
                      </div>
                      <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                          <span className="text-xs text-muted-foreground">
                            {t("settings.audioGenModel", "模型")}
                          </span>
                          <Input
                            value={provider.model ?? ""}
                            placeholder={meta.defaultModel}
                            onChange={(e) => updateProvider(index, { model: e.target.value || null })}
                          />
                          {(() => {
                            const known = catalog.filter((m) => m.provider === provider.id)
                            return known.length > 0 ? (
                              <p className="text-[11px] leading-snug text-muted-foreground">
                                {t("settings.audioGenKnownModels", "已知模型")}：
                                {known.map((m) => `${m.label} (${m.kind})`).join(" · ")}
                              </p>
                            ) : null
                          })()}
                        </div>
                        <div className="space-y-1.5">
                          <span className="flex items-center justify-between text-xs text-muted-foreground">
                            {t("settings.audioGenVoice", "语音 (TTS)")}
                            {provider.id === "elevenlabs" && (
                              <button
                                type="button"
                                onClick={() => void fetchVoices()}
                                disabled={voicesLoading}
                                className="inline-flex items-center gap-1 text-[11px] text-primary hover:underline disabled:opacity-50"
                              >
                                {voicesLoading ? (
                                  <Loader2 className="h-3 w-3 animate-spin" />
                                ) : (
                                  <RefreshCw className="h-3 w-3" />
                                )}
                                {t("settings.audioGenFetchVoices", "拉取语音")}
                              </button>
                            )}
                          </span>
                          {/* 拉到语音后给一个便捷 picker（选中即填入下方输入框）；raw id 输入框
                              **始终保留**——列表外的自定义 / 克隆 / 分页外语音仍可见可编辑（review 修复）。 */}
                          {provider.id === "elevenlabs" && voices.length > 0 && (
                            <Select value="" onValueChange={(v) => updateProvider(index, { voice: v || null })}>
                              <SelectTrigger className="h-8 text-xs">
                                <SelectValue placeholder={t("settings.audioGenPickVoice", "从列表选择…")} />
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
                          )}
                          <Input
                            value={provider.voice ?? ""}
                            placeholder={meta.voiceHint}
                            onChange={(e) =>
                              updateProvider(index, { voice: e.target.value || null })
                            }
                          />
                        </div>
                      </div>
                      <div className="space-y-1.5">
                        <span className="text-xs text-muted-foreground">
                          {t("settings.audioGenBaseUrl", "Base URL")}
                        </span>
                        <Input
                          value={provider.baseUrl ?? ""}
                          placeholder={meta.baseUrl}
                          onChange={(e) => updateProvider(index, { baseUrl: e.target.value || null })}
                        />
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
          </div>

          <div className="space-y-1.5 max-w-[220px]">
            <span className="text-sm font-medium">{t("settings.audioGenTimeout", "请求超时（秒）")}</span>
            <Input
              type="number"
              min={10}
              max={600}
              value={config.timeoutSeconds}
              onChange={(e) => {
                const num = parseInt(e.target.value, 10)
                if (!isNaN(num) && num >= 10) setConfig((prev) => ({ ...prev, timeoutSeconds: num }))
              }}
            />
          </div>
        </div>
      </div>

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

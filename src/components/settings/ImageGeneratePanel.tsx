import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { cn } from "@/lib/utils"
import { Check, Loader2, Info, Wifi, ChevronUp, ChevronDown } from "lucide-react"
import TestResultDisplay, { parseTestResult, type TestResult } from "./TestResultDisplay"

// ── Types ────────────────────────────────────────────────────────

interface ImageGenProviderEntry {
  id: string
  enabled: boolean
  apiKey: string | null
  baseUrl: string | null
  model: string | null
  thinkingLevel: string | null
}

interface ImageGenConfig {
  providers: ImageGenProviderEntry[]
  timeoutSeconds: number
  defaultSize: string
}

const DEFAULT_CONFIG: ImageGenConfig = {
  providers: [
    { id: "openai", enabled: false, apiKey: null, baseUrl: null, model: null, thinkingLevel: null },
    { id: "google", enabled: false, apiKey: null, baseUrl: null, model: null, thinkingLevel: null },
    { id: "fal", enabled: false, apiKey: null, baseUrl: null, model: null, thinkingLevel: null },
  ],
  timeoutSeconds: 60,
  defaultSize: "1024x1024",
}

// Provider display names and defaults
const PROVIDER_DISPLAY: Record<string, { name: string; defaultModel: string; baseUrl: string }> = {
  openai: { name: "OpenAI", defaultModel: "gpt-image-1", baseUrl: "https://api.openai.com" },
  google: { name: "Google", defaultModel: "gemini-3.1-flash-image-preview", baseUrl: "https://generativelanguage.googleapis.com" },
  fal: { name: "Fal", defaultModel: "fal-ai/flux/dev", baseUrl: "https://fal.run" },
}

const GOOGLE_MODEL_OPTIONS = [
  { value: "gemini-3.1-flash-image-preview", label: "Gemini 3.1 Flash Image Preview" },
  { value: "gemini-3-pro-image-preview", label: "Gemini 3 Pro Image Preview" },
  { value: "gemini-2.5-flash-image", label: "Gemini 2.5 Flash Image" },
  { value: "imagen-4.0-generate-001", label: "Imagen 4" },
  { value: "imagen-4.0-ultra-generate-001", label: "Imagen 4 Ultra" },
  { value: "imagen-4.0-fast-generate-001", label: "Imagen 4 Fast" },
]

const SIZE_OPTIONS = ["1024x1024", "1024x1536", "1536x1024"]

function GoogleModelSelect({ value, onChange }: { value: string | null; onChange: (v: string | null) => void }) {
  const { t } = useTranslation()
  const isPreset = !value || GOOGLE_MODEL_OPTIONS.some((o) => o.value === value)
  const [customMode, setCustomMode] = useState(!isPreset)

  if (customMode) {
    return (
      <div className="flex gap-1.5">
        <Input
          className="flex-1"
          value={value ?? ""}
          placeholder="gemini-3.1-flash-image-preview"
          onChange={(e) => onChange(e.target.value || null)}
        />
        <Button
          variant="ghost"
          size="sm"
          className="shrink-0 text-xs px-2"
          onClick={() => {
            setCustomMode(false)
            onChange(null)
          }}
        >
          {t("common.select")}
        </Button>
      </div>
    )
  }

  return (
    <div className="flex gap-1.5">
      <Select
        value={value || GOOGLE_MODEL_OPTIONS[0].value}
        onValueChange={(v) => onChange(v)}
      >
        <SelectTrigger className="flex-1">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {GOOGLE_MODEL_OPTIONS.map((opt) => (
            <SelectItem key={opt.value} value={opt.value}>
              <span className="text-xs">{opt.label}</span>
              <span className="text-[10px] text-muted-foreground ml-1.5">{opt.value}</span>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Button
        variant="ghost"
        size="sm"
        className="shrink-0 text-xs px-2"
        onClick={() => setCustomMode(true)}
      >
        {t("common.custom")}
      </Button>
    </div>
  )
}

export default function ImageGeneratePanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<ImageGenConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState<string>("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [testLoading, setTestLoading] = useState<Record<string, boolean>>({})
  const [testResults, setTestResults] = useState<Record<string, TestResult>>({})

  const isDirty = JSON.stringify(config) !== savedSnapshot

  useEffect(() => {
    let cancelled = false
    invoke<ImageGenConfig>("get_image_generate_config")
      .then((cfg) => {
        if (!cancelled) {
          setConfig(cfg)
          setSavedSnapshot(JSON.stringify(cfg))
        }
      })
      .catch((e) => {
        logger.error("settings", `Failed to load image generate config: ${e}`)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const save = async () => {
    setSaving(true)
    try {
      await invoke("save_image_generate_config", { config })
      setSavedSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", `Failed to save image generate config: ${e}`)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const updateProvider = (index: number, updates: Partial<ImageGenProviderEntry>) => {
    setConfig((prev) => {
      const providers = [...prev.providers]
      providers[index] = { ...providers[index], ...updates }
      return { ...prev, providers }
    })
  }

  const moveProvider = (index: number, direction: "up" | "down") => {
    setConfig((prev) => {
      const target = direction === "up" ? index - 1 : index + 1
      if (target < 0 || target >= prev.providers.length) return prev
      const providers = [...prev.providers]
      ;[providers[index], providers[target]] = [providers[target], providers[index]]
      return { ...prev, providers }
    })
  }

  const handleTest = async (provider: ImageGenProviderEntry) => {
    setTestLoading((prev) => ({ ...prev, [provider.id]: true }))
    setTestResults((prev) => {
      const next = { ...prev }
      delete next[provider.id]
      return next
    })
    try {
      const msg = await invoke<string>("test_image_generate", {
        providerId: provider.id,
        apiKey: provider.apiKey ?? "",
        baseUrl: provider.baseUrl,
      })
      setTestResults((prev) => ({ ...prev, [provider.id]: parseTestResult(msg, false) }))
    } catch (e) {
      setTestResults((prev) => ({ ...prev, [provider.id]: parseTestResult(String(e), true) }))
    } finally {
      setTestLoading((prev) => ({ ...prev, [provider.id]: false }))
    }
  }

  const hasAnyConfigured = config.providers.some(
    (p) => p.enabled && p.apiKey && p.apiKey.trim().length > 0
  )

  const getDisplayName = (id: string) => PROVIDER_DISPLAY[id]?.name ?? id
  const getDefaultModel = (id: string) => PROVIDER_DISPLAY[id]?.defaultModel ?? ""
  const getDefaultBaseUrl = (id: string) => PROVIDER_DISPLAY[id]?.baseUrl ?? ""

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-6">
        {/* Header */}
        <div>
          <p className="text-xs text-muted-foreground">{t("settings.imageGenerateDesc")}</p>
        </div>

        {/* Info banner when no provider is configured */}
        {!hasAnyConfigured && (
          <div className="flex items-start gap-2 rounded-md bg-muted/50 p-3">
            <Info className="h-4 w-4 mt-0.5 text-muted-foreground shrink-0" />
            <p className="text-xs text-muted-foreground">{t("settings.imageGenNoProvider")}</p>
          </div>
        )}

        {/* Providers */}
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
              {t("settings.imageGenProviders")}
            </h3>
          </div>

          {/* Priority hint */}
          <p className="text-xs text-muted-foreground">
            {t("settings.imageGenPriorityHint")}
          </p>

          <div className="space-y-4">
            {config.providers.map((provider, index) => (
              <div
                key={`${provider.id}-${index}`}
                className={cn(
                  "rounded-lg border p-4 space-y-3 transition-colors",
                  provider.enabled ? "border-primary/30 bg-primary/5" : "border-border"
                )}
              >
                {/* Provider header with priority, toggle, and reorder */}
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    {/* Priority badge */}
                    <span className="flex h-5 w-5 items-center justify-center rounded-full bg-muted text-[10px] font-medium text-muted-foreground">
                      {index + 1}
                    </span>
                    <span className="text-sm font-medium">
                      {getDisplayName(provider.id)}
                    </span>
                    {/* Reorder buttons */}
                    <div className="flex items-center">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-6 w-6"
                        disabled={index === 0}
                        onClick={() => moveProvider(index, "up")}
                      >
                        <ChevronUp className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-6 w-6"
                        disabled={index === config.providers.length - 1}
                        onClick={() => moveProvider(index, "down")}
                      >
                        <ChevronDown className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </div>
                  <Switch
                    checked={provider.enabled}
                    onCheckedChange={(v) => updateProvider(index, { enabled: v })}
                  />
                </div>

                {/* Provider details (shown when enabled) */}
                {provider.enabled && (
                  <div className="space-y-3 pt-1">
                    <div className="space-y-1.5">
                      <span className="text-xs text-muted-foreground">{t("settings.imageGenApiKey")}</span>
                      <Input
                        type="password"
                        value={provider.apiKey ?? ""}
                        placeholder="sk-..."
                        onChange={(e) =>
                          updateProvider(index, {
                            apiKey: e.target.value || null,
                          })
                        }
                      />
                    </div>

                    <div className="grid grid-cols-2 gap-3">
                      <div className="space-y-1.5">
                        <span className="text-xs text-muted-foreground">
                          {t("settings.imageGenBaseUrl")}
                        </span>
                        <Input
                          value={provider.baseUrl ?? ""}
                          placeholder={getDefaultBaseUrl(provider.id)}
                          onChange={(e) =>
                            updateProvider(index, {
                              baseUrl: e.target.value || null,
                            })
                          }
                        />
                      </div>

                      <div className="space-y-1.5">
                        <span className="text-xs text-muted-foreground">
                          {t("settings.imageGenModel")}
                        </span>
                        {provider.id === "google" ? (
                          <GoogleModelSelect
                            value={provider.model}
                            onChange={(v) => updateProvider(index, { model: v })}
                          />
                        ) : (
                          <Input
                            value={provider.model ?? ""}
                            placeholder={getDefaultModel(provider.id)}
                            onChange={(e) =>
                              updateProvider(index, {
                                model: e.target.value || null,
                              })
                            }
                          />
                        )}
                      </div>
                    </div>

                    {/* Google-specific: Thinking Level */}
                    {provider.id === "google" && (
                      <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                          <span className="text-xs text-muted-foreground">
                            {t("settings.imageGenThinkingLevel")}
                          </span>
                          <Select
                            value={provider.thinkingLevel || "MINIMAL"}
                            onValueChange={(v) => updateProvider(index, { thinkingLevel: v })}
                          >
                            <SelectTrigger>
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              <SelectItem value="MINIMAL">Minimal</SelectItem>
                              <SelectItem value="HIGH">High</SelectItem>
                            </SelectContent>
                          </Select>
                        </div>
                      </div>
                    )}

                    {/* Test button */}
                    <div className="flex items-center gap-2 pt-1">
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={testLoading[provider.id] || !provider.apiKey?.trim()}
                        onClick={() => handleTest(provider)}
                      >
                        {testLoading[provider.id] ? (
                          <span className="flex items-center gap-2">
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            {t("common.testing")}
                          </span>
                        ) : (
                          <span className="flex items-center gap-2">
                            <Wifi className="h-3.5 w-3.5" />
                            {t("common.test")}
                          </span>
                        )}
                      </Button>
                    </div>

                    {/* Test result */}
                    {testResults[provider.id] && (
                      <TestResultDisplay result={testResults[provider.id]} />
                    )}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>

        {/* General settings */}
        <div className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.imageGenDefaultSize")}</span>
              <Select
                value={config.defaultSize}
                onValueChange={(v) => setConfig((prev) => ({ ...prev, defaultSize: v }))}
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
              <span className="text-sm font-medium">{t("settings.imageGenTimeout")}</span>
              <Input
                type="number"
                min={10}
                max={300}
                value={config.timeoutSeconds}
                onChange={(e) => {
                  const num = parseInt(e.target.value, 10)
                  if (!isNaN(num) && num >= 10) {
                    setConfig((prev) => ({ ...prev, timeoutSeconds: num }))
                  }
                }}
              />
            </div>
          </div>
        </div>

        {/* Save button */}
        <div className="flex items-center gap-2 pt-2">
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
    </div>
  )
}

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import i18n from "@/i18n/i18n"
import { SUPPORTED_LANGUAGES } from "@/i18n/i18n"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  DndContext,
  closestCenter,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  verticalListSortingStrategy,
  arrayMove,
} from "@dnd-kit/sortable"
import {
  Check,
  ChevronDown,
  ChevronRight,
  Loader2,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { WebSearchConfig } from "./types"
import { PROVIDER_META, hasRequiredCredentials } from "./constants"
import { SortableProviderItem } from "./ProviderRow"

export default function WebSearchPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<WebSearchConfig | null>(null)
  const [savedJson, setSavedJson] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [advancedOpen, setAdvancedOpen] = useState(true)

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))

  useEffect(() => {
    invoke<WebSearchConfig>("get_web_search_config")
      .then((cfg) => {
        setConfig(cfg)
        setSavedJson(JSON.stringify(cfg))
      })
      .catch((e) => logger.error("settings", "WebSearchPanel::load", "Failed to load config", e))
  }, [])

  const isDirty = config ? JSON.stringify(config) !== savedJson : false

  const handleSave = useCallback(async () => {
    if (!config) return
    setSaving(true)
    try {
      await invoke("save_web_search_config", { config })
      setSavedJson(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "WebSearchPanel::save", "Failed to save config", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }, [config])

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event
      if (!over || !config || active.id === over.id) return
      const oldIndex = config.providers.findIndex((p) => p.id === active.id)
      const newIndex = config.providers.findIndex((p) => p.id === over.id)
      if (oldIndex === -1 || newIndex === -1) return
      setConfig((prev) =>
        prev ? { ...prev, providers: arrayMove(prev.providers, oldIndex, newIndex) } : prev,
      )
    },
    [config],
  )

  const handleToggleEnabled = useCallback((id: string, enabled: boolean) => {
    setConfig((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        providers: prev.providers.map((p) => (p.id === id ? { ...p, enabled } : p)),
      }
    })
  }, [])

  const handleFieldChange = useCallback(
    (id: string, key: "apiKey" | "apiKey2" | "baseUrl", value: string | null) => {
      setConfig((prev) => {
        if (!prev) return prev
        const providers = prev.providers.map((p) => {
          if (p.id !== id) return p
          const updated = { ...p, [key]: value }
          // Auto-disable if key was cleared and provider requires key
          const meta = PROVIDER_META[id]
          if (meta?.needsApiKey && !hasRequiredCredentials(updated)) {
            updated.enabled = false
          }
          return updated
        })
        return { ...prev, providers }
      })
    },
    [],
  )

  if (!config) return null

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-4">
        <p className="text-xs text-muted-foreground">{t("settings.webSearchDesc")}</p>

        {/* Drag-sortable provider list */}
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
          <SortableContext
            items={config.providers.map((p) => p.id)}
            strategy={verticalListSortingStrategy}
          >
            <div className="space-y-2">
              {config.providers.map((entry, index) => (
                <SortableProviderItem
                  key={entry.id}
                  entry={entry}
                  index={index}
                  expanded={expandedId === entry.id}
                  onToggleExpand={() =>
                    setExpandedId((prev) => (prev === entry.id ? null : entry.id))
                  }
                  onToggleEnabled={(enabled) => handleToggleEnabled(entry.id, enabled)}
                  onFieldChange={(key, value) => handleFieldChange(entry.id, key, value)}
                />
              ))}
            </div>
          </SortableContext>
        </DndContext>

        {/* Advanced settings */}
        <div className="rounded-lg border border-border/50 bg-secondary/20 overflow-hidden">
          <button
            className="flex items-center gap-2 px-3 py-2.5 w-full text-left"
            onClick={() => setAdvancedOpen((prev) => !prev)}
          >
            {advancedOpen ? (
              <ChevronDown className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            )}
            <span className="text-sm font-medium">{t("settings.webSearchAdvanced")}</span>
          </button>

          {advancedOpen && (
            <div className="px-3 pb-3 pt-1 space-y-3 border-t border-border/30">
              <div className="grid grid-cols-3 gap-3">
                {/* Default result count */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchDefaultCount")}
                  </label>
                  <Input
                    type="number"
                    min={1}
                    max={10}
                    className="h-8 text-sm"
                    value={config.defaultResultCount}
                    onChange={(e) =>
                      setConfig((prev) =>
                        prev
                          ? {
                              ...prev,
                              defaultResultCount: Math.max(
                                1,
                                Math.min(10, Number(e.target.value) || 5),
                              ),
                            }
                          : prev,
                      )
                    }
                  />
                  <p className="text-[10px] text-muted-foreground/60">
                    {t("settings.webSearchDefaultCountDesc")}
                  </p>
                </div>

                {/* Timeout */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchTimeout")}
                  </label>
                  <Input
                    type="number"
                    min={5}
                    max={120}
                    className="h-8 text-sm"
                    value={config.timeoutSeconds}
                    onChange={(e) =>
                      setConfig((prev) =>
                        prev
                          ? {
                              ...prev,
                              timeoutSeconds: Math.max(
                                5,
                                Math.min(120, Number(e.target.value) || 30),
                              ),
                            }
                          : prev,
                      )
                    }
                  />
                  <p className="text-[10px] text-muted-foreground/60">
                    {t("settings.webSearchTimeoutDesc")}
                  </p>
                </div>

                {/* Cache TTL */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchCacheTtl")}
                  </label>
                  <Input
                    type="number"
                    min={0}
                    max={60}
                    className="h-8 text-sm"
                    value={config.cacheTtlMinutes}
                    onChange={(e) =>
                      setConfig((prev) =>
                        prev
                          ? {
                              ...prev,
                              cacheTtlMinutes: Math.max(
                                0,
                                Math.min(60, Number(e.target.value) || 0),
                              ),
                            }
                          : prev,
                      )
                    }
                  />
                  <p className="text-[10px] text-muted-foreground/60">
                    {t("settings.webSearchCacheTtlDesc")}
                  </p>
                </div>
              </div>

              <div className="grid grid-cols-3 gap-3">
                {/* Default country */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchDefaultCountry")}
                  </label>
                  <Select
                    value={config.defaultCountry ?? "auto"}
                    onValueChange={(v) =>
                      setConfig((prev) =>
                        prev ? { ...prev, defaultCountry: v === "auto" ? null : v } : prev,
                      )
                    }
                  >
                    <SelectTrigger className="h-8 text-sm">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">{t("settings.webSearchCountryAuto")}</SelectItem>
                      <SelectItem value="CN">🇨🇳 China</SelectItem>
                      <SelectItem value="US">🇺🇸 United States</SelectItem>
                      <SelectItem value="JP">🇯🇵 Japan</SelectItem>
                      <SelectItem value="KR">🇰🇷 South Korea</SelectItem>
                      <SelectItem value="GB">🇬🇧 United Kingdom</SelectItem>
                      <SelectItem value="DE">🇩🇪 Germany</SelectItem>
                      <SelectItem value="FR">🇫🇷 France</SelectItem>
                      <SelectItem value="RU">🇷🇺 Russia</SelectItem>
                      <SelectItem value="BR">🇧🇷 Brazil</SelectItem>
                      <SelectItem value="IN">🇮🇳 India</SelectItem>
                      <SelectItem value="AU">🇦🇺 Australia</SelectItem>
                      <SelectItem value="CA">🇨🇦 Canada</SelectItem>
                      <SelectItem value="SG">🇸🇬 Singapore</SelectItem>
                      <SelectItem value="TW">🇹🇼 Taiwan</SelectItem>
                      <SelectItem value="HK">🇭🇰 Hong Kong</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                {/* Default language */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchDefaultLanguage")}
                  </label>
                  <Select
                    value={config.defaultLanguage ?? "auto"}
                    onValueChange={(v) =>
                      setConfig((prev) =>
                        prev ? { ...prev, defaultLanguage: v === "auto" ? null : v } : prev,
                      )
                    }
                  >
                    <SelectTrigger className="h-8 text-sm">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">
                        {t("settings.webSearchLanguageAuto")} (
                        {SUPPORTED_LANGUAGES.find((l) => l.code === i18n.language)?.label ??
                          i18n.language}
                        )
                      </SelectItem>
                      {SUPPORTED_LANGUAGES.map((lang) => (
                        <SelectItem key={lang.code} value={lang.code.split("-")[0]}>
                          {lang.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                {/* Default freshness */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchDefaultFreshness")}
                  </label>
                  <Select
                    value={config.defaultFreshness ?? "none"}
                    onValueChange={(v) =>
                      setConfig((prev) =>
                        prev ? { ...prev, defaultFreshness: v === "none" ? null : v } : prev,
                      )
                    }
                  >
                    <SelectTrigger className="h-8 text-sm">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">{t("settings.webSearchFreshnessNone")}</SelectItem>
                      <SelectItem value="day">{t("settings.webSearchFreshnessDay")}</SelectItem>
                      <SelectItem value="week">{t("settings.webSearchFreshnessWeek")}</SelectItem>
                      <SelectItem value="month">{t("settings.webSearchFreshnessMonth")}</SelectItem>
                      <SelectItem value="year">{t("settings.webSearchFreshnessYear")}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </div>
          )}
        </div>

        {/* Save button */}
        <div className="flex items-center gap-3 pt-2">
          <Button
            onClick={handleSave}
            disabled={(!isDirty && saveStatus === "idle") || saving}
            size="sm"
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

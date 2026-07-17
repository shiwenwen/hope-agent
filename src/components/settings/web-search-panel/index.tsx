import { useState, useEffect, useCallback, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { SUPPORTED_LANGUAGES } from "@/i18n/i18n"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
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
import { SortableContext, verticalListSortingStrategy, arrayMove } from "@dnd-kit/sortable"
import { Check, ChevronDown, ChevronRight, Loader2, Search, TriangleAlert } from "lucide-react"
import { cn } from "@/lib/utils"
import type { WebSearchConfig } from "./types"
import { PROVIDER_META, hasRequiredCredentials } from "./constants"
import { SortableProviderItem } from "./ProviderRow"

interface WebSearchPanelProps {
  embedded?: boolean
  onSaved?: () => void
  saveLabel?: string
  showAdvanced?: boolean
}

const COUNTRY_OPTIONS = [
  ["CN", "🇨🇳", "China"],
  ["US", "🇺🇸", "United States"],
  ["JP", "🇯🇵", "Japan"],
  ["KR", "🇰🇷", "South Korea"],
  ["GB", "🇬🇧", "United Kingdom"],
  ["DE", "🇩🇪", "Germany"],
  ["FR", "🇫🇷", "France"],
  ["RU", "🇷🇺", "Russia"],
  ["BR", "🇧🇷", "Brazil"],
  ["IN", "🇮🇳", "India"],
  ["AU", "🇦🇺", "Australia"],
  ["CA", "🇨🇦", "Canada"],
  ["SG", "🇸🇬", "Singapore"],
  ["TW", "🇹🇼", "Taiwan"],
  ["HK", "🇭🇰", "Hong Kong"],
] as const

export default function WebSearchPanel({
  embedded = false,
  onSaved,
  saveLabel,
  showAdvanced,
}: WebSearchPanelProps = {}) {
  const { t, i18n } = useTranslation()
  const regionNames = useMemo(() => {
    if (typeof Intl.DisplayNames !== "function") return null
    try {
      return new Intl.DisplayNames([i18n.resolvedLanguage ?? i18n.language], {
        type: "region",
      })
    } catch {
      return null
    }
  }, [i18n.language, i18n.resolvedLanguage])
  const [config, setConfig] = useState<WebSearchConfig | null>(null)
  const [savedJson, setSavedJson] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [advancedOpen, setAdvancedOpen] = useState(false)

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))

  useEffect(() => {
    getTransport()
      .call<WebSearchConfig>("get_web_search_config")
      .then((cfg) => {
        setConfig(cfg)
        setSavedJson(JSON.stringify(cfg))
      })
      .catch((e) => logger.error("settings", "WebSearchPanel::load", "Failed to load config", e))
  }, [])

  const isDirty = config ? JSON.stringify(config) !== savedJson : false

  const persistConfig = useCallback(
    async (nextConfig?: WebSearchConfig) => {
      const configToSave = nextConfig ?? config
      if (!configToSave) return false
      if (JSON.stringify(configToSave) === savedJson) return true
      setSaving(true)
      try {
        await getTransport().call("save_web_search_config", { config: configToSave })
        setSavedJson(JSON.stringify(configToSave))
        setSaveStatus("saved")
        setTimeout(() => setSaveStatus("idle"), 2000)
        return true
      } catch (e) {
        logger.error("settings", "WebSearchPanel::save", "Failed to save config", e)
        setSaveStatus("failed")
        setTimeout(() => setSaveStatus("idle"), 2000)
        return false
      } finally {
        setSaving(false)
      }
    },
    [config, savedJson],
  )

  const handleSave = useCallback(async () => {
    const ok = await persistConfig()
    if (ok) onSaved?.()
  }, [onSaved, persistConfig])

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

  const shouldShowAdvanced = showAdvanced ?? !embedded
  const enabledProviders = config.providers.filter((entry) => entry.enabled)
  const primaryProvider = enabledProviders[0]
  const primaryProviderName = primaryProvider
    ? t(PROVIDER_META[primaryProvider.id]?.labelKey ?? primaryProvider.id)
    : null
  const fallbackCount = Math.max(0, enabledProviders.length - 1)
  const ddgIsPrimary = !primaryProvider || primaryProvider.id === "duck-duck-go"

  return (
    <div className={embedded ? "flex flex-col" : "flex-1 flex flex-col min-h-0 overflow-hidden"}>
      <div className={embedded ? "space-y-4" : "flex-1 overflow-y-auto p-6"}>
        <div className="space-y-4">
          <div className={cn("space-y-1", embedded && "text-center")}>
            <p className="text-xs text-muted-foreground">{t("settings.webSearchDesc")}</p>
            <p className="text-[11px] text-muted-foreground/70">
              {t("settings.webSearchProviderNotice")}
            </p>
          </div>

          {!embedded && (
            <div
              className={`rounded-lg border px-4 py-3 ${
                ddgIsPrimary
                  ? "border-amber-500/25 bg-amber-500/5"
                  : "border-border/60 bg-secondary/20"
              }`}
            >
              <div className="flex items-start gap-3">
                <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
                  <Search className="h-4 w-4" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchRoutingTitle")}
                  </div>
                  {primaryProviderName ? (
                    <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
                      <span className="font-medium">
                        {t("settings.webSearchRouteSummary", {
                          provider: primaryProviderName,
                        })}
                      </span>
                      {fallbackCount > 0 && (
                        <span className="text-xs text-muted-foreground">
                          {t("settings.webSearchFallbackSummary", {
                            count: fallbackCount,
                          })}
                        </span>
                      )}
                    </div>
                  ) : (
                    <div className="mt-0.5 text-sm font-medium">
                      {t("settings.webSearchNoConfiguredRoute")}
                    </div>
                  )}
                </div>
              </div>
              {ddgIsPrimary && (
                <div className="mt-2 flex items-start gap-1.5 pl-11 text-xs leading-relaxed text-amber-700 dark:text-amber-300">
                  <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span>{t("settings.webSearchDdgPrimaryWarning")}</span>
                </div>
              )}
            </div>
          )}

          {/* Drag-sortable provider list */}
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragEnd={handleDragEnd}
          >
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
                    routingRole={
                      !embedded && entry.enabled
                        ? entry.id === primaryProvider?.id
                          ? "primary"
                          : "fallback"
                        : undefined
                    }
                    expanded={expandedId === entry.id}
                    searxngDockerUseProxy={config.searxngDockerUseProxy}
                    onToggleExpand={() =>
                      setExpandedId((prev) => (prev === entry.id ? null : entry.id))
                    }
                    onToggleEnabled={(enabled) => handleToggleEnabled(entry.id, enabled)}
                    onFieldChange={(key, value) => handleFieldChange(entry.id, key, value)}
                    onSearxngDockerUseProxyChange={async (enabled) => {
                      const prevConfig = config
                      const nextConfig = { ...config, searxngDockerUseProxy: enabled }
                      setConfig(nextConfig)
                      const ok = await persistConfig(nextConfig)
                      if (!ok) {
                        setConfig(prevConfig)
                      }
                      return ok
                    }}
                    saveConfig={persistConfig}
                  />
                ))}
              </div>
            </SortableContext>
          </DndContext>

          {shouldShowAdvanced && (
            <div className="rounded-lg border border-border/50 bg-secondary/20 overflow-hidden">
              <Button
                variant="ghost"
                className="h-auto w-full justify-start gap-2 rounded-none px-3 py-2.5 text-left font-normal hover:bg-secondary/40"
                onClick={() => setAdvancedOpen((prev) => !prev)}
              >
                {advancedOpen ? (
                  <ChevronDown className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                ) : (
                  <ChevronRight className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                )}
                <span className="text-sm font-medium">{t("settings.webSearchAdvanced")}</span>
              </Button>

              {advancedOpen && (
                <div className="px-3 pb-3 pt-1 space-y-3 border-t border-border/30">
                  <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                    {/* Default result count */}
                    <div className="space-y-1">
                      <label className="text-xs font-medium text-muted-foreground">
                        {t("settings.webSearchDefaultCount")}
                      </label>
                      <DeferredNumberInput
                        min={1}
                        max={10}
                        className="h-8 text-sm"
                        value={config.defaultResultCount}
                        onValueCommit={(value) =>
                          setConfig((prev) =>
                            prev
                              ? {
                                  ...prev,
                                  defaultResultCount: value,
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
                      <DeferredNumberInput
                        min={5}
                        max={120}
                        className="h-8 text-sm"
                        value={config.timeoutSeconds}
                        onValueCommit={(value) =>
                          setConfig((prev) =>
                            prev
                              ? {
                                  ...prev,
                                  timeoutSeconds: value,
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
                      <DeferredNumberInput
                        min={0}
                        max={60}
                        className="h-8 text-sm"
                        value={config.cacheTtlMinutes}
                        onValueCommit={(value) =>
                          setConfig((prev) =>
                            prev
                              ? {
                                  ...prev,
                                  cacheTtlMinutes: value,
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

                  <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
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
                          {COUNTRY_OPTIONS.map(([code, flag, fallback]) => (
                            <SelectItem key={code} value={code}>
                              {flag} {regionNames?.of(code) ?? fallback}
                            </SelectItem>
                          ))}
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
                          <SelectItem value="none">
                            {t("settings.webSearchFreshnessNone")}
                          </SelectItem>
                          <SelectItem value="day">{t("settings.webSearchFreshnessDay")}</SelectItem>
                          <SelectItem value="week">
                            {t("settings.webSearchFreshnessWeek")}
                          </SelectItem>
                          <SelectItem value="month">
                            {t("settings.webSearchFreshnessMonth")}
                          </SelectItem>
                          <SelectItem value="year">
                            {t("settings.webSearchFreshnessYear")}
                          </SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>

      {/* Save — fixed bottom */}
      <div
        className={
          embedded
            ? "shrink-0 flex justify-end pt-4"
            : "shrink-0 flex justify-end px-6 py-3 border-t border-border/30"
        }
      >
        <Button
          onClick={handleSave}
          disabled={(!embedded && !isDirty && saveStatus === "idle") || saving}
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
            (saveLabel ?? t("common.save"))
          )}
        </Button>
      </div>
    </div>
  )
}

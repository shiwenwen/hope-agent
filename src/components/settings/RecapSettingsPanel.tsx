import { useCallback, useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ModelChainEditor, type ModelChainRef } from "@/components/ui/model-chain-editor"
import type { AvailableModel } from "@/components/ui/model-selector"
import { SUPPORTED_LANGUAGES } from "@/i18n/i18n"

interface RecapConfig {
  /** Deprecated — superseded by `modelOverride`. Read-only display concern;
   * the GUI no longer writes this field. */
  analysisAgent?: string | null
  modelOverride?: ModelChainRef | null
  language?: string | null
  defaultRangeDays: number
  maxSessionsPerReport: number
  facetConcurrency: number
  cacheRetentionDays: number
}

const FOLLOW_LANGUAGE_SENTINEL = "__follow__"

const DEFAULT_CONFIG: RecapConfig = {
  analysisAgent: null,
  modelOverride: null,
  language: null,
  defaultRangeDays: 30,
  maxSessionsPerReport: 500,
  facetConcurrency: 4,
  cacheRetentionDays: 180,
}

export default function RecapSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<RecapConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState<string>("")
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [loaded, setLoaded] = useState(false)

  const persist = useCallback(async (next: RecapConfig) => {
    try {
      await getTransport().call("save_recap_config", { config: next })
      setSavedSnapshot(JSON.stringify(next))
    } catch (e) {
      logger.error("settings", "RecapSettingsPanel::save", "Failed to save recap config", e)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    Promise.all([
      getTransport().call<RecapConfig>("get_recap_config"),
      getTransport().call<AvailableModel[]>("get_available_models").catch(() => []),
    ])
      .then(([cfg, models]) => {
        if (cancelled) return
        const merged = { ...DEFAULT_CONFIG, ...cfg }
        setConfig(merged)
        setSavedSnapshot(JSON.stringify(merged))
        setAvailableModels(models)
        setLoaded(true)
      })
      .catch((e: unknown) => {
        logger.error("settings", "RecapSettingsPanel::load", "Failed to load", e)
        setLoaded(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const commitIfChanged = useCallback(
    (next: RecapConfig) => {
      if (JSON.stringify(next) !== savedSnapshot) {
        void persist(next)
      }
    },
    [persist, savedSnapshot],
  )

  const commitNumber =
    (key: keyof Pick<
      RecapConfig,
      "defaultRangeDays" | "maxSessionsPerReport" | "facetConcurrency" | "cacheRetentionDays"
    >, min: number) =>
    (raw: number) => {
      const clamped = Number.isFinite(raw) ? Math.max(min, Math.round(raw)) : min
      const next = { ...config, [key]: clamped }
      setConfig(next)
      commitIfChanged(next)
    }

  const handleModelOverrideChange = (next: ModelChainRef | null) => {
    // Also clears the deprecated `analysisAgent` field: this panel no longer
    // displays it, but the backend still falls back to it whenever
    // `modelOverride` is unset — leaving it set would keep silently
    // overriding this control with no way to fix it from the GUI.
    const nextConfig: RecapConfig = { ...config, modelOverride: next, analysisAgent: null }
    setConfig(nextConfig)
    commitIfChanged(nextConfig)
  }

  const handleLanguageChange = (value: string) => {
    const next: RecapConfig = {
      ...config,
      language: value === FOLLOW_LANGUAGE_SENTINEL ? null : value,
    }
    setConfig(next)
    commitIfChanged(next)
  }

  if (!loaded) return null

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-1">
        <p className="text-xs text-muted-foreground px-3">{t("settings.recapDesc")}</p>
      </div>

      <div className="mt-4 space-y-6">
        <div className="px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="min-w-0 space-y-0.5 mb-2">
            <div className="text-sm font-medium">{t("settings.recapAnalysisAgent")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapAnalysisAgentDesc")}
            </div>
          </div>
          <ModelChainEditor
            value={config.modelOverride ?? null}
            onChange={handleModelOverrideChange}
            availableModels={availableModels}
            inheritLabel={t("settings.recapAnalysisAgentDefault")}
          />
        </div>

        <div className="flex flex-col gap-3 px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-0.5 sm:pr-4">
            <div className="text-sm font-medium">{t("settings.recapLanguage")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapLanguageDesc")}
            </div>
          </div>
          <Select
            value={config.language?.trim() || FOLLOW_LANGUAGE_SENTINEL}
            onValueChange={handleLanguageChange}
          >
            <SelectTrigger className="h-8 w-full text-sm sm:w-72">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={FOLLOW_LANGUAGE_SENTINEL}>
                {t("settings.recapLanguageFollow")}
              </SelectItem>
              {SUPPORTED_LANGUAGES.map((lang) => (
                <SelectItem key={lang.code} value={lang.code}>
                  {lang.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.recapDefaultRangeDays")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapDefaultRangeDaysDesc")}
            </div>
          </div>
          <DeferredNumberInput
            min={1}
            step={1}
            value={config.defaultRangeDays}
            onValueCommit={commitNumber("defaultRangeDays", 1)}
            className="w-24 h-8 text-sm text-right"
          />
        </div>

        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.recapMaxSessions")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapMaxSessionsDesc")}
            </div>
          </div>
          <DeferredNumberInput
            min={1}
            step={50}
            value={config.maxSessionsPerReport}
            onValueCommit={commitNumber("maxSessionsPerReport", 1)}
            className="w-24 h-8 text-sm text-right"
          />
        </div>

        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.recapFacetConcurrency")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapFacetConcurrencyDesc")}
            </div>
          </div>
          <DeferredNumberInput
            min={1}
            max={32}
            step={1}
            value={config.facetConcurrency}
            onValueCommit={commitNumber("facetConcurrency", 1)}
            className="w-24 h-8 text-sm text-right"
          />
        </div>

        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.recapCacheRetentionDays")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapCacheRetentionDaysDesc")}
            </div>
          </div>
          <DeferredNumberInput
            min={0}
            step={30}
            value={config.cacheRetentionDays}
            onValueCommit={commitNumber("cacheRetentionDays", 0)}
            className="w-24 h-8 text-sm text-right"
          />
        </div>
      </div>
    </div>
  )
}

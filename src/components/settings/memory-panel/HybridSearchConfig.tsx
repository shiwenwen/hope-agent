import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Switch } from "@/components/ui/switch"
import { Slider } from "@/components/ui/slider"
import { Button } from "@/components/ui/button"
import { ChevronRight, Settings2 } from "lucide-react"
import type { useMemoryData } from "./useMemoryData"
import TemporalDecayConfig from "./TemporalDecayConfig"
import {
  memoryAdvancedConfigOperationErrorToast,
  type MemoryAdvancedConfigOperation,
  type MemoryAdvancedConfigOperationErrorToast,
} from "./memoryAdvancedConfigFeedback"
import SettingsResetControl from "../SettingsResetControl"

interface HybridSearchConfig { vectorWeight: number; textWeight: number; rrfK: number }
interface MmrConfig { enabled: boolean; lambda: number }
interface EmbeddingCacheConfig { enabled: boolean; maxEntries: number }
interface MultimodalConfig { enabled: boolean; modalities: string[]; maxFileBytes: number }
interface MemorySelectionConfig { enabled: boolean; threshold: number; maxSelected: number }

type MemoryData = ReturnType<typeof useMemoryData>

interface HybridSearchConfigProps {
  data: MemoryData
  onReset: () => void | Promise<void>
}

export default function HybridSearchConfigSection({ data, onReset }: HybridSearchConfigProps) {
  const { t } = useTranslation()
  const { embeddingConfig, dedupConfig, setDedupConfig, dedupExpanded, setDedupExpanded } = data

  const [hybridConfig, setHybridConfig] = useState<HybridSearchConfig>({ vectorWeight: 0.6, textWeight: 0.4, rrfK: 60 })
  const [mmrConfig, setMmrConfig] = useState<MmrConfig>({ enabled: true, lambda: 0.7 })
  const [cacheConfig, setCacheConfig] = useState<EmbeddingCacheConfig>({ enabled: true, maxEntries: 10000 })
  const [multimodalConfig, setMultimodalConfig] = useState<MultimodalConfig>({ enabled: false, modalities: ["image", "audio"], maxFileBytes: 10 * 1024 * 1024 })
  const [selectionConfig, setSelectionConfig] = useState<MemorySelectionConfig>({ enabled: false, threshold: 8, maxSelected: 5 })
  const [searchTuningExpanded, setSearchTuningExpanded] = useState(false)
  const [advancedConfigLoadError, setAdvancedConfigLoadError] =
    useState<MemoryAdvancedConfigOperationErrorToast | null>(null)
  const saveSeqRef = useRef<Record<string, number>>({})

  const loadAdvancedConfigs = useCallback(
    (isCancelled?: () => boolean) => {
      setAdvancedConfigLoadError(null)

      const loadConfig = <T,>(command: string, apply: (config: T) => void) => {
        void getTransport()
          .call<T>(command)
          .then((config) => {
            if (!isCancelled?.()) apply(config)
          })
          .catch((e) => {
            logger.warn("settings", "HybridSearchConfig::load", `Failed to load ${command}`, e)
            if (isCancelled?.()) return
            const failure = memoryAdvancedConfigOperationErrorToast("load", t, e)
            setAdvancedConfigLoadError((current) => current ?? failure)
          })
      }

      loadConfig<HybridSearchConfig>("get_hybrid_search_config", setHybridConfig)
      loadConfig<MmrConfig>("get_mmr_config", setMmrConfig)
      loadConfig<EmbeddingCacheConfig>("get_embedding_cache_config", setCacheConfig)
      loadConfig<MultimodalConfig>("get_multimodal_config", setMultimodalConfig)
      loadConfig<MemorySelectionConfig>("get_memory_selection_config", setSelectionConfig)
    },
    [t],
  )

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) loadAdvancedConfigs(() => cancelled)
    })
    return () => {
      cancelled = true
    }
  }, [loadAdvancedConfigs])

  const saveAdvancedConfig = useCallback(
    async <T,>(
      operation: Exclude<MemoryAdvancedConfigOperation, "load">,
      command: string,
      config: T,
      rollback: () => void,
    ) => {
      const seq = (saveSeqRef.current[operation] ?? 0) + 1
      saveSeqRef.current[operation] = seq
      try {
        await getTransport().call(command, { config })
      } catch (e) {
        logger.error("settings", "HybridSearchConfig::save", `Failed to save ${command}`, e)
        if (saveSeqRef.current[operation] !== seq) return
        rollback()
        const failure = memoryAdvancedConfigOperationErrorToast(operation, t, e)
        toast.error(failure.title, failure.description ? { description: failure.description } : undefined)
      }
    },
    [t],
  )

  return (
    <>
      <div className="mt-6 flex items-center justify-between gap-3 border-t border-border/50 pt-4">
        <div>
          <div className="text-sm font-medium">
            {t("settings.memorySearchTuning")}
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("settings.memorySearchTuningDesc")}
          </p>
        </div>
        <SettingsResetControl
          scope="memory"
          resetSection="retrieval"
          sectionLabel={t("settings.memorySearchTuning")}
          level="region"
          onReset={onReset}
        />
      </div>
      {advancedConfigLoadError && (
        <div className="mt-4 rounded border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
          <div className="font-medium text-foreground">{advancedConfigLoadError.title}</div>
          {advancedConfigLoadError.description && (
            <div className="mt-1 break-all text-muted-foreground">
              {advancedConfigLoadError.description}
            </div>
          )}
          <button
            type="button"
            className="mt-2 font-medium text-foreground underline underline-offset-2"
            onClick={() => loadAdvancedConfigs()}
          >
            {t("common.retry", "Retry")}
          </button>
        </div>
      )}

      {/* Dedup thresholds (advanced) */}
      <div className="mt-6 pt-4 border-t border-border/50">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setDedupExpanded(!dedupExpanded)}
          className="h-auto -ml-2 gap-1 px-2 py-1 text-sm font-medium text-muted-foreground hover:bg-transparent hover:text-foreground"
        >
          <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", dedupExpanded && "rotate-90")} />
          {t("settings.memoryDedupAdvanced")}
        </Button>
        {dedupExpanded && (
          <div className="mt-3 space-y-3">
            <p className="text-xs text-muted-foreground">{t("settings.memoryDedupAdvancedDesc")}</p>
            <div className="flex items-center gap-3">
              <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[100px]">{t("settings.memoryDedupHigh")}</label>
              <DeferredNumberInput
                step={0.001}
                min={0.005}
                max={0.1}
                value={dedupConfig.thresholdHigh}
                integer={false}
                onValueCommit={(value) => {
                  const previous = dedupConfig
                  const updated = { ...dedupConfig, thresholdHigh: value }
                  setDedupConfig(updated)
                  void saveAdvancedConfig("saveDedup", "save_dedup_config", updated, () =>
                    setDedupConfig(previous),
                  )
                }}
                className="h-7 text-xs w-24"
              />
            </div>
            <div className="flex items-center gap-3">
              <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[100px]">{t("settings.memoryDedupMerge")}</label>
              <DeferredNumberInput
                step={0.001}
                min={0.005}
                max={0.1}
                value={dedupConfig.thresholdMerge}
                integer={false}
                onValueCommit={(value) => {
                  const previous = dedupConfig
                  const updated = { ...dedupConfig, thresholdMerge: value }
                  setDedupConfig(updated)
                  void saveAdvancedConfig("saveDedup", "save_dedup_config", updated, () =>
                    setDedupConfig(previous),
                  )
                }}
                className="h-7 text-xs w-24"
              />
            </div>
          </div>
        )}
      </div>

      {/* Search Tuning (advanced) */}
      <div className="mt-6 pt-4 border-t border-border/50">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setSearchTuningExpanded(!searchTuningExpanded)}
          className="h-auto -ml-2 gap-1 px-2 py-1 text-sm font-medium text-muted-foreground hover:bg-transparent hover:text-foreground"
        >
          <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", searchTuningExpanded && "rotate-90")} />
          <Settings2 className="h-3.5 w-3.5 mr-0.5" />
          {t("settings.memorySearchTuning")}
        </Button>
        {searchTuningExpanded && (
          <div className="mt-3 space-y-5">
            <p className="text-xs text-muted-foreground">{t("settings.memorySearchTuningDesc")}</p>

            {/* Hybrid search weights */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium">{t("settings.memoryVectorWeight")}</label>
                <span className="text-xs text-muted-foreground tabular-nums">
                  {t("settings.memoryVectorTextRatio", { vector: hybridConfig.vectorWeight.toFixed(1), text: hybridConfig.textWeight.toFixed(1) })}
                </span>
              </div>
              <Slider
                aria-label={t("settings.memoryVectorWeight")}
                value={[hybridConfig.vectorWeight]}
                min={0} max={1} step={0.1}
                onValueChange={([v]) => {
                  const previous = hybridConfig
                  const updated = { ...hybridConfig, vectorWeight: v, textWeight: parseFloat((1 - v).toFixed(1)) }
                  setHybridConfig(updated)
                  void saveAdvancedConfig("saveHybrid", "save_hybrid_search_config", updated, () =>
                    setHybridConfig(previous),
                  )
                }}
              />
            </div>

            {/* Temporal decay */}
            <TemporalDecayConfig />

            {/* MMR diversity */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium">{t("settings.memoryMmr")}</label>
                <Switch
                  checked={mmrConfig.enabled}
                  aria-label={t("settings.memoryMmr")}
                  onCheckedChange={(v) => {
                    const previous = mmrConfig
                    const updated = { ...mmrConfig, enabled: v }
                    setMmrConfig(updated)
                    void saveAdvancedConfig("saveMmr", "save_mmr_config", updated, () =>
                      setMmrConfig(previous),
                    )
                  }}
                />
              </div>
              <p className="text-xs text-muted-foreground">{t("settings.memoryMmrDesc")}</p>
              {mmrConfig.enabled && (
                <div className="space-y-1">
                  <div className="flex items-center justify-between">
                    <label className="text-xs text-muted-foreground">{t("settings.memoryMmrLambda")}</label>
                    <span className="text-xs text-muted-foreground tabular-nums">{mmrConfig.lambda.toFixed(1)}</span>
                  </div>
                  <Slider
                    aria-label={t("settings.memoryMmrLambda")}
                    value={[mmrConfig.lambda]}
                    min={0} max={1} step={0.1}
                    onValueChange={([v]) => {
                      const previous = mmrConfig
                      const updated = { ...mmrConfig, lambda: v }
                      setMmrConfig(updated)
                      void saveAdvancedConfig("saveMmr", "save_mmr_config", updated, () =>
                        setMmrConfig(previous),
                      )
                    }}
                  />
                  <div className="flex justify-between text-[10px] text-muted-foreground/50">
                    <span>{t("settings.memoryMmrDiversity")}</span>
                    <span>{t("settings.memoryMmrRelevance")}</span>
                  </div>
                </div>
              )}
            </div>

            {/* Embedding cache */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium">{t("settings.memoryEmbeddingCache")}</label>
                <Switch
                  checked={cacheConfig.enabled}
                  aria-label={t("settings.memoryEmbeddingCache")}
                  onCheckedChange={(v) => {
                    const previous = cacheConfig
                    const updated = { ...cacheConfig, enabled: v }
                    setCacheConfig(updated)
                    void saveAdvancedConfig("saveCache", "save_embedding_cache_config", updated, () =>
                      setCacheConfig(previous),
                    )
                  }}
                />
              </div>
              <p className="text-xs text-muted-foreground">{t("settings.memoryEmbeddingCacheDesc")}</p>
            </div>

            {/* Multimodal embedding */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium">{t("settings.memoryMultimodal")}</label>
                <Switch
                  checked={multimodalConfig.enabled}
                  aria-label={t("settings.memoryMultimodal")}
                  onCheckedChange={(v) => {
                    const previous = multimodalConfig
                    const updated = { ...multimodalConfig, enabled: v }
                    setMultimodalConfig(updated)
                    void saveAdvancedConfig(
                      "saveMultimodal",
                      "save_multimodal_config",
                      updated,
                      () => setMultimodalConfig(previous),
                    )
                  }}
                />
              </div>
              <p className="text-xs text-muted-foreground">{t("settings.memoryMultimodalDesc")}</p>
              {multimodalConfig.enabled && (
                <div className="space-y-2 pl-1">
                  <div className="flex items-center gap-3">
                    <label className="text-xs text-muted-foreground">{t("settings.memoryMultimodalModalities")}:</label>
                    <label className="flex items-center gap-1.5 text-xs">
                      <Switch
                        checked={multimodalConfig.modalities.includes("image")}
                        aria-label={t("settings.memoryMultimodalImage")}
                        onCheckedChange={(checked) => {
                          const previous = multimodalConfig
                          const mods = checked
                            ? [...multimodalConfig.modalities, "image"]
                            : multimodalConfig.modalities.filter(m => m !== "image")
                          const updated = { ...multimodalConfig, modalities: mods }
                          setMultimodalConfig(updated)
                          void saveAdvancedConfig(
                            "saveMultimodal",
                            "save_multimodal_config",
                            updated,
                            () => setMultimodalConfig(previous),
                          )
                        }}
                      />
                      {t("settings.memoryMultimodalImage")}
                    </label>
                    <label className="flex items-center gap-1.5 text-xs">
                      <Switch
                        checked={multimodalConfig.modalities.includes("audio")}
                        aria-label={t("settings.memoryMultimodalAudio")}
                        onCheckedChange={(checked) => {
                          const previous = multimodalConfig
                          const mods = checked
                            ? [...multimodalConfig.modalities, "audio"]
                            : multimodalConfig.modalities.filter(m => m !== "audio")
                          const updated = { ...multimodalConfig, modalities: mods }
                          setMultimodalConfig(updated)
                          void saveAdvancedConfig(
                            "saveMultimodal",
                            "save_multimodal_config",
                            updated,
                            () => setMultimodalConfig(previous),
                          )
                        }}
                      />
                      {t("settings.memoryMultimodalAudio")}
                    </label>
                  </div>
                  <div className="flex items-center gap-2">
                    <label className="text-xs text-muted-foreground">{t("settings.memoryMultimodalMaxSize")}:</label>
                    <DeferredNumberInput
                      min={1} max={50}
                      value={Math.round(multimodalConfig.maxFileBytes / (1024 * 1024))}
                      onValueCommit={(mb) => {
                        const previous = multimodalConfig
                        const updated = { ...multimodalConfig, maxFileBytes: mb * 1024 * 1024 }
                        setMultimodalConfig(updated)
                        void saveAdvancedConfig(
                          "saveMultimodal",
                          "save_multimodal_config",
                          updated,
                          () => setMultimodalConfig(previous),
                        )
                      }}
                      className="h-7 text-xs w-16"
                    />
                    <span className="text-xs text-muted-foreground">MB</span>
                  </div>
                  {!(embeddingConfig.providerType === "google" && embeddingConfig.apiModel?.includes("embedding-2")) && (
                    <p className="text-xs text-amber-500">{t("settings.memoryMultimodalRequiresGemini")}</p>
                  )}
                </div>
              )}
            </div>

            {/* LLM memory selection */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium">{t("settings.memorySelection")}</label>
                <Switch
                  checked={selectionConfig.enabled}
                  aria-label={t("settings.memorySelection")}
                  onCheckedChange={(v) => {
                    const previous = selectionConfig
                    const updated = { ...selectionConfig, enabled: v }
                    setSelectionConfig(updated)
                    void saveAdvancedConfig(
                      "saveSelection",
                      "save_memory_selection_config",
                      updated,
                      () => setSelectionConfig(previous),
                    )
                  }}
                />
              </div>
              <p className="text-xs text-muted-foreground">{t("settings.memorySelectionDesc")}</p>
              {selectionConfig.enabled && (
                <div className="space-y-2 pl-1">
                  <div className="flex items-center gap-3">
                    <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[140px]">
                      {t("settings.memorySelectionThreshold")}
                    </label>
                    <DeferredNumberInput
                      min={1}
                      max={50}
                      value={selectionConfig.threshold}
                      onValueCommit={(threshold) => {
                        const previous = selectionConfig
                        const updated = { ...selectionConfig, threshold }
                        setSelectionConfig(updated)
                        void saveAdvancedConfig(
                          "saveSelection",
                          "save_memory_selection_config",
                          updated,
                          () => setSelectionConfig(previous),
                        )
                      }}
                      className="h-7 text-xs w-20"
                    />
                  </div>
                  <p className="text-xs text-muted-foreground/70 pl-[148px]">
                    {t("settings.memorySelectionThresholdDesc")}
                  </p>
                  <div className="flex items-center gap-3">
                    <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[140px]">
                      {t("settings.memorySelectionMaxSelected")}
                    </label>
                    <DeferredNumberInput
                      min={1}
                      max={20}
                      value={selectionConfig.maxSelected}
                      onValueCommit={(maxSelected) => {
                        const previous = selectionConfig
                        const updated = { ...selectionConfig, maxSelected }
                        setSelectionConfig(updated)
                        void saveAdvancedConfig(
                          "saveSelection",
                          "save_memory_selection_config",
                          updated,
                          () => setSelectionConfig(previous),
                        )
                      }}
                      className="h-7 text-xs w-20"
                    />
                  </div>
                  <p className="text-xs text-muted-foreground/70 pl-[148px]">
                    {t("settings.memorySelectionMaxSelectedDesc")}
                  </p>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </>
  )
}

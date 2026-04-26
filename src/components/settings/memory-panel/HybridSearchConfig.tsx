import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Slider } from "@/components/ui/slider"
import { Button } from "@/components/ui/button"
import { ChevronRight, Settings2 } from "lucide-react"
import type { useMemoryData } from "./useMemoryData"
import TemporalDecayConfig from "./TemporalDecayConfig"

interface HybridSearchConfig { vectorWeight: number; textWeight: number; rrfK: number }
interface MmrConfig { enabled: boolean; lambda: number }
interface EmbeddingCacheConfig { enabled: boolean; maxEntries: number }
interface MultimodalConfig { enabled: boolean; modalities: string[]; maxFileBytes: number }
interface MemorySelectionConfig { enabled: boolean; threshold: number; maxSelected: number }

type MemoryData = ReturnType<typeof useMemoryData>

interface HybridSearchConfigProps {
  data: MemoryData
}

export default function HybridSearchConfigSection({ data }: HybridSearchConfigProps) {
  const { t } = useTranslation()
  const { embeddingConfig, dedupConfig, setDedupConfig, dedupExpanded, setDedupExpanded } = data

  const [hybridConfig, setHybridConfig] = useState<HybridSearchConfig>({ vectorWeight: 0.6, textWeight: 0.4, rrfK: 60 })
  const [mmrConfig, setMmrConfig] = useState<MmrConfig>({ enabled: true, lambda: 0.7 })
  const [cacheConfig, setCacheConfig] = useState<EmbeddingCacheConfig>({ enabled: true, maxEntries: 10000 })
  const [multimodalConfig, setMultimodalConfig] = useState<MultimodalConfig>({ enabled: false, modalities: ["image", "audio"], maxFileBytes: 10 * 1024 * 1024 })
  const [selectionConfig, setSelectionConfig] = useState<MemorySelectionConfig>({ enabled: false, threshold: 8, maxSelected: 5 })
  const [searchTuningExpanded, setSearchTuningExpanded] = useState(false)

  useEffect(() => {
    getTransport().call<HybridSearchConfig>("get_hybrid_search_config").then(setHybridConfig).catch(() => {})
    getTransport().call<MmrConfig>("get_mmr_config").then(setMmrConfig).catch(() => {})
    getTransport().call<EmbeddingCacheConfig>("get_embedding_cache_config").then(setCacheConfig).catch(() => {})
    getTransport().call<MultimodalConfig>("get_multimodal_config").then(setMultimodalConfig).catch(() => {})
    getTransport().call<MemorySelectionConfig>("get_memory_selection_config").then(setSelectionConfig).catch(() => {})
  }, [])

  return (
    <>
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
              <Input
                type="number"
                step={0.001}
                min={0.005}
                max={0.1}
                value={dedupConfig.thresholdHigh}
                onChange={(e) => {
                  const val = parseFloat(e.target.value)
                  if (!isNaN(val)) {
                    const updated = { ...dedupConfig, thresholdHigh: val }
                    setDedupConfig(updated)
                    getTransport().call("save_dedup_config", { config: updated }).catch(() => {})
                  }
                }}
                className="h-7 text-xs w-24"
              />
            </div>
            <div className="flex items-center gap-3">
              <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[100px]">{t("settings.memoryDedupMerge")}</label>
              <Input
                type="number"
                step={0.001}
                min={0.005}
                max={0.1}
                value={dedupConfig.thresholdMerge}
                onChange={(e) => {
                  const val = parseFloat(e.target.value)
                  if (!isNaN(val)) {
                    const updated = { ...dedupConfig, thresholdMerge: val }
                    setDedupConfig(updated)
                    getTransport().call("save_dedup_config", { config: updated }).catch(() => {})
                  }
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
                value={[hybridConfig.vectorWeight]}
                min={0} max={1} step={0.1}
                onValueChange={([v]) => {
                  const updated = { ...hybridConfig, vectorWeight: v, textWeight: parseFloat((1 - v).toFixed(1)) }
                  setHybridConfig(updated)
                  getTransport().call("save_hybrid_search_config", { config: updated }).catch(() => {})
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
                  onCheckedChange={(v) => {
                    const updated = { ...mmrConfig, enabled: v }
                    setMmrConfig(updated)
                    getTransport().call("save_mmr_config", { config: updated }).catch(() => {})
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
                    value={[mmrConfig.lambda]}
                    min={0} max={1} step={0.1}
                    onValueChange={([v]) => {
                      const updated = { ...mmrConfig, lambda: v }
                      setMmrConfig(updated)
                      getTransport().call("save_mmr_config", { config: updated }).catch(() => {})
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
                  onCheckedChange={(v) => {
                    const updated = { ...cacheConfig, enabled: v }
                    setCacheConfig(updated)
                    getTransport().call("save_embedding_cache_config", { config: updated }).catch(() => {})
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
                  onCheckedChange={(v) => {
                    const updated = { ...multimodalConfig, enabled: v }
                    setMultimodalConfig(updated)
                    getTransport().call("save_multimodal_config", { config: updated }).catch(() => {})
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
                        onCheckedChange={(checked) => {
                          const mods = checked
                            ? [...multimodalConfig.modalities, "image"]
                            : multimodalConfig.modalities.filter(m => m !== "image")
                          const updated = { ...multimodalConfig, modalities: mods }
                          setMultimodalConfig(updated)
                          getTransport().call("save_multimodal_config", { config: updated }).catch(() => {})
                        }}
                      />
                      {t("settings.memoryMultimodalImage")}
                    </label>
                    <label className="flex items-center gap-1.5 text-xs">
                      <Switch
                        checked={multimodalConfig.modalities.includes("audio")}
                        onCheckedChange={(checked) => {
                          const mods = checked
                            ? [...multimodalConfig.modalities, "audio"]
                            : multimodalConfig.modalities.filter(m => m !== "audio")
                          const updated = { ...multimodalConfig, modalities: mods }
                          setMultimodalConfig(updated)
                          getTransport().call("save_multimodal_config", { config: updated }).catch(() => {})
                        }}
                      />
                      {t("settings.memoryMultimodalAudio")}
                    </label>
                  </div>
                  <div className="flex items-center gap-2">
                    <label className="text-xs text-muted-foreground">{t("settings.memoryMultimodalMaxSize")}:</label>
                    <Input
                      type="number"
                      min={1} max={50}
                      value={Math.round(multimodalConfig.maxFileBytes / (1024 * 1024))}
                      onChange={(e) => {
                        const mb = parseInt(e.target.value)
                        if (!isNaN(mb) && mb > 0) {
                          const updated = { ...multimodalConfig, maxFileBytes: mb * 1024 * 1024 }
                          setMultimodalConfig(updated)
                          getTransport().call("save_multimodal_config", { config: updated }).catch(() => {})
                        }
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
                  onCheckedChange={(v) => {
                    const updated = { ...selectionConfig, enabled: v }
                    setSelectionConfig(updated)
                    getTransport().call("save_memory_selection_config", { config: updated }).catch(() => {})
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
                    <Input
                      type="number"
                      min={1}
                      max={50}
                      value={selectionConfig.threshold}
                      onChange={(e) => {
                        const raw = parseInt(e.target.value)
                        if (!Number.isFinite(raw)) return
                        setSelectionConfig((prev) => ({ ...prev, threshold: raw }))
                      }}
                      onBlur={() => {
                        setSelectionConfig((prev) => {
                          const clamped = Math.max(1, Math.min(50, prev.threshold || 1))
                          const updated = { ...prev, threshold: clamped }
                          getTransport().call("save_memory_selection_config", { config: updated }).catch(() => {})
                          return updated
                        })
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
                    <Input
                      type="number"
                      min={1}
                      max={20}
                      value={selectionConfig.maxSelected}
                      onChange={(e) => {
                        const raw = parseInt(e.target.value)
                        if (!Number.isFinite(raw)) return
                        setSelectionConfig((prev) => ({ ...prev, maxSelected: raw }))
                      }}
                      onBlur={() => {
                        setSelectionConfig((prev) => {
                          const clamped = Math.max(1, Math.min(20, prev.maxSelected || 1))
                          const updated = { ...prev, maxSelected: clamped }
                          getTransport().call("save_memory_selection_config", { config: updated }).catch(() => {})
                          return updated
                        })
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

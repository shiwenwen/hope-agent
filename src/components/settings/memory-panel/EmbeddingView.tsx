import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { invoke } from "@tauri-apps/api/core"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Slider } from "@/components/ui/slider"
import {
  ArrowLeft,
  ChevronRight,
  Download,
  Loader2,
  Check,
  Zap,
  Wifi,
  Sparkles,
  Settings2,
} from "lucide-react"
import TestResultDisplay, { parseTestResult } from "../TestResultDisplay"
import type { useMemoryData } from "./useMemoryData"

interface HybridSearchConfig { vectorWeight: number; textWeight: number; rrfK: number }
interface TemporalDecayConfig { enabled: boolean; halfLifeDays: number }
interface MmrConfig { enabled: boolean; lambda: number }
interface EmbeddingCacheConfig { enabled: boolean; maxEntries: number }
interface MultimodalConfig { enabled: boolean; modalities: string[]; maxFileBytes: number }

type MemoryData = ReturnType<typeof useMemoryData>

interface EmbeddingViewProps {
  data: MemoryData
}

export default function EmbeddingView({ data }: EmbeddingViewProps) {
  const { t } = useTranslation()

  // Search tuning configs
  const [hybridConfig, setHybridConfig] = useState<HybridSearchConfig>({ vectorWeight: 0.6, textWeight: 0.4, rrfK: 60 })
  const [decayConfig, setDecayConfig] = useState<TemporalDecayConfig>({ enabled: false, halfLifeDays: 30 })
  const [mmrConfig, setMmrConfig] = useState<MmrConfig>({ enabled: true, lambda: 0.7 })
  const [cacheConfig, setCacheConfig] = useState<EmbeddingCacheConfig>({ enabled: true, maxEntries: 10000 })
  const [multimodalConfig, setMultimodalConfig] = useState<MultimodalConfig>({ enabled: false, modalities: ["image", "audio"], maxFileBytes: 10 * 1024 * 1024 })
  const [searchTuningExpanded, setSearchTuningExpanded] = useState(false)

  useEffect(() => {
    invoke<HybridSearchConfig>("get_hybrid_search_config").then(setHybridConfig).catch(() => {})
    invoke<TemporalDecayConfig>("get_temporal_decay_config").then(setDecayConfig).catch(() => {})
    invoke<MmrConfig>("get_mmr_config").then(setMmrConfig).catch(() => {})
    invoke<EmbeddingCacheConfig>("get_embedding_cache_config").then(setCacheConfig).catch(() => {})
    invoke<MultimodalConfig>("get_multimodal_config").then(setMultimodalConfig).catch(() => {})
  }, [])

  const {
    setView,
    totalCount,
    embeddingConfig, setEmbeddingConfig,
    presets,
    localModels,
    embeddingDirty, setEmbeddingDirty,
    embeddingTestLoading, setEmbeddingTestLoading,
    embeddingTestResult, setEmbeddingTestResult,
    embeddingSaving,
    embeddingSaveStatus,
    dedupConfig, setDedupConfig,
    dedupExpanded, setDedupExpanded,
    batchLoading,
    saveEmbeddingConfig,
    handleReembedAll,
  } = data

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-4xl">
        <button
          onClick={() => setView("list")}
          className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground mb-4"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("settings.memory")}
        </button>

        <h2 className="text-lg font-semibold mb-1">{t("settings.memoryEmbedding")}</h2>
        <p className="text-xs text-muted-foreground mb-6">{t("settings.memoryEmbeddingDesc")}</p>

        {/* Enable toggle */}
        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 mb-4">
          <div>
            <div className="text-sm font-medium">{t("settings.memoryEmbeddingEnabled")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.memoryEmbeddingEnabledDesc")}
            </div>
          </div>
          <Switch
            checked={embeddingConfig.enabled}
            onCheckedChange={(v) => {
              setEmbeddingConfig({ ...embeddingConfig, enabled: v })
              setEmbeddingDirty(true)
            }}
          />
        </div>

        {embeddingConfig.enabled && (
          <div className="space-y-4">
            {/* Provider type selector */}
            <div>
              <label className="text-sm font-medium mb-1.5 block">
                {t("settings.memoryEmbeddingProvider")}
              </label>
              <div className="flex flex-wrap gap-2">
                <button
                  onClick={() => {
                    setEmbeddingConfig({
                      ...embeddingConfig,
                      providerType: "auto",
                      apiBaseUrl: null,
                      apiKey: null,
                      apiModel: null,
                      apiDimensions: null,
                    })
                    setEmbeddingDirty(true)
                  }}
                  className={cn(
                    "px-3 py-1.5 rounded-lg text-xs border transition-colors flex items-center gap-1",
                    embeddingConfig.providerType === "auto"
                      ? "border-primary bg-primary/10 text-primary"
                      : "border-border text-muted-foreground hover:border-foreground/30",
                  )}
                >
                  <Sparkles className="h-3 w-3" />
                  Auto
                </button>
                {presets.map((preset) => (
                  <button
                    key={preset.name}
                    onClick={() => {
                      setEmbeddingConfig({
                        ...embeddingConfig,
                        providerType: preset.providerType,
                        apiBaseUrl: preset.baseUrl,
                        apiKey: null,
                        apiModel: preset.defaultModel,
                        apiDimensions: preset.defaultDimensions,
                      })
                      setEmbeddingDirty(true)
                    }}
                    className={cn(
                      "px-3 py-1.5 rounded-lg text-xs border transition-colors",
                      embeddingConfig.apiBaseUrl === preset.baseUrl
                        ? "border-primary bg-primary/10 text-primary"
                        : "border-border text-muted-foreground hover:border-foreground/30",
                    )}
                  >
                    {preset.name}
                  </button>
                ))}
                <button
                  onClick={() => {
                    setEmbeddingConfig({
                      ...embeddingConfig,
                      providerType: "local",
                      apiBaseUrl: null,
                      apiKey: null,
                      apiModel: null,
                    })
                    setEmbeddingDirty(true)
                  }}
                  className={cn(
                    "px-3 py-1.5 rounded-lg text-xs border transition-colors",
                    embeddingConfig.providerType === "local"
                      ? "border-primary bg-primary/10 text-primary"
                      : "border-border text-muted-foreground hover:border-foreground/30",
                  )}
                >
                  {t("settings.memoryLocalModel")}
                </button>
              </div>
            </div>

            {embeddingConfig.providerType === "auto" ? (
              /* Auto mode info */
              <div className="rounded-lg bg-primary/5 border border-primary/20 p-3">
                <p className="text-xs text-muted-foreground">
                  {t("settings.memoryEmbeddingAutoDesc")}
                </p>
              </div>
            ) : embeddingConfig.providerType !== "local" ? (
              /* API mode fields */
              <div className="space-y-3">
                <div>
                  <label className="text-xs text-muted-foreground mb-1 block">API Base URL</label>
                  <Input
                    value={embeddingConfig.apiBaseUrl ?? ""}
                    onChange={(e) => {
                      setEmbeddingConfig({ ...embeddingConfig, apiBaseUrl: e.target.value })
                      setEmbeddingDirty(true)
                    }}
                    placeholder="https://api.openai.com"
                    className="text-sm"
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                  />
                </div>
                <div>
                  <label className="text-xs text-muted-foreground mb-1 block">API Key</label>
                  <Input
                    type="password"
                    value={embeddingConfig.apiKey ?? ""}
                    onChange={(e) => {
                      setEmbeddingConfig({ ...embeddingConfig, apiKey: e.target.value })
                      setEmbeddingDirty(true)
                    }}
                    placeholder="sk-..."
                    className="text-sm"
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                  />
                </div>
                <div className="flex gap-3">
                  <div className="flex-1">
                    <label className="text-xs text-muted-foreground mb-1 block">
                      {t("settings.memoryModel")}
                    </label>
                    <Input
                      value={embeddingConfig.apiModel ?? ""}
                      onChange={(e) => {
                        setEmbeddingConfig({ ...embeddingConfig, apiModel: e.target.value })
                        setEmbeddingDirty(true)
                      }}
                      placeholder="text-embedding-3-small"
                      className="text-sm"
                      autoCapitalize="off"
                      autoCorrect="off"
                      spellCheck={false}
                    />
                  </div>
                  <div className="w-28">
                    <label className="text-xs text-muted-foreground mb-1 block">
                      {t("settings.memoryDimensions")}
                    </label>
                    <Input
                      type="number"
                      value={embeddingConfig.apiDimensions ?? ""}
                      onChange={(e) => {
                        setEmbeddingConfig({
                          ...embeddingConfig,
                          apiDimensions: e.target.value ? Number(e.target.value) : null,
                        })
                        setEmbeddingDirty(true)
                      }}
                      placeholder="1536"
                      className="text-sm"
                    />
                  </div>
                </div>
              </div>
            ) : (
              /* Local model selector */
              <div className="space-y-2">
                <label className="text-sm font-medium mb-1.5 block">
                  {t("settings.memorySelectModel")}
                </label>
                {localModels.map((model) => (
                  <button
                    key={model.id}
                    onClick={() => {
                      setEmbeddingConfig({
                        ...embeddingConfig,
                        localModelId: model.id,
                        apiDimensions: model.dimensions,
                      })
                      setEmbeddingDirty(true)
                    }}
                    className={cn(
                      "w-full flex items-center justify-between px-3 py-2.5 rounded-lg border transition-colors text-left",
                      embeddingConfig.localModelId === model.id
                        ? "border-primary bg-primary/10"
                        : "border-border hover:border-foreground/30",
                    )}
                  >
                    <div>
                      <div className="text-sm font-medium">{model.name}</div>
                      <div className="text-xs text-muted-foreground">
                        {model.dimensions}d | {model.sizeMb}MB | RAM {model.minRamGb}GB+ |{" "}
                        {model.languages.join(", ")}
                      </div>
                    </div>
                    {model.downloaded ? (
                      <span className="text-xs text-green-500">✓</span>
                    ) : (
                      <Download className="h-4 w-4 text-muted-foreground" />
                    )}
                  </button>
                ))}
              </div>
            )}

            {/* Test & Save buttons */}
            <div className="flex items-center gap-2 mt-4">
              <Button
                onClick={saveEmbeddingConfig}
                size="sm"
                disabled={(!embeddingDirty && embeddingSaveStatus === "idle") || embeddingSaving}
                className={cn(
                  embeddingSaveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
                  embeddingSaveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
                )}
              >
                {embeddingSaving ? (
                  <span className="flex items-center gap-1.5">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    {t("common.saving")}
                  </span>
                ) : embeddingSaveStatus === "saved" ? (
                  <span className="flex items-center gap-1.5">
                    <Check className="h-3.5 w-3.5" />
                    {t("common.saved")}
                  </span>
                ) : embeddingSaveStatus === "failed" ? (
                  t("common.saveFailed")
                ) : (
                  t("common.save")
                )}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={
                  embeddingTestLoading ||
                  (embeddingConfig.providerType === "local"
                    ? !embeddingConfig.localModelId
                    : !embeddingConfig.apiBaseUrl?.trim())
                }
                onClick={async () => {
                  setEmbeddingTestLoading(true)
                  setEmbeddingTestResult(null)
                  try {
                    const msg = await invoke<string>("test_embedding", {
                      config: embeddingConfig,
                    })
                    setEmbeddingTestResult(parseTestResult(msg, false))
                  } catch (e) {
                    setEmbeddingTestResult(parseTestResult(String(e), true))
                  } finally {
                    setEmbeddingTestLoading(false)
                  }
                }}
              >
                {embeddingTestLoading ? (
                  <span className="flex items-center gap-2">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    {t("common.testing")}
                  </span>
                ) : (
                  <span className="flex items-center gap-2">
                    <Wifi className="h-3.5 w-3.5" />
                    {t("settings.memoryEmbeddingTest")}
                  </span>
                )}
              </Button>
            </div>

            {embeddingTestResult && (
              <div className="mt-3">
                <TestResultDisplay result={embeddingTestResult} />
              </div>
            )}

            {/* Re-embed All */}
            {embeddingConfig.enabled && totalCount > 0 && (
              <div className="mt-6 pt-4 border-t border-border/50">
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-sm font-medium">{t("settings.memoryReembedAll")}</div>
                    <div className="text-xs text-muted-foreground">
                      {t("settings.memoryCount", { count: totalCount })}
                    </div>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={batchLoading}
                    onClick={handleReembedAll}
                  >
                    {batchLoading ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />
                    ) : (
                      <Zap className="h-3.5 w-3.5 mr-1.5" />
                    )}
                    {t("settings.memoryReembedAll")}
                  </Button>
                </div>
              </div>
            )}

            {/* Dedup thresholds (advanced) */}
            <div className="mt-6 pt-4 border-t border-border/50">
              <button
                onClick={() => setDedupExpanded(!dedupExpanded)}
                className="flex items-center gap-1 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
              >
                <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", dedupExpanded && "rotate-90")} />
                {t("settings.memoryDedupAdvanced")}
              </button>
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
                          invoke("save_dedup_config", { config: updated }).catch(() => {})
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
                          invoke("save_dedup_config", { config: updated }).catch(() => {})
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
              <button
                onClick={() => setSearchTuningExpanded(!searchTuningExpanded)}
                className="flex items-center gap-1 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
              >
                <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", searchTuningExpanded && "rotate-90")} />
                <Settings2 className="h-3.5 w-3.5 mr-0.5" />
                {t("settings.memorySearchTuning")}
              </button>
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
                        invoke("save_hybrid_search_config", { config: updated }).catch(() => {})
                      }}
                    />
                  </div>

                  {/* Temporal decay */}
                  <div className="space-y-2">
                    <div className="flex items-center justify-between">
                      <label className="text-xs font-medium">{t("settings.memoryTemporalDecay")}</label>
                      <Switch
                        checked={decayConfig.enabled}
                        onCheckedChange={(v) => {
                          const updated = { ...decayConfig, enabled: v }
                          setDecayConfig(updated)
                          invoke("save_temporal_decay_config", { config: updated }).catch(() => {})
                        }}
                      />
                    </div>
                    <p className="text-xs text-muted-foreground">{t("settings.memoryTemporalDecayDesc")}</p>
                    {decayConfig.enabled && (
                      <div className="flex items-center gap-2">
                        <label className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.memoryTemporalDecayHalfLife")}</label>
                        <Input
                          type="number"
                          min={1} max={365}
                          value={decayConfig.halfLifeDays}
                          onChange={(e) => {
                            const val = parseFloat(e.target.value)
                            if (!isNaN(val) && val > 0) {
                              const updated = { ...decayConfig, halfLifeDays: val }
                              setDecayConfig(updated)
                              invoke("save_temporal_decay_config", { config: updated }).catch(() => {})
                            }
                          }}
                          className="h-7 text-xs w-20"
                        />
                        <span className="text-xs text-muted-foreground">{t("settings.memoryDays")}</span>
                      </div>
                    )}
                  </div>

                  {/* MMR diversity */}
                  <div className="space-y-2">
                    <div className="flex items-center justify-between">
                      <label className="text-xs font-medium">{t("settings.memoryMmr")}</label>
                      <Switch
                        checked={mmrConfig.enabled}
                        onCheckedChange={(v) => {
                          const updated = { ...mmrConfig, enabled: v }
                          setMmrConfig(updated)
                          invoke("save_mmr_config", { config: updated }).catch(() => {})
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
                            invoke("save_mmr_config", { config: updated }).catch(() => {})
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
                          invoke("save_embedding_cache_config", { config: updated }).catch(() => {})
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
                          invoke("save_multimodal_config", { config: updated }).catch(() => {})
                        }}
                      />
                    </div>
                    <p className="text-xs text-muted-foreground">{t("settings.memoryMultimodalDesc")}</p>
                    {multimodalConfig.enabled && (
                      <div className="space-y-2 pl-1">
                        <div className="flex items-center gap-3">
                          <label className="text-xs text-muted-foreground">{t("settings.memoryMultimodalModalities")}:</label>
                          <label className="flex items-center gap-1 text-xs">
                            <input type="checkbox" className="rounded"
                              checked={multimodalConfig.modalities.includes("image")}
                              onChange={(e) => {
                                const mods = e.target.checked
                                  ? [...multimodalConfig.modalities, "image"]
                                  : multimodalConfig.modalities.filter(m => m !== "image")
                                const updated = { ...multimodalConfig, modalities: mods }
                                setMultimodalConfig(updated)
                                invoke("save_multimodal_config", { config: updated }).catch(() => {})
                              }}
                            />
                            {t("settings.memoryMultimodalImage")}
                          </label>
                          <label className="flex items-center gap-1 text-xs">
                            <input type="checkbox" className="rounded"
                              checked={multimodalConfig.modalities.includes("audio")}
                              onChange={(e) => {
                                const mods = e.target.checked
                                  ? [...multimodalConfig.modalities, "audio"]
                                  : multimodalConfig.modalities.filter(m => m !== "audio")
                                const updated = { ...multimodalConfig, modalities: mods }
                                setMultimodalConfig(updated)
                                invoke("save_multimodal_config", { config: updated }).catch(() => {})
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
                                invoke("save_multimodal_config", { config: updated }).catch(() => {})
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
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

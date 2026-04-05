import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  EmbeddingConfig,
  EmbeddingPreset,
  LocalEmbeddingModel,
  MemoryStats,
} from "./types"

export function useMemoryStats() {
  // ── Stats state ──
  const [stats, setStats] = useState<MemoryStats | null>(null)

  // ── Embedding config state ──
  const [embeddingConfig, setEmbeddingConfig] = useState<EmbeddingConfig>({
    enabled: false,
    providerType: "openai-compatible",
  })
  const [presets, setPresets] = useState<EmbeddingPreset[]>([])
  const [localModels, setLocalModels] = useState<LocalEmbeddingModel[]>([])
  const [embeddingDirty, setEmbeddingDirty] = useState(false)
  const [embeddingTestLoading, setEmbeddingTestLoading] = useState(false)
  const [embeddingTestResult, setEmbeddingTestResult] = useState<import("../TestResultDisplay").TestResult | null>(null)
  const [embeddingSaving, setEmbeddingSaving] = useState(false)
  const [embeddingSaveStatus, setEmbeddingSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  // ── Dedup config state ──
  const [dedupConfig, setDedupConfig] = useState({ thresholdHigh: 0.02, thresholdMerge: 0.012 })
  const [dedupExpanded, setDedupExpanded] = useState(false)

  // ── Load embedding config ──
  useEffect(() => {
    async function loadEmbedding() {
      try {
        const [config, presetList, models, dedup] = await Promise.all([
          getTransport().call<EmbeddingConfig>("get_embedding_config"),
          getTransport().call<EmbeddingPreset[]>("get_embedding_presets"),
          getTransport().call<LocalEmbeddingModel[]>("list_local_embedding_models"),
          getTransport().call<{ thresholdHigh: number; thresholdMerge: number }>("get_dedup_config"),
        ])
        setEmbeddingConfig(config)
        setPresets(presetList)
        setLocalModels(models)
        setDedupConfig(dedup)
      } catch (e) {
        logger.error("settings", "MemoryPanel::loadEmbedding", "Failed to load embedding config", e)
      }
    }
    loadEmbedding()
  }, [])

  async function saveEmbeddingConfig() {
    setEmbeddingSaving(true)
    try {
      await getTransport().call("save_embedding_config", { config: embeddingConfig })
      setEmbeddingDirty(false)
      setEmbeddingSaveStatus("saved")
      setTimeout(() => setEmbeddingSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "MemoryPanel::saveEmbedding", "Failed to save", e)
      setEmbeddingSaveStatus("failed")
      setTimeout(() => setEmbeddingSaveStatus("idle"), 2000)
    } finally {
      setEmbeddingSaving(false)
    }
  }

  function updateStats(statsData: MemoryStats | null) {
    if (statsData) setStats(statsData)
  }

  return {
    stats,
    updateStats,
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
    saveEmbeddingConfig,
  }
}

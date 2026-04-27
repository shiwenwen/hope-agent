import { useCallback, useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  EmbeddingConfig,
  EmbeddingModelConfig,
  EmbeddingModelTemplate,
  MemoryEmbeddingState,
  MemoryStats,
} from "./types"

export function useMemoryStats() {
  const [stats, setStats] = useState<MemoryStats | null>(null)

  // `embeddingConfig` is kept around for HybridSearchConfig's "is this Gemini
  // embedding-2?" multimodal warning; the rest of the embedding flow now lives
  // in EmbeddingModelsPanel and writes go through the model-config endpoints.
  const [embeddingConfig, setEmbeddingConfig] = useState<EmbeddingConfig>({
    enabled: false,
    providerType: "openai-compatible",
  })
  const [embeddingModels, setEmbeddingModels] = useState<EmbeddingModelConfig[]>([])
  const [embeddingTemplates, setEmbeddingTemplates] = useState<EmbeddingModelTemplate[]>([])
  const [memoryEmbeddingState, setMemoryEmbeddingState] = useState<MemoryEmbeddingState>({
    selection: { enabled: false },
    currentModel: null,
    needsReembed: false,
  })

  const [dedupConfig, setDedupConfig] = useState({ thresholdHigh: 0.02, thresholdMerge: 0.012 })
  const [dedupExpanded, setDedupExpanded] = useState(false)

  const loadEmbedding = useCallback(async () => {
    try {
      const [config, templateList, modelConfigs, memoryEmbedding, dedup] = await Promise.all([
        getTransport().call<EmbeddingConfig>("get_embedding_config"),
        getTransport().call<EmbeddingModelTemplate[]>("embedding_model_config_templates"),
        getTransport().call<EmbeddingModelConfig[]>("embedding_model_config_list"),
        getTransport().call<MemoryEmbeddingState>("memory_embedding_get"),
        getTransport().call<{ thresholdHigh: number; thresholdMerge: number }>("get_dedup_config"),
      ])
      setEmbeddingConfig(config)
      setEmbeddingTemplates(templateList)
      setEmbeddingModels(modelConfigs)
      setMemoryEmbeddingState(memoryEmbedding)
      setDedupConfig(dedup)
    } catch (e) {
      logger.error("settings", "MemoryPanel::loadEmbedding", "Failed to load embedding config", e)
    }
  }, [])

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      void loadEmbedding()
    }, 0)
    return () => window.clearTimeout(timeout)
  }, [loadEmbedding])

  function updateStats(statsData: MemoryStats | null) {
    if (statsData) setStats(statsData)
  }

  return {
    stats,
    updateStats,
    embeddingConfig,
    embeddingModels, setEmbeddingModels,
    embeddingTemplates,
    memoryEmbeddingState, setMemoryEmbeddingState,
    reloadEmbeddingConfig: loadEmbedding,
    dedupConfig, setDedupConfig,
    dedupExpanded, setDedupExpanded,
  }
}

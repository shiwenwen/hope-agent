import { useEffect, useState } from "react"

import { TRANSPORT_EVENT_RESYNC_REQUIRED, type Transport } from "@/lib/transport"
import { useTransport } from "@/lib/transport-provider"
import type { KnowledgeSourceLimitsConfig } from "@/types/knowledge"

export const DEFAULT_KNOWLEDGE_SOURCE_LIMITS: KnowledgeSourceLimitsConfig = {
  maxTextSourceMb: 5,
  maxBinarySourceMb: 24,
  maxUrlResponseMb: 2,
}
export const MIN_KNOWLEDGE_TEXT_SOURCE_MB = 1
export const MAX_KNOWLEDGE_TEXT_SOURCE_MB = 20
export const MIN_KNOWLEDGE_BINARY_SOURCE_MB = 1
export const MAX_KNOWLEDGE_BINARY_SOURCE_MB = 100
export const MIN_KNOWLEDGE_URL_RESPONSE_MB = 1
export const MAX_KNOWLEDGE_URL_RESPONSE_MB = 20

function clamp(value: unknown, fallback: number, min: number, max: number): number {
  const number = Number(value)
  const rounded = Number.isFinite(number) ? Math.round(number) : fallback
  return Math.min(max, Math.max(min, rounded))
}

export function normalizeKnowledgeSourceLimits(
  value?: Partial<KnowledgeSourceLimitsConfig> | null,
): KnowledgeSourceLimitsConfig {
  return {
    maxTextSourceMb: clamp(
      value?.maxTextSourceMb,
      DEFAULT_KNOWLEDGE_SOURCE_LIMITS.maxTextSourceMb,
      MIN_KNOWLEDGE_TEXT_SOURCE_MB,
      MAX_KNOWLEDGE_TEXT_SOURCE_MB,
    ),
    maxBinarySourceMb: clamp(
      value?.maxBinarySourceMb,
      DEFAULT_KNOWLEDGE_SOURCE_LIMITS.maxBinarySourceMb,
      MIN_KNOWLEDGE_BINARY_SOURCE_MB,
      MAX_KNOWLEDGE_BINARY_SOURCE_MB,
    ),
    maxUrlResponseMb: clamp(
      value?.maxUrlResponseMb,
      DEFAULT_KNOWLEDGE_SOURCE_LIMITS.maxUrlResponseMb,
      MIN_KNOWLEDGE_URL_RESPONSE_MB,
      MAX_KNOWLEDGE_URL_RESPONSE_MB,
    ),
  }
}

export async function readKnowledgeSourceLimits(
  transport: Transport,
): Promise<KnowledgeSourceLimitsConfig> {
  const value = await transport.call<Partial<KnowledgeSourceLimitsConfig>>(
    "knowledge_source_limits_config_get_cmd",
  )
  return normalizeKnowledgeSourceLimits(value)
}

export async function writeKnowledgeSourceLimits(
  transport: Transport,
  config: KnowledgeSourceLimitsConfig,
): Promise<KnowledgeSourceLimitsConfig> {
  const value = await transport.call<Partial<KnowledgeSourceLimitsConfig>>(
    "knowledge_source_limits_config_set_cmd",
    { config: normalizeKnowledgeSourceLimits(config) },
  )
  return normalizeKnowledgeSourceLimits(value)
}

export function useKnowledgeSourceLimits(): {
  config: KnowledgeSourceLimitsConfig
  loading: boolean
  refresh: () => Promise<void>
  save: (config: KnowledgeSourceLimitsConfig) => Promise<KnowledgeSourceLimitsConfig>
} {
  const transport = useTransport()
  const [config, setConfig] = useState(DEFAULT_KNOWLEDGE_SOURCE_LIMITS)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      try {
        const next = await readKnowledgeSourceLimits(transport)
        if (!cancelled) setConfig(next)
      } catch {
        // Keep the last known value while a remote transport reconnects.
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    void refresh()
    const unlistenConfig = transport.listen("config:changed", () => void refresh())
    const unlistenReconnect = transport.listen(
      TRANSPORT_EVENT_RESYNC_REQUIRED,
      () => void refresh(),
    )
    return () => {
      cancelled = true
      unlistenConfig()
      unlistenReconnect()
    }
  }, [transport])

  return {
    config,
    loading,
    refresh: async () => setConfig(await readKnowledgeSourceLimits(transport)),
    save: async (next) => {
      const saved = await writeKnowledgeSourceLimits(transport, next)
      setConfig(saved)
      return saved
    },
  }
}

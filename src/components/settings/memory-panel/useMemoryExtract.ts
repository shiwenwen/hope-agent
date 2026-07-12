import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import {
  memoryExtractOperationError,
  type MemoryExtractOperationError,
} from "./memoryExtractOperationFeedback"

interface UseMemoryExtractParams {
  agentId?: string
  isAgentMode: boolean
}

export function useMemoryExtract({ agentId, isAgentMode }: UseMemoryExtractParams) {
  const { t } = useTranslation()
  const [globalExtract, setGlobalExtract] = useState({
    enabled: true,
    autoExtract: false,
    extractProviderId: null as string | null,
    extractModelId: null as string | null,
    flushBeforeCompact: false,
    extractTokenThreshold: 8000,
    extractTimeThresholdSecs: 300,
    extractMessageThreshold: 10,
    extractIdleTimeoutSecs: 1800,
    // Declared so the whole-struct save round-trip (save_extract_config
    // replaces the entire config) never drops it.
    enableReflection: true,
    // Next-gen claim dual-write (default on). Global-only; also gates the Claims view.
    extractClaims: true,
    // Global-only: new structured claims land in the Review Inbox first.
    reviewFirst: false,
  })
  const [agentExtractOverride, setAgentExtractOverride] = useState<{
    autoExtract: boolean | null
    extractProviderId: string | null
    extractModelId: string | null
    flushBeforeCompact: boolean | null
    extractTokenThreshold: number | null
    extractTimeThresholdSecs: number | null
    extractMessageThreshold: number | null
    extractIdleTimeoutSecs: number | null
  }>({
    autoExtract: null,
    extractProviderId: null,
    extractModelId: null,
    flushBeforeCompact: null,
    extractTokenThreshold: null,
    extractTimeThresholdSecs: null,
    extractMessageThreshold: null,
    extractIdleTimeoutSecs: null,
  })
  const [extractConfigLoaded, setExtractConfigLoaded] = useState(false)
  const [extractConfigError, setExtractConfigError] =
    useState<MemoryExtractOperationError | null>(null)
  const [availableProviders, setAvailableProviders] = useState<{ id: string; name: string; models: { id: string; name: string }[] }[]>([])

  // ── Effective values (agent override -> global fallback) ──
  const effectiveAutoExtract = globalExtract.enabled
    ? isAgentMode
      ? (agentExtractOverride.autoExtract ?? globalExtract.autoExtract)
      : globalExtract.autoExtract
    : false
  const effectiveProviderId = isAgentMode
    ? (agentExtractOverride.extractProviderId ?? globalExtract.extractProviderId)
    : globalExtract.extractProviderId
  const effectiveModelId = isAgentMode
    ? (agentExtractOverride.extractModelId ?? globalExtract.extractModelId)
    : globalExtract.extractModelId
  const effectiveFlushBeforeCompact = globalExtract.enabled
    ? isAgentMode
      ? (agentExtractOverride.flushBeforeCompact ?? globalExtract.flushBeforeCompact)
      : globalExtract.flushBeforeCompact
    : false
  const effectiveTokenThreshold = isAgentMode
    ? (agentExtractOverride.extractTokenThreshold ?? globalExtract.extractTokenThreshold)
    : globalExtract.extractTokenThreshold
  const effectiveTimeThresholdSecs = isAgentMode
    ? (agentExtractOverride.extractTimeThresholdSecs ?? globalExtract.extractTimeThresholdSecs)
    : globalExtract.extractTimeThresholdSecs
  const effectiveMessageThreshold = isAgentMode
    ? (agentExtractOverride.extractMessageThreshold ?? globalExtract.extractMessageThreshold)
    : globalExtract.extractMessageThreshold
  const effectiveIdleTimeoutSecs = isAgentMode
    ? (agentExtractOverride.extractIdleTimeoutSecs ?? globalExtract.extractIdleTimeoutSecs)
    : globalExtract.extractIdleTimeoutSecs

  const agentHasOverride = isAgentMode && (
    agentExtractOverride.autoExtract !== null ||
    agentExtractOverride.extractProviderId !== null ||
    agentExtractOverride.extractModelId !== null ||
    agentExtractOverride.flushBeforeCompact !== null ||
    agentExtractOverride.extractTokenThreshold !== null ||
    agentExtractOverride.extractTimeThresholdSecs !== null ||
    agentExtractOverride.extractMessageThreshold !== null ||
    agentExtractOverride.extractIdleTimeoutSecs !== null
  )

  // ── Load extract config (global + agent override) ──
  useEffect(() => {
    async function loadExtractConfig() {
      try {
        const global = await getTransport().call<{
          enabled?: boolean
          autoExtract: boolean
          extractProviderId: string | null
          extractModelId: string | null
          flushBeforeCompact: boolean
          extractTokenThreshold: number
          extractTimeThresholdSecs: number
          extractMessageThreshold: number
          extractIdleTimeoutSecs: number
          enableReflection?: boolean
          extractClaims?: boolean
          reviewFirst?: boolean
        }>("get_extract_config")
        setGlobalExtract({ enabled: true, enableReflection: true, extractClaims: true, reviewFirst: false, ...global })

        if (isAgentMode && agentId) {
          const cfg = await getTransport().call<{ memory?: {
            autoExtract?: boolean | null
            extractProviderId?: string | null
            extractModelId?: string | null
            flushBeforeCompact?: boolean | null
            extractTokenThreshold?: number | null
            extractTimeThresholdSecs?: number | null
            extractMessageThreshold?: number | null
            extractIdleTimeoutSecs?: number | null
          } }>("get_agent_config", { id: agentId })
          setAgentExtractOverride({
            autoExtract: cfg?.memory?.autoExtract ?? null,
            extractProviderId: cfg?.memory?.extractProviderId ?? null,
            extractModelId: cfg?.memory?.extractModelId ?? null,
            flushBeforeCompact: cfg?.memory?.flushBeforeCompact ?? null,
            extractTokenThreshold: cfg?.memory?.extractTokenThreshold ?? null,
            extractTimeThresholdSecs: cfg?.memory?.extractTimeThresholdSecs ?? null,
            extractMessageThreshold: cfg?.memory?.extractMessageThreshold ?? null,
            extractIdleTimeoutSecs: cfg?.memory?.extractIdleTimeoutSecs ?? null,
          })
        }

        const providers = await getTransport().call<{ id: string; name: string; models: { id: string; name: string }[]; enabled?: boolean }[]>("get_providers")
        setAvailableProviders(
          providers
            .filter((p) => p.enabled !== false)
            .map((p) => ({ id: p.id, name: p.name, models: p.models.map((m) => ({ id: m.id, name: m.name })) }))
        )
        setExtractConfigError(null)
      } catch (e) {
        logger.error("settings", "MemoryPanel::loadExtractConfig", "Failed", e)
        setExtractConfigError(memoryExtractOperationError("load", t, e))
      } finally {
        setExtractConfigLoaded(true)
      }
    }
    loadExtractConfig()
  }, [isAgentMode, agentId, t])

  // ── Save global extract config ──
  async function saveGlobalExtract(updates: Partial<typeof globalExtract>) {
    const previous = globalExtract
    const updated = { ...globalExtract, ...updates }
    setGlobalExtract(updated)
    try {
      await getTransport().call("save_extract_config", { config: updated })
      setExtractConfigError(null)
    } catch (e) {
      logger.error("settings", "MemoryPanel::saveGlobalExtract", "Failed", e)
      setGlobalExtract(previous)
      setExtractConfigError(memoryExtractOperationError("saveGlobal", t, e))
    }
  }

  // ── Save per-agent extract override ──
  async function saveAgentExtract(updates: Partial<typeof agentExtractOverride>) {
    if (!agentId) return
    const previous = agentExtractOverride
    const updated = { ...agentExtractOverride, ...updates }
    setAgentExtractOverride(updated)
    try {
      const cfg = await getTransport().call<Record<string, unknown>>("get_agent_config", { id: agentId })
      const memory = (cfg?.memory ?? {}) as Record<string, unknown>
      Object.assign(memory, updates)
      cfg.memory = memory
      await getTransport().call("save_agent_config_cmd", { id: agentId, config: cfg })
      setExtractConfigError(null)
    } catch (e) {
      logger.error("settings", "MemoryPanel::saveAgentExtract", "Failed", e)
      setAgentExtractOverride(previous)
      setExtractConfigError(memoryExtractOperationError("saveAgent", t, e))
    }
  }

  // ── Reset agent overrides to inherit global ──
  async function resetAgentExtract() {
    if (!agentId) return
    const previous = agentExtractOverride
    setAgentExtractOverride({
      autoExtract: null,
      extractProviderId: null,
      extractModelId: null,
      flushBeforeCompact: null,
      extractTokenThreshold: null,
      extractTimeThresholdSecs: null,
      extractMessageThreshold: null,
      extractIdleTimeoutSecs: null,
    })
    try {
      const cfg = await getTransport().call<Record<string, unknown>>("get_agent_config", { id: agentId })
      const memory = (cfg?.memory ?? {}) as Record<string, unknown>
      delete memory.autoExtract
      delete memory.extractProviderId
      delete memory.extractModelId
      delete memory.flushBeforeCompact
      delete memory.extractTokenThreshold
      delete memory.extractTimeThresholdSecs
      delete memory.extractMessageThreshold
      delete memory.extractIdleTimeoutSecs
      cfg.memory = memory
      await getTransport().call("save_agent_config_cmd", { id: agentId, config: cfg })
      setExtractConfigError(null)
    } catch (e) {
      logger.error("settings", "MemoryPanel::resetAgentExtract", "Failed", e)
      setAgentExtractOverride(previous)
      setExtractConfigError(memoryExtractOperationError("resetAgent", t, e))
    }
  }

  function handleToggleAutoExtract(enabled: boolean) {
    if (isAgentMode) {
      saveAgentExtract({ autoExtract: enabled })
    } else {
      saveGlobalExtract({ autoExtract: enabled })
    }
  }

  function handleUpdateExtractModel(value: string) {
    const updates = value === "__chat__"
      ? { extractProviderId: null, extractModelId: null }
      : { extractProviderId: value.split("::", 2)[0], extractModelId: value.split("::", 2)[1] }
    if (isAgentMode) {
      saveAgentExtract(updates)
    } else {
      saveGlobalExtract(updates)
    }
  }

  function handleUpdateTokenThreshold(val: number) {
    const clamped = Math.max(1000, Math.min(50000, val))
    if (isAgentMode) {
      saveAgentExtract({ extractTokenThreshold: clamped })
    } else {
      saveGlobalExtract({ extractTokenThreshold: clamped })
    }
  }

  function handleUpdateTimeThresholdMins(val: number) {
    const clamped = Math.max(1, Math.min(60, val))
    if (isAgentMode) {
      saveAgentExtract({ extractTimeThresholdSecs: clamped * 60 })
    } else {
      saveGlobalExtract({ extractTimeThresholdSecs: clamped * 60 })
    }
  }

  function handleUpdateMessageThreshold(val: number) {
    const clamped = Math.max(2, Math.min(50, val))
    if (isAgentMode) {
      saveAgentExtract({ extractMessageThreshold: clamped })
    } else {
      saveGlobalExtract({ extractMessageThreshold: clamped })
    }
  }

  function handleUpdateIdleTimeoutMins(val: number) {
    const clamped = val === 0 ? 0 : Math.max(5, Math.min(120, val))
    if (isAgentMode) {
      saveAgentExtract({ extractIdleTimeoutSecs: clamped * 60 })
    } else {
      saveGlobalExtract({ extractIdleTimeoutSecs: clamped * 60 })
    }
  }

  function handleToggleFlushBeforeCompact(enabled: boolean) {
    if (isAgentMode) {
      saveAgentExtract({ flushBeforeCompact: enabled })
    } else {
      saveGlobalExtract({ flushBeforeCompact: enabled })
    }
  }

  const effectiveMemoryLearningMode =
    !globalExtract.enabled
      ? "off"
      : !isAgentMode && effectiveAutoExtract && effectiveFlushBeforeCompact && globalExtract.extractClaims && globalExtract.reviewFirst
      ? "review_first"
      : effectiveAutoExtract && effectiveFlushBeforeCompact
        ? "automatic"
      : !effectiveAutoExtract && !effectiveFlushBeforeCompact
        ? "manual_only"
        : "custom"

  function applyMemoryLearningMode(mode: "automatic" | "review_first" | "manual_only" | "off") {
    if (mode === "automatic") {
      if (isAgentMode) {
        saveAgentExtract({ autoExtract: true, flushBeforeCompact: true })
      } else {
        saveGlobalExtract({ enabled: true, autoExtract: true, flushBeforeCompact: true, extractClaims: true, reviewFirst: false })
      }
      return
    }

    if (mode === "review_first") {
      if (isAgentMode) {
        saveAgentExtract({ autoExtract: true, flushBeforeCompact: true })
      } else {
        saveGlobalExtract({ enabled: true, autoExtract: true, flushBeforeCompact: true, extractClaims: true, reviewFirst: true })
      }
      return
    }

    if (mode === "off") {
      if (!isAgentMode) {
        saveGlobalExtract({ enabled: false, autoExtract: false, flushBeforeCompact: false, reviewFirst: false })
      }
      return
    }

    if (isAgentMode) {
      saveAgentExtract({ autoExtract: false, flushBeforeCompact: false })
    } else {
      saveGlobalExtract({ enabled: true, autoExtract: false, flushBeforeCompact: false, reviewFirst: false })
    }
  }

  // Structured claim dual-write. Global-only — no agent override.
  const effectiveExtractClaims = globalExtract.extractClaims
  function handleToggleExtractClaims(enabled: boolean) {
    saveGlobalExtract({ extractClaims: enabled })
  }

  return {
    globalExtract,
    agentExtractOverride,
    extractConfigLoaded,
    extractConfigError,
    availableProviders,
    effectiveAutoExtract,
    effectiveProviderId,
    effectiveModelId,
    effectiveFlushBeforeCompact,
    effectiveTokenThreshold,
    effectiveTimeThresholdSecs,
    effectiveMessageThreshold,
    effectiveIdleTimeoutSecs,
    effectiveExtractClaims,
    effectiveMemoryEnabled: globalExtract.enabled,
    effectiveMemoryLearningMode,
    agentHasOverride,
    applyMemoryLearningMode,
    handleToggleAutoExtract,
    handleUpdateExtractModel,
    handleUpdateTokenThreshold,
    handleUpdateTimeThresholdMins,
    handleUpdateMessageThreshold,
    handleUpdateIdleTimeoutMins,
    handleToggleFlushBeforeCompact,
    handleToggleExtractClaims,
    resetAgentExtract,
  }
}

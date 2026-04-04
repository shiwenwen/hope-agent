import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { logger } from "@/lib/logger"

interface UseMemoryExtractParams {
  agentId?: string
  isAgentMode: boolean
}

export function useMemoryExtract({ agentId, isAgentMode }: UseMemoryExtractParams) {
  const [globalExtract, setGlobalExtract] = useState({ autoExtract: false, extractMinTurns: 3, extractProviderId: null as string | null, extractModelId: null as string | null, flushBeforeCompact: false })
  const [agentExtractOverride, setAgentExtractOverride] = useState<{ autoExtract: boolean | null; extractMinTurns: number | null; extractProviderId: string | null; extractModelId: string | null }>({ autoExtract: null, extractMinTurns: null, extractProviderId: null, extractModelId: null })
  const [extractConfigLoaded, setExtractConfigLoaded] = useState(false)
  const [availableProviders, setAvailableProviders] = useState<{ id: string; name: string; models: { id: string; name: string }[] }[]>([])

  // ── Effective values (agent override -> global fallback) ──
  const effectiveAutoExtract = isAgentMode
    ? (agentExtractOverride.autoExtract ?? globalExtract.autoExtract)
    : globalExtract.autoExtract
  const effectiveMinTurns = isAgentMode
    ? (agentExtractOverride.extractMinTurns ?? globalExtract.extractMinTurns)
    : globalExtract.extractMinTurns
  const effectiveProviderId = isAgentMode
    ? (agentExtractOverride.extractProviderId ?? globalExtract.extractProviderId)
    : globalExtract.extractProviderId
  const effectiveModelId = isAgentMode
    ? (agentExtractOverride.extractModelId ?? globalExtract.extractModelId)
    : globalExtract.extractModelId
  const effectiveFlushBeforeCompact = globalExtract.flushBeforeCompact

  const agentHasOverride = isAgentMode && (
    agentExtractOverride.autoExtract !== null ||
    agentExtractOverride.extractMinTurns !== null ||
    agentExtractOverride.extractProviderId !== null ||
    agentExtractOverride.extractModelId !== null
  )

  // ── Load extract config (global + agent override) ──
  useEffect(() => {
    async function loadExtractConfig() {
      try {
        const global = await invoke<{ autoExtract: boolean; extractMinTurns: number; extractProviderId: string | null; extractModelId: string | null }>("get_extract_config")
        setGlobalExtract(prev => ({ ...global, flushBeforeCompact: prev.flushBeforeCompact }))

        if (isAgentMode && agentId) {
          const cfg = await invoke<{ memory?: { autoExtract?: boolean | null; extractMinTurns?: number | null; extractProviderId?: string | null; extractModelId?: string | null } }>("get_agent_config", { id: agentId })
          setAgentExtractOverride({
            autoExtract: cfg?.memory?.autoExtract ?? null,
            extractMinTurns: cfg?.memory?.extractMinTurns ?? null,
            extractProviderId: cfg?.memory?.extractProviderId ?? null,
            extractModelId: cfg?.memory?.extractModelId ?? null,
          })
        }

        const providers = await invoke<{ id: string; name: string; models: { id: string; name: string }[]; enabled?: boolean }[]>("get_providers")
        setAvailableProviders(
          providers
            .filter((p) => p.enabled !== false)
            .map((p) => ({ id: p.id, name: p.name, models: p.models.map((m) => ({ id: m.id, name: m.name })) }))
        )
      } catch {
        // ignore
      } finally {
        setExtractConfigLoaded(true)
      }
    }
    loadExtractConfig()
  }, [isAgentMode, agentId])

  // ── Save global extract config ──
  async function saveGlobalExtract(updates: Partial<typeof globalExtract>) {
    const updated = { ...globalExtract, ...updates }
    setGlobalExtract(updated)
    try {
      await invoke("save_extract_config", { config: updated })
    } catch (e) {
      logger.error("settings", "MemoryPanel::saveGlobalExtract", "Failed", e)
    }
  }

  // ── Save per-agent extract override ──
  async function saveAgentExtract(updates: Partial<typeof agentExtractOverride>) {
    if (!agentId) return
    const updated = { ...agentExtractOverride, ...updates }
    setAgentExtractOverride(updated)
    try {
      const cfg = await invoke<Record<string, unknown>>("get_agent_config", { id: agentId })
      const memory = (cfg?.memory ?? {}) as Record<string, unknown>
      Object.assign(memory, updates)
      cfg.memory = memory
      await invoke("save_agent_config_cmd", { id: agentId, config: cfg })
    } catch (e) {
      logger.error("settings", "MemoryPanel::saveAgentExtract", "Failed", e)
    }
  }

  // ── Reset agent overrides to inherit global ──
  async function resetAgentExtract() {
    if (!agentId) return
    setAgentExtractOverride({ autoExtract: null, extractMinTurns: null, extractProviderId: null, extractModelId: null })
    try {
      const cfg = await invoke<Record<string, unknown>>("get_agent_config", { id: agentId })
      const memory = (cfg?.memory ?? {}) as Record<string, unknown>
      delete memory.autoExtract
      delete memory.extractMinTurns
      delete memory.extractProviderId
      delete memory.extractModelId
      cfg.memory = memory
      await invoke("save_agent_config_cmd", { id: agentId, config: cfg })
    } catch (e) {
      logger.error("settings", "MemoryPanel::resetAgentExtract", "Failed", e)
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

  function handleUpdateExtractMinTurns(val: number) {
    const clamped = Math.max(1, Math.min(20, val))
    if (isAgentMode) {
      saveAgentExtract({ extractMinTurns: clamped })
    } else {
      saveGlobalExtract({ extractMinTurns: clamped })
    }
  }

  function handleToggleFlushBeforeCompact(enabled: boolean) {
    saveGlobalExtract({ flushBeforeCompact: enabled })
  }

  return {
    globalExtract,
    agentExtractOverride,
    extractConfigLoaded,
    availableProviders,
    effectiveAutoExtract,
    effectiveMinTurns,
    effectiveProviderId,
    effectiveModelId,
    effectiveFlushBeforeCompact,
    agentHasOverride,
    handleToggleAutoExtract,
    handleUpdateExtractModel,
    handleUpdateExtractMinTurns,
    handleToggleFlushBeforeCompact,
    resetAgentExtract,
  }
}

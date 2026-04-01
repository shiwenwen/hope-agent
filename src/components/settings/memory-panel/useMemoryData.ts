import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { logger } from "@/lib/logger"
import type {
  MemoryEntry,
  MemorySearchQuery,
  NewMemory,
  EmbeddingConfig,
  EmbeddingPreset,
  LocalEmbeddingModel,
  AgentInfo,
  MemoryStats,
  MemoryView,
} from "./types"

interface UseMemoryDataParams {
  agentId?: string
  isAgentMode: boolean
}

export function useMemoryData({ agentId, isAgentMode }: UseMemoryDataParams) {
  // ── Core state ──
  const [view, setView] = useState<MemoryView>("list")
  const [memories, setMemories] = useState<MemoryEntry[]>([])
  const [totalCount, setTotalCount] = useState(0)
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState("")
  const [filterType, setFilterType] = useState<string | null>(null)
  const [filterScope, setFilterScope] = useState<"all" | "global" | "agent">("all")
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(agentId ?? null)

  // ── Edit/Add state ──
  const [editingMemory, setEditingMemory] = useState<MemoryEntry | null>(null)
  const [formContent, setFormContent] = useState("")
  const [formType, setFormType] = useState<"user" | "feedback" | "project" | "reference">("user")
  const [formTags, setFormTags] = useState("")
  const [formScope, setFormScope] = useState<"global" | "agent">(isAgentMode ? "agent" : "global")

  // ── Auto-extract state ──
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

  // ── Multi-select state ──
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set())
  const [batchLoading, setBatchLoading] = useState(false)

  // ── Dedup confirmation state ──
  const [dedupSimilar, setDedupSimilar] = useState<MemoryEntry[]>([])
  const [dedupPendingEntry, setDedupPendingEntry] = useState<NewMemory | null>(null)

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

  // ── Stats state ──
  const [stats, setStats] = useState<MemoryStats | null>(null)

  // ── Load agents for scope picker (standalone mode) ──
  useEffect(() => {
    if (!isAgentMode) {
      invoke<AgentInfo[]>("list_agents")
        .then(setAgents)
        .catch(() => {})
    }
  }, [isAgentMode])

  // ── Build scope for queries ──
  const buildScope = useCallback((): { kind: "global" } | { kind: "agent"; id: string } | null => {
    if (isAgentMode) {
      if (filterScope === "global") return { kind: "global" }
      if (filterScope === "agent") return { kind: "agent", id: agentId! }
      return null
    }
    if (filterScope === "global") return { kind: "global" }
    if (filterScope === "agent" && selectedAgentId) return { kind: "agent", id: selectedAgentId }
    return null
  }, [isAgentMode, filterScope, agentId, selectedAgentId])

  // ── Load memories ──
  const loadMemories = useCallback(async () => {
    try {
      setLoading(true)
      const scope = buildScope()

      if (searchQuery.trim()) {
        const query: MemorySearchQuery = {
          query: searchQuery,
          types: filterType ? [filterType] : null,
          agentId: isAgentMode && filterScope === "all" ? agentId : null,
          scope: isAgentMode && filterScope === "all" ? null : scope,
          limit: 50,
        }
        const results = await invoke<MemoryEntry[]>("memory_search", { query })
        setMemories(results)
      } else {
        const types = filterType ? [filterType] : null
        const results = await invoke<MemoryEntry[]>("memory_list", {
          scope,
          types,
          limit: 50,
          offset: 0,
        })
        setMemories(results)
      }
      const [count, statsData] = await Promise.all([
        invoke<number>("memory_count", { scope }),
        invoke<MemoryStats>("memory_stats", { scope }).catch(() => null),
      ])
      setTotalCount(count)
      if (statsData) setStats(statsData)
    } catch (e) {
      logger.error("settings", "MemoryPanel::load", "Failed to load memories", e)
    } finally {
      setLoading(false)
    }
  }, [searchQuery, filterType, buildScope, isAgentMode, filterScope, agentId])

  useEffect(() => {
    loadMemories()
  }, [loadMemories])

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

  // ── Load embedding config ──
  useEffect(() => {
    async function loadEmbedding() {
      try {
        const [config, presetList, models, dedup] = await Promise.all([
          invoke<EmbeddingConfig>("get_embedding_config"),
          invoke<EmbeddingPreset[]>("get_embedding_presets"),
          invoke<LocalEmbeddingModel[]>("list_local_embedding_models"),
          invoke<{ thresholdHigh: number; thresholdMerge: number }>("get_dedup_config"),
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

  // ── CRUD handlers ──
  function buildNewMemoryEntry(): NewMemory {
    const scopeAgentId = isAgentMode ? agentId! : (selectedAgentId ?? "default")
    return {
      memoryType: formType,
      scope: formScope === "global" ? { kind: "global" } : { kind: "agent", id: scopeAgentId },
      content: formContent.trim(),
      tags: formTags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
      source: "user",
    }
  }

  async function handleAdd() {
    try {
      const entry = buildNewMemoryEntry()

      const similar = await invoke<MemoryEntry[]>("memory_find_similar", {
        content: entry.content,
        threshold: 0.008,
        limit: 3,
      })

      if (similar.length > 0) {
        setDedupSimilar(similar)
        setDedupPendingEntry(entry)
        return
      }

      await doAddMemory(entry)
    } catch (e) {
      logger.error("settings", "MemoryPanel::add", "Failed to add memory", e)
    }
  }

  async function doAddMemory(entry: NewMemory) {
    try {
      await invoke("memory_add", { entry })
      setView("list")
      setFormContent("")
      setFormTags("")
      setDedupSimilar([])
      setDedupPendingEntry(null)
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::add", "Failed to add memory", e)
    }
  }

  function handleDedupConfirm() {
    if (dedupPendingEntry) doAddMemory(dedupPendingEntry)
  }

  function handleDedupCancel() {
    setDedupSimilar([])
    setDedupPendingEntry(null)
  }

  async function handleDedupUpdate(existingId: number) {
    if (!dedupPendingEntry) return
    try {
      const existing = dedupSimilar.find((m) => m.id === existingId)
      if (!existing) return
      const mergedContent = existing.content + "\n" + dedupPendingEntry.content
      const mergedTags = [...new Set([...existing.tags, ...dedupPendingEntry.tags])]
      await invoke("memory_update", { id: existingId, content: mergedContent, tags: mergedTags })
      setView("list")
      setFormContent("")
      setFormTags("")
      setDedupSimilar([])
      setDedupPendingEntry(null)
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::dedupUpdate", "Failed to update existing memory", e)
    }
  }

  async function handleUpdate() {
    if (!editingMemory) return
    try {
      const tags = formTags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean)
      await invoke("memory_update", {
        id: editingMemory.id,
        content: formContent.trim(),
        tags,
      })
      setView("list")
      setEditingMemory(null)
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::update", "Failed to update memory", e)
    }
  }

  async function handleDelete(id: number) {
    try {
      await invoke("memory_delete", { id })
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::delete", "Failed to delete memory", e)
    }
  }

  async function handleTogglePin(id: number, pinned: boolean) {
    try {
      // Optimistic update
      setMemories((prev) =>
        prev.map((m) => (m.id === id ? { ...m, pinned } : m))
      )
      await invoke("memory_toggle_pin", { id, pinned })
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::togglePin", "Failed to toggle pin", e)
      loadMemories() // Revert on error
    }
  }

  async function handleExport() {
    try {
      const md = await invoke<string>("memory_export", { scope: null })
      await navigator.clipboard.writeText(md)
    } catch (e) {
      logger.error("settings", "MemoryPanel::export", "Failed to export", e)
    }
  }

  // ── Batch & Import handlers ──

  function toggleSelect(id: number) {
    setSelectedIds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  function toggleSelectAll() {
    if (selectedIds.size === memories.length) {
      setSelectedIds(new Set())
    } else {
      setSelectedIds(new Set(memories.map((m) => m.id)))
    }
  }

  async function handleDeleteBatch() {
    if (selectedIds.size === 0) return
    setBatchLoading(true)
    try {
      await invoke("memory_delete_batch", { ids: [...selectedIds] })
      setSelectedIds(new Set())
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::deleteBatch", "Failed to batch delete", e)
    } finally {
      setBatchLoading(false)
    }
  }

  async function handleReembedBatch() {
    if (selectedIds.size === 0) return
    setBatchLoading(true)
    try {
      await invoke("memory_reembed", { ids: [...selectedIds] })
      setSelectedIds(new Set())
    } catch (e) {
      logger.error("settings", "MemoryPanel::reembedBatch", "Failed to batch re-embed", e)
    } finally {
      setBatchLoading(false)
    }
  }

  async function handleReembedAll() {
    setBatchLoading(true)
    try {
      await invoke("memory_reembed", { ids: null })
    } catch (e) {
      logger.error("settings", "MemoryPanel::reembedAll", "Failed to re-embed all", e)
    } finally {
      setBatchLoading(false)
    }
  }

  async function handleImport() {
    try {
      const input = document.createElement("input")
      input.type = "file"
      input.accept = ".json,.md,.markdown"
      input.onchange = async () => {
        const file = input.files?.[0]
        if (!file) return
        const text = await file.text()
        const format = file.name.endsWith(".json") ? "json" : "markdown"
        try {
          const result = await invoke<{ created: number; skippedDuplicate: number; failed: number }>(
            "memory_import",
            { content: text, format, dedup: true },
          )
          logger.info(
            "settings",
            "MemoryPanel::import",
            `Import done: ${result.created} created, ${result.skippedDuplicate} skipped, ${result.failed} failed`,
          )
          loadMemories()
        } catch (e) {
          logger.error("settings", "MemoryPanel::import", "Failed to import", e)
        }
      }
      input.click()
    } catch (e) {
      logger.error("settings", "MemoryPanel::import", "Failed to open file picker", e)
    }
  }

  async function saveEmbeddingConfig() {
    setEmbeddingSaving(true)
    try {
      await invoke("save_embedding_config", { config: embeddingConfig })
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

  function startEdit(mem: MemoryEntry) {
    setEditingMemory(mem)
    setFormContent(mem.content)
    setFormType(mem.memoryType)
    setFormTags(mem.tags.join(", "))
    setView("edit")
  }

  function startAdd() {
    setFormContent("")
    setFormType("user")
    setFormTags("")
    setFormScope("global")
    setView("add")
  }

  return {
    // Core state
    view, setView,
    memories,
    totalCount,
    loading,
    searchQuery, setSearchQuery,
    filterType, setFilterType,
    filterScope, setFilterScope,
    agents,
    selectedAgentId, setSelectedAgentId,

    // Edit/Add state
    editingMemory, setEditingMemory,
    formContent, setFormContent,
    formType, setFormType,
    formTags, setFormTags,
    formScope, setFormScope,

    // Auto-extract state
    globalExtract,
    agentExtractOverride,
    extractConfigLoaded,
    availableProviders,
    effectiveAutoExtract,
    effectiveMinTurns,
    effectiveProviderId,
    effectiveModelId,
    agentHasOverride,

    // Multi-select state
    selectedIds,
    batchLoading,

    // Dedup state
    dedupSimilar,
    dedupPendingEntry,

    // Embedding config state
    embeddingConfig, setEmbeddingConfig,
    presets,
    localModels,
    embeddingDirty, setEmbeddingDirty,
    embeddingTestLoading, setEmbeddingTestLoading,
    embeddingTestResult, setEmbeddingTestResult,
    embeddingSaving,
    embeddingSaveStatus,

    // Dedup config state
    dedupConfig, setDedupConfig,
    dedupExpanded, setDedupExpanded,

    // Stats state
    stats,

    // Handlers
    loadMemories,
    handleAdd,
    handleDedupConfirm,
    handleDedupCancel,
    handleDedupUpdate,
    handleUpdate,
    handleDelete,
    handleTogglePin,
    handleExport,
    toggleSelect,
    toggleSelectAll,
    handleDeleteBatch,
    handleReembedBatch,
    handleReembedAll,
    handleImport,
    saveEmbeddingConfig,
    startEdit,
    startAdd,
    handleToggleAutoExtract,
    handleUpdateExtractModel,
    handleUpdateExtractMinTurns,
    handleToggleFlushBeforeCompact,
    effectiveFlushBeforeCompact,
    resetAgentExtract,
  }
}

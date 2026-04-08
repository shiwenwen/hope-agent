import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  MemoryEntry,
  MemorySearchQuery,
  NewMemory,
  AgentInfo,
  MemoryView,
} from "./types"
import { useMemoryExtract } from "./useMemoryExtract"
import { useMemoryStats } from "./useMemoryStats"

interface UseMemoryDataParams {
  agentId?: string
  isAgentMode: boolean
}

export function useMemoryData({ agentId, isAgentMode }: UseMemoryDataParams) {
  // ── Sub-hooks ──
  const extract = useMemoryExtract({ agentId, isAgentMode })
  const statsHook = useMemoryStats()

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

  // ── Multi-select state ──
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set())
  const [batchLoading, setBatchLoading] = useState(false)

  // ── Dedup confirmation state ──
  const [dedupSimilar, setDedupSimilar] = useState<MemoryEntry[]>([])
  const [dedupPendingEntry, setDedupPendingEntry] = useState<NewMemory | null>(null)

  // ── Load agents for scope picker (standalone mode) ──
  useEffect(() => {
    if (!isAgentMode) {
      getTransport().call<AgentInfo[]>("list_agents")
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
        const results = await getTransport().call<MemoryEntry[]>("memory_search", { query })
        setMemories(results)
      } else {
        const types = filterType ? [filterType] : null
        const results = await getTransport().call<MemoryEntry[]>("memory_list", {
          scope,
          types,
          limit: 50,
          offset: 0,
        })
        setMemories(results)
      }
      const [count, statsData] = await Promise.all([
        getTransport().call<number>("memory_count", { scope }),
        getTransport().call<import("./types").MemoryStats>("memory_stats", { scope }).catch(() => null),
      ])
      setTotalCount(count)
      statsHook.updateStats(statsData)
    } catch (e) {
      logger.error("settings", "MemoryPanel::load", "Failed to load memories", e)
    } finally {
      setLoading(false)
    }
  }, [searchQuery, filterType, buildScope, isAgentMode, filterScope, agentId, statsHook])

  useEffect(() => {
    loadMemories()
  }, [loadMemories])

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

      const similar = await getTransport().call<MemoryEntry[]>("memory_find_similar", {
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
      await getTransport().call("memory_add", { entry })
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
      await getTransport().call("memory_update", { id: existingId, content: mergedContent, tags: mergedTags })
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
      await getTransport().call("memory_update", {
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
      await getTransport().call("memory_delete", { id })
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
      await getTransport().call("memory_toggle_pin", { id, pinned })
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::togglePin", "Failed to toggle pin", e)
      loadMemories() // Revert on error
    }
  }

  async function handleExport() {
    try {
      const md = await getTransport().call<string>("memory_export", { scope: null })
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
      await getTransport().call("memory_delete_batch", { ids: [...selectedIds] })
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
      await getTransport().call("memory_reembed", { ids: [...selectedIds] })
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
      await getTransport().call("memory_reembed", { ids: null })
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
          const result = await getTransport().call<{ created: number; skippedDuplicate: number; failed: number }>(
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

    // Auto-extract state (from sub-hook)
    globalExtract: extract.globalExtract,
    agentExtractOverride: extract.agentExtractOverride,
    extractConfigLoaded: extract.extractConfigLoaded,
    availableProviders: extract.availableProviders,
    effectiveAutoExtract: extract.effectiveAutoExtract,
    effectiveProviderId: extract.effectiveProviderId,
    effectiveModelId: extract.effectiveModelId,
    effectiveTokenThreshold: extract.effectiveTokenThreshold,
    effectiveTimeThresholdSecs: extract.effectiveTimeThresholdSecs,
    effectiveMessageThreshold: extract.effectiveMessageThreshold,
    effectiveIdleTimeoutSecs: extract.effectiveIdleTimeoutSecs,
    agentHasOverride: extract.agentHasOverride,

    // Multi-select state
    selectedIds,
    batchLoading,

    // Dedup state
    dedupSimilar,
    dedupPendingEntry,

    // Embedding config state (from sub-hook)
    embeddingConfig: statsHook.embeddingConfig, setEmbeddingConfig: statsHook.setEmbeddingConfig,
    presets: statsHook.presets,
    localModels: statsHook.localModels,
    embeddingDirty: statsHook.embeddingDirty, setEmbeddingDirty: statsHook.setEmbeddingDirty,
    embeddingTestLoading: statsHook.embeddingTestLoading, setEmbeddingTestLoading: statsHook.setEmbeddingTestLoading,
    embeddingTestResult: statsHook.embeddingTestResult, setEmbeddingTestResult: statsHook.setEmbeddingTestResult,
    embeddingSaving: statsHook.embeddingSaving,
    embeddingSaveStatus: statsHook.embeddingSaveStatus,

    // Dedup config state (from sub-hook)
    dedupConfig: statsHook.dedupConfig, setDedupConfig: statsHook.setDedupConfig,
    dedupExpanded: statsHook.dedupExpanded, setDedupExpanded: statsHook.setDedupExpanded,

    // Stats state (from sub-hook)
    stats: statsHook.stats,

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
    saveEmbeddingConfig: statsHook.saveEmbeddingConfig,
    startEdit,
    startAdd,
    handleToggleAutoExtract: extract.handleToggleAutoExtract,
    handleUpdateExtractModel: extract.handleUpdateExtractModel,
    handleUpdateTokenThreshold: extract.handleUpdateTokenThreshold,
    handleUpdateTimeThresholdMins: extract.handleUpdateTimeThresholdMins,
    handleUpdateMessageThreshold: extract.handleUpdateMessageThreshold,
    handleUpdateIdleTimeoutMins: extract.handleUpdateIdleTimeoutMins,
    handleToggleFlushBeforeCompact: extract.handleToggleFlushBeforeCompact,
    effectiveFlushBeforeCompact: extract.effectiveFlushBeforeCompact,
    resetAgentExtract: extract.resetAgentExtract,
  }
}

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  ArrowLeft,
  Plus,
  Trash2,
  Search,
  Download,
  Upload,
  User,
  MessageSquareHeart,
  FolderKanban,
  BookOpen,
  ChevronRight,
  X,
  FileDown,
  Zap,
  Wifi,
  Loader2,
  Check,
  CheckSquare,
  Square,
} from "lucide-react"
import TestResultDisplay, { parseTestResult, type TestResult } from "./TestResultDisplay"

// ── Types ─────────────────────────────────────────────────────────

interface MemoryEntry {
  id: number
  memoryType: "user" | "feedback" | "project" | "reference"
  scope: { kind: "global" } | { kind: "agent"; id: string }
  content: string
  tags: string[]
  source: string
  sourceSessionId?: string | null
  createdAt: string
  updatedAt: string
  relevanceScore?: number | null
}

interface MemorySearchQuery {
  query: string
  types?: string[] | null
  scope?: { kind: "global" } | { kind: "agent"; id: string } | null
  agentId?: string | null
  limit?: number | null
}

interface NewMemory {
  memoryType: "user" | "feedback" | "project" | "reference"
  scope: { kind: "global" } | { kind: "agent"; id: string }
  content: string
  tags: string[]
  source: string
}

interface EmbeddingConfig {
  enabled: boolean
  providerType: string
  apiBaseUrl?: string | null
  apiKey?: string | null
  apiModel?: string | null
  apiDimensions?: number | null
  localModelId?: string | null
}

interface EmbeddingPreset {
  name: string
  providerType: string
  baseUrl: string
  defaultModel: string
  defaultDimensions: number
}

interface LocalEmbeddingModel {
  id: string
  name: string
  dimensions: number
  sizeMb: number
  minRamGb: number
  languages: string[]
  downloaded: boolean
}

// ── Constants ─────────────────────────────────────────────────────

const MEMORY_TYPE_ICONS: Record<string, typeof User> = {
  user: User,
  feedback: MessageSquareHeart,
  project: FolderKanban,
  reference: BookOpen,
}

const MEMORY_TYPES = ["user", "feedback", "project", "reference"] as const

// ── Component ─────────────────────────────────────────────────────

type MemoryView = "list" | "add" | "edit" | "embedding"

interface AgentInfo {
  id: string
  name: string
  emoji?: string | null
}

/**
 * MemoryPanel - Memory management UI.
 *
 * Two modes:
 * - **Standalone** (no agentId): Global view with agent scope filter dropdown.
 *   Used in Settings > Memory tab.
 * - **Embedded** (agentId provided): Agent-scoped view showing only that agent's
 *   memories + global memories. Used inside Agent edit panel's Memory tab.
 */
export default function MemoryPanel({ agentId, compact }: { agentId?: string; compact?: boolean }) {
  const { t } = useTranslation()

  const isAgentMode = !!agentId

  // State
  const [view, setView] = useState<MemoryView>("list")
  const [memories, setMemories] = useState<MemoryEntry[]>([])
  const [totalCount, setTotalCount] = useState(0)
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState("")
  const [filterType, setFilterType] = useState<string | null>(null)
  const [filterScope, setFilterScope] = useState<"all" | "global" | "agent">(
    isAgentMode ? "all" : "all",
  )
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(agentId ?? null)

  // Edit/Add state
  const [editingMemory, setEditingMemory] = useState<MemoryEntry | null>(null)
  const [formContent, setFormContent] = useState("")
  const [formType, setFormType] = useState<"user" | "feedback" | "project" | "reference">("user")
  const [formTags, setFormTags] = useState("")
  const [formScope, setFormScope] = useState<"global" | "agent">(isAgentMode ? "agent" : "global")

  // Auto-extract state — global config + per-agent override
  const [globalExtract, setGlobalExtract] = useState({ autoExtract: false, extractMinTurns: 3, extractProviderId: null as string | null, extractModelId: null as string | null })
  const [agentExtractOverride, setAgentExtractOverride] = useState<{ autoExtract: boolean | null; extractMinTurns: number | null; extractProviderId: string | null; extractModelId: string | null }>({ autoExtract: null, extractMinTurns: null, extractProviderId: null, extractModelId: null })
  const [extractConfigLoaded, setExtractConfigLoaded] = useState(false)
  const [availableProviders, setAvailableProviders] = useState<{ id: string; name: string; models: { id: string; name: string }[] }[]>([])

  // Effective values (agent override → global fallback)
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
  // Whether agent has any overrides
  const agentHasOverride = isAgentMode && (
    agentExtractOverride.autoExtract !== null ||
    agentExtractOverride.extractMinTurns !== null ||
    agentExtractOverride.extractProviderId !== null ||
    agentExtractOverride.extractModelId !== null
  )

  // Multi-select state
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set())
  const [batchLoading, setBatchLoading] = useState(false)

  // Dedup confirmation state
  const [dedupSimilar, setDedupSimilar] = useState<MemoryEntry[]>([])
  const [dedupPendingEntry, setDedupPendingEntry] = useState<NewMemory | null>(null)

  // Embedding config state
  const [embeddingConfig, setEmbeddingConfig] = useState<EmbeddingConfig>({
    enabled: false,
    providerType: "openai-compatible",
  })
  const [presets, setPresets] = useState<EmbeddingPreset[]>([])
  const [localModels, setLocalModels] = useState<LocalEmbeddingModel[]>([])
  const [embeddingDirty, setEmbeddingDirty] = useState(false)
  const [embeddingTestLoading, setEmbeddingTestLoading] = useState(false)
  const [embeddingTestResult, setEmbeddingTestResult] = useState<TestResult | null>(null)
  const [embeddingSaving, setEmbeddingSaving] = useState(false)
  const [embeddingSaveStatus, setEmbeddingSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  // Dedup config state
  const [dedupConfig, setDedupConfig] = useState({ thresholdHigh: 0.02, thresholdMerge: 0.012 })
  const [dedupExpanded, setDedupExpanded] = useState(false)

  // Stats state
  const [stats, setStats] = useState<{ total: number; byType: Record<string, number>; withEmbedding: number } | null>(null)

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
      // Agent mode: filter by scope toggle
      if (filterScope === "global") return { kind: "global" }
      if (filterScope === "agent") return { kind: "agent", id: agentId! }
      return null // "all" → use agentId shorthand for global + this agent
    }
    // Standalone mode: filter by selected agent
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
          // In agent mode with "all" filter, use agentId to get global + agent memories
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
        invoke<{ total: number; byType: Record<string, number>; withEmbedding: number }>("memory_stats", { scope }).catch(() => null),
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
        // Always load global extract config
        const global = await invoke<{ autoExtract: boolean; extractMinTurns: number; extractProviderId: string | null; extractModelId: string | null }>("get_extract_config")
        setGlobalExtract(global)

        // In agent mode, also load per-agent override
        if (isAgentMode && agentId) {
          const cfg = await invoke<{ memory?: { autoExtract?: boolean | null; extractMinTurns?: number | null; extractProviderId?: string | null; extractModelId?: string | null } }>("get_agent_config", { id: agentId })
          setAgentExtractOverride({
            autoExtract: cfg?.memory?.autoExtract ?? null,
            extractMinTurns: cfg?.memory?.extractMinTurns ?? null,
            extractProviderId: cfg?.memory?.extractProviderId ?? null,
            extractModelId: cfg?.memory?.extractModelId ?? null,
          })
        }

        // Load available providers for model picker
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

  // Save global extract config
  async function saveGlobalExtract(updates: Partial<typeof globalExtract>) {
    const updated = { ...globalExtract, ...updates }
    setGlobalExtract(updated)
    try {
      await invoke("save_extract_config", { config: updated })
    } catch (e) {
      logger.error("settings", "MemoryPanel::saveGlobalExtract", "Failed", e)
    }
  }

  // Save per-agent extract override
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

  // Reset agent overrides to inherit global
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

      // Check for similar memories before adding
      const similar = await invoke<MemoryEntry[]>("memory_find_similar", {
        content: entry.content,
        threshold: 0.008,
        limit: 3,
      })

      if (similar.length > 0) {
        setDedupSimilar(similar)
        setDedupPendingEntry(entry)
        return // Show confirmation dialog
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

  async function handleExport() {
    try {
      const md = await invoke<string>("memory_export", { scope: null })
      // Copy to clipboard
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

  // ── Embedding Config View ──
  if (view === "embedding") {
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

              {embeddingConfig.providerType !== "local" ? (
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
            </div>
          )}
        </div>
      </div>
    )
  }

  // ── Add / Edit View ──
  if (view === "add" || view === "edit") {
    const isEdit = view === "edit"
    return (
      <div className="flex-1 overflow-y-auto p-6">
        <div className="max-w-4xl">
          <button
            onClick={() => {
              setView("list")
              setEditingMemory(null)
            }}
            className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground mb-4"
          >
            <ArrowLeft className="h-4 w-4" />
            {t("settings.memory")}
          </button>

          <h2 className="text-lg font-semibold mb-4">
            {isEdit ? t("settings.memoryEdit") : t("settings.memoryAdd")}
          </h2>

          <div className="space-y-4">
            {/* Type selector */}
            <div>
              <label className="text-sm font-medium mb-1.5 block">{t("settings.memoryType")}</label>
              <div className="flex gap-2">
                {MEMORY_TYPES.map((type) => {
                  const Icon = MEMORY_TYPE_ICONS[type]
                  return (
                    <button
                      key={type}
                      onClick={() => !isEdit && setFormType(type)}
                      className={cn(
                        "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs border transition-colors",
                        formType === type
                          ? "border-primary bg-primary/10 text-primary"
                          : "border-border text-muted-foreground hover:border-foreground/30",
                        isEdit && "opacity-60 cursor-default",
                      )}
                    >
                      <Icon className="h-3.5 w-3.5" />
                      {t(`settings.memoryType_${type}`)}
                    </button>
                  )
                })}
              </div>
            </div>

            {/* Scope selector (add only) */}
            {!isEdit && (
              <div>
                <label className="text-sm font-medium mb-1.5 block">
                  {t("settings.memoryScope")}
                </label>
                <div className="flex gap-2">
                  <button
                    onClick={() => setFormScope("global")}
                    className={cn(
                      "px-3 py-1.5 rounded-lg text-xs border transition-colors",
                      formScope === "global"
                        ? "border-primary bg-primary/10 text-primary"
                        : "border-border text-muted-foreground",
                    )}
                  >
                    {t("settings.memoryScopeGlobal")}
                  </button>
                  <button
                    onClick={() => setFormScope("agent")}
                    className={cn(
                      "px-3 py-1.5 rounded-lg text-xs border transition-colors",
                      formScope === "agent"
                        ? "border-primary bg-primary/10 text-primary"
                        : "border-border text-muted-foreground",
                    )}
                  >
                    {t("settings.memoryScopeAgent")}
                  </button>
                </div>
              </div>
            )}

            {/* Content */}
            <div>
              <label className="text-sm font-medium mb-1.5 block">
                {t("settings.memoryContent")}
              </label>
              <Textarea
                value={formContent}
                onChange={(e) => setFormContent(e.target.value)}
                placeholder={t("settings.memoryContentPlaceholder")}
                rows={5}
                className="text-sm"
              />
            </div>

            {/* Tags */}
            <div>
              <label className="text-sm font-medium mb-1.5 block">{t("settings.memoryTags")}</label>
              <Input
                value={formTags}
                onChange={(e) => setFormTags(e.target.value)}
                placeholder={t("settings.memoryTagsPlaceholder")}
                className="text-sm"
              />
            </div>

            <div className="flex gap-2">
              <Button
                onClick={isEdit ? handleUpdate : handleAdd}
                size="sm"
                disabled={!formContent.trim()}
              >
                {isEdit ? t("common.save") : t("settings.memoryAdd")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setView("list")
                  setEditingMemory(null)
                }}
              >
                {t("common.cancel")}
              </Button>
            </div>

            {/* Dedup confirmation dialog */}
            {dedupSimilar.length > 0 && dedupPendingEntry && (
              <div className="mt-4 rounded-lg border border-yellow-500/30 bg-yellow-500/5 p-4 space-y-3">
                <p className="text-sm font-medium text-yellow-600 dark:text-yellow-400">
                  {t("settings.memoryDuplicateFound")}
                </p>
                <div className="space-y-2">
                  {dedupSimilar.map((mem) => {
                    const Icon = MEMORY_TYPE_ICONS[mem.memoryType] || User
                    return (
                      <div
                        key={mem.id}
                        className="flex items-start gap-2 rounded-md border border-border/50 bg-background p-2.5"
                      >
                        <Icon className="h-4 w-4 mt-0.5 shrink-0 text-muted-foreground" />
                        <div className="flex-1 min-w-0">
                          <p className="text-xs text-muted-foreground line-clamp-2">{mem.content}</p>
                          {mem.relevanceScore != null && (
                            <span className="text-[10px] text-muted-foreground/60">
                              {t("settings.memorySimilarity")}: {(mem.relevanceScore * 100).toFixed(0)}%
                            </span>
                          )}
                        </div>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="shrink-0 text-xs h-7"
                          onClick={() => handleDedupUpdate(mem.id)}
                        >
                          {t("settings.memoryUpdateExisting")}
                        </Button>
                      </div>
                    )
                  })}
                </div>
                <div className="flex gap-2">
                  <Button size="sm" variant="outline" onClick={handleDedupConfirm}>
                    {t("settings.memoryAddAnyway")}
                  </Button>
                  <Button size="sm" variant="ghost" onClick={handleDedupCancel}>
                    {t("common.cancel")}
                  </Button>
                </div>
              </div>
            )}
          </div>
        </div>
      </div>
    )
  }

  // ── List View (default) ──
  return (
    <TooltipProvider>
      <div className="flex-1 flex flex-col min-h-0 overflow-hidden p-6">
        <div className="max-w-4xl w-full flex flex-col min-h-0">
          {/* Header */}
          <div className="flex items-center justify-between mb-1 shrink-0">
            <h2 className="text-lg font-semibold">{t("settings.memory")}</h2>
            <div className="flex items-center gap-2">
              <IconTip label={t("settings.memoryImport")}>
                <Button variant="ghost" size="sm" onClick={handleImport}>
                  <Upload className="h-4 w-4" />
                </Button>
              </IconTip>
              <IconTip label={t("settings.memoryExport")}>
                <Button variant="ghost" size="sm" onClick={handleExport}>
                  <FileDown className="h-4 w-4" />
                </Button>
              </IconTip>
              {!compact && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setView("embedding")}
                  className={cn(
                    "gap-1.5 text-xs",
                    embeddingConfig.enabled
                      ? "border-primary/40 text-primary hover:bg-primary/10"
                      : "text-muted-foreground",
                  )}
                >
                  <Zap className="h-3.5 w-3.5" />
                  {t("settings.memoryEmbedding")}
                  {embeddingConfig.enabled && (
                    <span className="h-1.5 w-1.5 rounded-full bg-primary" />
                  )}
                </Button>
              )}
              <Button size="sm" onClick={startAdd} className="gap-1.5">
                <Plus className="h-3.5 w-3.5" />
                {t("settings.memoryAdd")}
              </Button>
            </div>
          </div>
          <p className="text-xs text-muted-foreground mb-4 shrink-0">{t("settings.memoryDesc")}</p>

          {/* Auto-extract settings (global + per-agent override) */}
          {extractConfigLoaded && (
            <div className="rounded-lg bg-secondary/30 mb-4 shrink-0">
              <div className="flex items-center justify-between px-3 py-2">
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium flex items-center gap-1.5">
                    {t("settings.memoryAutoExtract")}
                    {isAgentMode && (
                      <span className="text-[10px] font-normal text-muted-foreground/70">
                        {agentHasOverride ? t("settings.memoryOverridden") : t("settings.memoryInherited")}
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-muted-foreground">{t("settings.memoryAutoExtractDesc")}</div>
                </div>
                <Switch
                  checked={effectiveAutoExtract}
                  onCheckedChange={handleToggleAutoExtract}
                />
              </div>
              {effectiveAutoExtract && (
                <div className="px-3 pb-3 space-y-2 border-t border-border/30 pt-2">
                  {/* Extraction model selector */}
                  <div className="flex items-center gap-2">
                    <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">{t("settings.memoryExtractModel")}</label>
                    <Select
                      value={effectiveProviderId && effectiveModelId ? `${effectiveProviderId}::${effectiveModelId}` : "__chat__"}
                      onValueChange={handleUpdateExtractModel}
                    >
                      <SelectTrigger className="h-7 text-xs flex-1">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="__chat__">{t("settings.memoryUseChatModel")}</SelectItem>
                        {availableProviders.map((prov) =>
                          prov.models.map((m) => (
                            <SelectItem key={`${prov.id}::${m.id}`} value={`${prov.id}::${m.id}`}>
                              {prov.name} / {m.name}
                            </SelectItem>
                          ))
                        )}
                      </SelectContent>
                    </Select>
                  </div>
                  {/* Min turns */}
                  <div className="flex items-center gap-2">
                    <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">{t("settings.memoryExtractMinTurns")}</label>
                    <Input
                      type="number"
                      min={1}
                      max={20}
                      value={effectiveMinTurns}
                      onChange={(e) => handleUpdateExtractMinTurns(parseInt(e.target.value) || 3)}
                      className="h-7 text-xs w-20"
                    />
                  </div>
                  {/* Reset to global (agent mode only) */}
                  {isAgentMode && agentHasOverride && (
                    <button
                      onClick={resetAgentExtract}
                      className="text-[11px] text-muted-foreground hover:text-foreground transition-colors underline underline-offset-2"
                    >
                      {t("settings.memoryResetToGlobal")}
                    </button>
                  )}
                </div>
              )}
            </div>
          )}

          {/* Stats bar */}
          {stats && stats.total > 0 && (
            <div className="flex items-center gap-3 text-xs text-muted-foreground mb-3 px-1 shrink-0 flex-wrap">
              <span>{t("settings.memoryStatsTotal", { count: stats.total })}</span>
              <span className="text-border">|</span>
              {(["user", "feedback", "project", "reference"] as const).map((type) => {
                const count = stats.byType[type] || 0
                if (count === 0) return null
                const Icon = MEMORY_TYPE_ICONS[type]
                return (
                  <span key={type} className="flex items-center gap-0.5">
                    <Icon className="h-3 w-3" />
                    {count}
                  </span>
                )
              })}
              {embeddingConfig.enabled && stats.total > 0 && (
                <>
                  <span className="text-border">|</span>
                  <span>{t("settings.memoryStatsVec", { pct: Math.round((stats.withEmbedding / stats.total) * 100) })}</span>
                </>
              )}
            </div>
          )}

          {/* Search + Filter */}
          <div className="flex gap-2 mb-4 shrink-0">
            <div className="relative flex-1">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
              <Input
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={t("settings.memorySearch")}
                className="pl-8 text-sm"
              />
              {searchQuery && (
                <button
                  onClick={() => setSearchQuery("")}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>
            <div className="flex gap-1">
              {MEMORY_TYPES.map((type) => {
                const Icon = MEMORY_TYPE_ICONS[type]
                return (
                  <IconTip key={type} label={t(`settings.memoryType_${type}`)}>
                    <button
                      onClick={() => setFilterType(filterType === type ? null : type)}
                      className={cn(
                        "p-2 rounded-lg border transition-colors",
                        filterType === type
                          ? "border-primary bg-primary/10 text-primary"
                          : "border-transparent text-muted-foreground hover:text-foreground hover:bg-secondary/40",
                      )}
                    >
                      <Icon className="h-4 w-4" />
                    </button>
                  </IconTip>
                )
              })}
            </div>
          </div>

          {/* Scope filter */}
          <div className="flex items-center gap-2 mb-3 shrink-0">
            <div className="flex gap-1">
              {(["all", "global", "agent"] as const).map((scope) => (
                <button
                  key={scope}
                  onClick={() => setFilterScope(scope)}
                  className={cn(
                    "px-2.5 py-1 rounded-md text-xs transition-colors",
                    filterScope === scope
                      ? "bg-secondary text-foreground font-medium"
                      : "text-muted-foreground hover:text-foreground hover:bg-secondary/40",
                  )}
                >
                  {scope === "all"
                    ? t("settings.memoryScopeAll")
                    : scope === "global"
                      ? t("settings.memoryScopeGlobal")
                      : t("settings.memoryScopeAgent")}
                </button>
              ))}
            </div>
            {/* Agent picker (standalone mode, agent scope selected) */}
            {!isAgentMode && filterScope === "agent" && agents.length > 0 && (
              <Select
                value={selectedAgentId ?? ""}
                onValueChange={(v) => setSelectedAgentId(v || null)}
              >
                <SelectTrigger className="w-40 h-7 text-xs">
                  <SelectValue placeholder={t("settings.memorySelectAgent")} />
                </SelectTrigger>
                <SelectContent>
                  {agents.map((a) => (
                    <SelectItem key={a.id} value={a.id} className="text-xs">
                      {a.emoji ? `${a.emoji} ` : ""}
                      {a.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </div>

          {/* Stats + Batch actions */}
          <div className="flex items-center justify-between text-xs text-muted-foreground mb-3 shrink-0">
            <div className="flex items-center gap-2">
              {memories.length > 0 && (
                <button
                  onClick={toggleSelectAll}
                  className="p-0.5 hover:text-foreground transition-colors"
                >
                  {selectedIds.size === memories.length ? (
                    <CheckSquare className="h-3.5 w-3.5" />
                  ) : (
                    <Square className="h-3.5 w-3.5" />
                  )}
                </button>
              )}
              <span>{t("settings.memoryCount", { count: totalCount })}</span>
              {embeddingConfig.enabled && (
                <span className="text-primary">
                  <Zap className="h-3 w-3 inline -mt-0.5 mr-0.5" />
                  {t("settings.memoryVectorEnabled")}
                </span>
              )}
            </div>
            {selectedIds.size > 0 && (
              <div className="flex items-center gap-1.5">
                <Button
                  variant="destructive"
                  size="sm"
                  className="h-6 text-xs px-2"
                  disabled={batchLoading}
                  onClick={handleDeleteBatch}
                >
                  {t("settings.memoryDeleteBatch", { count: selectedIds.size })}
                </Button>
                {embeddingConfig.enabled && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-6 text-xs px-2"
                    disabled={batchLoading}
                    onClick={handleReembedBatch}
                  >
                    {t("settings.memoryReembed", { count: selectedIds.size })}
                  </Button>
                )}
              </div>
            )}
          </div>

          {/* Memory List */}
          <div className="flex-1 overflow-y-auto space-y-1.5">
            {loading && memories.length === 0 ? (
              <div className="text-sm text-muted-foreground py-8 text-center">
                {t("settings.loading")}
              </div>
            ) : memories.length === 0 ? (
              <div className="text-sm text-muted-foreground py-8 text-center">
                {searchQuery ? t("settings.memoryNoResults") : t("settings.memoryEmpty")}
              </div>
            ) : (
              memories.map((mem) => {
                const Icon = MEMORY_TYPE_ICONS[mem.memoryType] || User
                const isSelected = selectedIds.has(mem.id)
                const scopeLabel =
                  mem.scope.kind === "global"
                    ? "Global"
                    : `Agent: ${(mem.scope as { kind: "agent"; id: string }).id}`
                return (
                  <div
                    key={mem.id}
                    className={cn(
                      "group flex items-start gap-3 px-3 py-2.5 rounded-lg hover:bg-secondary/40 cursor-pointer transition-colors",
                      isSelected && "bg-primary/5 border border-primary/20",
                    )}
                    onClick={() => startEdit(mem)}
                  >
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        toggleSelect(mem.id)
                      }}
                      className="mt-0.5 shrink-0 p-0 text-muted-foreground hover:text-foreground transition-colors"
                    >
                      {isSelected ? (
                        <CheckSquare className="h-4 w-4 text-primary" />
                      ) : (
                        <Square className="h-4 w-4 opacity-0 group-hover:opacity-100 transition-opacity" />
                      )}
                    </button>
                    <Icon className="h-4 w-4 text-muted-foreground mt-0.5 shrink-0" />
                    <div className="flex-1 min-w-0">
                      <div className="text-sm line-clamp-2">{mem.content}</div>
                      <div className="flex items-center gap-2 mt-1 text-xs text-muted-foreground">
                        <span>{t(`settings.memoryType_${mem.memoryType}`)}</span>
                        <span>·</span>
                        <span>{scopeLabel}</span>
                        {mem.tags.length > 0 && (
                          <>
                            <span>·</span>
                            <span>{mem.tags.join(", ")}</span>
                          </>
                        )}
                        {mem.relevanceScore != null && (
                          <>
                            <span>·</span>
                            <span className="text-primary">
                              {(mem.relevanceScore * 100).toFixed(0)}%
                            </span>
                          </>
                        )}
                      </div>
                    </div>
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        handleDelete(mem.id)
                      }}
                      className="opacity-0 group-hover:opacity-100 p-1 text-muted-foreground hover:text-destructive transition-opacity"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                    <ChevronRight className="h-4 w-4 text-muted-foreground/30 mt-0.5 shrink-0" />
                  </div>
                )
              })
            )}
          </div>
        </div>
      </div>
    </TooltipProvider>
  )
}

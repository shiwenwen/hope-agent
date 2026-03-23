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
  const [filterScope, setFilterScope] = useState<"all" | "global" | "agent">(isAgentMode ? "all" : "all")
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(agentId ?? null)

  // Edit/Add state
  const [editingMemory, setEditingMemory] = useState<MemoryEntry | null>(null)
  const [formContent, setFormContent] = useState("")
  const [formType, setFormType] = useState<"user" | "feedback" | "project" | "reference">("user")
  const [formTags, setFormTags] = useState("")
  const [formScope, setFormScope] = useState<"global" | "agent">(isAgentMode ? "agent" : "global")

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

  // ── Load agents for scope picker (standalone mode) ──
  useEffect(() => {
    if (!isAgentMode) {
      invoke<AgentInfo[]>("list_agents").then(setAgents).catch(() => {})
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
      const count = await invoke<number>("memory_count", { scope })
      setTotalCount(count)
    } catch (e) {
      logger.error("settings", "MemoryPanel::load", "Failed to load memories", e)
    } finally {
      setLoading(false)
    }
  }, [searchQuery, filterType, buildScope, isAgentMode, filterScope, agentId])

  useEffect(() => {
    loadMemories()
  }, [loadMemories])

  // ── Load embedding config ──
  useEffect(() => {
    async function loadEmbedding() {
      try {
        const [config, presetList, models] = await Promise.all([
          invoke<EmbeddingConfig>("get_embedding_config"),
          invoke<EmbeddingPreset[]>("get_embedding_presets"),
          invoke<LocalEmbeddingModel[]>("list_local_embedding_models"),
        ])
        setEmbeddingConfig(config)
        setPresets(presetList)
        setLocalModels(models)
      } catch (e) {
        logger.error("settings", "MemoryPanel::loadEmbedding", "Failed to load embedding config", e)
      }
    }
    loadEmbedding()
  }, [])

  // ── CRUD handlers ──
  async function handleAdd() {
    try {
      const scopeAgentId = isAgentMode ? agentId! : (selectedAgentId ?? "default")
      const entry: NewMemory = {
        memoryType: formType,
        scope: formScope === "global" ? { kind: "global" } : { kind: "agent", id: scopeAgentId },
        content: formContent.trim(),
        tags: formTags.split(",").map((t) => t.trim()).filter(Boolean),
        source: "user",
      }
      await invoke("memory_add", { entry })
      setView("list")
      setFormContent("")
      setFormTags("")
      loadMemories()
    } catch (e) {
      logger.error("settings", "MemoryPanel::add", "Failed to add memory", e)
    }
  }

  async function handleUpdate() {
    if (!editingMemory) return
    try {
      const tags = formTags.split(",").map((t) => t.trim()).filter(Boolean)
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

  async function saveEmbeddingConfig() {
    try {
      await invoke("save_embedding_config", { config: embeddingConfig })
      setEmbeddingDirty(false)
    } catch (e) {
      logger.error("settings", "MemoryPanel::saveEmbedding", "Failed to save", e)
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
              <div className="text-xs text-muted-foreground">{t("settings.memoryEmbeddingEnabledDesc")}</div>
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
                <label className="text-sm font-medium mb-1.5 block">{t("settings.memoryEmbeddingProvider")}</label>
                <div className="flex flex-wrap gap-2">
                  {presets.map((preset) => (
                    <button
                      key={preset.name}
                      onClick={() => {
                        setEmbeddingConfig({
                          ...embeddingConfig,
                          providerType: preset.providerType,
                          apiBaseUrl: preset.baseUrl,
                          apiModel: preset.defaultModel,
                          apiDimensions: preset.defaultDimensions,
                        })
                        setEmbeddingDirty(true)
                      }}
                      className={cn(
                        "px-3 py-1.5 rounded-lg text-xs border transition-colors",
                        embeddingConfig.apiBaseUrl === preset.baseUrl
                          ? "border-primary bg-primary/10 text-primary"
                          : "border-border text-muted-foreground hover:border-foreground/30"
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
                        : "border-border text-muted-foreground hover:border-foreground/30"
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
                      <label className="text-xs text-muted-foreground mb-1 block">{t("settings.memoryModel")}</label>
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
                      <label className="text-xs text-muted-foreground mb-1 block">{t("settings.memoryDimensions")}</label>
                      <Input
                        type="number"
                        value={embeddingConfig.apiDimensions ?? ""}
                        onChange={(e) => {
                          setEmbeddingConfig({ ...embeddingConfig, apiDimensions: e.target.value ? Number(e.target.value) : null })
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
                  <label className="text-sm font-medium mb-1.5 block">{t("settings.memorySelectModel")}</label>
                  {localModels.map((model) => (
                    <button
                      key={model.id}
                      onClick={() => {
                        setEmbeddingConfig({ ...embeddingConfig, localModelId: model.id, apiDimensions: model.dimensions })
                        setEmbeddingDirty(true)
                      }}
                      className={cn(
                        "w-full flex items-center justify-between px-3 py-2.5 rounded-lg border transition-colors text-left",
                        embeddingConfig.localModelId === model.id
                          ? "border-primary bg-primary/10"
                          : "border-border hover:border-foreground/30"
                      )}
                    >
                      <div>
                        <div className="text-sm font-medium">{model.name}</div>
                        <div className="text-xs text-muted-foreground">
                          {model.dimensions}d | {model.sizeMb}MB | RAM {model.minRamGb}GB+ | {model.languages.join(", ")}
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
                {embeddingDirty && (
                  <Button onClick={saveEmbeddingConfig} size="sm">
                    {t("common.save")}
                  </Button>
                )}
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={embeddingTestLoading || (embeddingConfig.providerType === "local" ? !embeddingConfig.localModelId : !embeddingConfig.apiBaseUrl?.trim())}
                  onClick={async () => {
                    setEmbeddingTestLoading(true)
                    setEmbeddingTestResult(null)
                    try {
                      const msg = await invoke<string>("test_embedding", { config: embeddingConfig })
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
            onClick={() => { setView("list"); setEditingMemory(null) }}
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
                        isEdit && "opacity-60 cursor-default"
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
                <label className="text-sm font-medium mb-1.5 block">{t("settings.memoryScope")}</label>
                <div className="flex gap-2">
                  <button
                    onClick={() => setFormScope("global")}
                    className={cn(
                      "px-3 py-1.5 rounded-lg text-xs border transition-colors",
                      formScope === "global"
                        ? "border-primary bg-primary/10 text-primary"
                        : "border-border text-muted-foreground"
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
                        : "border-border text-muted-foreground"
                    )}
                  >
                    {t("settings.memoryScopeAgent")}
                  </button>
                </div>
              </div>
            )}

            {/* Content */}
            <div>
              <label className="text-sm font-medium mb-1.5 block">{t("settings.memoryContent")}</label>
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
                onClick={() => { setView("list"); setEditingMemory(null) }}
              >
                {t("common.cancel")}
              </Button>
            </div>
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
                    : "text-muted-foreground"
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
                        : "border-transparent text-muted-foreground hover:text-foreground hover:bg-secondary/40"
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
                    : "text-muted-foreground hover:text-foreground hover:bg-secondary/40"
                )}
              >
                {scope === "all" ? t("settings.memoryScopeAll") : scope === "global" ? t("settings.memoryScopeGlobal") : t("settings.memoryScopeAgent")}
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
                    {a.emoji ? `${a.emoji} ` : ""}{a.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </div>

        {/* Stats */}
        <div className="text-xs text-muted-foreground mb-3 shrink-0">
          {t("settings.memoryCount", { count: totalCount })}
          {embeddingConfig.enabled && (
            <span className="ml-2 text-primary">
              <Zap className="h-3 w-3 inline -mt-0.5 mr-0.5" />
              {t("settings.memoryVectorEnabled")}
            </span>
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
              const scopeLabel = mem.scope.kind === "global" ? "Global" : `Agent: ${(mem.scope as { kind: "agent"; id: string }).id}`
              return (
                <div
                  key={mem.id}
                  className="group flex items-start gap-3 px-3 py-2.5 rounded-lg hover:bg-secondary/40 cursor-pointer transition-colors"
                  onClick={() => startEdit(mem)}
                >
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
                          <span className="text-primary">{(mem.relevanceScore * 100).toFixed(0)}%</span>
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

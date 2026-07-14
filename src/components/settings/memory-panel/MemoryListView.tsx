import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { IconTip } from "@/components/ui/tooltip"
import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { SearchInput } from "@/components/ui/search-input"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import {
  Plus,
  Trash2,
  Search,
  AlertTriangle,
  Upload,
  ChevronRight,
  X,
  FileDown,
  FileSearch,
  Copy,
  BookmarkPlus,
  Zap,
  CheckSquare,
  Square,
  User,
  Pin,
  Sparkles,
  Archive,
  Loader2,
  LockKeyhole,
  Package,
} from "lucide-react"
import {
  MEMORY_SOURCE_FILTERS,
  MEMORY_SOURCE_FILTER_SOURCES,
  MEMORY_TYPES,
  MEMORY_TYPE_ICONS,
  evaluateMemoryBackupPassphrase,
  type MemorySourceFilter,
  type MemoryEntry,
} from "./types"
import ImportFromAIDialog from "./ImportFromAIDialog"
import { explainMemorySearchMatch } from "./memorySearchExplain"
import {
  copyMemoryBackupPreviewDiagnostics,
  formatMemoryBackupPreviewIssueMessage,
  formatMemoryBackupPreviewNextStep,
} from "./memoryBackupPreviewDiagnostics"
import {
  formatMemoryBackupAlreadyPresentSummary,
  formatMemoryBackupAttachmentSummary,
  formatMemoryBackupClaimPlanSummary,
  formatMemoryBackupClaimConflictHeader,
  formatMemoryBackupExperiencePlanSummary,
  formatMemoryBackupHistorySummary,
  formatMemoryBackupProfilePlanSummary,
} from "./memoryBackupPreviewSummary"
import {
  buildMemoryBackupRestorePlan,
  formatMemoryBackupRestorePlanActionLabel,
  formatMemoryBackupRestorePlanRowDetail,
  formatMemoryBackupRestorePlanRowLabel,
  formatMemoryBackupRestorePlanSummaryDetail,
  formatMemoryBackupRestorePlanSummaryNextStep,
  formatMemoryBackupRestorePlanSummaryTitle,
  hasMemoryBackupStructuredRestoreCandidates,
  summarizeMemoryBackupRestorePlan,
  type MemoryBackupRestorePlanTone,
} from "./memoryBackupRestorePlan"
import {
  copyMemoryImportPreviewDiagnostics,
  formatMemoryImportScopeLabel,
  formatMemoryImportScopeSummaryKey,
  memoryImportPreviewIssueMessages,
  memoryImportPreviewSampleWindowLabel,
  memoryImportPreviewStatusLabel,
  memoryImportSortedCountEntries,
} from "./memoryImportFeedback"
import { memoryCrudOperationErrorToast } from "./memoryCrudOperationFeedback"
import type { useMemoryData } from "./useMemoryData"

type MemoryData = ReturnType<typeof useMemoryData>

interface MemoryListViewProps {
  data: MemoryData
  isAgentMode: boolean
  compact?: boolean
  embedded?: boolean
  focus?: { nonce: number; memoryId: number } | null
  onOpenClaims?: (focus?: { statusFilter?: string }) => void
}

interface MemoryListFilterPreset {
  id: string
  query: string
  type: MemoryEntry["memoryType"] | null
  sources: MemorySourceFilter[]
  scope: "all" | "global" | "agent"
  agentId?: string | null
  updatedAt: number
}

const MEMORY_LIST_PRESET_LIMIT = 6
const MEMORY_LIST_PRESET_STORAGE_KEY = "hope.memory.listFilterPresets.v1"

function sourceLabelKey(source: string): string | null {
  if (source === "flush") return "auto"
  if (Object.prototype.hasOwnProperty.call(MEMORY_SOURCE_FILTER_SOURCES, source)) return source
  return null
}

function normalizePresetSources(value: unknown): MemorySourceFilter[] {
  if (!Array.isArray(value)) return []
  const selected = new Set(
    value.filter(
      (item): item is MemorySourceFilter =>
        typeof item === "string" && MEMORY_SOURCE_FILTERS.includes(item as MemorySourceFilter),
    ),
  )
  return MEMORY_SOURCE_FILTERS.filter((source) => selected.has(source))
}

function memoryListPresetId(
  query: string,
  type: string | null,
  sources: MemorySourceFilter[],
  scope: "all" | "global" | "agent",
  agentId?: string | null,
): string {
  const normalizedSources = normalizePresetSources(sources)
  return [
    query.trim().toLocaleLowerCase(),
    type ?? "",
    normalizedSources.join(","),
    scope,
    scope === "agent" ? (agentId ?? "") : "",
  ].join("|")
}

function normalizeMemoryListPreset(raw: unknown): MemoryListFilterPreset | null {
  if (!raw || typeof raw !== "object") return null
  const value = raw as Record<string, unknown>
  const query = typeof value.query === "string" ? value.query.trim().slice(0, 200) : ""
  const type =
    typeof value.type === "string" && MEMORY_TYPES.includes(value.type as MemoryEntry["memoryType"])
      ? (value.type as MemoryEntry["memoryType"])
      : null
  const sources = normalizePresetSources(value.sources)
  const scope =
    value.scope === "global" || value.scope === "agent" || value.scope === "all"
      ? value.scope
      : "all"
  const agentId =
    typeof value.agentId === "string" && value.agentId.trim() ? value.agentId.trim() : null
  const updatedAt = Number(value.updatedAt)
  return {
    id: memoryListPresetId(query, type, sources, scope, agentId),
    query,
    type,
    sources,
    scope,
    agentId,
    updatedAt: Number.isFinite(updatedAt) && updatedAt > 0 ? updatedAt : Date.now(),
  }
}

function loadMemoryListFilterPresets(): MemoryListFilterPreset[] {
  if (typeof window === "undefined") return []
  try {
    const raw = window.localStorage.getItem(MEMORY_LIST_PRESET_STORAGE_KEY)
    const parsed = raw ? JSON.parse(raw) : []
    if (!Array.isArray(parsed)) return []
    const deduped = new Map<string, MemoryListFilterPreset>()
    for (const item of parsed) {
      const preset = normalizeMemoryListPreset(item)
      if (!preset) continue
      const existing = deduped.get(preset.id)
      if (!existing || existing.updatedAt < preset.updatedAt) {
        deduped.set(preset.id, preset)
      }
    }
    return [...deduped.values()]
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .slice(0, MEMORY_LIST_PRESET_LIMIT)
  } catch {
    return []
  }
}

function persistMemoryListFilterPresets(presets: MemoryListFilterPreset[]) {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(
      MEMORY_LIST_PRESET_STORAGE_KEY,
      JSON.stringify(presets.slice(0, MEMORY_LIST_PRESET_LIMIT)),
    )
  } catch {
    // localStorage may be unavailable in private / restricted contexts.
  }
}

function previewTypeOrder(type: string): number {
  const idx = ["user", "feedback", "project", "reference"].indexOf(type)
  return idx >= 0 ? idx : 99
}

function sampleDedupClass(status?: string | null): string {
  if (status === "new") {
    return "rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300"
  }
  if (status === "duplicate") {
    return "rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-700 dark:text-amber-300"
  }
  if (status === "merge") {
    return "rounded bg-muted px-1.5 py-0.5 text-muted-foreground"
  }
  return "rounded bg-muted px-1.5 py-0.5 text-muted-foreground"
}

export default function MemoryListView({
  data,
  isAgentMode,
  embedded = false,
  focus,
  onOpenClaims,
}: MemoryListViewProps) {
  const { t } = useTranslation()
  const [confirmBatchDeleteOpen, setConfirmBatchDeleteOpen] = useState(false)
  const [filterPresets, setFilterPresets] = useState<MemoryListFilterPreset[]>(() =>
    loadMemoryListFilterPresets(),
  )
  const lastFocusNonceRef = useRef<number | null>(null)

  const {
    memories,
    totalCount,
    loading,
    searchQuery,
    setSearchQuery,
    filterType,
    setFilterType,
    filterSources,
    setFilterSources,
    filterScope,
    setFilterScope,
    agents,
    agentListError,
    statsLoadError,
    selectedAgentId,
    setSelectedAgentId,
    selectedIds,
    batchLoading,
    backupLoading,
    backupPreviewLoading,
    backupRestoreLoading,
    backupPreviewOpen,
    setBackupPreviewOpen,
    backupPreview,
    backupReviewInboxHintCount,
    backupRestoreProfileConflicts,
    setBackupRestoreProfileConflicts,
    backupPassphraseDialogMode,
    backupPassphrase,
    setBackupPassphrase,
    backupPassphraseConfirm,
    setBackupPassphraseConfirm,
    closeBackupPassphraseDialog,
    submitBackupPassphraseDialog,
    memoryEmbeddingState,
    stats,
    handleExport,
    handleBackupExport,
    handleBackupExportArchive,
    handleBackupExportEncrypted,
    handleBackupPreview,
    handleBackupRestoreLegacy,
    handleBackupRestoreStructured,
    handleImport,
    importPreviewOpen,
    setImportPreviewOpen,
    importPreview,
    importPreviewFilename,
    importPreviewLoading,
    importApplyLoading,
    closeImportPreview,
    handleImportPreviewApply,
    importFromAIOpen,
    setImportFromAIOpen,
    loadMemories,
    handleDelete,
    handleDeleteBatch,
    handleReembedBatch,
    handleTogglePin,
    toggleSelect,
    toggleSelectAll,
    startEdit,
    startAdd,
  } = data
  const selectedAgent = agents.find((agent) => agent.id === selectedAgentId)
  const memoryEnabled = data.effectiveMemoryEnabled
  const backupClaimPlan = backupPreview?.claimRestorePlan
  const backupProfilePlan = backupPreview?.profileRestorePlan
  const hasBackupProfileConflicts = (backupProfilePlan?.conflictingScopeCandidates ?? 0) > 0
  const backupRestorePlan = backupPreview
    ? buildMemoryBackupRestorePlan(backupPreview, {
        allowProfileScopeConflicts: backupRestoreProfileConflicts,
      })
    : []
  const backupRestorePlanSummary =
    backupRestorePlan.length > 0 ? summarizeMemoryBackupRestorePlan(backupRestorePlan) : null
  const hasBackupStructuredRestoreCandidates = hasMemoryBackupStructuredRestoreCandidates(backupPreview)
  const hasBackupReviewInboxEntry =
    !isAgentMode &&
    !!onOpenClaims &&
    (backupReviewInboxHintCount > 0 ||
      (backupClaimPlan?.conflictingCandidates ?? 0) > 0 ||
      (backupClaimPlan?.needsReviewCandidates ?? 0) > 0)
  const backupPassphrasePolicy = evaluateMemoryBackupPassphrase(backupPassphrase)
  const backupPassphraseScore = backupPassphrasePolicy.score
  const backupPassphraseLabel =
    backupPassphrasePolicy.accepted && backupPassphraseScore >= 4
      ? t("settings.memoryBackupPassphraseStrong", "Strong")
      : backupPassphrasePolicy.accepted || backupPassphraseScore >= 2
        ? t("settings.memoryBackupPassphraseOkay", "Okay")
        : t("settings.memoryBackupPassphraseWeak", "Weak")
  const backupPassphraseMeterClass =
    backupPassphrasePolicy.accepted && backupPassphraseScore >= 4
      ? "bg-emerald-500"
      : backupPassphrasePolicy.accepted || backupPassphraseScore >= 2
        ? "bg-amber-500"
        : "bg-destructive"
  const backupPassphraseHint =
    backupPassphraseDialogMode === "export" &&
    backupPassphrase.length > 0 &&
    !backupPassphrasePolicy.accepted
      ? t(
          backupPassphrasePolicy.reasonKey ?? "settings.memoryBackupEncryptedPassphraseVariety",
          backupPassphrasePolicy.reasonDefault ?? "Use more variety, or a longer four-word phrase",
        )
      : null
  const importPreviewTypeEntries = importPreview
    ? Object.entries(importPreview.byType).sort(
        ([left], [right]) => previewTypeOrder(left) - previewTypeOrder(right),
      )
    : []
  const importPreviewScopeEntries = importPreview
    ? memoryImportSortedCountEntries(importPreview.byScope)
    : []
  const importPreviewLikelyImportCount =
    (importPreview?.likelyNewCount ?? importPreview?.candidateCount ?? 0) +
    (importPreview?.likelyMergeCount ?? 0)
  const importPreviewIssueMessages = importPreview
    ? memoryImportPreviewIssueMessages(t, importPreview)
    : []
  const importPreviewVisibleSamples = importPreview ? importPreview.samples.slice(0, 8) : []
  const importPreviewSampleWindowLabel = importPreview
    ? memoryImportPreviewSampleWindowLabel(
        t,
        importPreview.samples.length,
        importPreviewVisibleSamples.length,
      )
    : null

  const backupPlanToneClass = (tone: MemoryBackupRestorePlanTone): string => {
    switch (tone) {
      case "good":
        return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
      case "warn":
        return "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300"
      case "danger":
        return "border-destructive/30 bg-destructive/10 text-destructive"
      case "muted":
        return "border-border/60 bg-muted/30 text-muted-foreground"
    }
  }

  useEffect(() => {
    if (!focus || lastFocusNonceRef.current === focus.nonce) return
    lastFocusNonceRef.current = focus.nonce
    const local = memories.find((mem) => mem.id === focus.memoryId)
    if (local) {
      startEdit(local)
      return
    }
    let cancelled = false
    void getTransport()
      .call<MemoryEntry | null>("memory_get", { id: focus.memoryId })
      .then((memory) => {
        if (cancelled) return
        if (memory) {
          startEdit(memory)
          return
        }
        const failure = memoryCrudOperationErrorToast("focus", t, null)
        toast.error(failure.title, {
          description: t(
            "settings.memoryCrudErrors.focusMissing",
            "The selected memory no longer exists. Memory ID: {{id}}",
            { id: focus.memoryId },
          ),
        })
      })
      .catch((e) => {
        if (cancelled) return
        logger.warn("settings", "MemoryListView::focus", "Failed to focus memory", e)
        const failure = memoryCrudOperationErrorToast("focus", t, e)
        toast.error(
          failure.title,
          failure.description ? { description: failure.description } : undefined,
        )
      })
    return () => {
      cancelled = true
    }
  }, [focus, memories, startEdit, t])

  const activeEmbeddingModel = memoryEmbeddingState.selection.enabled
    ? (memoryEmbeddingState.currentModel?.name ??
      memoryEmbeddingState.currentModel?.apiModel ??
      t("settings.memoryModel"))
    : null
  const embeddingEnabled = memoryEmbeddingState.selection.enabled
  const hasActiveFilter =
    !!searchQuery || !!filterType || filterSources.length > 0 || filterScope !== "all"
  const currentPresetId = memoryListPresetId(
    searchQuery,
    filterType as MemoryEntry["memoryType"] | null,
    filterSources,
    filterScope,
    selectedAgentId,
  )
  const toggleSourceFilter = (source: MemorySourceFilter) => {
    setFilterSources((prev) =>
      prev.includes(source) ? prev.filter((item) => item !== source) : [...prev, source],
    )
  }
  const presetScopeLabel = (preset: MemoryListFilterPreset): string => {
    if (preset.scope === "all") return t("settings.memoryScopeAll")
    if (preset.scope === "global") return t("settings.memoryScopeGlobal")
    const agentName = agents.find((agent) => agent.id === preset.agentId)?.name
    return agentName
      ? `${t("settings.memoryScopeAgent")}: ${agentName}`
      : t("settings.memoryScopeAgent")
  }
  const memoryListPresetLabel = (preset: MemoryListFilterPreset): string => {
    const parts = [
      preset.type ? t(`settings.memoryType_${preset.type}`) : t("settings.memoryFilterAnyType"),
      preset.sources.length > 0
        ? preset.sources.map((source) => t(`settings.memorySource_${source}`)).join(" + ")
        : t("settings.memorySourceAll"),
      presetScopeLabel(preset),
    ]
    if (preset.query) parts.push(`"${preset.query}"`)
    return parts.join(" / ")
  }
  const saveFilterPreset = () => {
    const preset: MemoryListFilterPreset = {
      id: currentPresetId,
      query: searchQuery.trim(),
      type: filterType as MemoryEntry["memoryType"] | null,
      sources: normalizePresetSources(filterSources),
      scope: filterScope,
      agentId: filterScope === "agent" ? selectedAgentId : null,
      updatedAt: Date.now(),
    }
    setFilterPresets((prev) => {
      const next = [preset, ...prev.filter((item) => item.id !== preset.id)].slice(
        0,
        MEMORY_LIST_PRESET_LIMIT,
      )
      persistMemoryListFilterPresets(next)
      return next
    })
    toast.success(t("settings.memoryFilterPresetSaved"))
  }
  const applyFilterPreset = (preset: MemoryListFilterPreset) => {
    setSearchQuery(preset.query)
    setFilterType(preset.type)
    setFilterSources(normalizePresetSources(preset.sources))
    setFilterScope(preset.scope)
    if (preset.scope === "agent") {
      setSelectedAgentId(preset.agentId ?? selectedAgentId)
    }
  }
  const removeFilterPreset = (id: string) => {
    setFilterPresets((prev) => {
      const next = prev.filter((item) => item.id !== id)
      persistMemoryListFilterPresets(next)
      return next
    })
  }

  return (
    <div className={embedded ? "w-full" : "flex-1 overflow-y-auto p-6"}>
      <div className="w-full">
        {/* Header */}
        <div className="flex items-center justify-between mb-1">
          <h2 className="text-lg font-semibold">{t("settings.memory")}</h2>
          <div className="flex items-center gap-2">
            <IconTip label={t("settings.memoryImportFromAI")}>
              <Button variant="ghost" size="sm" onClick={() => setImportFromAIOpen(true)}>
                <Sparkles className="h-4 w-4" />
              </Button>
            </IconTip>
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
            {!isAgentMode && (
              <IconTip label={t("settings.memoryBackupExport", "Export backup bundle")}>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleBackupExport}
                  disabled={backupLoading}
                >
                  {backupLoading ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Archive className="h-4 w-4" />
                  )}
                </Button>
              </IconTip>
            )}
            {!isAgentMode && (
              <IconTip label={t("settings.memoryBackupArchiveExport", "Export backup package")}>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleBackupExportArchive}
                  disabled={backupLoading}
                >
                  {backupLoading ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Package className="h-4 w-4" />
                  )}
                </Button>
              </IconTip>
            )}
            {!isAgentMode && (
              <IconTip label={t("settings.memoryBackupExportEncrypted", "Export encrypted backup")}>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleBackupExportEncrypted}
                  disabled={backupLoading}
                >
                  {backupLoading ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <LockKeyhole className="h-4 w-4" />
                  )}
                </Button>
              </IconTip>
            )}
            {!isAgentMode && (
              <IconTip label={t("settings.memoryBackupPreview", "Preview backup import")}>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleBackupPreview}
                  disabled={backupPreviewLoading}
                >
                  {backupPreviewLoading ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <FileSearch className="h-4 w-4" />
                  )}
                </Button>
              </IconTip>
            )}
            <Button size="sm" onClick={startAdd} className="gap-1.5">
              <Plus className="h-3.5 w-3.5" />
              {t("settings.memoryAdd")}
            </Button>
          </div>
        </div>
        <p className={cn("text-xs text-muted-foreground", activeEmbeddingModel ? "mb-1" : "mb-4")}>
          {t("settings.memoryDesc")}
        </p>
        {!isAgentMode && !memoryEnabled && (
          <div className="mb-4 flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-muted-foreground">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-300" />
            <div className="min-w-0">
              <div className="font-medium text-foreground">
                {t("settings.memoryOffOwnerNoticeTitle")}
              </div>
              <div className="mt-0.5">{t("settings.memoryOffOwnerNoticeDesc")}</div>
            </div>
          </div>
        )}
        {activeEmbeddingModel && (
          <div className="mb-4 flex min-h-4 items-center gap-1.5 text-xs text-green-600 dark:text-green-400">
            <span className="h-1.5 w-1.5 rounded-full bg-green-500" />
            <span className="truncate">
              {t("settings.memoryVectorEnabled")} {activeEmbeddingModel}
            </span>
          </div>
        )}

        {/* Stats bar */}
        {statsLoadError && (
          <div className="mb-3 flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-muted-foreground">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-300" />
            <div className="min-w-0">
              <div className="font-medium text-foreground">{statsLoadError.title}</div>
              {statsLoadError.description && (
                <div className="mt-0.5 break-words">{statsLoadError.description}</div>
              )}
            </div>
          </div>
        )}
        {stats && stats.total > 0 && (
          <div className="flex items-center gap-3 text-xs text-muted-foreground mb-3 px-1 flex-wrap">
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
            {embeddingEnabled && stats.total > 0 && (
              <>
                <span className="text-border">|</span>
                <span>
                  {t("settings.memoryStatsVec", {
                    pct: Math.round((stats.withEmbedding / stats.total) * 100),
                  })}
                </span>
              </>
            )}
          </div>
        )}

        {/* Search + Filter */}
        <div className="flex gap-2 mb-4">
          <div className="relative flex-1">
            <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
            <SearchInput
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={t("settings.memorySearch")}
              className="pl-8 text-sm"
            />
            {searchQuery && (
              <IconTip label={t("common.clear")}>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => setSearchQuery("")}
                  className="absolute right-1 top-1/2 -translate-y-1/2 h-6 w-6 text-muted-foreground hover:text-foreground"
                  aria-label={t("common.clear")}
                >
                  <X className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
            )}
          </div>
          <div className="flex gap-1">
            {MEMORY_TYPES.map((type) => {
              const Icon = MEMORY_TYPE_ICONS[type]
              return (
                <IconTip key={type} label={t(`settings.memoryType_${type}`)}>
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={() => setFilterType(filterType === type ? null : type)}
                    aria-label={t(`settings.memoryType_${type}`)}
                    aria-pressed={filterType === type}
                    className={cn(
                      "h-9 w-9 rounded-lg",
                      filterType === type
                        ? "border-primary bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary"
                        : "border-transparent text-muted-foreground hover:text-foreground hover:bg-secondary/40",
                    )}
                  >
                    <Icon className="h-4 w-4" />
                  </Button>
                </IconTip>
              )
            })}
          </div>
          <div className="flex min-w-0 flex-wrap gap-1">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className={cn(
                "h-9 rounded-lg px-2 text-xs",
                filterSources.length === 0
                  ? "border-primary bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary"
                  : "border-transparent text-muted-foreground hover:bg-secondary/40 hover:text-foreground",
              )}
              onClick={() => setFilterSources([])}
            >
              {t("settings.memorySourceAll")}
            </Button>
            {MEMORY_SOURCE_FILTERS.map((source) => {
              const selected = filterSources.includes(source)
              return (
                <Button
                  key={source}
                  type="button"
                  variant="outline"
                  size="sm"
                  className={cn(
                    "h-9 rounded-lg px-2 text-xs",
                    selected
                      ? "border-primary bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary"
                      : "border-transparent text-muted-foreground hover:bg-secondary/40 hover:text-foreground",
                  )}
                  onClick={() => toggleSourceFilter(source)}
                >
                  {t(`settings.memorySource_${source}`)}
                </Button>
              )
            })}
          </div>
        </div>

        {/* Scope filter */}
        <div className="flex items-center gap-2 mb-3">
          <div className="flex gap-1">
            {(["all", "global", "agent"] as const).map((scope) => (
              <Button
                key={scope}
                variant="ghost"
                size="sm"
                onClick={() => setFilterScope(scope)}
                className={cn(
                  "h-auto rounded-md px-2.5 py-1 text-xs",
                  filterScope === scope
                    ? "bg-secondary text-foreground font-medium hover:bg-secondary hover:text-foreground"
                    : "text-muted-foreground hover:text-foreground hover:bg-secondary/40",
                )}
              >
                {scope === "all"
                  ? t("settings.memoryScopeAll")
                  : scope === "global"
                    ? t("settings.memoryScopeGlobal")
                    : t("settings.memoryScopeAgent")}
              </Button>
            ))}
          </div>
          {/* Agent picker (standalone mode, agent scope selected) */}
          {!isAgentMode && filterScope === "agent" && agents.length > 0 && (
            <Select
              value={selectedAgentId ?? ""}
              onValueChange={(v) => setSelectedAgentId(v || null)}
            >
              <SelectTrigger className="w-40 h-7 text-xs">
                {selectedAgent ? (
                  <AgentSelectDisplay agent={selectedAgent} size="xs" />
                ) : (
                  <SelectValue placeholder={t("settings.memorySelectAgent")} />
                )}
              </SelectTrigger>
              <SelectContent>
                {agents.map((a) => (
                  <SelectItem key={a.id} value={a.id} className="text-xs" textValue={a.name}>
                    <AgentSelectDisplay agent={a} size="xs" />
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 gap-1 px-2 text-xs"
            onClick={saveFilterPreset}
          >
            <BookmarkPlus className="h-3.5 w-3.5" />
            {t("settings.memoryFilterPresetSave")}
          </Button>
        </div>
        {!isAgentMode && filterScope === "agent" && agentListError && (
          <div className="mb-3 flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-muted-foreground">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-300" />
            <div className="min-w-0">
              <div className="font-medium text-foreground">{agentListError.title}</div>
              {agentListError.description && (
                <div className="mt-0.5 break-words">{agentListError.description}</div>
              )}
            </div>
          </div>
        )}
        {filterPresets.length > 0 && (
          <div className="mb-3 flex flex-wrap items-center gap-1.5 text-xs">
            <span className="text-muted-foreground">{t("settings.memoryFilterPresets")}</span>
            {filterPresets.map((preset) => {
              const label = memoryListPresetLabel(preset)
              const active = preset.id === currentPresetId
              return (
                <span
                  key={preset.id}
                  className={cn(
                    "inline-flex max-w-full items-center rounded-md border border-border/70",
                    active ? "bg-primary/10 text-foreground" : "bg-background",
                  )}
                >
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-6 min-w-0 max-w-[260px] justify-start truncate px-2 text-xs"
                    data-ha-title-tip={label}
                    onClick={() => applyFilterPreset(preset)}
                  >
                    <span className="truncate">{label}</span>
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    aria-label={t("settings.memoryFilterPresetRemove")}
                    className="h-6 w-6 shrink-0 text-muted-foreground hover:text-foreground"
                    onClick={() => removeFilterPreset(preset.id)}
                  >
                    <X className="h-3 w-3" />
                  </Button>
                </span>
              )
            })}
          </div>
        )}

        {/* Stats + Batch actions */}
        <div className="flex items-center justify-between text-xs text-muted-foreground mb-3">
          <div className="flex items-center gap-2">
            {memories.length > 0 && (
              <IconTip
                label={selectedIds.size === memories.length
                  ? t("settings.memoryDeselectAll")
                  : t("settings.memorySelectAll")}
              >
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={toggleSelectAll}
                  className="h-5 w-5 hover:bg-transparent hover:text-foreground"
                  aria-label={selectedIds.size === memories.length
                    ? t("settings.memoryDeselectAll")
                    : t("settings.memorySelectAll")}
                >
                  {selectedIds.size === memories.length ? (
                    <CheckSquare className="h-3.5 w-3.5" />
                  ) : (
                    <Square className="h-3.5 w-3.5" />
                  )}
                </Button>
              </IconTip>
            )}
            <span>{t("settings.memoryCount", { count: totalCount })}</span>
            {embeddingEnabled && (
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
                onClick={() => setConfirmBatchDeleteOpen(true)}
              >
                {t("settings.memoryDeleteBatch", { count: selectedIds.size })}
              </Button>
              {embeddingEnabled && (
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
        <div className="space-y-1.5">
          {loading && memories.length === 0 ? (
            <div className="text-sm text-muted-foreground py-8 text-center">
              {t("settings.loading")}
            </div>
          ) : memories.length === 0 ? (
            <div className="text-sm text-muted-foreground py-8 text-center">
              {hasActiveFilter ? t("settings.memoryNoResults") : t("settings.memoryEmpty")}
            </div>
          ) : (
            memories.map((mem) => {
              const Icon = MEMORY_TYPE_ICONS[mem.memoryType] || User
              const isSelected = selectedIds.has(mem.id)
              const scopeLabel =
                mem.scope.kind === "global"
                  ? "Global"
                  : `Agent: ${(mem.scope as { kind: "agent"; id: string }).id}`
              const sourceKey = sourceLabelKey(mem.source)
              const sourceLabel = sourceKey ? t(`settings.memorySource_${sourceKey}`) : mem.source
              const searchMatches = searchQuery.trim()
                ? explainMemorySearchMatch(mem, searchQuery)
                : []
              return (
                <div
                  key={mem.id}
                  className={cn(
                    "group flex items-start gap-3 px-3 py-2.5 rounded-lg hover:bg-secondary/40 cursor-pointer transition-colors",
                    isSelected && "bg-primary/5 border border-primary/20",
                  )}
                  onClick={() => startEdit(mem)}
                >
                  <IconTip label={t("common.select")}>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={(e) => {
                        e.stopPropagation()
                        toggleSelect(mem.id)
                      }}
                      className="h-5 w-5 mt-0.5 shrink-0 text-muted-foreground hover:bg-transparent hover:text-foreground"
                      aria-label={t("common.select")}
                      aria-pressed={isSelected}
                    >
                      {isSelected ? (
                        <CheckSquare className="h-4 w-4 text-primary" />
                      ) : (
                        <Square className="h-4 w-4 opacity-0 group-hover:opacity-100 transition-opacity" />
                      )}
                    </Button>
                  </IconTip>
                  <IconTip label={mem.pinned ? t("settings.memoryUnpin") : t("settings.memoryPin")}>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={(e) => {
                        e.stopPropagation()
                        handleTogglePin(mem.id, !mem.pinned)
                      }}
                      className={cn(
                        "h-5 w-5 mt-0.5 shrink-0 hover:bg-transparent",
                        mem.pinned
                          ? "text-amber-500 hover:text-amber-500"
                          : "text-muted-foreground/30 opacity-0 group-hover:opacity-100 transition-opacity hover:text-amber-500",
                      )}
                      aria-label={mem.pinned ? t("settings.memoryUnpin") : t("settings.memoryPin")}
                      aria-pressed={mem.pinned}
                    >
                      <Pin className="h-3.5 w-3.5" />
                    </Button>
                  </IconTip>
                  <Icon className="h-4 w-4 text-muted-foreground mt-0.5 shrink-0" />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm line-clamp-2">{mem.content}</div>
                    <div className="flex items-center gap-2 mt-1 text-xs text-muted-foreground">
                      <span>{t(`settings.memoryType_${mem.memoryType}`)}</span>
                      <span>·</span>
                      <span>{scopeLabel}</span>
                      <span>·</span>
                      <span>{sourceLabel}</span>
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
                    {searchMatches.length > 0 && (
                      <div className="mt-1 flex flex-wrap items-center gap-1 text-[10px]">
                        <span className="text-muted-foreground">
                          {t("settings.memorySearchMatchedBy")}
                        </span>
                        {searchMatches.map((match) => (
                          <span
                            key={match.kind}
                            className="rounded bg-primary/10 px-1.5 py-0.5 text-primary"
                          >
                            {t(`settings.memorySearchMatch_${match.kind}`)}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                  <IconTip label={t("common.delete")}>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={(e) => {
                        e.stopPropagation()
                        handleDelete(mem.id)
                      }}
                      className="h-7 w-7 opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-destructive transition-opacity"
                      aria-label={t("common.delete")}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </IconTip>
                  <ChevronRight className="h-4 w-4 text-muted-foreground/30 mt-0.5 shrink-0" />
                </div>
              )
            })
          )}
        </div>
      </div>
      <ImportFromAIDialog
        open={importFromAIOpen}
        onOpenChange={setImportFromAIOpen}
        onImported={loadMemories}
        memoryEnabled={memoryEnabled}
      />

      <Dialog
        open={importPreviewOpen}
        onOpenChange={(open) => {
          if (open) setImportPreviewOpen(true)
          else closeImportPreview()
        }}
      >
        <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <FileSearch className="h-4 w-4 text-primary" />
              {t("settings.memoryImportPreviewTitle")}
            </DialogTitle>
            <DialogDescription>
              {importPreviewFilename || t("settings.memoryImport", "Import")}
            </DialogDescription>
          </DialogHeader>

          {importPreview && (
            <div className="space-y-3">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="rounded border bg-background px-2 py-0.5 text-xs text-muted-foreground">
                  {t("settings.memoryImportPreviewCount", "{{count}} memories", {
                    count: importPreview.candidateCount,
                  })}
                </span>
                <span
                  className={cn(
                    "rounded border px-2 py-0.5 text-xs",
                    importPreview.valid
                      ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                      : "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
                  )}
                >
                  {memoryImportPreviewStatusLabel(t, importPreview)}
                </span>
                {importPreview.dedupChecked && (
                  <>
                    <span className="rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-xs text-emerald-700 dark:text-emerald-300">
                      {t("settings.memoryImportPreviewLikelyImport", "{{count}} will import", {
                        count: importPreviewLikelyImportCount,
                      })}
                    </span>
                    {(importPreview.likelyDuplicateCount ?? 0) > 0 && (
                      <span className="rounded border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-xs text-amber-700 dark:text-amber-300">
                        {t(
                          "settings.memoryImportPreviewLikelyDuplicate",
                          "{{count}} duplicates",
                          { count: importPreview.likelyDuplicateCount },
                        )}
                      </span>
                    )}
                    {(importPreview.likelyMergeCount ?? 0) > 0 && (
                      <span className="rounded border bg-background px-2 py-0.5 text-xs text-muted-foreground">
                        {t("settings.memoryImportPreviewLikelyMerge", "{{count}} may merge", {
                          count: importPreview.likelyMergeCount,
                        })}
                      </span>
                    )}
                  </>
                )}
              </div>

              <div className="flex flex-wrap gap-1.5">
                {importPreviewTypeEntries.map(([type, count]) => (
                  <span key={type} className="rounded border bg-background px-2 py-0.5 text-xs">
                    {t(`settings.memoryType_${type}`)} · {count}
                  </span>
                ))}
                {importPreviewScopeEntries.map(([scope, count]) => (
                  <span
                    key={scope}
                    className="rounded border bg-background px-2 py-0.5 text-xs text-muted-foreground"
                  >
                    {formatMemoryImportScopeSummaryKey(t, scope)} · {count}
                  </span>
                ))}
              </div>

              {importPreviewIssueMessages.length > 0 && (
                <div className="space-y-1 rounded border border-amber-500/30 bg-amber-500/5 px-2 py-1.5 text-xs text-amber-700 dark:text-amber-300">
                  <div className="font-medium">
                    {t("settings.memoryImportPreviewReport.issues", "Issues")}
                  </div>
                  {importPreviewIssueMessages.map((message, index) => (
                    <div key={`${index}:${message}`} className="leading-relaxed">
                      {message}
                    </div>
                  ))}
                </div>
              )}

              <div className="space-y-2">
                {importPreviewSampleWindowLabel && (
                  <div className="text-xs text-muted-foreground">
                    {importPreviewSampleWindowLabel}
                  </div>
                )}
                {importPreviewVisibleSamples.map((sample, index) => (
                  <div
                    key={`${sample.contentPreview}-${index}`}
                    className="rounded border bg-background p-2"
                  >
                    <div className="mb-1 flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
                      <span>{t(`settings.memoryType_${sample.memoryType}`)}</span>
                      <span>·</span>
                      <span>{formatMemoryImportScopeLabel(t, sample.scope)}</span>
                      {sample.dedupStatus && (
                        <span className={sampleDedupClass(sample.dedupStatus)}>
                          {t(`settings.memoryImportPreviewSample_${sample.dedupStatus}`)}
                        </span>
                      )}
                      {sample.tags.slice(0, 3).map((tag) => (
                        <span key={tag} className="rounded bg-muted px-1.5 py-0.5">
                          {tag}
                        </span>
                      ))}
                    </div>
                    <p className="text-xs leading-relaxed text-foreground">
                      {sample.contentPreview}
                    </p>
                    {sample.dedupExistingPreview && (
                      <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
                        {t("settings.memoryImportPreviewExisting", {
                          id: sample.dedupExistingId ?? "",
                        })}
                        : {sample.dedupExistingPreview}
                      </p>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          <DialogFooter>
            {importPreview && (
              <Button
                type="button"
                variant="outline"
                onClick={() =>
                  void copyMemoryImportPreviewDiagnostics(t, importPreview, importPreviewFilename)
                }
                disabled={importApplyLoading}
                className="mr-auto gap-1.5"
              >
                <Copy className="h-3.5 w-3.5" />
                {t("chat.copy")}
              </Button>
            )}
            <Button variant="ghost" onClick={closeImportPreview} disabled={importApplyLoading}>
              {t("common.cancel")}
            </Button>
            <Button
              onClick={() => void handleImportPreviewApply()}
              disabled={!importPreview?.valid || importPreviewLoading || importApplyLoading}
              className="gap-1.5"
            >
              {importApplyLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {t("settings.memoryImportConfirmBtn")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={backupPassphraseDialogMode !== null}
        onOpenChange={(open) => {
          if (!open) closeBackupPassphraseDialog()
        }}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>
              {backupPassphraseDialogMode === "export"
                ? t("settings.memoryBackupEncryptedExportTitle", "Encrypt memory backup")
                : t("settings.memoryBackupEncryptedUnlockTitle", "Unlock encrypted backup")}
            </DialogTitle>
            <DialogDescription>
              {backupPassphraseDialogMode === "export"
                ? t(
                    "settings.memoryBackupEncryptedExportDesc",
                    "Choose a passphrase you can remember. It is required to restore this backup.",
                  )
                : t(
                    "settings.memoryBackupEncryptedUnlockDesc",
                    "Enter the passphrase for this backup before previewing its contents.",
                  )}
            </DialogDescription>
          </DialogHeader>

          <form
            className="space-y-3"
            onSubmit={(event) => {
              event.preventDefault()
              void submitBackupPassphraseDialog()
            }}
          >
            <div className="space-y-1.5">
              <Input
                autoFocus
                type="password"
                value={backupPassphrase}
                onChange={(event) => setBackupPassphrase(event.target.value)}
                placeholder={t("settings.memoryBackupEncryptedPassphrase", "Backup passphrase")}
                autoComplete="new-password"
              />
              {backupPassphraseDialogMode === "export" && (
                <div className="flex items-center gap-2">
                  <div className="h-1.5 flex-1 overflow-hidden rounded bg-muted">
                    <div
                      className={cn("h-full transition-all", backupPassphraseMeterClass)}
                      style={{ width: `${Math.max(backupPassphraseScore, 1) * 20}%` }}
                    />
                  </div>
                  <span className="w-14 text-right text-[11px] text-muted-foreground">
                    {backupPassphraseLabel}
                  </span>
                </div>
              )}
              {backupPassphraseHint && (
                <div className="text-[11px] text-muted-foreground">{backupPassphraseHint}</div>
              )}
            </div>
            {backupPassphraseDialogMode === "export" && (
              <Input
                type="password"
                value={backupPassphraseConfirm}
                onChange={(event) => setBackupPassphraseConfirm(event.target.value)}
                placeholder={t(
                  "settings.memoryBackupEncryptedPassphraseConfirm",
                  "Confirm passphrase",
                )}
                autoComplete="new-password"
              />
            )}

            <DialogFooter>
              <Button type="button" variant="ghost" onClick={closeBackupPassphraseDialog}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button
                type="submit"
                disabled={backupLoading || backupPreviewLoading}
                className="gap-1.5"
              >
                {(backupLoading || backupPreviewLoading) && (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                )}
                {backupPassphraseDialogMode === "export"
                  ? t("settings.memoryBackupExportEncrypted", "Export encrypted backup")
                  : t("settings.memoryBackupEncryptedUnlock", "Unlock backup")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={backupPreviewOpen} onOpenChange={setBackupPreviewOpen}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t("settings.memoryBackupPreview", "Preview backup import")}</DialogTitle>
            <DialogDescription>
              {backupPreview?.valid
                ? t(
                    "settings.memoryBackupPreviewDesc",
                    "Review what this backup contains before any restore work is allowed.",
                  )
                : t(
                    "settings.memoryBackupPreviewInvalidDesc",
                    "This file is not a compatible Hope Agent memory backup.",
                  )}
            </DialogDescription>
          </DialogHeader>

          {backupPreview && (
            <div className="space-y-4 text-sm">
              {!memoryEnabled && backupPreview.valid && (
                <div className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-muted-foreground">
                  <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-300" />
                  <span>{t("settings.memoryOffBackupRestoreNotice")}</span>
                </div>
              )}

              <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
                <div className="rounded border border-border/60 p-2">
                  <div className="text-[11px] text-muted-foreground">
                    {t("settings.memoryBackupPreviewMemories", "Memories")}
                  </div>
                  <div className="font-semibold tabular-nums">
                    {backupPreview.legacyMemoryCount}
                  </div>
                </div>
                <div className="rounded border border-border/60 p-2">
                  <div className="text-[11px] text-muted-foreground">
                    {t("settings.memoryBackupPreviewNew", "New candidates")}
                  </div>
                  <div className="font-semibold tabular-nums">
                    {backupPreview.legacyImportCandidates}
                  </div>
                </div>
                <div className="rounded border border-border/60 p-2">
                  <div className="text-[11px] text-muted-foreground">
                    {t("settings.memoryBackupPreviewClaims", "Claims")}
                  </div>
                  <div className="font-semibold tabular-nums">{backupPreview.claimCount}</div>
                </div>
                <div className="rounded border border-border/60 p-2">
                  <div className="text-[11px] text-muted-foreground">
                    {t("settings.memoryBackupPreviewProfiles", "Profiles")}
                  </div>
                  <div className="font-semibold tabular-nums">
                    {backupPreview.profileSnapshotCount}
                  </div>
                </div>
                <div className="rounded border border-border/60 p-2">
                  <div className="text-[11px] text-muted-foreground">
                    {t("settings.memoryBackupPreviewEpisodes", "Episodes")}
                  </div>
                  <div className="font-semibold tabular-nums">
                    {backupPreview.episodeCount ?? 0}
                  </div>
                </div>
                <div className="rounded border border-border/60 p-2">
                  <div className="text-[11px] text-muted-foreground">
                    {t("settings.memoryBackupPreviewProcedures", "Procedures")}
                  </div>
                  <div className="font-semibold tabular-nums">
                    {backupPreview.procedureCount ?? 0}
                  </div>
                </div>
              </div>

              {backupPreview.valid && (
                <div className="rounded border border-border/60 p-3 text-xs text-muted-foreground">
                  <div>
                    {t("settings.memoryBackupPreviewExported", "Exported")}:{" "}
                    <span className="text-foreground">
                      {backupPreview.exportedAt ?? t("common.unknown", "Unknown")}
                    </span>
                  </div>
                  <div>
                    {t("settings.memoryBackupPreviewMatches", "Already present")}:{" "}
                    <span className="text-foreground">
                      {formatMemoryBackupAlreadyPresentSummary(backupPreview, t)}
                    </span>
                  </div>
                  {(backupPreview.legacyHistoryCount ?? 0) > 0 && (
                    <div>
                      {t("settings.memoryBackupPreviewHistory", "Memory history")}:{" "}
                      <span className="text-foreground">
                        {formatMemoryBackupHistorySummary(
                          backupPreview.legacyHistoryRestorable,
                          backupPreview.legacyHistoryCount,
                          backupPreview.legacyHistorySkippedUnmapped,
                          t,
                        )}
                      </span>
                    </div>
                  )}
                  {backupClaimPlan && backupClaimPlan.total > 0 && (
                    <div>
                      {t("settings.memoryBackupPreviewClaimPlan", "Structured claim plan")}:{" "}
                      <span className="text-foreground">
                        {formatMemoryBackupClaimPlanSummary(backupClaimPlan, t)}
                      </span>
                      {backupClaimPlan.conflictExamples.length > 0 && (
                        <div className="mt-2 space-y-1 rounded border border-amber-500/30 bg-amber-500/5 p-2 text-[11px] text-amber-800 dark:text-amber-200">
                          <div className="font-medium">
                            {t(
                              "settings.memoryBackupClaimConflictExamples",
                              "Conflicts will go to Review Inbox",
                            )}
                          </div>
                          {backupClaimPlan.conflictExamples.map((example) => (
                            <div
                              key={`${example.incomingClaimId}:${example.existingClaimId}`}
                              className="space-y-0.5"
                            >
                              <div className="truncate">
                                {formatMemoryBackupClaimConflictHeader(example, t)}
                              </div>
                              <div className="text-amber-700 dark:text-amber-300">
                                {t("settings.memoryBackupIncoming", "Incoming")}:{" "}
                                {example.incomingObject || example.incomingContent}
                              </div>
                              <div className="text-amber-700 dark:text-amber-300">
                                {t("settings.memoryBackupExisting", "Existing")}:{" "}
                                {example.existingObject || example.existingContent}
                              </div>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  )}
                  {backupProfilePlan && backupProfilePlan.total > 0 && (
                    <div>
                      {t("settings.memoryBackupPreviewProfilePlan", "Profile snapshot plan")}:{" "}
                      <span className="text-foreground">
                        {formatMemoryBackupProfilePlanSummary(backupProfilePlan, t)}
                      </span>
                    </div>
                  )}
                  {((backupPreview.episodeCount ?? 0) > 0 ||
                    (backupPreview.procedureCount ?? 0) > 0 ||
                    (backupPreview.experienceHistoryCount ?? 0) > 0) && (
                    <div>
                      {t("settings.memoryBackupPreviewExperiencePlan", "Experience plan")}:{" "}
                      <span className="text-foreground">
                        {formatMemoryBackupExperiencePlanSummary(backupPreview, t)}
                      </span>
                    </div>
                  )}
                  {hasBackupProfileConflicts && (
                    <div className="mt-2 flex items-start justify-between gap-3 rounded border border-amber-500/30 bg-amber-500/5 p-2 text-amber-800 dark:text-amber-200">
                      <div>
                        <div className="font-medium">
                          {t(
                            "settings.memoryBackupRestoreProfileConflicts",
                            "Replace existing profile snapshots",
                          )}
                        </div>
                        <div className="mt-0.5 text-[11px] text-amber-700 dark:text-amber-300">
                          {t(
                            "settings.memoryBackupRestoreProfileConflictsDesc",
                            "Off by default. Turn on only if this backup should become the latest profile for matching scopes.",
                          )}
                        </div>
                      </div>
                      <Switch
                        checked={backupRestoreProfileConflicts}
                        onCheckedChange={setBackupRestoreProfileConflicts}
                        aria-label={t(
                          "settings.memoryBackupRestoreProfileConflicts",
                          "Replace existing profile snapshots",
                        )}
                      />
                    </div>
                  )}
                  {backupPreview.unsupportedSections.length > 0 && (
                    <div>
                      {t("settings.memoryBackupPreviewUnsupported", "Preview-only sections")}:{" "}
                      <span className="text-foreground">
                        {backupPreview.unsupportedSections.join(", ")}
                      </span>
                    </div>
                  )}
                  {backupPreview.attachmentRefCount > 0 && (
                    <div>
                      {t("settings.memoryBackupPreviewAttachments", "Attachments")}:{" "}
                      <span className="text-foreground">
                        {formatMemoryBackupAttachmentSummary(backupPreview, t)}
                      </span>
                    </div>
                  )}
                </div>
              )}

              {backupRestorePlan.length > 0 && (
                <div className="space-y-2 rounded border border-border/60 p-3 text-xs">
                  <div className="flex items-center gap-2">
                    <Package className="h-3.5 w-3.5 text-primary" />
                    <span className="font-medium">
                      {t("settings.memoryBackupRestoreDecisionPlan", "Restore decision plan")}
                    </span>
                  </div>
                  {backupRestorePlanSummary && (
                    <div
                      className={cn(
                        "rounded border px-2 py-1.5",
                        backupPlanToneClass(backupRestorePlanSummary.tone),
                      )}
                    >
                      <div className="font-medium">
                        {formatMemoryBackupRestorePlanSummaryTitle(backupRestorePlanSummary, t)}
                      </div>
                      <div className="mt-0.5 text-[11px] leading-relaxed">
                        {formatMemoryBackupRestorePlanSummaryDetail(backupRestorePlanSummary, t)}
                      </div>
                      <div className="mt-1 text-[11px] leading-relaxed">
                        {formatMemoryBackupRestorePlanSummaryNextStep(backupRestorePlanSummary, t)}
                      </div>
                    </div>
                  )}
                  <div className="space-y-1">
                    {backupRestorePlan.map((row) => (
                      <div
                        key={row.id}
                        className="flex min-w-0 items-start gap-2 rounded border border-border/50 bg-background/70 px-2 py-1.5"
                      >
                        <span
                          className={cn(
                            "mt-0.5 inline-flex shrink-0 items-center rounded-full border px-1.5 py-0.5 text-[10px] font-medium",
                            backupPlanToneClass(row.tone),
                          )}
                        >
                          {formatMemoryBackupRestorePlanActionLabel(row.action, t)}
                        </span>
                        <div className="min-w-0 flex-1">
                          <div className="flex min-w-0 items-center gap-1.5">
                            <span className="truncate font-medium text-foreground">
                              {formatMemoryBackupRestorePlanRowLabel(row, t)}
                            </span>
                            <span className="shrink-0 tabular-nums text-muted-foreground">
                              {row.count}
                            </span>
                          </div>
                          <div className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
                            {formatMemoryBackupRestorePlanRowDetail(row, t)}
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {backupPreview.issues.length > 0 && (
                <div className="space-y-2">
                  {backupPreview.issues.slice(0, 5).map((issue) => (
                    <div
                      key={`${issue.code}:${issue.message}`}
                      className={cn(
                        "rounded border px-2 py-1.5 text-xs",
                        issue.severity === "error"
                          ? "border-destructive/30 bg-destructive/5 text-destructive"
                          : issue.severity === "warning"
                            ? "border-amber-500/30 bg-amber-500/5 text-amber-700 dark:text-amber-300"
                            : "border-border/60 bg-muted/30 text-muted-foreground",
                      )}
                    >
                      {formatMemoryBackupPreviewIssueMessage(issue, t)}
                    </div>
                  ))}
                </div>
              )}

              {backupPreview.nextSteps.length > 0 && (
                <div className="space-y-1 text-xs">
                  <div className="font-medium">
                    {t("settings.memoryBackupPreviewNextSteps", "Next steps")}
                  </div>
                  {backupPreview.nextSteps.map((step) => (
                    <div key={step} className="text-muted-foreground">
                      {formatMemoryBackupPreviewNextStep(step, t)}
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}

          <DialogFooter>
            {backupPreview && (
              <Button
                type="button"
                variant="outline"
                onClick={() =>
                  void copyMemoryBackupPreviewDiagnostics(t, backupPreview, {
                    allowProfileScopeConflicts: backupRestoreProfileConflicts,
                  })
                }
                disabled={backupRestoreLoading}
                className="mr-auto gap-1.5"
              >
                <Copy className="h-3.5 w-3.5" />
                {t("chat.copy")}
              </Button>
            )}
            {backupPreview?.valid && backupPreview.legacyImportCandidates > 0 && (
              <Button
                onClick={handleBackupRestoreLegacy}
                disabled={backupRestoreLoading}
                className="gap-1.5"
              >
                {backupRestoreLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                {t("settings.memoryBackupRestoreLegacy", "Restore missing memories")}
              </Button>
            )}
            {backupPreview?.valid &&
              hasBackupStructuredRestoreCandidates && (
                <Button
                  variant="secondary"
                  onClick={handleBackupRestoreStructured}
                  disabled={backupRestoreLoading}
                  className="gap-1.5"
                >
                  {backupRestoreLoading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  {t("settings.memoryBackupRestoreStructured", "Restore structured memory")}
                </Button>
              )}
            {hasBackupReviewInboxEntry && (
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setBackupPreviewOpen(false)
                  onOpenClaims?.({ statusFilter: "needs_review" })
                }}
              >
                {backupReviewInboxHintCount > 0
                  ? t("settings.memoryBackupOpenReviewInboxAfterRestore", {
                      defaultValue: "Review {{count}} restored item(s)",
                      count: backupReviewInboxHintCount,
                    })
                  : t("settings.memoryBackupOpenReviewInbox", "Open Review Inbox")}
              </Button>
            )}
            <Button variant="ghost" onClick={() => setBackupPreviewOpen(false)}>
              {t("common.close", "Close")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <AlertDialog open={confirmBatchDeleteOpen} onOpenChange={setConfirmBatchDeleteOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.memoryDeleteBatch", { count: selectedIds.size })}
            </AlertDialogTitle>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => {
                void handleDeleteBatch()
                setConfirmBatchDeleteOpen(false)
              }}
            >
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

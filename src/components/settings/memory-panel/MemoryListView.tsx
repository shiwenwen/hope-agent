import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
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
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Plus,
  Trash2,
  Search,
  Upload,
  ChevronRight,
  X,
  FileDown,
  Zap,
  CheckSquare,
  Square,
  User,
  Pin,
  Sparkles,
} from "lucide-react"
import { MEMORY_TYPES, MEMORY_TYPE_ICONS } from "./types"
import ExtractConfig from "./ExtractConfig"
import BudgetConfig from "./BudgetConfig"
import CoreMemoryEditor from "./CoreMemoryEditor"
import ImportFromAIDialog from "./ImportFromAIDialog"
import type { useMemoryData } from "./useMemoryData"

type MemoryData = ReturnType<typeof useMemoryData>

interface MemoryListViewProps {
  data: MemoryData
  isAgentMode: boolean
  compact?: boolean
}

export default function MemoryListView({ data, isAgentMode, compact }: MemoryListViewProps) {
  const { t } = useTranslation()
  const [confirmBatchDeleteOpen, setConfirmBatchDeleteOpen] = useState(false)

  const {
    setView,
    memories,
    totalCount,
    loading,
    searchQuery,
    setSearchQuery,
    filterType,
    setFilterType,
    filterScope,
    setFilterScope,
    agents,
    selectedAgentId,
    setSelectedAgentId,
    selectedIds,
    batchLoading,
    embeddingConfig,
    stats,
    handleExport,
    handleImport,
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

  return (
    <div className="flex-1 overflow-y-auto p-6">
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
        <p className="text-xs text-muted-foreground mb-4">{t("settings.memoryDesc")}</p>

        {/* Global Core Memory editor (standalone mode only) */}
        {!isAgentMode && <CoreMemoryEditor scope="global" />}

        {/* Auto-extract settings */}
        <ExtractConfig data={data} isAgentMode={isAgentMode} />

        {/* Memory section budget (global defaults). Agent tab has override UI. */}
        {!isAgentMode && <BudgetConfig />}

        {/* Stats bar */}
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
            {embeddingConfig.enabled && stats.total > 0 && (
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
        <div className="flex items-center gap-2 mb-3">
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
        <div className="flex items-center justify-between text-xs text-muted-foreground mb-3">
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
                onClick={() => setConfirmBatchDeleteOpen(true)}
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
        <div className="space-y-1.5">
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
                    mem.pinned && "border-l-2 border-l-amber-400",
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
                  <IconTip label={mem.pinned ? t("settings.memoryUnpin") : t("settings.memoryPin")}>
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        handleTogglePin(mem.id, !mem.pinned)
                      }}
                      className={cn(
                        "mt-0.5 shrink-0 p-0 transition-colors",
                        mem.pinned
                          ? "text-amber-500"
                          : "text-muted-foreground/30 hover:text-amber-500 opacity-0 group-hover:opacity-100 transition-opacity",
                      )}
                    >
                      <Pin className="h-3.5 w-3.5" />
                    </button>
                  </IconTip>
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
      <ImportFromAIDialog
        open={importFromAIOpen}
        onOpenChange={setImportFromAIOpen}
        onImported={loadMemories}
      />

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

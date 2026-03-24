import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import {
  ArrowLeft,
  User,
} from "lucide-react"
import { MEMORY_TYPES, MEMORY_TYPE_ICONS } from "./types"
import type { useMemoryData } from "./useMemoryData"

type MemoryData = ReturnType<typeof useMemoryData>

interface MemoryFormViewProps {
  data: MemoryData
  isAgentMode: boolean
}

export default function MemoryFormView({ data, isAgentMode }: MemoryFormViewProps) {
  const { t } = useTranslation()

  const {
    view, setView,
    editingMemory, setEditingMemory,
    formContent, setFormContent,
    formType, setFormType,
    formTags, setFormTags,
    formScope, setFormScope,
    dedupSimilar,
    dedupPendingEntry,
    handleAdd,
    handleUpdate,
    handleDedupConfirm,
    handleDedupCancel,
    handleDedupUpdate,
  } = data

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

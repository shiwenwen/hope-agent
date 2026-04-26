import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Brain, CheckCircle2, Loader2, Settings, Zap } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { useMemoryData } from "./useMemoryData"
import LocalEmbeddingAssistantCard from "./LocalEmbeddingAssistantCard"
import type { MemoryEmbeddingSetDefaultResult } from "./types"
import {
  embeddingProviderLabel,
  openEmbeddingModelSettings,
} from "@/types/embedding-models"

type MemoryData = ReturnType<typeof useMemoryData>

interface EmbeddingModelSectionProps {
  data: MemoryData
}

export default function EmbeddingModelSection({ data }: EmbeddingModelSectionProps) {
  const { t } = useTranslation()
  const {
    totalCount,
    embeddingModels,
    memoryEmbeddingState,
    setMemoryEmbeddingState,
    reloadEmbeddingConfig,
    batchLoading,
    handleReembedAll,
  } = data
  const [pendingModelId, setPendingModelId] = useState<string | null>(null)
  const [switching, setSwitching] = useState(false)

  const currentId = memoryEmbeddingState.selection.enabled
    ? memoryEmbeddingState.selection.modelConfigId
    : undefined
  const pendingModel = useMemo(
    () => embeddingModels.find((model) => model.id === pendingModelId) ?? null,
    [embeddingModels, pendingModelId],
  )

  async function confirmSwitchDefault() {
    if (!pendingModelId) return
    setSwitching(true)
    try {
      const result = await getTransport().call<MemoryEmbeddingSetDefaultResult>(
        "memory_embedding_set_default",
        { modelConfigId: pendingModelId, reembed: true },
      )
      setMemoryEmbeddingState(result.state)
      await reloadEmbeddingConfig()
      if (result.reembedError) {
        toast.warning(t("settings.embeddingModels.reembedFailed"))
      } else {
        toast.success(t("settings.embeddingModels.defaultSet"))
      }
    } catch (e) {
      logger.error("settings", "EmbeddingModelSection::confirmSwitchDefault", "Failed to switch", e)
      toast.error(String(e))
    } finally {
      setSwitching(false)
      setPendingModelId(null)
    }
  }

  return (
    <>
      <LocalEmbeddingAssistantCard
        onActivated={(result) => {
          setMemoryEmbeddingState(result.state)
          void reloadEmbeddingConfig()
          if (result.reembedError) {
            toast.warning(t("settings.embeddingModels.reembedFailed"))
          } else {
            toast.success(t("settings.localEmbedding.activated"))
          }
        }}
      />

      <div className="rounded-lg border border-border bg-card p-4 space-y-4">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <div className="flex items-center gap-2 text-sm font-medium">
              <Brain className="h-4 w-4 text-primary" />
              {t("settings.embeddingModels.memoryDefault")}
            </div>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("settings.embeddingModels.memoryDefaultDesc")}
            </p>
          </div>
          <Button variant="outline" size="sm" onClick={openEmbeddingModelSettings}>
            <Settings className="mr-1.5 h-3.5 w-3.5" />
            {t("settings.embeddingModels.goConfig")}
          </Button>
        </div>

        {embeddingModels.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border bg-secondary/30 p-4 text-sm text-muted-foreground">
            {t("settings.embeddingModels.emptyMemory")}
          </div>
        ) : (
          <div className="space-y-3">
            <Select
              value={currentId ?? ""}
              onValueChange={(value) => {
                if (value && value !== currentId) setPendingModelId(value)
              }}
            >
              <SelectTrigger className="w-full">
                <SelectValue placeholder={t("settings.embeddingModels.selectPlaceholder")} />
              </SelectTrigger>
              <SelectContent>
                {embeddingModels.map((model) => (
                  <SelectItem key={model.id} value={model.id}>
                    {model.name} · {embeddingProviderLabel(model)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            {memoryEmbeddingState.currentModel && (
              <div className="rounded-lg border border-border/70 bg-secondary/25 p-3">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium">
                    {memoryEmbeddingState.currentModel.name}
                  </span>
                  <span className="rounded border border-emerald-500/25 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                    {t("settings.embeddingModels.memoryActive")}
                  </span>
                  {memoryEmbeddingState.needsReembed && (
                    <span className="rounded border border-amber-500/25 bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-300">
                      {t("settings.embeddingModels.needsReembed")}
                    </span>
                  )}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {embeddingProviderLabel(memoryEmbeddingState.currentModel)} ·{" "}
                  {memoryEmbeddingState.currentModel.apiModel}
                  {memoryEmbeddingState.currentModel.apiDimensions
                    ? ` · ${memoryEmbeddingState.currentModel.apiDimensions}d`
                    : ""}
                </div>
              </div>
            )}
          </div>
        )}

        {memoryEmbeddingState.selection.enabled && totalCount > 0 && (
          <div className="flex items-center justify-between border-t border-border/60 pt-4">
            <div>
              <div className="text-sm font-medium">{t("settings.memoryReembedAll")}</div>
              <div className="text-xs text-muted-foreground">
                {t("settings.memoryCount", { count: totalCount })}
              </div>
            </div>
            <Button variant="outline" size="sm" disabled={batchLoading} onClick={handleReembedAll}>
              {batchLoading ? (
                <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
              ) : memoryEmbeddingState.needsReembed ? (
                <Zap className="mr-1.5 h-3.5 w-3.5" />
              ) : (
                <CheckCircle2 className="mr-1.5 h-3.5 w-3.5" />
              )}
              {t("settings.memoryReembedAll")}
            </Button>
          </div>
        )}
      </div>

      <AlertDialog open={!!pendingModel} onOpenChange={(open) => !open && setPendingModelId(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.embeddingModels.confirmSwitchTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.embeddingModels.confirmSwitchDesc", {
                model: pendingModel?.name ?? "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={switching}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction disabled={switching} onClick={() => void confirmSwitchDefault()}>
              {switching && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
              {t("settings.embeddingModels.confirmSwitchAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

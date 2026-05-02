import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Brain, Loader2, Settings } from "lucide-react"
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
import { cn } from "@/lib/utils"
import type { useMemoryData } from "./useMemoryData"
import LocalEmbeddingAssistantCard from "./LocalEmbeddingAssistantCard"
import type { MemoryEmbeddingSetDefaultResult } from "./types"
import { embeddingProviderLabel, openEmbeddingModelSettings } from "@/types/embedding-models"

type MemoryData = ReturnType<typeof useMemoryData>
type ReembedMode = "keep_existing" | "delete_all"

interface EmbeddingModelSectionProps {
  data: MemoryData
}

export default function EmbeddingModelSection({ data }: EmbeddingModelSectionProps) {
  const { t } = useTranslation()
  const { embeddingModels, memoryEmbeddingState, setMemoryEmbeddingState, reloadEmbeddingConfig } =
    data
  const [pendingModelId, setPendingModelId] = useState<string | null>(null)
  const [pendingMode, setPendingMode] = useState<ReembedMode>("keep_existing")
  const [switching, setSwitching] = useState(false)

  const currentId = memoryEmbeddingState.selection.enabled
    ? memoryEmbeddingState.selection.modelConfigId
    : undefined
  const pendingModel = embeddingModels.find((model) => model.id === pendingModelId) ?? null

  function startSwitch(modelId: string) {
    setPendingMode("keep_existing")
    setPendingModelId(modelId)
  }

  async function confirmSwitchDefault() {
    if (!pendingModelId) return
    setSwitching(true)
    try {
      const result = await getTransport().call<MemoryEmbeddingSetDefaultResult>(
        "memory_embedding_set_default",
        { modelConfigId: pendingModelId, mode: pendingMode },
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
                if (value && value !== currentId) startSwitch(value)
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

          <div className="space-y-2 py-2">
            <div className="text-xs font-medium text-foreground">
              {t("settings.embedding.reembedMode.label")}
            </div>
            <ModeOption
              active={pendingMode === "keep_existing"}
              label={t("settings.embedding.reembedMode.keepExisting")}
              description={t("settings.embedding.reembedMode.keepExistingDesc")}
              onSelect={() => setPendingMode("keep_existing")}
              disabled={switching}
            />
            <ModeOption
              active={pendingMode === "delete_all"}
              label={t("settings.embedding.reembedMode.deleteAll")}
              description={t("settings.embedding.reembedMode.deleteAllDesc")}
              onSelect={() => setPendingMode("delete_all")}
              disabled={switching}
            />
          </div>

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

function ModeOption({
  active,
  label,
  description,
  onSelect,
  disabled,
}: {
  active: boolean
  label: string
  description: string
  onSelect: () => void
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={disabled}
      className={cn(
        "flex w-full flex-col items-start rounded-md border px-3 py-2 text-left transition-colors",
        active ? "border-primary bg-primary/10" : "border-border hover:bg-secondary",
        disabled && "opacity-60",
      )}
    >
      <span className="text-sm font-medium">{label}</span>
      <span className="mt-0.5 text-xs text-muted-foreground">{description}</span>
    </button>
  )
}

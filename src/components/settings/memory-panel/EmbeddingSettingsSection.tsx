import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Switch } from "@/components/ui/switch"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { useMemoryData } from "./useMemoryData"
import EmbeddingModelSection from "./EmbeddingModelSection"
import HybridSearchConfigSection from "./HybridSearchConfig"
import EmbeddingActivationDialog from "./EmbeddingActivationDialog"
import ReembedJobCard from "./ReembedJobCard"
import type { MemoryEmbeddingSetDefaultResult } from "./types"
import { memoryEmbeddingOperationErrorToast } from "./memoryEmbeddingFeedback"

type MemoryData = ReturnType<typeof useMemoryData>

interface EmbeddingSettingsSectionProps {
  data: MemoryData
}

export default function EmbeddingSettingsSection({ data }: EmbeddingSettingsSectionProps) {
  const { t } = useTranslation()
  const [activationDialogOpen, setActivationDialogOpen] = useState(false)

  const {
    embeddingModels,
    memoryEmbeddingState,
    setMemoryEmbeddingState,
    embeddingConfigError,
    reloadEmbeddingConfig,
  } = data

  async function activateModel(modelConfigId: string): Promise<boolean> {
    try {
      const result = await getTransport().call<MemoryEmbeddingSetDefaultResult>(
        "memory_embedding_set_default",
        { modelConfigId, mode: "keep_existing" },
      )
      setMemoryEmbeddingState(result.state)
      await reloadEmbeddingConfig()
      if (result.reembedError) {
        toast.warning(t("settings.embeddingModels.reembedFailed"))
      } else {
        toast.success(t("settings.embeddingModels.defaultSet"))
      }
      return true
    } catch (e) {
      logger.error("settings", "EmbeddingSettingsSection::activate", "Failed to set default", e)
      const failure = memoryEmbeddingOperationErrorToast("setDefault", t, e)
      toast.error(failure.title, failure.description ? { description: failure.description } : undefined)
      return false
    }
  }

  function handleToggle(next: boolean) {
    if (!next) {
      void getTransport()
        .call("memory_embedding_disable")
        .then((state) => {
          setMemoryEmbeddingState(state as typeof memoryEmbeddingState)
          return reloadEmbeddingConfig()
        })
        .catch((e) => {
          logger.error("settings", "EmbeddingSettingsSection::disable", "Failed to disable", e)
          const failure = memoryEmbeddingOperationErrorToast("disable", t, e)
          toast.error(
            failure.title,
            failure.description ? { description: failure.description } : undefined,
          )
        })
      return
    }

    const remembered = memoryEmbeddingState.selection.modelConfigId
    const stillValid =
      remembered && embeddingModels.some((model) => model.id === remembered)
    if (stillValid) {
      void activateModel(remembered)
    } else {
      setActivationDialogOpen(true)
    }
  }

  return (
    <>
      <div className="flex items-center justify-between rounded-lg bg-secondary/30 px-3 py-3">
        <div>
          <div className="text-sm font-medium">{t("settings.memoryEmbeddingEnabled")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.memoryEmbeddingEnabledDesc")}
          </div>
        </div>
        <Switch
          checked={memoryEmbeddingState.selection.enabled}
          onCheckedChange={handleToggle}
          aria-label={t("settings.memoryEmbeddingEnabled")}
        />
      </div>

      {embeddingConfigError && (
        <div className="rounded border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
          <div className="font-medium text-foreground">{embeddingConfigError.title}</div>
          {embeddingConfigError.description && (
            <div className="mt-1 break-all text-muted-foreground">
              {embeddingConfigError.description}
            </div>
          )}
        </div>
      )}

      <div className="space-y-4">
        <EmbeddingModelSection data={data} />
        {memoryEmbeddingState.selection.enabled && <HybridSearchConfigSection data={data} />}
        <ReembedJobCard data={data} />
      </div>

      <EmbeddingActivationDialog
        open={activationDialogOpen}
        onOpenChange={setActivationDialogOpen}
        embeddingModels={embeddingModels}
        onConfirm={activateModel}
      />
    </>
  )
}

import { useState } from "react"
import { useTranslation } from "react-i18next"
import { ArrowLeft } from "lucide-react"
import { toast } from "sonner"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { useMemoryData } from "./useMemoryData"
import EmbeddingModelSection from "./EmbeddingModelSection"
import HybridSearchConfigSection from "./HybridSearchConfig"
import EmbeddingActivationDialog from "./EmbeddingActivationDialog"
import ReembedJobCard from "./ReembedJobCard"
import type { MemoryEmbeddingSetDefaultResult } from "./types"

type MemoryData = ReturnType<typeof useMemoryData>

interface EmbeddingViewProps {
  data: MemoryData
}

export default function EmbeddingView({ data }: EmbeddingViewProps) {
  const { t } = useTranslation()
  const [activationDialogOpen, setActivationDialogOpen] = useState(false)

  const {
    setView,
    embeddingModels,
    memoryEmbeddingState,
    setMemoryEmbeddingState,
    reloadEmbeddingConfig,
  } = data

  // Persist a chosen model + spawn a reembed job. Used both by the activation
  // dialog (first-time enable) and by the silent re-enable path when the user
  // already has a remembered model.
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
      logger.error("settings", "EmbeddingView::activate", "Failed to set default", e)
      toast.error(String(e))
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
          logger.error("settings", "EmbeddingView::disable", "Failed to disable", e)
          toast.error(String(e))
        })
      return
    }

    // Re-enable: prefer the previously selected model if it is still around.
    // Otherwise prompt the user with the selection dialog.
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
    <div className="flex-1 overflow-y-auto p-6">
      <div className="w-full">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setView("list")}
          className="mb-4 -ml-3 gap-1.5 text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("settings.memory")}
        </Button>

        <h2 className="text-lg font-semibold mb-1">{t("settings.memoryEmbedding")}</h2>
        <p className="text-xs text-muted-foreground mb-6">
          {t("settings.memoryEmbeddingDesc")}
        </p>

        {/* Enable toggle */}
        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 mb-4">
          <div>
            <div className="text-sm font-medium">{t("settings.memoryEmbeddingEnabled")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.memoryEmbeddingEnabledDesc")}
            </div>
          </div>
          <Switch
            checked={memoryEmbeddingState.selection.enabled}
            onCheckedChange={handleToggle}
          />
        </div>

        <div className="space-y-4">
          <EmbeddingModelSection data={data} />
          {memoryEmbeddingState.selection.enabled && <HybridSearchConfigSection data={data} />}
          <ReembedJobCard data={data} />
        </div>
      </div>

      <EmbeddingActivationDialog
        open={activationDialogOpen}
        onOpenChange={setActivationDialogOpen}
        embeddingModels={embeddingModels}
        onConfirm={activateModel}
      />
    </div>
  )
}

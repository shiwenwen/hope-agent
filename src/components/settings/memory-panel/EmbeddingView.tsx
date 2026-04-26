import { useTranslation } from "react-i18next"
import { ArrowLeft } from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import type { useMemoryData } from "./useMemoryData"
import EmbeddingModelSection from "./EmbeddingModelSection"
import HybridSearchConfigSection from "./HybridSearchConfig"

type MemoryData = ReturnType<typeof useMemoryData>

interface EmbeddingViewProps {
  data: MemoryData
}

export default function EmbeddingView({ data }: EmbeddingViewProps) {
  const { t } = useTranslation()

  const {
    setView,
    embeddingConfig, setEmbeddingConfig,
    setEmbeddingDirty,
  } = data

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
            <EmbeddingModelSection data={data} />
            <HybridSearchConfigSection data={data} />
          </div>
        )}
      </div>
    </div>
  )
}

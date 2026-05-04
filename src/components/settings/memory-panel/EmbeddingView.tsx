import { useTranslation } from "react-i18next"
import { ArrowLeft } from "lucide-react"
import { Button } from "@/components/ui/button"
import type { useMemoryData } from "./useMemoryData"
import EmbeddingSettingsSection from "./EmbeddingSettingsSection"

type MemoryData = ReturnType<typeof useMemoryData>

interface EmbeddingViewProps {
  data: MemoryData
}

export default function EmbeddingView({ data }: EmbeddingViewProps) {
  const { t } = useTranslation()

  const { setView } = data

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

        <EmbeddingSettingsSection data={data} />
      </div>
    </div>
  )
}

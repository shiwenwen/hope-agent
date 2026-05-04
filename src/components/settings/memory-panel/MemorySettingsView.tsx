import { useTranslation } from "react-i18next"
import ExtractConfig from "./ExtractConfig"
import BudgetConfig from "./BudgetConfig"
import EmbeddingSettingsSection from "./EmbeddingSettingsSection"
import type { useMemoryData } from "./useMemoryData"

type MemoryData = ReturnType<typeof useMemoryData>

interface MemorySettingsViewProps {
  data: MemoryData
  isAgentMode: boolean
}

export default function MemorySettingsView({ data, isAgentMode }: MemorySettingsViewProps) {
  const { t } = useTranslation()

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="w-full space-y-5">
        <div>
          <h2 className="text-lg font-semibold">{t("settings.memoryTabs.settings")}</h2>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("settings.memoryDesc")}
          </p>
        </div>

        <ExtractConfig data={data} isAgentMode={isAgentMode} />

        {!isAgentMode && <BudgetConfig />}

        {!isAgentMode && (
          <div className="space-y-4 border-t border-border/50 pt-5">
            <div>
              <h3 className="text-sm font-semibold">{t("settings.memoryEmbedding")}</h3>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.memoryEmbeddingDesc")}
              </p>
            </div>
            <EmbeddingSettingsSection data={data} />
          </div>
        )}
      </div>
    </div>
  )
}

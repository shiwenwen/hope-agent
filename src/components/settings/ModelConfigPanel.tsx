import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import ProviderSettings from "@/components/settings/ProviderSettings"
import type { ProviderConfig } from "@/components/settings/ProviderSettings"
import GlobalModelPanel from "@/components/settings/GlobalModelPanel"
import LocalModelsPanel from "@/components/settings/local-llm/LocalModelsPanel"
import EmbeddingModelsPanel from "@/components/settings/embedding-models/EmbeddingModelsPanel"

export default function ModelConfigPanel({
  onAddProvider,
  onEditProvider,
  onCodexReauth,
  initialTab,
}: {
  onAddProvider: () => void
  onEditProvider: (provider: ProviderConfig) => void
  onCodexReauth?: () => void
  initialTab?: string
}) {
  const { t } = useTranslation()
  const [tab, setTab] = useState(initialTab ?? "providers")

  useEffect(() => {
    if (initialTab) setTab(initialTab)
  }, [initialTab])

  return (
    <Tabs value={tab} onValueChange={setTab} className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="px-6 pt-4 pb-2 shrink-0">
        <TabsList className="w-fit">
          <TabsTrigger value="providers">{t("settings.providers")}</TabsTrigger>
          <TabsTrigger value="models">{t("settings.globalModel")}</TabsTrigger>
          <TabsTrigger value="localModels">{t("settings.localModels.tab")}</TabsTrigger>
          <TabsTrigger value="embeddingModels">{t("settings.embeddingModels.tab")}</TabsTrigger>
        </TabsList>
      </div>
      <TabsContent value="providers" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <ProviderSettings
          onAddProvider={onAddProvider}
          onEditProvider={onEditProvider}
          onCodexReauth={onCodexReauth}
        />
      </TabsContent>
      <TabsContent value="models" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <GlobalModelPanel />
      </TabsContent>
      <TabsContent value="localModels" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <LocalModelsPanel />
      </TabsContent>
      <TabsContent value="embeddingModels" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <EmbeddingModelsPanel />
      </TabsContent>
    </Tabs>
  )
}

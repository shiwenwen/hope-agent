import { useTranslation } from "react-i18next"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import ProviderSettings from "@/components/settings/ProviderSettings"
import type { ProviderConfig } from "@/components/settings/ProviderSettings"
import GlobalModelPanel from "@/components/settings/GlobalModelPanel"

export default function ModelConfigPanel({
  onAddProvider,
  onEditProvider,
  onCodexReauth,
}: {
  onAddProvider: () => void
  onEditProvider: (provider: ProviderConfig) => void
  onCodexReauth?: () => void
}) {
  const { t } = useTranslation()

  return (
    <Tabs defaultValue="providers" className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="px-6 pt-4 pb-2 shrink-0">
        <TabsList className="w-fit">
          <TabsTrigger value="providers">{t("settings.providers")}</TabsTrigger>
          <TabsTrigger value="models">{t("settings.globalModel")}</TabsTrigger>
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
    </Tabs>
  )
}

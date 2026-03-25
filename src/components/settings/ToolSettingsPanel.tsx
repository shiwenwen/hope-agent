import { useTranslation } from "react-i18next"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import ToolGeneralPanel from "@/components/settings/ToolGeneralPanel"
import WebSearchPanel from "@/components/settings/WebSearchPanel"
import WebFetchPanel from "@/components/settings/WebFetchPanel"
import ImageGeneratePanel from "@/components/settings/ImageGeneratePanel"
import CanvasSettingsPanel from "@/components/settings/CanvasSettingsPanel"

export default function ToolSettingsPanel() {
  const { t } = useTranslation()

  return (
    <Tabs defaultValue="general" className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="px-6 pt-4 pb-2 shrink-0">
        <TabsList className="w-fit">
          <TabsTrigger value="general">{t("settings.toolGeneral")}</TabsTrigger>
          <TabsTrigger value="webSearch">{t("settings.webSearch")}</TabsTrigger>
          <TabsTrigger value="webFetch">{t("settings.webFetch")}</TabsTrigger>
          <TabsTrigger value="imageGenerate">{t("settings.imageGenerate")}</TabsTrigger>
          <TabsTrigger value="canvas">{t("settings.canvas")}</TabsTrigger>
        </TabsList>
      </div>
      <TabsContent value="general" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <ToolGeneralPanel />
      </TabsContent>
      <TabsContent value="webSearch" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <WebSearchPanel />
      </TabsContent>
      <TabsContent value="webFetch" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <WebFetchPanel />
      </TabsContent>
      <TabsContent value="imageGenerate" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <ImageGeneratePanel />
      </TabsContent>
      <TabsContent value="canvas" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <CanvasSettingsPanel />
      </TabsContent>
    </Tabs>
  )
}

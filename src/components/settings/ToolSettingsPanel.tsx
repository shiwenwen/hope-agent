import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import ToolGeneralPanel from "@/components/settings/ToolGeneralPanel"
import WebSearchPanel from "@/components/settings/WebSearchPanel"
import WebFetchPanel from "@/components/settings/WebFetchPanel"
import ImageGeneratePanel from "@/components/settings/ImageGeneratePanel"
import AudioGeneratePanel from "@/components/settings/AudioGeneratePanel"
import IssueReportingPanel from "@/components/settings/IssueReportingPanel"
import CanvasSettingsPanel from "@/components/settings/CanvasSettingsPanel"
import AsyncToolsPanel from "@/components/settings/AsyncToolsPanel"
import SettingsResetControl from "./SettingsResetControl"
import { RESET_SECTION_BY_TAB, type ToolTab } from "./toolSettingsReset"

export default function ToolSettingsPanel() {
  const { t } = useTranslation()
  const [tab, setTab] = useState<ToolTab>("general")
  const [revisions, setRevisions] = useState<Record<ToolTab, number>>({
    general: 0,
    webSearch: 0,
    webFetch: 0,
    imageGenerate: 0,
    audioGenerate: 0,
    canvas: 0,
    asyncTools: 0,
    issueReporting: 0,
  })
  const labels: Record<ToolTab, string> = {
    general: t("settings.toolGeneral"),
    webSearch: t("settings.webSearch"),
    webFetch: t("settings.webFetch"),
    imageGenerate: t("settings.imageGenerate"),
    audioGenerate: t("settings.audioGenerate", "音频生成"),
    canvas: t("settings.canvas"),
    asyncTools: t("settings.asyncTools"),
    issueReporting: t("settings.issueReporting"),
  }

  const refreshTab = () => {
    setRevisions((current) => ({ ...current, [tab]: current[tab] + 1 }))
  }

  return (
    <Tabs
      value={tab}
      onValueChange={(value) => setTab(value as ToolTab)}
      className="flex-1 flex flex-col min-h-0 overflow-hidden"
    >
      <div className="flex items-center gap-3 px-6 pt-4 pb-2 shrink-0">
        <div className="min-w-0 flex-1 overflow-x-auto">
          <TabsList className="w-fit">
            <TabsTrigger value="general">{t("settings.toolGeneral")}</TabsTrigger>
            <TabsTrigger value="webSearch">{t("settings.webSearch")}</TabsTrigger>
            <TabsTrigger value="webFetch">{t("settings.webFetch")}</TabsTrigger>
            <TabsTrigger value="imageGenerate">{t("settings.imageGenerate")}</TabsTrigger>
            <TabsTrigger value="audioGenerate">{t("settings.audioGenerate", "音频生成")}</TabsTrigger>
            <TabsTrigger value="canvas">{t("settings.canvas")}</TabsTrigger>
            <TabsTrigger value="asyncTools">{t("settings.asyncTools")}</TabsTrigger>
            <TabsTrigger value="issueReporting">{t("settings.issueReporting")}</TabsTrigger>
          </TabsList>
        </div>
        <SettingsResetControl
          scope="tools"
          resetSection={RESET_SECTION_BY_TAB[tab]}
          sectionLabel={labels[tab]}
          level="tab"
          onReset={refreshTab}
        />
      </div>
      <TabsContent value="general" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <ToolGeneralPanel key={revisions.general} />
      </TabsContent>
      <TabsContent value="webSearch" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <WebSearchPanel key={revisions.webSearch} />
      </TabsContent>
      <TabsContent value="webFetch" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <WebFetchPanel key={revisions.webFetch} />
      </TabsContent>
      <TabsContent value="imageGenerate" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <ImageGeneratePanel key={revisions.imageGenerate} />
      </TabsContent>
      <TabsContent value="audioGenerate" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <AudioGeneratePanel key={revisions.audioGenerate} />
      </TabsContent>
      <TabsContent value="canvas" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <CanvasSettingsPanel key={revisions.canvas} />
      </TabsContent>
      <TabsContent value="asyncTools" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <AsyncToolsPanel key={revisions.asyncTools} />
      </TabsContent>
      <TabsContent value="issueReporting" className="flex-1 min-h-0 overflow-hidden mt-0 flex flex-col">
        <IssueReportingPanel key={revisions.issueReporting} />
      </TabsContent>
    </Tabs>
  )
}

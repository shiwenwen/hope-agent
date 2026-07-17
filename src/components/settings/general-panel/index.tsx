import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import ThemeSection from "./ThemeSection"
import LanguageSection from "./LanguageSection"
import {
  AutostartToggle,
  ChatDisplayModeSelector,
  PreventSleepToggle,
  SidebarDisplayModeSelector,
  UiEffectsToggle,
} from "./SystemSection"
import ShortcutSection from "./ShortcutSection"
import ProxySection from "./ProxySection"
import OnboardingResetSection from "./OnboardingResetSection"
import FocusIndicatorSection from "./FocusIndicatorSection"
import SettingsResetControl from "../SettingsResetControl"

type GeneralTab = "appearance" | "system" | "network"

export default function GeneralPanel() {
  const { t } = useTranslation()
  const [tab, setTab] = useState<GeneralTab>("appearance")
  const [revisions, setRevisions] = useState<Record<GeneralTab, number>>({
    appearance: 0,
    system: 0,
    network: 0,
  })
  const labels: Record<GeneralTab, string> = {
    appearance: t("settings.tabAppearance"),
    system: t("settings.tabSystem"),
    network: t("settings.tabNetwork"),
  }

  const refreshTab = () => {
    setRevisions((current) => ({ ...current, [tab]: current[tab] + 1 }))
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as GeneralTab)}
        className="flex-1 flex flex-col min-h-0"
      >
        <div className="flex items-center gap-3 px-6 pt-2 shrink-0">
          <div className="min-w-0 flex-1 overflow-x-auto">
            <TabsList>
              <TabsTrigger value="appearance">{t("settings.tabAppearance")}</TabsTrigger>
              <TabsTrigger value="system">{t("settings.tabSystem")}</TabsTrigger>
              <TabsTrigger value="network">{t("settings.tabNetwork")}</TabsTrigger>
            </TabsList>
          </div>
          <SettingsResetControl
            scope="general"
            resetSection={tab}
            sectionLabel={labels[tab]}
            level="tab"
            onReset={refreshTab}
          />
        </div>

        {/* Appearance & Language */}
        <TabsContent value="appearance" className="flex-1 overflow-y-auto px-6 pb-6">
          <div key={revisions.appearance} className="w-full space-y-8 pt-4">
            <ThemeSection />
            <FocusIndicatorSection />
            <LanguageSection />
            <div>
              <SidebarDisplayModeSelector />
              <ChatDisplayModeSelector />
            </div>
            <UiEffectsToggle />
          </div>
        </TabsContent>

        {/* System & Shortcuts */}
        <TabsContent value="system" className="flex-1 overflow-y-auto px-6 pb-6">
          <div key={revisions.system} className="w-full space-y-8 pt-4">
            <div>
              <AutostartToggle />
              <PreventSleepToggle />
            </div>
            <ShortcutSection />
            <OnboardingResetSection />
          </div>
        </TabsContent>

        {/* Network / Proxy */}
        <TabsContent value="network" className="flex-1 overflow-y-auto px-6 pb-6">
          <ProxySection key={revisions.network} />
        </TabsContent>
      </Tabs>
    </div>
  )
}

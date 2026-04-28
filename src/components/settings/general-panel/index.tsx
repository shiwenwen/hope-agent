import { useTranslation } from "react-i18next"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import ThemeSection from "./ThemeSection"
import LanguageSection from "./LanguageSection"
import { AutostartToggle, UiEffectsToggle } from "./SystemSection"
import ShortcutSection from "./ShortcutSection"
import ProxySection from "./ProxySection"
import OnboardingResetSection from "./OnboardingResetSection"
import DefaultAgentSection from "./DefaultAgentSection"

export default function GeneralPanel() {
  const { t } = useTranslation()

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <Tabs defaultValue="appearance" className="flex-1 flex flex-col min-h-0">
        <div className="px-6 pt-2 shrink-0">
          <TabsList>
            <TabsTrigger value="appearance">{t("settings.tabAppearance")}</TabsTrigger>
            <TabsTrigger value="system">{t("settings.tabSystem")}</TabsTrigger>
            <TabsTrigger value="network">{t("settings.tabNetwork")}</TabsTrigger>
          </TabsList>
        </div>

        {/* Appearance & Language */}
        <TabsContent value="appearance" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="w-full space-y-8 pt-4">
            <ThemeSection />
            <LanguageSection />
            <UiEffectsToggle />
          </div>
        </TabsContent>

        {/* System & Shortcuts */}
        <TabsContent value="system" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="w-full space-y-8 pt-4">
            <AutostartToggle />
            <DefaultAgentSection />
            <ShortcutSection />
            <OnboardingResetSection />
          </div>
        </TabsContent>

        {/* Network / Proxy */}
        <TabsContent value="network" className="flex-1 overflow-y-auto px-6 pb-6">
          <ProxySection />
        </TabsContent>
      </Tabs>
    </div>
  )
}

import { useTranslation } from "react-i18next"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import DangerousModeSection from "./DangerousModeSection"
import SsrfPolicySection from "./SsrfPolicySection"

export default function SecurityPanel() {
  const { t } = useTranslation()

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <Tabs defaultValue="dangerous" className="flex-1 flex flex-col min-h-0">
        <div className="px-6 pt-2 shrink-0">
          <TabsList>
            <TabsTrigger value="dangerous">
              {t("settings.tabDangerous", "危险模式")}
            </TabsTrigger>
            <TabsTrigger value="ssrf">{t("settings.tabSsrf", "SSRF 策略")}</TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="dangerous" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="pt-4">
            <DangerousModeSection />
          </div>
        </TabsContent>

        <TabsContent value="ssrf" className="flex-1 flex flex-col min-h-0 outline-none">
          <SsrfPolicySection />
        </TabsContent>
      </Tabs>
    </div>
  )
}

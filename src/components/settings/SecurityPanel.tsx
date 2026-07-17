import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import DangerousModeSection from "./DangerousModeSection"
import SsrfPolicySection from "./SsrfPolicySection"
import SettingsResetControl from "./SettingsResetControl"

type SecurityTab = "dangerous" | "ssrf"

export default function SecurityPanel() {
  const { t } = useTranslation()
  const [tab, setTab] = useState<SecurityTab>("dangerous")
  const [revisions, setRevisions] = useState<Record<SecurityTab, number>>({
    dangerous: 0,
    ssrf: 0,
  })
  const labels: Record<SecurityTab, string> = {
    dangerous: t("settings.tabDangerous", "危险模式"),
    ssrf: t("settings.tabSsrf", "SSRF 策略"),
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as SecurityTab)}
        className="flex-1 flex flex-col min-h-0"
      >
        <div className="flex items-center gap-3 px-6 pt-2 shrink-0">
          <div className="min-w-0 flex-1 overflow-x-auto">
            <TabsList>
              <TabsTrigger value="dangerous">
                {t("settings.tabDangerous", "危险模式")}
              </TabsTrigger>
              <TabsTrigger value="ssrf">{t("settings.tabSsrf", "SSRF 策略")}</TabsTrigger>
            </TabsList>
          </div>
          <SettingsResetControl
            scope="security"
            resetSection={tab}
            sectionLabel={labels[tab]}
            level="tab"
            onReset={() =>
              setRevisions((current) => ({ ...current, [tab]: current[tab] + 1 }))
            }
          />
        </div>

        <TabsContent value="dangerous" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="pt-4">
            <DangerousModeSection key={revisions.dangerous} />
          </div>
        </TabsContent>

        <TabsContent value="ssrf" className="flex-1 flex flex-col min-h-0 outline-none">
          <SsrfPolicySection key={revisions.ssrf} />
        </TabsContent>
      </Tabs>
    </div>
  )
}

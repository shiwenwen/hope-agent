import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"

export default function PlanSettingsPanel() {
  const { t } = useTranslation()
  const [planSubagent, setPlanSubagent] = useState(false)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    invoke<boolean>("get_plan_subagent")
      .then((val) => {
        setPlanSubagent(val)
        setLoaded(true)
      })
      .catch((e: unknown) => logger.error("settings", "PlanSettingsPanel::load", "Failed to load", e))
  }, [])

  async function togglePlanSubagent(checked: boolean) {
    setPlanSubagent(checked)
    try {
      await invoke("set_plan_subagent", { enabled: checked })
    } catch (e) {
      logger.error("settings", "PlanSettingsPanel::save", "Failed to save", e)
      setPlanSubagent(!checked)
    }
  }

  if (!loaded) return null

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-6">
        <div
          className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
          onClick={() => togglePlanSubagent(!planSubagent)}
        >
          <div className="space-y-0.5">
            <div className="text-sm font-medium">{t("settings.planSubagent")}</div>
            <div className="text-xs text-muted-foreground">{t("settings.planSubagentDesc")}</div>
          </div>
          <Switch
            checked={planSubagent}
            onCheckedChange={togglePlanSubagent}
          />
        </div>
      </div>
    </div>
  )
}

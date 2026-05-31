import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
import { Slider } from "@/components/ui/slider"

const DEFAULT_ASK_USER_TIMEOUT_SECS = 1800

export default function PlanSettingsPanel() {
  const { t } = useTranslation()
  const [planSubagent, setPlanSubagent] = useState(false)
  const [questionTimeoutEnabled, setQuestionTimeoutEnabled] = useState(false)
  const [questionTimeout, setQuestionTimeout] = useState(DEFAULT_ASK_USER_TIMEOUT_SECS)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    Promise.all([
      getTransport().call<boolean>("get_plan_subagent"),
      getTransport().call<boolean>("get_ask_user_question_timeout_enabled"),
      getTransport().call<number>("get_ask_user_question_timeout"),
    ])
      .then(([subagent, timeoutEnabled, timeout]) => {
        setPlanSubagent(subagent)
        setQuestionTimeoutEnabled(timeoutEnabled)
        setQuestionTimeout(timeout)
        setLoaded(true)
      })
      .catch((e: unknown) => logger.error("settings", "PlanSettingsPanel::load", "Failed to load", e))
  }, [])

  async function togglePlanSubagent(checked: boolean) {
    setPlanSubagent(checked)
    try {
      await getTransport().call("set_plan_subagent", { enabled: checked })
    } catch (e) {
      logger.error("settings", "PlanSettingsPanel::save", "Failed to save", e)
      setPlanSubagent(!checked)
    }
  }

  function handleTimeoutDrag(value: number[]) {
    if (!questionTimeoutEnabled) return
    setQuestionTimeout(value[0])
  }

  async function handleTimeoutCommit(value: number[]) {
    if (!questionTimeoutEnabled) return
    const secs = value[0]
    try {
      await getTransport().call("set_ask_user_question_timeout", { secs })
    } catch (e) {
      logger.error("settings", "PlanSettingsPanel::saveTimeout", "Failed to save", e)
    }
  }

  async function toggleQuestionTimeout(checked: boolean) {
    const previousEnabled = questionTimeoutEnabled
    const previousTimeout = questionTimeout
    const nextTimeout =
      checked && questionTimeout <= 0 ? DEFAULT_ASK_USER_TIMEOUT_SECS : questionTimeout
    setQuestionTimeoutEnabled(checked)
    if (nextTimeout !== questionTimeout) setQuestionTimeout(nextTimeout)
    try {
      await Promise.all([
        getTransport().call("set_ask_user_question_timeout_enabled", { enabled: checked }),
        nextTimeout !== previousTimeout
          ? getTransport().call("set_ask_user_question_timeout", { secs: nextTimeout })
          : Promise.resolve(),
      ])
    } catch (e) {
      logger.error("settings", "PlanSettingsPanel::saveTimeoutEnabled", "Failed to save", e)
      setQuestionTimeoutEnabled(previousEnabled)
      setQuestionTimeout(previousTimeout)
    }
  }

  function formatTimeout(secs: number): string {
    if (secs === 0) return t("settings.noLimit")
    const mins = Math.floor(secs / 60)
    const remainSecs = secs % 60
    if (remainSecs === 0) return `${mins} ${t("settings.minutes")}`
    return `${mins} ${t("settings.minutes")} ${remainSecs} ${t("settings.seconds")}`
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

        <div className="px-3 py-3 rounded-lg space-y-3">
          <div className="flex items-center justify-between gap-3">
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.planQuestionTimeout")}</div>
              <div className="text-xs text-muted-foreground">{t("settings.planQuestionTimeoutDesc")}</div>
            </div>
            <Switch
              checked={questionTimeoutEnabled}
              onCheckedChange={toggleQuestionTimeout}
            />
          </div>
          <div className="flex items-center gap-4">
            <Slider
              value={[questionTimeout]}
              onValueChange={handleTimeoutDrag}
              onValueCommit={handleTimeoutCommit}
              min={0}
              max={3600}
              step={60}
              disabled={!questionTimeoutEnabled}
              className="flex-1"
            />
            <span className="text-sm text-muted-foreground w-24 text-right shrink-0">
              {questionTimeoutEnabled ? formatTimeout(questionTimeout) : t("settings.noLimit")}
            </span>
          </div>
        </div>
      </div>
    </div>
  )
}

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
import ContextCompactPanel from "@/components/settings/ContextCompactPanel"
import { invalidateThinkingExpandCache } from "@/components/chat/ThinkingBlock"

interface ChatConfig {
  autoSendPending: boolean
  autoExpandThinking: boolean
}

export default function ChatSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<ChatConfig>({ autoSendPending: true, autoExpandThinking: true })
  const [planSubagent, setPlanSubagent] = useState(false)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    Promise.all([
      invoke<{ autoSendPending?: boolean; autoExpandThinking?: boolean }>("get_user_config"),
      invoke<boolean>("get_plan_subagent"),
    ])
      .then(([cfg, subagent]) => {
        setConfig({
          autoSendPending: cfg.autoSendPending !== false,
          autoExpandThinking: cfg.autoExpandThinking !== false,
        })
        setPlanSubagent(subagent)
        setLoaded(true)
      })
      .catch((e: unknown) => logger.error("settings", "ChatSettingsPanel::load", "Failed to load config", e))
  }, [])

  async function toggle(key: keyof ChatConfig) {
    const updated = { ...config, [key]: !config[key] }
    setConfig(updated)
    try {
      const full = await invoke<Record<string, unknown>>("get_user_config")
      await invoke("save_user_config", { config: { ...full, ...updated } })
      if (key === "autoExpandThinking") {
        invalidateThinkingExpandCache()
      }
    } catch (e) {
      logger.error("settings", "ChatSettingsPanel::save", "Failed to save chat config", e)
    }
  }

  if (!loaded) return null

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-6">
        <div
          className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
          onClick={() => toggle("autoSendPending")}
        >
          <div className="space-y-0.5">
            <div className="text-sm font-medium">{t("settings.chatAutoSend")}</div>
            <div className="text-xs text-muted-foreground">{t("settings.chatAutoSendDesc")}</div>
          </div>
          <Switch
            checked={config.autoSendPending}
            onCheckedChange={() => toggle("autoSendPending")}
          />
        </div>

        <div
          className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
          onClick={() => toggle("autoExpandThinking")}
        >
          <div className="space-y-0.5">
            <div className="text-sm font-medium">{t("settings.chatAutoExpandThinking")}</div>
            <div className="text-xs text-muted-foreground">{t("settings.chatAutoExpandThinkingDesc")}</div>
          </div>
          <Switch
            checked={config.autoExpandThinking}
            onCheckedChange={() => toggle("autoExpandThinking")}
          />
        </div>

        {/* Plan Mode 子 Agent 开关 */}
        <div
          className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
          onClick={async () => {
            const next = !planSubagent
            setPlanSubagent(next)
            try {
              await invoke("set_plan_subagent", { enabled: next })
            } catch (e) {
              logger.error("settings", "ChatSettingsPanel::planSubagent", "Failed to save", e)
              setPlanSubagent(!next)
            }
          }}
        >
          <div className="space-y-0.5">
            <div className="text-sm font-medium">{t("settings.planSubagent")}</div>
            <div className="text-xs text-muted-foreground">{t("settings.planSubagentDesc")}</div>
          </div>
          <Switch
            checked={planSubagent}
            onCheckedChange={async (checked) => {
              setPlanSubagent(checked)
              try {
                await invoke("set_plan_subagent", { enabled: checked })
              } catch (e) {
                logger.error("settings", "ChatSettingsPanel::planSubagent", "Failed to save", e)
                setPlanSubagent(!checked)
              }
            }}
          />
        </div>

        {/* 上下文管理 */}
        <ContextCompactPanel />
      </div>
    </div>
  )
}

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"

interface ChatConfig {
  autoSendPending: boolean
}

export default function ChatSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<ChatConfig>({ autoSendPending: true })
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    invoke<{ autoSendPending?: boolean }>("get_user_config").then((cfg) => {
      setConfig({ autoSendPending: cfg.autoSendPending !== false })
      setLoaded(true)
    }).catch(console.error)
  }, [])

  async function toggle(key: keyof ChatConfig) {
    const updated = { ...config, [key]: !config[key] }
    setConfig(updated)
    try {
      const full = await invoke<Record<string, unknown>>("get_user_config")
      await invoke("save_user_config", { config: { ...full, ...updated } })
    } catch (e) {
      console.error("Failed to save chat config:", e)
    }
  }

  if (!loaded) return null

  return (
    <div className="space-y-4">
      <div
        className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
        onClick={() => toggle("autoSendPending")}
      >
        <div className="space-y-0.5">
          <div className="text-sm font-medium">{t("settings.chatAutoSend")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.chatAutoSendDesc")}</div>
        </div>
        <Switch checked={config.autoSendPending} onCheckedChange={() => toggle("autoSendPending")} />
      </div>
    </div>
  )
}

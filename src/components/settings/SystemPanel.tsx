import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"

export default function SystemPanel() {
  const { t } = useTranslation()
  const [autostart, setAutostart] = useState(false)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    getTransport().call<boolean>("get_autostart_enabled")
      .then((enabled) => {
        setAutostart(enabled)
        setLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "SystemPanel::load", "Failed to get autostart status", e)
        setLoaded(true)
      })
  }, [])

  async function toggleAutostart() {
    const next = !autostart
    setAutostart(next) // optimistic update
    try {
      await getTransport().call("set_autostart_enabled", { enabled: next })
    } catch (e) {
      setAutostart(!next) // rollback
      logger.error("settings", "SystemPanel::toggle", "Failed to set autostart", e)
    }
  }

  if (!loaded) return null

  return (
    <div className="space-y-4">
      <div
        className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
        onClick={toggleAutostart}
      >
        <div className="space-y-0.5">
          <div className="text-sm font-medium">{t("settings.systemAutostart")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.systemAutostartDesc")}</div>
        </div>
        <Switch checked={autostart} onCheckedChange={toggleAutostart} />
      </div>
    </div>
  )
}

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Input } from "@/components/ui/input"

export default function ToolGeneralPanel() {
  const { t } = useTranslation()
  const [toolTimeout, setToolTimeout] = useState(300)
  const [savedTimeout, setSavedTimeout] = useState(300)

  useEffect(() => {
    invoke<number>("get_tool_timeout")
      .then((v) => {
        setToolTimeout(v)
        setSavedTimeout(v)
      })
      .catch((e) =>
        logger.error("settings", "ToolGeneralPanel::load", "Failed to load tool timeout", e)
      )
  }, [])

  const saveTimeout = useCallback(async (value: number) => {
    try {
      await invoke("set_tool_timeout", { seconds: value })
      setSavedTimeout(value)
    } catch (e) {
      setToolTimeout(savedTimeout)
      logger.error("settings", "ToolGeneralPanel::save", "Failed to save tool timeout", e)
    }
  }, [savedTimeout])

  return (
    <div className="px-6 py-4 overflow-y-auto flex-1">
      <div className="space-y-4">
        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5">
            <div className="text-sm font-medium">{t("settings.toolTimeout")}</div>
            <div className="text-xs text-muted-foreground">{t("settings.toolTimeoutDesc")}</div>
          </div>
          <div className="flex items-center gap-2">
            <Input
              type="number"
              min={0}
              step={30}
              value={toolTimeout}
              onChange={(e) => setToolTimeout(Number(e.target.value))}
              onBlur={() => {
                const clamped = Math.max(0, Math.round(toolTimeout))
                setToolTimeout(clamped)
                if (clamped !== savedTimeout) saveTimeout(clamped)
              }}
              className="w-24 h-8 text-sm text-right"
            />
            <span className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.seconds")}</span>
          </div>
        </div>
      </div>
    </div>
  )
}

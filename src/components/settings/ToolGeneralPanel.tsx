import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { WeatherSection } from "@/components/settings/WeatherSection"
import { cn } from "@/lib/utils"
import { Check, Loader2 } from "lucide-react"

interface UserConfig {
  weatherEnabled?: boolean
  weatherCity?: string | null
  weatherLatitude?: number | null
  weatherLongitude?: number | null
}

export default function ToolGeneralPanel() {
  const { t } = useTranslation()
  const [toolTimeout, setToolTimeout] = useState(300)
  const [savedTimeout, setSavedTimeout] = useState(300)
  
  const [config, setConfig] = useState<UserConfig>({})
  const [savedConfigSnapshot, setSavedConfigSnapshot] = useState<string>("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const isConfigDirty = JSON.stringify(config) !== savedConfigSnapshot

  useEffect(() => {
    let cancelled = false
    
    // Load tool timeout
    invoke<number>("get_tool_timeout")
      .then((v) => { if (!cancelled) { setToolTimeout(v); setSavedTimeout(v); } })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load tool timeout", e))
      
    // Load user config
    invoke<UserConfig>("get_user_config")
      .then((cfg) => {
        if (!cancelled) {
          setConfig(cfg)
          setSavedConfigSnapshot(JSON.stringify(cfg))
        }
      })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load user config", e))
      
    return () => { cancelled = true }
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

  const saveConfig = async () => {
    setSaving(true)
    try {
      await invoke("save_user_config", { config })
      setSavedConfigSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "ToolGeneralPanel::saveConfig", "Failed to save user config", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const update = (key: string, value: any) => {
    setConfig((prev) => ({ ...prev, [key]: value }))
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6">
        <div className="space-y-6">
          {/* Timeout Setting */}
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
          
          <div className="border-t border-border/50" />
          
          {/* Weather Settings */}
          <WeatherSection config={config} update={update} />
        </div>
      </div>
      
      {/* Save — fixed bottom */}
      <div className="shrink-0 flex justify-end px-6 py-3 border-t border-border/30">
        <Button
          onClick={saveConfig}
          disabled={(!isConfigDirty && saveStatus === "idle") || saving}
          className={cn(
            saveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
            saveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
          )}
        >
          {saving ? (
            <span className="flex items-center gap-1.5">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.saving")}
            </span>
          ) : saveStatus === "saved" ? (
            <span className="flex items-center gap-1.5">
              <Check className="h-3.5 w-3.5" />
              {t("common.saved")}
            </span>
          ) : saveStatus === "failed" ? (
            t("common.saveFailed")
          ) : (
            t("common.save")
          )}
        </Button>
      </div>
    </div>
  )
}

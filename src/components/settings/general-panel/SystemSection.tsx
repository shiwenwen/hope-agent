import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { logger } from "@/lib/logger"

/**
 * AutostartToggle -- rendered in the System tab
 */
export function AutostartToggle() {
  const { t } = useTranslation()

  const [autostart, setAutostart] = useState(false)
  const [autostartLoaded, setAutostartLoaded] = useState(false)

  useEffect(() => {
    let cancelled = false
    invoke<boolean>("get_autostart_enabled")
      .then((enabled) => {
        if (cancelled) return
        setAutostart(enabled)
        setAutostartLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "AutostartToggle::load", "Failed to load autostart", e)
        setAutostartLoaded(true)
      })
    return () => { cancelled = true }
  }, [])

  async function toggleAutostart() {
    const next = !autostart
    setAutostart(next)
    try {
      await invoke("set_autostart_enabled", { enabled: next })
    } catch (e) {
      setAutostart(!next)
      logger.error("settings", "AutostartToggle::toggle", "Failed to set autostart", e)
    }
  }

  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.system")}</h3>
      {autostartLoaded && (
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
      )}
    </div>
  )
}

/**
 * UiEffectsToggle -- rendered in the Appearance tab
 */
export function UiEffectsToggle() {
  const { t } = useTranslation()

  const [uiEffectsEnabled, setUiEffectsEnabled] = useState(true)
  const [uiEffectsLoaded, setUiEffectsLoaded] = useState(false)

  useEffect(() => {
    let cancelled = false
    invoke<boolean>("get_ui_effects_enabled")
      .then((effectsEnabled) => {
        if (cancelled) return
        setUiEffectsEnabled(effectsEnabled)
        setUiEffectsLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "UiEffectsToggle::load", "Failed to load UI effects setting", e)
        setUiEffectsLoaded(true)
      })
    return () => { cancelled = true }
  }, [])

  async function toggleUiEffects() {
    const next = !uiEffectsEnabled
    setUiEffectsEnabled(next)
    try {
      await invoke("set_ui_effects_enabled", { enabled: next })
      window.dispatchEvent(new Event("ui-effects-changed"))
    } catch (e) {
      setUiEffectsEnabled(!next)
      logger.error("settings", "UiEffectsToggle::toggle", "Failed to set UI effects", e)
    }
  }

  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.uiEffects", "背景动效")}</h3>
      {uiEffectsLoaded && (
        <div
          className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
          onClick={toggleUiEffects}
        >
          <div className="space-y-0.5">
            <div className="text-sm font-medium">{t("settings.uiEffectsToggle", "开启动效")}</div>
            <div className="text-xs text-muted-foreground">{t("settings.uiEffectsDesc", "开启全天候背景及天气特效联动")}</div>
          </div>
          <Switch checked={uiEffectsEnabled} onCheckedChange={toggleUiEffects} />
        </div>
      )}
    </div>
  )
}

/**
 * Default export combines both toggles (for use when rendering together)
 */
export default function SystemSection() {
  return (
    <>
      <AutostartToggle />
      <UiEffectsToggle />
    </>
  )
}

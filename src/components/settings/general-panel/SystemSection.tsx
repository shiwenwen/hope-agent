import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { logger } from "@/lib/logger"
import {
  DEFAULT_SIDEBAR_DISPLAY_MODE,
  normalizeSidebarDisplayMode,
  type SidebarDisplayMode,
} from "@/components/chat/sidebar/types"
import {
  normalizeChatDisplayMode,
  readChatDisplayModePreference,
  writeChatDisplayModePreference,
} from "@/components/chat/chatDisplayModePreference"
import type { ChatDisplayMode } from "@/types/chat"

/**
 * AutostartToggle -- rendered in the System tab
 */
export function AutostartToggle() {
  const { t } = useTranslation()

  const [autostart, setAutostart] = useState(false)
  const [autostartLoaded, setAutostartLoaded] = useState(false)

  useEffect(() => {
    let cancelled = false
    getTransport().call<boolean>("get_autostart_enabled")
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
      await getTransport().call("set_autostart_enabled", { enabled: next })
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
    getTransport().call<boolean>("get_ui_effects_enabled")
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
      await getTransport().call("set_ui_effects_enabled", { enabled: next })
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
 * SidebarDisplayModeSelector -- rendered in the Appearance tab
 */
export function SidebarDisplayModeSelector() {
  const { t } = useTranslation()

  const [mode, setMode] = useState<SidebarDisplayMode>(DEFAULT_SIDEBAR_DISPLAY_MODE)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    let cancelled = false
    getTransport().call<string>("get_sidebar_display_mode")
      .then((value) => {
        if (cancelled) return
        setMode(normalizeSidebarDisplayMode(value))
        setLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "SidebarDisplayModeSelector::load", "Failed to load sidebar display mode", e)
        setLoaded(true)
      })
    return () => { cancelled = true }
  }, [])

  async function updateMode(next: SidebarDisplayMode) {
    if (next === mode) return
    const previous = mode
    setMode(next)
    window.dispatchEvent(new CustomEvent("sidebar-display-mode-changed", { detail: { mode: next } }))
    try {
      await getTransport().call("set_sidebar_display_mode", { mode: next })
    } catch (e) {
      setMode(previous)
      window.dispatchEvent(new CustomEvent("sidebar-display-mode-changed", { detail: { mode: previous } }))
      logger.error("settings", "SidebarDisplayModeSelector::update", "Failed to set sidebar display mode", e)
    }
  }

  function toggleCompactMode() {
    void updateMode(mode === "compact" ? "detailed" : "compact")
  }

  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground mb-1">
        {t("settings.sidebarDisplayMode", "界面显示")}
      </h3>
      {loaded && (
        <div
          className="flex items-center justify-between gap-4 px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
          onClick={toggleCompactMode}
        >
          <div className="space-y-0.5">
            <div className="text-sm font-medium">
              {t("settings.sidebarCompactMode", "简约模式")}
            </div>
            <div className="text-xs text-muted-foreground">
              {t("settings.sidebarCompactModeDesc", "隐藏会话、Agent 和项目的头像与 emoji，让侧边栏更清爽。")}
            </div>
          </div>
          <Switch
            checked={mode === "compact"}
            onCheckedChange={(checked) => updateMode(checked ? "compact" : "detailed")}
            onClick={(event) => event.stopPropagation()}
          />
        </div>
      )}
    </div>
  )
}

/**
 * ChatDisplayModeSelector -- rendered in the Appearance tab
 */
export function ChatDisplayModeSelector() {
  const { t } = useTranslation()

  const [mode, setMode] = useState<ChatDisplayMode>(readChatDisplayModePreference())
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<{ chatDisplayMode?: unknown }>("get_user_config")
      .then((cfg) => {
        if (cancelled) return
        const next = normalizeChatDisplayMode(cfg.chatDisplayMode) ?? readChatDisplayModePreference()
        setMode(next)
        writeChatDisplayModePreference(next, { emit: false })
        setLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "ChatDisplayModeSelector::load", "Failed to load chat display mode", e)
        setLoaded(true)
      })
    return () => { cancelled = true }
  }, [])

  async function updateMode(next: ChatDisplayMode) {
    if (next === mode) return
    const previous = mode
    setMode(next)
    writeChatDisplayModePreference(next)
    try {
      const full = await getTransport().call<Record<string, unknown>>("get_user_config")
      await getTransport().call("save_user_config", {
        config: { ...full, chatDisplayMode: next },
      })
    } catch (e) {
      setMode(previous)
      writeChatDisplayModePreference(previous)
      logger.error(
        "settings",
        "ChatDisplayModeSelector::update",
        "Failed to save chat display mode",
        e,
      )
    }
  }

  return (
    <div>
      {loaded && (
        <div className="flex items-center justify-between gap-3 px-3 py-3 rounded-lg transition-colors hover:bg-secondary/40">
          <div className="space-y-0.5 min-w-0">
            <div className="text-sm font-medium">{t("settings.chatDisplayMode")}</div>
            <div className="text-xs text-muted-foreground">{t("settings.chatDisplayModeDesc")}</div>
          </div>
          <Select value={mode} onValueChange={(value) => void updateMode(value as ChatDisplayMode)}>
            <SelectTrigger className="h-8 w-[128px] shrink-0 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="bubble">{t("settings.chatDisplayModeBubble")}</SelectItem>
              <SelectItem value="timeline">{t("settings.chatDisplayModeTimeline")}</SelectItem>
            </SelectContent>
          </Select>
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
      <SidebarDisplayModeSelector />
      <ChatDisplayModeSelector />
      <UiEffectsToggle />
    </>
  )
}

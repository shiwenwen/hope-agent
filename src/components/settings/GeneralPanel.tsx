import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { useTheme, type ThemeMode } from "@/hooks/useTheme"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage } from "@/i18n/i18n"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem, SelectSeparator } from "@/components/ui/select"
import { Monitor, Sun, Moon, Check } from "lucide-react"

const THEME_OPTIONS: { mode: ThemeMode; icon: React.ReactNode; labelKey: string; descKey: string }[] = [
  { mode: "auto", icon: <Monitor className="h-5 w-5" />, labelKey: "theme.auto", descKey: "theme.autoDesc" },
  { mode: "light", icon: <Sun className="h-5 w-5" />, labelKey: "theme.light", descKey: "theme.lightDesc" },
  { mode: "dark", icon: <Moon className="h-5 w-5" />, labelKey: "theme.dark", descKey: "theme.darkDesc" },
]

export default function GeneralPanel() {
  const { t, i18n } = useTranslation()
  const { theme, setTheme } = useTheme()

  // ── Language state ──
  const [followSystem, setFollowSystem] = useState(isFollowingSystem)

  const isCurrentLang = (code: string) => {
    if (followSystem) return false
    return (
      i18n.language === code ||
      (i18n.language.startsWith(code + "-") && code !== "zh")
    )
  }

  const handleFollowSystem = () => {
    setFollowSystemLanguage()
    setFollowSystem(true)
  }

  const handleSelectLanguage = (code: string) => {
    i18n.changeLanguage(code)
    setFollowSystem(false)
  }

  // ── System state ──
  const [autostart, setAutostart] = useState(false)
  const [autostartLoaded, setAutostartLoaded] = useState(false)

  useEffect(() => {
    invoke<boolean>("get_autostart_enabled")
      .then((enabled) => {
        setAutostart(enabled)
        setAutostartLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "GeneralPanel::load", "Failed to get autostart status", e)
        setAutostartLoaded(true)
      })
  }, [])

  async function toggleAutostart() {
    const next = !autostart
    setAutostart(next)
    try {
      await invoke("set_autostart_enabled", { enabled: next })
    } catch (e) {
      setAutostart(!next)
      logger.error("settings", "GeneralPanel::toggle", "Failed to set autostart", e)
    }
  }

  return (
    <div className="flex-1 overflow-y-auto p-6 max-w-4xl">
      {/* ── Appearance ── */}
      <h3 className="text-sm font-semibold text-foreground mb-1">
        {t("settings.appearance")}
      </h3>
      <p className="text-xs text-muted-foreground mb-3">
        {t("settings.appearanceDesc")}
      </p>
      <div className="space-y-1 mb-8">
        {THEME_OPTIONS.map((opt) => (
          <button
            key={opt.mode}
            className={cn(
              "flex items-center gap-3 w-full px-3 py-3 rounded-lg text-sm transition-colors",
              theme === opt.mode
                ? "bg-primary/10 text-primary font-medium"
                : "text-foreground hover:bg-secondary/60"
            )}
            onClick={() => setTheme(opt.mode)}
          >
            <span
              className={cn(
                "shrink-0",
                theme === opt.mode ? "text-primary" : "text-muted-foreground"
              )}
            >
              {opt.icon}
            </span>
            <div className="flex-1 text-left">
              <div>{t(opt.labelKey)}</div>
              <div className="text-xs text-muted-foreground font-normal">
                {t(opt.descKey)}
              </div>
            </div>
            {theme === opt.mode && (
              <Check className="h-4 w-4 text-primary shrink-0" />
            )}
          </button>
        ))}
      </div>

      {/* ── Language ── */}
      <h3 className="text-sm font-semibold text-foreground mb-1">
        {t("settings.language")}
      </h3>
      <p className="text-xs text-muted-foreground mb-3">
        {t("settings.languageDesc")}
      </p>
      <div className="mb-8">
        <Select
          value={followSystem ? "system" : (SUPPORTED_LANGUAGES.find((l) => i18n.language === l.code || i18n.language.startsWith(l.code + "-"))?.code ?? "system")}
          onValueChange={(val) => {
            if (val === "system") {
              handleFollowSystem()
            } else {
              handleSelectLanguage(val)
            }
          }}
        >
          <SelectTrigger className="w-full max-w-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="system">
              {t("language.system")}
            </SelectItem>
            <SelectSeparator />
            {SUPPORTED_LANGUAGES.map((lang) => (
              <SelectItem key={lang.code} value={lang.code}>
                {lang.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* ── System ── */}
      <h3 className="text-sm font-semibold text-foreground mb-1">
        {t("settings.system")}
      </h3>
      {autostartLoaded && (
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
      )}
    </div>
  )
}

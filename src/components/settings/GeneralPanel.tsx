import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { useTheme, type ThemeMode } from "@/hooks/useTheme"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage, setLanguage } from "@/i18n/i18n"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
  SelectSeparator,
} from "@/components/ui/select"
import { Monitor, Sun, Moon, Check, Loader2, Wifi, Globe, WifiOff, Settings2 } from "lucide-react"

const THEME_OPTIONS: {
  mode: ThemeMode
  icon: React.ReactNode
  labelKey: string
  descKey: string
}[] = [
  {
    mode: "auto",
    icon: <Monitor className="h-5 w-5" />,
    labelKey: "theme.auto",
    descKey: "theme.autoDesc",
  },
  {
    mode: "light",
    icon: <Sun className="h-5 w-5" />,
    labelKey: "theme.light",
    descKey: "theme.lightDesc",
  },
  {
    mode: "dark",
    icon: <Moon className="h-5 w-5" />,
    labelKey: "theme.dark",
    descKey: "theme.darkDesc",
  },
]

interface ProxyConfig {
  mode: "system" | "none" | "custom"
  url: string | null
}

const DEFAULT_PROXY: ProxyConfig = { mode: "system", url: null }

export default function GeneralPanel() {
  const { t, i18n } = useTranslation()
  const { theme, setTheme } = useTheme()

  // ── Language state ──
  const [followSystem, setFollowSystem] = useState(isFollowingSystem)

  const handleFollowSystem = () => {
    setFollowSystemLanguage()
    setFollowSystem(true)
  }

  const handleSelectLanguage = (code: string) => {
    setLanguage(code)
    setFollowSystem(false)
  }

  // ── System state ──
  const [autostart, setAutostart] = useState(false)
  const [autostartLoaded, setAutostartLoaded] = useState(false)

  // ── Proxy state ──
  const [proxy, setProxy] = useState<ProxyConfig>(DEFAULT_PROXY)
  const [proxySaved, setProxySaved] = useState("")
  const [proxySaving, setProxySaving] = useState(false)
  const [proxySaveStatus, setProxySaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [proxyTesting, setProxyTesting] = useState(false)
  const [proxyTestResult, setProxyTestResult] = useState<{ ok: boolean; msg: string } | null>(null)

  const proxyDirty = JSON.stringify(proxy) !== proxySaved

  useEffect(() => {
    let cancelled = false
    Promise.all([
      invoke<boolean>("get_autostart_enabled"),
      invoke<ProxyConfig>("get_proxy_config"),
    ])
      .then(([enabled, cfg]) => {
        if (cancelled) return
        setAutostart(enabled)
        setAutostartLoaded(true)
        setProxy(cfg)
        setProxySaved(JSON.stringify(cfg))
      })
      .catch((e) => {
        logger.error("settings", "GeneralPanel::load", "Failed to load settings", e)
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
      logger.error("settings", "GeneralPanel::toggle", "Failed to set autostart", e)
    }
  }

  const saveProxy = useCallback(async () => {
    setProxySaving(true)
    try {
      await invoke("save_proxy_config", { config: proxy })
      setProxySaved(JSON.stringify(proxy))
      setProxySaveStatus("saved")
      setTimeout(() => setProxySaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "GeneralPanel::saveProxy", "Failed to save proxy config", e)
      setProxySaveStatus("failed")
      setTimeout(() => setProxySaveStatus("idle"), 2000)
    } finally {
      setProxySaving(false)
    }
  }, [proxy])

  const testProxy = useCallback(async () => {
    setProxyTesting(true)
    setProxyTestResult(null)
    try {
      const msg = await invoke<string>("test_proxy", { config: proxy })
      setProxyTestResult({ ok: true, msg })
    } catch (e) {
      setProxyTestResult({ ok: false, msg: String(e) })
    } finally {
      setProxyTesting(false)
    }
  }, [proxy])

  const proxyModeOptions: { value: ProxyConfig["mode"]; icon: React.ReactNode; label: string; desc: string }[] = [
    { value: "system", icon: <Globe className="h-4 w-4" />, label: t("settings.proxyModeSystem"), desc: t("settings.proxyModeSystemDesc") },
    { value: "none", icon: <WifiOff className="h-4 w-4" />, label: t("settings.proxyModeNone"), desc: t("settings.proxyModeNoneDesc") },
    { value: "custom", icon: <Settings2 className="h-4 w-4" />, label: t("settings.proxyModeCustom"), desc: t("settings.proxyModeCustomDesc") },
  ]

  return (
    <div className="flex-1 overflow-y-auto p-6 max-w-4xl">
      {/* ── Appearance ── */}
      <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.appearance")}</h3>
      <p className="text-xs text-muted-foreground mb-3">{t("settings.appearanceDesc")}</p>
      <div className="space-y-1 mb-8">
        {THEME_OPTIONS.map((opt) => (
          <button
            key={opt.mode}
            className={cn(
              "flex items-center gap-3 w-full px-3 py-3 rounded-lg text-sm transition-colors",
              theme === opt.mode
                ? "bg-primary/10 text-primary font-medium"
                : "text-foreground hover:bg-secondary/60",
            )}
            onClick={() => setTheme(opt.mode)}
          >
            <span
              className={cn(
                "shrink-0",
                theme === opt.mode ? "text-primary" : "text-muted-foreground",
              )}
            >
              {opt.icon}
            </span>
            <div className="flex-1 text-left">
              <div>{t(opt.labelKey)}</div>
              <div className="text-xs text-muted-foreground font-normal">{t(opt.descKey)}</div>
            </div>
            {theme === opt.mode && <Check className="h-4 w-4 text-primary shrink-0" />}
          </button>
        ))}
      </div>

      {/* ── Language ── */}
      <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.language")}</h3>
      <p className="text-xs text-muted-foreground mb-3">{t("settings.languageDesc")}</p>
      <div className="mb-8">
        <Select
          value={
            followSystem
              ? "system"
              : (SUPPORTED_LANGUAGES.find(
                  (l) => i18n.language === l.code || i18n.language.startsWith(l.code + "-"),
                )?.code ?? "system")
          }
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
            <SelectItem value="system">{t("language.system")}</SelectItem>
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
      <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.system")}</h3>
      {autostartLoaded && (
        <div className="space-y-4 mb-8">
          <div
            className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
            onClick={toggleAutostart}
          >
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.systemAutostart")}</div>
              <div className="text-xs text-muted-foreground">
                {t("settings.systemAutostartDesc")}
              </div>
            </div>
            <Switch checked={autostart} onCheckedChange={toggleAutostart} />
          </div>
        </div>
      )}

      {/* ── Proxy ── */}
      <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.proxySettings")}</h3>
      <p className="text-xs text-muted-foreground mb-3">{t("settings.proxySettingsDesc")}</p>
      <div className="space-y-3">
        {/* Mode selector */}
        <div className="space-y-1.5">
          {proxyModeOptions.map((opt) => (
            <div
              key={opt.value}
              className={cn(
                "flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer transition-colors",
                proxy.mode === opt.value
                  ? "bg-primary/10 border border-primary/30"
                  : "hover:bg-secondary/40 border border-transparent"
              )}
              onClick={() => setProxy((p) => ({ ...p, mode: opt.value }))}
            >
              <div className={cn(
                "shrink-0",
                proxy.mode === opt.value ? "text-primary" : "text-muted-foreground"
              )}>
                {opt.icon}
              </div>
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium">{opt.label}</div>
                <div className="text-xs text-muted-foreground">{opt.desc}</div>
              </div>
              <div className={cn(
                "h-4 w-4 rounded-full border-2 shrink-0 transition-colors",
                proxy.mode === opt.value
                  ? "border-primary bg-primary"
                  : "border-muted-foreground/30"
              )}>
                {proxy.mode === opt.value && (
                  <div className="h-full w-full flex items-center justify-center">
                    <div className="h-1.5 w-1.5 rounded-full bg-primary-foreground" />
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>

        {/* Custom proxy URL input */}
        {proxy.mode === "custom" && (
          <div className="space-y-1.5">
            <span className="text-xs text-muted-foreground">{t("settings.proxyUrl")}</span>
            <Input
              value={proxy.url ?? ""}
              placeholder={t("settings.proxyUrlPlaceholder")}
              onChange={(e) => setProxy((p) => ({ ...p, url: e.target.value || null }))}
            />
          </div>
        )}

        {/* Save + Test buttons */}
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            onClick={saveProxy}
            disabled={(!proxyDirty && proxySaveStatus === "idle") || proxySaving}
            className={cn(
              proxySaveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
              proxySaveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
            )}
          >
            {proxySaving ? (
              <span className="flex items-center gap-1.5">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("common.saving")}
              </span>
            ) : proxySaveStatus === "saved" ? (
              <span className="flex items-center gap-1.5">
                <Check className="h-3.5 w-3.5" />
                {t("common.saved")}
              </span>
            ) : proxySaveStatus === "failed" ? (
              t("common.saveFailed")
            ) : (
              t("common.save")
            )}
          </Button>

          <Button
            variant="secondary"
            size="sm"
            disabled={proxyTesting || (proxy.mode === "custom" && !proxy.url?.trim())}
            onClick={testProxy}
          >
            {proxyTesting ? (
              <span className="flex items-center gap-1.5">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("common.testing")}
              </span>
            ) : (
              <span className="flex items-center gap-1.5">
                <Wifi className="h-3.5 w-3.5" />
                {t("common.test")}
              </span>
            )}
          </Button>
        </div>

        {/* Test result */}
        {proxyTestResult && (
          <div className={cn(
            "px-3 py-2 rounded-md text-xs",
            proxyTestResult.ok
              ? "bg-green-500/10 text-green-600"
              : "bg-destructive/10 text-destructive"
          )}>
            <div className="font-medium">
              {proxyTestResult.ok ? t("settings.proxyTestSuccess") : t("settings.proxyTestFailed")}
            </div>
            <pre className="mt-1 whitespace-pre-wrap break-all opacity-80">{proxyTestResult.msg}</pre>
          </div>
        )}
      </div>
    </div>
  )
}

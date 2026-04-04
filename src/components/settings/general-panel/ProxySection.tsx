import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { logger } from "@/lib/logger"
import { Globe, WifiOff, Settings2, Wifi, Check, Loader2 } from "lucide-react"

interface ProxyConfig {
  mode: "system" | "none" | "custom"
  url: string | null
}

const DEFAULT_PROXY: ProxyConfig = { mode: "system", url: null }

export default function ProxySection() {
  const { t } = useTranslation()

  const [proxy, setProxy] = useState<ProxyConfig>(DEFAULT_PROXY)
  const [proxySaved, setProxySaved] = useState("")
  const [proxySaving, setProxySaving] = useState(false)
  const [proxySaveStatus, setProxySaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [proxyTesting, setProxyTesting] = useState(false)
  const [proxyTestResult, setProxyTestResult] = useState<{ ok: boolean; msg: string } | null>(null)

  const proxyDirty = JSON.stringify(proxy) !== proxySaved

  useEffect(() => {
    let cancelled = false
    invoke<ProxyConfig>("get_proxy_config")
      .then((cfg) => {
        if (cancelled) return
        setProxy(cfg)
        setProxySaved(JSON.stringify(cfg))
      })
      .catch((e) => {
        logger.error("settings", "ProxySection::load", "Failed to load proxy config", e)
      })
    return () => { cancelled = true }
  }, [])

  const saveProxy = useCallback(async () => {
    setProxySaving(true)
    try {
      await invoke("save_proxy_config", { config: proxy })
      setProxySaved(JSON.stringify(proxy))
      setProxySaveStatus("saved")
      setTimeout(() => setProxySaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "ProxySection::saveProxy", "Failed to save proxy config", e)
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
    <div className="max-w-4xl pt-4">
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
                  : "hover:bg-secondary/40 border border-transparent",
              )}
              onClick={() => setProxy((p) => ({ ...p, mode: opt.value }))}
            >
              <div className={cn("shrink-0", proxy.mode === opt.value ? "text-primary" : "text-muted-foreground")}>
                {opt.icon}
              </div>
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium">{opt.label}</div>
                <div className="text-xs text-muted-foreground">{opt.desc}</div>
              </div>
              <div className={cn(
                "h-4 w-4 rounded-full border-2 shrink-0 transition-colors",
                proxy.mode === opt.value ? "border-primary bg-primary" : "border-muted-foreground/30",
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
            proxyTestResult.ok ? "bg-green-500/10 text-green-600" : "bg-destructive/10 text-destructive",
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

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Check } from "lucide-react"

// ── Types ────────────────────────────────────────────────────────

interface WebFetchConfig {
  maxChars: number
  maxCharsCap: number
  maxResponseBytes: number
  maxRedirects: number
  timeoutSeconds: number
  cacheTtlMinutes: number
  userAgent: string
  ssrfProtection: boolean
}

const DEFAULT_CONFIG: WebFetchConfig = {
  maxChars: 50000,
  maxCharsCap: 200000,
  maxResponseBytes: 2097152,
  maxRedirects: 5,
  timeoutSeconds: 30,
  cacheTtlMinutes: 15,
  userAgent: "",
  ssrfProtection: true,
}

const DEFAULT_USER_AGENT = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"

export default function WebFetchPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<WebFetchConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState<string>("")
  const [saved, setSaved] = useState(false)

  const isDirty = JSON.stringify(config) !== savedSnapshot

  useEffect(() => {
    let cancelled = false
    invoke<WebFetchConfig>("get_web_fetch_config").then(cfg => {
      if (!cancelled) {
        setConfig(cfg)
        setSavedSnapshot(JSON.stringify(cfg))
      }
    }).catch(e => {
      logger.error("settings", `Failed to load web fetch config: ${e}`)
    })
    return () => { cancelled = true }
  }, [])

  const save = async () => {
    try {
      await invoke("save_web_fetch_config", { config })
      setSavedSnapshot(JSON.stringify(config))
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
    } catch (e) {
      logger.error("settings", `Failed to save web fetch config: ${e}`)
    }
  }

  const updateNumber = (key: keyof WebFetchConfig, value: string) => {
    const num = parseInt(value, 10)
    if (!isNaN(num) && num >= 0) {
      setConfig(prev => ({ ...prev, [key]: num }))
    }
  }

  const bytesToMB = (bytes: number) => (bytes / 1048576).toFixed(1)
  const mbToBytes = (mb: string) => {
    const num = parseFloat(mb)
    if (!isNaN(num) && num > 0) return Math.round(num * 1048576)
    return config.maxResponseBytes
  }

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-6">
      {/* Header */}
      <div>
        <p className="text-xs text-muted-foreground">{t("settings.webFetchDesc")}</p>
      </div>

      {/* Content Limits */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
          {t("settings.webFetchSectionLimits")}
        </h3>

        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-1.5">
            <span className="text-sm font-medium">{t("settings.webFetchMaxChars")}</span>
            <Input
              type="number"
              min={1000}
              value={config.maxChars}
              onChange={e => updateNumber("maxChars", e.target.value)}
            />
            <p className="text-xs text-muted-foreground">{t("settings.webFetchMaxCharsDesc")}</p>
          </div>

          <div className="space-y-1.5">
            <span className="text-sm font-medium">{t("settings.webFetchMaxCharsCap")}</span>
            <Input
              type="number"
              min={1000}
              value={config.maxCharsCap}
              onChange={e => updateNumber("maxCharsCap", e.target.value)}
            />
            <p className="text-xs text-muted-foreground">{t("settings.webFetchMaxCharsCapDesc")}</p>
          </div>

          <div className="space-y-1.5">
            <span className="text-sm font-medium">{t("settings.webFetchMaxResponseBytes")}</span>
            <Input
              type="number"
              min={0.1}
              step={0.1}
              value={bytesToMB(config.maxResponseBytes)}
              onChange={e => setConfig(prev => ({ ...prev, maxResponseBytes: mbToBytes(e.target.value) }))}
            />
            <p className="text-xs text-muted-foreground">{t("settings.webFetchMaxResponseBytesDesc")}</p>
          </div>
        </div>
      </div>

      {/* Network */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
          {t("settings.webFetchSectionNetwork")}
        </h3>

        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-1.5">
            <span className="text-sm font-medium">{t("settings.webFetchTimeout")}</span>
            <Input
              type="number"
              min={1}
              max={120}
              value={config.timeoutSeconds}
              onChange={e => updateNumber("timeoutSeconds", e.target.value)}
            />
          </div>

          <div className="space-y-1.5">
            <span className="text-sm font-medium">{t("settings.webFetchMaxRedirects")}</span>
            <Input
              type="number"
              min={0}
              max={20}
              value={config.maxRedirects}
              onChange={e => updateNumber("maxRedirects", e.target.value)}
            />
          </div>
        </div>

        <div className="space-y-1.5">
          <span className="text-sm font-medium">{t("settings.webFetchUserAgent")}</span>
          <Input
            value={config.userAgent}
            placeholder={DEFAULT_USER_AGENT}
            onChange={e => setConfig(prev => ({ ...prev, userAgent: e.target.value }))}
          />
        </div>
      </div>

      {/* Cache */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
          {t("settings.webFetchSectionCache")}
        </h3>

        <div className="space-y-1.5">
          <span className="text-sm font-medium">{t("settings.webFetchCacheTtl")}</span>
          <Input
            type="number"
            min={0}
            max={1440}
            value={config.cacheTtlMinutes}
            onChange={e => updateNumber("cacheTtlMinutes", e.target.value)}
            className="max-w-32"
          />
          <p className="text-xs text-muted-foreground">{t("settings.webFetchCacheTtlDesc")}</p>
        </div>
      </div>

      {/* Security */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
          {t("settings.webFetchSectionSecurity")}
        </h3>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <span className="text-sm font-medium">{t("settings.webFetchSsrf")}</span>
            <p className="text-xs text-muted-foreground">{t("settings.webFetchSsrfDesc")}</p>
          </div>
          <Switch
            checked={config.ssrfProtection}
            onCheckedChange={v => setConfig(prev => ({ ...prev, ssrfProtection: v }))}
          />
        </div>
      </div>

      {/* Save button */}
      <div className="flex items-center gap-2 pt-2">
        <Button onClick={save} disabled={!isDirty}>
          {saved ? (
            <>
              <Check className="h-4 w-4 mr-1" />
              {t("settings.webFetchSaved")}
            </>
          ) : (
            t("common.save")
          )}
        </Button>
      </div>
      </div>
    </div>
  )
}

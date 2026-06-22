import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"
import { Check, Loader2 } from "lucide-react"

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

const DEFAULT_USER_AGENT =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"

export default function WebFetchPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<WebFetchConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState<string>("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const isDirty = JSON.stringify(config) !== savedSnapshot

  useEffect(() => {
    let cancelled = false
    getTransport().call<WebFetchConfig>("get_web_fetch_config")
      .then((cfg) => {
        if (!cancelled) {
          setConfig(cfg)
          setSavedSnapshot(JSON.stringify(cfg))
        }
      })
      .catch((e) => {
        logger.error("settings", "WebFetchPanel", `Failed to load web fetch config: ${e}`)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const save = async () => {
    setSaving(true)
    try {
      await getTransport().call("save_web_fetch_config", { config })
      setSavedSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "WebFetchPanel", `Failed to save web fetch config: ${e}`)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const bytesToMB = (bytes: number) => bytes / 1048576

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
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
              <DeferredNumberInput
                min={1000}
                value={config.maxChars}
                onValueCommit={(value) => setConfig((prev) => ({ ...prev, maxChars: value }))}
              />
              <p className="text-xs text-muted-foreground">{t("settings.webFetchMaxCharsDesc")}</p>
            </div>

            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.webFetchMaxCharsCap")}</span>
              <DeferredNumberInput
                min={1000}
                value={config.maxCharsCap}
                onValueCommit={(value) =>
                  setConfig((prev) => ({ ...prev, maxCharsCap: value }))
                }
              />
              <p className="text-xs text-muted-foreground">
                {t("settings.webFetchMaxCharsCapDesc")}
              </p>
            </div>

            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.webFetchMaxResponseBytes")}</span>
              <DeferredNumberInput
                min={0.1}
                step={0.1}
                value={bytesToMB(config.maxResponseBytes)}
                integer={false}
                onValueCommit={(mb) =>
                  setConfig((prev) => ({ ...prev, maxResponseBytes: Math.round(mb * 1048576) }))
                }
              />
              <p className="text-xs text-muted-foreground">
                {t("settings.webFetchMaxResponseBytesDesc")}
              </p>
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
              <DeferredNumberInput
                min={1}
                max={120}
                value={config.timeoutSeconds}
                onValueCommit={(value) =>
                  setConfig((prev) => ({ ...prev, timeoutSeconds: value }))
                }
              />
            </div>

            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.webFetchMaxRedirects")}</span>
              <DeferredNumberInput
                min={0}
                max={20}
                value={config.maxRedirects}
                onValueCommit={(value) =>
                  setConfig((prev) => ({ ...prev, maxRedirects: value }))
                }
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <span className="text-sm font-medium">{t("settings.webFetchUserAgent")}</span>
            <Input
              value={config.userAgent}
              placeholder={DEFAULT_USER_AGENT}
              onChange={(e) => setConfig((prev) => ({ ...prev, userAgent: e.target.value }))}
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
            <DeferredNumberInput
              min={0}
              max={1440}
              value={config.cacheTtlMinutes}
              onValueCommit={(value) =>
                setConfig((prev) => ({ ...prev, cacheTtlMinutes: value }))
              }
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
              onCheckedChange={(v) => setConfig((prev) => ({ ...prev, ssrfProtection: v }))}
            />
          </div>
        </div>

      </div>
      </div>

      {/* Save — fixed bottom */}
      <div className="shrink-0 flex justify-end px-6 py-3 border-t border-border/30">
        <Button
          onClick={save}
          disabled={(!isDirty && saveStatus === "idle") || saving}
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

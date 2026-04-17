import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Loader2, X } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { cn } from "@/lib/utils"

// ── Types ────────────────────────────────────────────────────────

type SsrfPolicy = "strict" | "default" | "allowPrivate"

interface SsrfConfig {
  defaultPolicy: SsrfPolicy
  trustedHosts: string[]
  browserPolicy: SsrfPolicy | null
  webFetchPolicy: SsrfPolicy | null
  imageGeneratePolicy: SsrfPolicy | null
  urlPreviewPolicy: SsrfPolicy | null
}

const DEFAULT_CONFIG: SsrfConfig = {
  defaultPolicy: "default",
  trustedHosts: [],
  browserPolicy: null,
  webFetchPolicy: null,
  imageGeneratePolicy: null,
  urlPreviewPolicy: null,
}

const INHERIT = "__inherit__"

const POLICY_LABEL_KEYS: Record<SsrfPolicy, string> = {
  strict: "settings.ssrfPolicyStrict",
  default: "settings.ssrfPolicyDefault",
  allowPrivate: "settings.ssrfPolicyAllowPrivate",
}

const POLICY_VALUES: SsrfPolicy[] = ["strict", "default", "allowPrivate"]

export default function SecurityPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<SsrfConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState<string>("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [newHost, setNewHost] = useState("")

  const isDirty = JSON.stringify(config) !== savedSnapshot

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<SsrfConfig>("get_ssrf_config")
      .then((cfg) => {
        if (!cancelled) {
          setConfig(cfg)
          setSavedSnapshot(JSON.stringify(cfg))
        }
      })
      .catch((e) => {
        logger.error("settings", "SecurityPanel", `Failed to load SSRF config: ${e}`)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const save = async () => {
    setSaving(true)
    try {
      await getTransport().call("save_ssrf_config", { config })
      setSavedSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "SecurityPanel", `Failed to save SSRF config: ${e}`)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const toolPolicies = useMemo(
    () =>
      [
        {
          key: "browserPolicy" as const,
          label: t("settings.ssrfBrowserPolicy"),
          desc: t("settings.ssrfBrowserPolicyDesc"),
        },
        {
          key: "webFetchPolicy" as const,
          label: t("settings.ssrfWebFetchPolicy"),
          desc: t("settings.ssrfWebFetchPolicyDesc"),
        },
        {
          key: "imageGeneratePolicy" as const,
          label: t("settings.ssrfImageGeneratePolicy"),
          desc: t("settings.ssrfImageGeneratePolicyDesc"),
        },
        {
          key: "urlPreviewPolicy" as const,
          label: t("settings.ssrfUrlPreviewPolicy"),
          desc: t("settings.ssrfUrlPreviewPolicyDesc"),
        },
      ],
    [t],
  )

  const addHost = () => {
    const trimmed = newHost.trim().toLowerCase()
    if (!trimmed) return
    if (config.trustedHosts.includes(trimmed)) {
      setNewHost("")
      return
    }
    setConfig((prev) => ({
      ...prev,
      trustedHosts: [...prev.trustedHosts, trimmed],
    }))
    setNewHost("")
  }

  const removeHost = (host: string) => {
    setConfig((prev) => ({
      ...prev,
      trustedHosts: prev.trustedHosts.filter((h) => h !== host),
    }))
  }

  const policyOptions = POLICY_VALUES.map((v) => (
    <SelectItem key={v} value={v}>
      {t(POLICY_LABEL_KEYS[v])}
    </SelectItem>
  ))

  const renderPolicySelect = (value: SsrfPolicy, onChange: (v: SsrfPolicy) => void) => (
    <Select value={value} onValueChange={(v) => onChange(v as SsrfPolicy)}>
      <SelectTrigger className="max-w-60">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>{policyOptions}</SelectContent>
    </Select>
  )

  const renderToolPolicySelect = (
    value: SsrfPolicy | null,
    onChange: (v: SsrfPolicy | null) => void,
  ) => (
    <Select
      value={value ?? INHERIT}
      onValueChange={(v) => onChange(v === INHERIT ? null : (v as SsrfPolicy))}
    >
      <SelectTrigger className="max-w-60">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={INHERIT}>{t("settings.ssrfPolicyInherit")}</SelectItem>
        {policyOptions}
      </SelectContent>
    </Select>
  )

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6">
        <div className="space-y-6">
          <div>
            <p className="text-xs text-muted-foreground">{t("settings.ssrfDesc")}</p>
          </div>

          {/* Default policy */}
          <div className="space-y-4">
            <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
              {t("settings.ssrfDefaultSection")}
            </h3>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.ssrfDefaultPolicy")}</span>
              {renderPolicySelect(config.defaultPolicy, (v) =>
                setConfig((prev) => ({ ...prev, defaultPolicy: v })),
              )}
              <p className="text-xs text-muted-foreground">
                {t("settings.ssrfDefaultPolicyDesc")}
              </p>
            </div>
          </div>

          {/* Per-tool policy */}
          <div className="space-y-4">
            <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
              {t("settings.ssrfPerToolSection")}
            </h3>
            {toolPolicies.map((row) => (
              <div key={row.key} className="space-y-1.5">
                <span className="text-sm font-medium">{row.label}</span>
                {renderToolPolicySelect(config[row.key], (v) =>
                  setConfig((prev) => ({ ...prev, [row.key]: v })),
                )}
                <p className="text-xs text-muted-foreground">{row.desc}</p>
              </div>
            ))}
          </div>

          {/* Trusted hosts */}
          <div className="space-y-4">
            <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
              {t("settings.ssrfTrustedHostsSection")}
            </h3>
            <p className="text-xs text-muted-foreground">
              {t("settings.ssrfTrustedHostsDesc")}
            </p>

            <div className="flex gap-2">
              <Input
                value={newHost}
                placeholder={t("settings.ssrfTrustedHostsPlaceholder")}
                onChange={(e) => setNewHost(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault()
                    addHost()
                  }
                }}
                className="max-w-md"
              />
              <Button
                type="button"
                variant="outline"
                onClick={addHost}
                disabled={!newHost.trim()}
              >
                {t("common.add")}
              </Button>
            </div>

            {config.trustedHosts.length === 0 ? (
              <p className="text-xs text-muted-foreground italic">
                {t("settings.ssrfTrustedHostsEmpty")}
              </p>
            ) : (
              <ul className="space-y-1.5 max-w-md">
                {config.trustedHosts.map((host) => (
                  <li
                    key={host}
                    className="flex items-center justify-between px-3 py-1.5 rounded-md bg-secondary/40"
                  >
                    <span className="text-sm font-mono">{host}</span>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={() => removeHost(host)}
                      aria-label={t("common.delete")}
                    >
                      <X className="h-3.5 w-3.5" />
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>

      {/* Save */}
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

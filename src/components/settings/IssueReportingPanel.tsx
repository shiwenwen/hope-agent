import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Check, Loader2 } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { SecretInput } from "@/components/ui/secret-input"
import { Switch } from "@/components/ui/switch"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"

interface IssueLabelsByKind {
  bug: string[]
  feature: string[]
  improvement: string[]
}

interface IssueReportingConfig {
  enabled: boolean
  owner: string
  repo: string
  apiBaseUrl: string
  labelsByKind: IssueLabelsByKind
  maxEvidenceChars: number
  duplicateCheckEnabled: boolean
}

interface IssueReportingStatus {
  config: IssueReportingConfig
  hasToken: boolean
}

const DEFAULT_CONFIG: IssueReportingConfig = {
  enabled: true,
  owner: "shiwenwen",
  repo: "hope-agent",
  apiBaseUrl: "https://api.github.com",
  labelsByKind: {
    bug: ["bug"],
    feature: ["enhancement"],
    improvement: ["improvement"],
  },
  maxEvidenceChars: 24000,
  duplicateCheckEnabled: true,
}

function parseLabels(raw: string): string[] {
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean)
}

function labelsToText(labels: string[]): string {
  return labels.join(", ")
}

export default function IssueReportingPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<IssueReportingConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState("")
  const [hasToken, setHasToken] = useState(false)
  const [token, setToken] = useState("")
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)
  const [tokenSaving, setTokenSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const isDirty = useMemo(() => JSON.stringify(config) !== savedSnapshot, [config, savedSnapshot])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<IssueReportingStatus>("get_issue_reporting_config")
      .then((status) => {
        if (cancelled) return
        const merged = { ...DEFAULT_CONFIG, ...status.config }
        merged.labelsByKind = { ...DEFAULT_CONFIG.labelsByKind, ...status.config.labelsByKind }
        setConfig(merged)
        setSavedSnapshot(JSON.stringify(merged))
        setHasToken(status.hasToken)
        setLoaded(true)
      })
      .catch((e: unknown) => {
        logger.error("settings", "IssueReportingPanel::load", "Failed to load", e)
        setLoaded(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const save = useCallback(async () => {
    setSaving(true)
    try {
      await getTransport().call("save_issue_reporting_config", { config })
      setSavedSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "IssueReportingPanel::save", "Failed to save", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }, [config])

  const saveToken = useCallback(async () => {
    setTokenSaving(true)
    try {
      await getTransport().call("save_issue_reporting_token", {
        token: token.trim() || null,
      })
      setHasToken(Boolean(token.trim()))
      setToken("")
      toast.success(
        token.trim()
          ? t("settings.issueReportingTokenSaved")
          : t("settings.issueReportingTokenCleared"),
      )
    } catch (e) {
      logger.error("settings", "IssueReportingPanel::saveToken", "Failed to save token", e)
      toast.error(t("settings.issueReportingTokenSaveFailed"))
    } finally {
      setTokenSaving(false)
    }
  }, [t, token])

  const testConnection = useCallback(async () => {
    setTesting(true)
    try {
      const result = await getTransport().call<{ hasToken: boolean }>(
        "test_issue_reporting_connection",
      )
      toast.success(
        result.hasToken
          ? t("settings.issueReportingTestSuccessToken")
          : t("settings.issueReportingTestSuccessGh"),
      )
    } catch (e) {
      logger.error("settings", "IssueReportingPanel::test", "Failed to test connection", e)
      toast.error(t("settings.issueReportingTestFailed"))
    } finally {
      setTesting(false)
    }
  }, [t])

  if (!loaded) return null

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-6">
        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.issueReportingTitle")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.issueReportingDesc")}
            </div>
          </div>
          <Switch
            checked={config.enabled}
            onCheckedChange={(enabled) => setConfig((prev) => ({ ...prev, enabled }))}
          />
        </div>

        <div className={cn("space-y-4", !config.enabled && "opacity-50")}>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.issueReportingOwner")}</span>
              <Input
                value={config.owner}
                onChange={(e) => setConfig((prev) => ({ ...prev, owner: e.target.value }))}
              />
            </div>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.issueReportingRepo")}</span>
              <Input
                value={config.repo}
                onChange={(e) => setConfig((prev) => ({ ...prev, repo: e.target.value }))}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("settings.issueReportingApiBaseUrl")}
              </span>
              <Input
                value={config.apiBaseUrl}
                onChange={(e) => setConfig((prev) => ({ ...prev, apiBaseUrl: e.target.value }))}
              />
            </div>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("settings.issueReportingMaxEvidenceChars")}
              </span>
              <Input
                type="number"
                min={1000}
                step={1000}
                value={config.maxEvidenceChars}
                onChange={(e) => {
                  const n = Number(e.target.value)
                  if (Number.isFinite(n)) {
                    setConfig((prev) => ({
                      ...prev,
                      maxEvidenceChars: Math.max(1000, Math.round(n)),
                    }))
                  }
                }}
              />
            </div>
          </div>

          <div className="space-y-3">
            <div className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
              {t("settings.issueReportingLabels")}
            </div>
            <div className="grid grid-cols-3 gap-4">
              {(["bug", "feature", "improvement"] as const).map((kind) => (
                <div key={kind} className="space-y-1.5">
                  <span className="text-sm font-medium">
                    {t(`settings.issueReportingKind.${kind}`)}
                  </span>
                  <Input
                    value={labelsToText(config.labelsByKind[kind])}
                    onChange={(e) =>
                      setConfig((prev) => ({
                        ...prev,
                        labelsByKind: {
                          ...prev.labelsByKind,
                          [kind]: parseLabels(e.target.value),
                        },
                      }))
                    }
                  />
                </div>
              ))}
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.issueReportingDuplicateSearch")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t("settings.issueReportingDuplicateSearchDesc")}
              </div>
            </div>
            <Switch
              checked={config.duplicateCheckEnabled}
              onCheckedChange={(duplicateCheckEnabled) =>
                setConfig((prev) => ({ ...prev, duplicateCheckEnabled }))
              }
            />
          </div>
        </div>

        <div className="space-y-3 border-t border-border pt-5">
          <div className="flex items-center justify-between gap-4">
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.issueReportingToken")}</div>
              <div className="text-xs text-muted-foreground">
                {hasToken
                  ? t("settings.issueReportingTokenConfigured")
                  : t("settings.issueReportingTokenMissing")}
              </div>
            </div>
            <Button variant="outline" size="sm" onClick={testConnection} disabled={testing}>
              {testing && <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />}
              {testing ? t("common.testing") : t("common.test")}
            </Button>
          </div>
          <div className="flex gap-2">
            <SecretInput
              value={token}
              onChange={setToken}
              placeholder={
                hasToken
                  ? t("settings.issueReportingTokenPlaceholderClear")
                  : t("settings.issueReportingTokenPlaceholderNew")
              }
              className="flex-1"
            />
            <Button onClick={saveToken} disabled={tokenSaving}>
              {tokenSaving && <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />}
              {t("settings.issueReportingSaveToken")}
            </Button>
          </div>
        </div>
      </div>

      <div className="sticky bottom-0 bg-background/95 backdrop-blur border-t mt-6 py-3 flex justify-end">
        <Button onClick={save} disabled={!isDirty || saving} size="sm">
          {saving ? (
            <Loader2 className="h-4 w-4 animate-spin mr-1.5" />
          ) : saveStatus === "saved" ? (
            <Check className="h-4 w-4 mr-1.5" />
          ) : null}
          {saveStatus === "saved"
            ? t("common.saved")
            : saveStatus === "failed"
              ? t("common.saveFailed")
              : t("common.save")}
        </Button>
      </div>
    </div>
  )
}

import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { WeatherSection } from "@/components/settings/WeatherSection"
import { cn } from "@/lib/utils"
import { Check, Loader2 } from "lucide-react"

interface UserConfig {
  weatherEnabled?: boolean
  weatherCity?: string | null
  weatherLatitude?: number | null
  weatherLongitude?: number | null
}

interface ToolLimitsConfig {
  maxImages: number
  maxPdfs: number
  maxVisionPages: number
}

type ApprovalTimeoutAction = "deny" | "proceed"

const DEFAULT_LIMITS: ToolLimitsConfig = {
  maxImages: 10,
  maxPdfs: 5,
  maxVisionPages: 10,
}

export default function ToolGeneralPanel() {
  const { t } = useTranslation()
  const [toolTimeout, setToolTimeout] = useState(300)
  const [savedTimeout, setSavedTimeout] = useState(300)
  const [approvalTimeout, setApprovalTimeout] = useState(300)
  const [savedApprovalTimeout, setSavedApprovalTimeout] = useState(300)
  const [approvalTimeoutAction, setApprovalTimeoutAction] = useState<ApprovalTimeoutAction>("deny")
  const [savedApprovalTimeoutAction, setSavedApprovalTimeoutAction] = useState<ApprovalTimeoutAction>("deny")
  const [diskThreshold, setDiskThreshold] = useState(50)
  const [savedDiskThreshold, setSavedDiskThreshold] = useState(50)
  const [deferredToolsEnabled, setDeferredToolsEnabled] = useState(false)

  const [limits, setLimits] = useState<ToolLimitsConfig>(DEFAULT_LIMITS)
  const [savedLimitsSnapshot, setSavedLimitsSnapshot] = useState("")

  const [config, setConfig] = useState<UserConfig>({})
  const [savedConfigSnapshot, setSavedConfigSnapshot] = useState<string>("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const isConfigDirty = JSON.stringify(config) !== savedConfigSnapshot
  const isLimitsDirty = JSON.stringify(limits) !== savedLimitsSnapshot
  const isDirty = isConfigDirty || isLimitsDirty

  useEffect(() => {
    let cancelled = false

    // Load tool timeout
    getTransport().call<number>("get_tool_timeout")
      .then((v) => { if (!cancelled) { setToolTimeout(v); setSavedTimeout(v); } })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load tool timeout", e))

    // Load approval timeout
    getTransport().call<number>("get_approval_timeout")
      .then((v) => { if (!cancelled) { setApprovalTimeout(v); setSavedApprovalTimeout(v); } })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load approval timeout", e))

    // Load approval timeout action
    getTransport().call<ApprovalTimeoutAction>("get_approval_timeout_action")
      .then((v) => { if (!cancelled) { setApprovalTimeoutAction(v); setSavedApprovalTimeoutAction(v); } })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load approval timeout action", e))

    // Load disk persistence threshold (bytes → KB for display)
    getTransport().call<number>("get_tool_result_disk_threshold")
      .then((v) => { if (!cancelled) { const kb = Math.round(v / 1000); setDiskThreshold(kb); setSavedDiskThreshold(kb); } })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load disk threshold", e))

    // Load deferred tools config
    getTransport().call<{ enabled: boolean }>("get_deferred_tools_config")
      .then((cfg) => { if (!cancelled) setDeferredToolsEnabled(cfg?.enabled ?? false) })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load deferred tools config", e))

    // Load user config
    getTransport().call<UserConfig>("get_user_config")
      .then((cfg) => {
        if (!cancelled) {
          setConfig(cfg)
          setSavedConfigSnapshot(JSON.stringify(cfg))
        }
      })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load user config", e))

    // Load tool limits
    getTransport().call<ToolLimitsConfig>("get_tool_limits")
      .then((cfg) => {
        if (!cancelled) {
          setLimits(cfg)
          setSavedLimitsSnapshot(JSON.stringify(cfg))
        }
      })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load tool limits", e))

    return () => { cancelled = true }
  }, [])

  const saveTimeout = useCallback(async (value: number) => {
    try {
      await getTransport().call("set_tool_timeout", { seconds: value })
      setSavedTimeout(value)
    } catch (e) {
      setToolTimeout(savedTimeout)
      logger.error("settings", "ToolGeneralPanel::save", "Failed to save tool timeout", e)
    }
  }, [savedTimeout])

  const saveApprovalTimeout = useCallback(async (value: number) => {
    try {
      await getTransport().call("set_approval_timeout", { seconds: value })
      setSavedApprovalTimeout(value)
    } catch (e) {
      setApprovalTimeout(savedApprovalTimeout)
      logger.error("settings", "ToolGeneralPanel::save", "Failed to save approval timeout", e)
    }
  }, [savedApprovalTimeout])

  const saveApprovalTimeoutAction = useCallback(async (value: ApprovalTimeoutAction) => {
    try {
      await getTransport().call("set_approval_timeout_action", { action: value })
      setSavedApprovalTimeoutAction(value)
    } catch (e) {
      setApprovalTimeoutAction(savedApprovalTimeoutAction)
      logger.error("settings", "ToolGeneralPanel::save", "Failed to save approval timeout action", e)
    }
  }, [savedApprovalTimeoutAction])

  const handleDeferredToolsChange = useCallback(async (enabled: boolean) => {
    setDeferredToolsEnabled(enabled)
    try {
      await getTransport().call("save_deferred_tools_config", { config: { enabled } })
    } catch (e) {
      setDeferredToolsEnabled(!enabled)
      logger.error(
        "settings",
        "ToolGeneralPanel::save",
        "Failed to save deferred tools config",
        e,
      )
    }
  }, [])

  const saveDiskThreshold = useCallback(async (kb: number) => {
    try {
      await getTransport().call("set_tool_result_disk_threshold", { bytes: kb * 1000 })
      setSavedDiskThreshold(kb)
    } catch (e) {
      setDiskThreshold(savedDiskThreshold)
      logger.error("settings", "ToolGeneralPanel::save", "Failed to save disk threshold", e)
    }
  }, [savedDiskThreshold])

  const saveAll = async () => {
    setSaving(true)
    try {
      const promises: Promise<void>[] = []
      if (isConfigDirty) {
        promises.push(getTransport().call("save_user_config", { config }))
      }
      if (isLimitsDirty) {
        promises.push(getTransport().call("set_tool_limits", { config: limits }))
      }
      await Promise.all(promises)
      setSavedConfigSnapshot(JSON.stringify(config))
      setSavedLimitsSnapshot(JSON.stringify(limits))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "ToolGeneralPanel::saveAll", "Failed to save", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const update = (key: string, value: any) => {
    setConfig((prev) => ({ ...prev, [key]: value }))
  }

  const updateLimit = (key: keyof ToolLimitsConfig, value: number) => {
    setLimits((prev) => ({ ...prev, [key]: value }))
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

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.approvalTimeout")}</div>
              <div className="text-xs text-muted-foreground">{t("settings.approvalTimeoutDesc")}</div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={30}
                value={approvalTimeout}
                onChange={(e) => setApprovalTimeout(Number(e.target.value))}
                onBlur={() => {
                  const clamped = Math.max(0, Math.round(approvalTimeout))
                  setApprovalTimeout(clamped)
                  if (clamped !== savedApprovalTimeout) saveApprovalTimeout(clamped)
                }}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.seconds")}</span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.approvalTimeoutAction")}</div>
              <div className="text-xs text-muted-foreground">{t("settings.approvalTimeoutActionDesc")}</div>
            </div>
            <Select
              value={approvalTimeoutAction}
              onValueChange={(value: ApprovalTimeoutAction) => {
                setApprovalTimeoutAction(value)
                void saveApprovalTimeoutAction(value)
              }}
            >
              <SelectTrigger className="w-40 h-8 text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="deny">{t("settings.approvalTimeoutActionDeny")}</SelectItem>
                <SelectItem value="proceed">{t("settings.approvalTimeoutActionProceed")}</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {/* Disk Persistence Threshold */}
          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.toolResultDiskThreshold")}</div>
              <div className="text-xs text-muted-foreground">{t("settings.toolResultDiskThresholdDesc")}</div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={10}
                value={diskThreshold}
                onChange={(e) => setDiskThreshold(Number(e.target.value))}
                onBlur={() => {
                  const clamped = Math.max(0, Math.round(diskThreshold))
                  setDiskThreshold(clamped)
                  if (clamped !== savedDiskThreshold) saveDiskThreshold(clamped)
                }}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.kb")}</span>
            </div>
          </div>

          {/* Deferred Tool Loading */}
          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">{t("settings.deferredToolsEnabled")}</div>
              <div className="text-xs text-muted-foreground">
                {t("settings.deferredToolsEnabledDesc")}
              </div>
            </div>
            <Switch
              checked={deferredToolsEnabled}
              onCheckedChange={handleDeferredToolsChange}
            />
          </div>

          <div className="border-t border-border/50" />

          {/* Tool Limits */}
          <div className="space-y-1 px-3">
            <div className="text-sm font-medium">{t("settings.toolLimits")}</div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.maxImages")}</div>
              <div className="text-xs text-muted-foreground">{t("settings.maxImagesDesc")}</div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={1}
                max={20}
                value={limits.maxImages}
                onChange={(e) => updateLimit("maxImages", Number(e.target.value))}
                onBlur={() => updateLimit("maxImages", Math.max(1, Math.min(20, Math.round(limits.maxImages))))}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.items")}</span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.maxPdfs")}</div>
              <div className="text-xs text-muted-foreground">{t("settings.maxPdfsDesc")}</div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={1}
                max={10}
                value={limits.maxPdfs}
                onChange={(e) => updateLimit("maxPdfs", Number(e.target.value))}
                onBlur={() => updateLimit("maxPdfs", Math.max(1, Math.min(10, Math.round(limits.maxPdfs))))}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.items")}</span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.maxVisionPages")}</div>
              <div className="text-xs text-muted-foreground">{t("settings.maxVisionPagesDesc")}</div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={1}
                max={50}
                value={limits.maxVisionPages}
                onChange={(e) => updateLimit("maxVisionPages", Number(e.target.value))}
                onBlur={() => updateLimit("maxVisionPages", Math.max(1, Math.min(50, Math.round(limits.maxVisionPages))))}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.pages")}</span>
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
          onClick={saveAll}
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

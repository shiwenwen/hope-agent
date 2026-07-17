import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { WeatherSection } from "@/components/settings/WeatherSection"
import { cn } from "@/lib/utils"
import { Check, Loader2 } from "lucide-react"
import {
  toolDisplayDescFallback,
  toolDisplayDescriptionKey,
  toolDisplayNameFallback,
  toolDisplayNameKey,
} from "@/types/tools"

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

interface DeferredToolsConfig {
  enabled: boolean
  mode?: "recommended" | "custom" | "disabled"
  toolNames?: string[]
}

type ModelRuntimeTimeoutOverrides = "allow" | "warn" | "ignore_when_user_unlimited"

interface TimeoutPolicyConfig {
  modelRuntimeOverrides: ModelRuntimeTimeoutOverrides
}

interface BuiltinTool {
  name: string
  description: string
  internal?: boolean
  tier?: "core" | "standard" | "configured" | "memory" | "mcp"
  defer_capable?: boolean
}

type ApprovalTimeoutAction = "deny" | "proceed"

const DEFAULT_LIMITS: ToolLimitsConfig = {
  maxImages: 10,
  maxPdfs: 5,
  maxVisionPages: 10,
}

const DEFAULT_TIMEOUT_POLICY: TimeoutPolicyConfig = {
  modelRuntimeOverrides: "warn",
}

const RECOMMENDED_EAGER_TOOLS = new Set([
  "tool_search",
  "ask_user_question",
  "runtime_cancel",
  "skill",
  "read",
  "grep",
  "exec",
  "apply_patch",
  "job_status",
  "note_read",
  "note_search",
  "note_create",
  "note_patch",
])

export default function ToolGeneralPanel() {
  const { t, i18n } = useTranslation()
  const [toolTimeout, setToolTimeout] = useState(0)
  const [savedTimeout, setSavedTimeout] = useState(0)
  const [timeoutPolicy, setTimeoutPolicy] = useState<ModelRuntimeTimeoutOverrides>(
    DEFAULT_TIMEOUT_POLICY.modelRuntimeOverrides,
  )
  const [savedTimeoutPolicy, setSavedTimeoutPolicy] = useState<ModelRuntimeTimeoutOverrides>(
    DEFAULT_TIMEOUT_POLICY.modelRuntimeOverrides,
  )
  const [approvalTimeoutEnabled, setApprovalTimeoutEnabled] = useState(false)
  const [savedApprovalTimeoutEnabled, setSavedApprovalTimeoutEnabled] = useState(false)
  const [approvalTimeout, setApprovalTimeout] = useState(300)
  const [savedApprovalTimeout, setSavedApprovalTimeout] = useState(300)
  const [approvalTimeoutAction, setApprovalTimeoutAction] = useState<ApprovalTimeoutAction>("deny")
  const [savedApprovalTimeoutAction, setSavedApprovalTimeoutAction] = useState<ApprovalTimeoutAction>("deny")
  const [diskThreshold, setDiskThreshold] = useState(50)
  const [savedDiskThreshold, setSavedDiskThreshold] = useState(50)
  const [deferredToolsEnabled, setDeferredToolsEnabled] = useState(false)
  const [deferredToolsMode, setDeferredToolsMode] = useState<DeferredToolsConfig["mode"]>("disabled")
  const [deferredToolNames, setDeferredToolNames] = useState<string[]>([])
  const [builtinTools, setBuiltinTools] = useState<BuiltinTool[]>([])

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
    if (deferredToolsMode !== "recommended" || builtinTools.length === 0) return
    setDeferredToolNames(
      builtinTools
        .filter((tool) => tool.defer_capable && !RECOMMENDED_EAGER_TOOLS.has(tool.name))
        .map((tool) => tool.name),
    )
  }, [builtinTools, deferredToolsMode])

  useEffect(() => {
    let cancelled = false

    // Load tool timeout
    getTransport().call<number>("get_tool_timeout")
      .then((v) => { if (!cancelled) { setToolTimeout(v); setSavedTimeout(v); } })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load tool timeout", e))

    // Load model runtime timeout policy
    getTransport().call<TimeoutPolicyConfig>("get_timeout_policy_config")
      .then((cfg) => {
        if (!cancelled) {
          const value = cfg?.modelRuntimeOverrides ?? DEFAULT_TIMEOUT_POLICY.modelRuntimeOverrides
          setTimeoutPolicy(value)
          setSavedTimeoutPolicy(value)
        }
      })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load timeout policy", e))

    // Load approval timeout
    getTransport().call<boolean>("get_approval_timeout_enabled")
      .then((v) => { if (!cancelled) { setApprovalTimeoutEnabled(v); setSavedApprovalTimeoutEnabled(v); } })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load approval timeout enabled", e))

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
    getTransport().call<DeferredToolsConfig>("get_deferred_tools_config")
      .then((cfg) => {
        if (!cancelled) {
          const mode = cfg?.mode ?? ((cfg?.enabled ?? false) ? "custom" : "disabled")
          setDeferredToolsEnabled(mode !== "disabled")
          setDeferredToolsMode(mode)
          setDeferredToolNames(cfg?.toolNames ?? [])
        }
      })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load deferred tools config", e))

    getTransport().call<BuiltinTool[]>("list_builtin_tools")
      .then((tools) => { if (!cancelled) setBuiltinTools(tools) })
      .catch((e) => logger.error("settings", "ToolGeneralPanel::load", "Failed to load built-in tools", e))

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

  const saveTimeoutPolicy = useCallback(async (value: ModelRuntimeTimeoutOverrides) => {
    try {
      await getTransport().call("save_timeout_policy_config", {
        config: { modelRuntimeOverrides: value },
      })
      setSavedTimeoutPolicy(value)
    } catch (e) {
      setTimeoutPolicy(savedTimeoutPolicy)
      logger.error("settings", "ToolGeneralPanel::save", "Failed to save timeout policy", e)
    }
  }, [savedTimeoutPolicy])

  const saveApprovalTimeout = useCallback(async (value: number) => {
    try {
      await getTransport().call("set_approval_timeout", { seconds: value })
      setSavedApprovalTimeout(value)
    } catch (e) {
      setApprovalTimeout(savedApprovalTimeout)
      logger.error("settings", "ToolGeneralPanel::save", "Failed to save approval timeout", e)
    }
  }, [savedApprovalTimeout])

  const saveApprovalTimeoutEnabled = useCallback(async (enabled: boolean) => {
    try {
      const nextTimeout = enabled && approvalTimeout <= 0 ? 300 : approvalTimeout
      await Promise.all([
        getTransport().call("set_approval_timeout_enabled", { enabled }),
        nextTimeout !== approvalTimeout
          ? getTransport().call("set_approval_timeout", { seconds: nextTimeout })
          : Promise.resolve(),
      ])
      setSavedApprovalTimeoutEnabled(enabled)
      if (nextTimeout !== approvalTimeout) {
        setApprovalTimeout(nextTimeout)
        setSavedApprovalTimeout(nextTimeout)
      }
    } catch (e) {
      setApprovalTimeoutEnabled(savedApprovalTimeoutEnabled)
      logger.error("settings", "ToolGeneralPanel::save", "Failed to save approval timeout enabled", e)
    }
  }, [approvalTimeout, savedApprovalTimeoutEnabled])

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
    const previousMode = deferredToolsMode
    const mode: DeferredToolsConfig["mode"] = enabled ? "recommended" : "disabled"
    setDeferredToolsMode(mode)
    try {
      await getTransport().call("save_deferred_tools_config", {
        config: { mode, enabled, toolNames: deferredToolNames },
      })
    } catch (e) {
      setDeferredToolsEnabled(!enabled)
      setDeferredToolsMode(previousMode)
      logger.error(
        "settings",
        "ToolGeneralPanel::save",
        "Failed to save deferred tools config",
        e,
      )
    }
  }, [deferredToolNames, deferredToolsMode])

  const handleDeferredToolToggle = useCallback(async (name: string, enabled: boolean) => {
    const previous = deferredToolNames
    const next = enabled
      ? [...previous.filter((n) => n !== name), name]
      : previous.filter((n) => n !== name)
    setDeferredToolNames(next)
    const previousMode = deferredToolsMode
    setDeferredToolsMode("custom")
    try {
      await getTransport().call("save_deferred_tools_config", {
        config: { mode: "custom", enabled: deferredToolsEnabled, toolNames: next },
      })
    } catch (e) {
      setDeferredToolNames(previous)
      setDeferredToolsMode(previousMode)
      logger.error(
        "settings",
        "ToolGeneralPanel::save",
        "Failed to save deferred tool list",
        e,
      )
    }
  }, [deferredToolNames, deferredToolsEnabled, deferredToolsMode])

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

  const update = (key: string, value: unknown) => {
    setConfig((prev) => ({ ...prev, [key]: value }))
  }

  const updateLimit = (key: keyof ToolLimitsConfig, value: number) => {
    setLimits((prev) => ({ ...prev, [key]: value }))
  }

  const deferCapableTools = builtinTools.filter((tool) => tool.defer_capable)
  const toolDisplayName = (name: string) => {
    const key = toolDisplayNameKey(name)
    return i18n.exists(key) ? t(key) : toolDisplayNameFallback(name, i18n.language)
  }
  const toolDisplayDesc = (tool: BuiltinTool) => {
    const key = toolDisplayDescriptionKey(tool.name)
    return i18n.exists(key)
      ? t(key)
      : toolDisplayDescFallback(tool.name, tool.description, i18n.language)
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
              <DeferredNumberInput
                min={0}
                step={30}
                value={toolTimeout}
                onValueCommit={(value) => {
                  const clamped = Math.max(0, Math.round(value))
                  setToolTimeout(clamped)
                  if (clamped !== savedTimeout) saveTimeout(clamped)
                }}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.seconds")}</span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">{t("settings.timeoutPolicy")}</div>
              <div className="text-xs text-muted-foreground">{t("settings.timeoutPolicyDesc")}</div>
            </div>
            <Select
              value={timeoutPolicy}
              onValueChange={(value: ModelRuntimeTimeoutOverrides) => {
                setTimeoutPolicy(value)
                void saveTimeoutPolicy(value)
              }}
            >
              <SelectTrigger className="w-56 h-8 text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="warn">{t("settings.timeoutPolicyWarn")}</SelectItem>
                <SelectItem value="ignore_when_user_unlimited">
                  {t("settings.timeoutPolicyIgnoreWhenUserUnlimited")}
                </SelectItem>
                <SelectItem value="allow">{t("settings.timeoutPolicyAllow")}</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5">
              <div className="text-sm font-medium">{t("settings.approvalTimeout")}</div>
              <div className="text-xs text-muted-foreground">{t("settings.approvalTimeoutDesc")}</div>
            </div>
            <div className="flex items-center gap-3">
              <Switch
                checked={approvalTimeoutEnabled}
                onCheckedChange={(checked) => {
                  setApprovalTimeoutEnabled(checked)
                  void saveApprovalTimeoutEnabled(checked)
                }}
              />
              <DeferredNumberInput
                min={0}
                step={30}
                value={approvalTimeout}
                onValueCommit={(value) => {
                  const clamped = Math.max(0, Math.round(value))
                  setApprovalTimeout(clamped)
                  if (clamped !== savedApprovalTimeout) saveApprovalTimeout(clamped)
                }}
                disabled={!approvalTimeoutEnabled}
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
              disabled={!approvalTimeoutEnabled}
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
              <DeferredNumberInput
                min={0}
                step={10}
                value={diskThreshold}
                onValueCommit={(value) => {
                  const clamped = Math.max(0, Math.round(value))
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

          {deferredToolsEnabled && deferCapableTools.length > 0 && (
            <div className="mx-3 rounded-lg border border-border/50 overflow-hidden">
              <div className="px-3 py-2 border-b border-border/40 bg-secondary/20">
                <div className="text-xs font-medium text-muted-foreground">
                  {t("settings.deferredToolsPerToolTitle")}
                </div>
                <div className="text-[11px] text-muted-foreground/60 mt-0.5">
                  {t("settings.deferredToolsPerToolDesc")}
                </div>
              </div>
              {deferCapableTools.map((tool, idx) => (
                <div
                  key={tool.name}
                  className={cn(
                    "flex items-center justify-between px-3 py-2 gap-3",
                    idx > 0 && "border-t border-border/30",
                  )}
                >
                  <div className="min-w-0 flex-1">
                    <div className="text-xs font-medium text-foreground">
                      {toolDisplayName(tool.name)}
                    </div>
                    <div className="text-[11px] text-muted-foreground/60 line-clamp-1">
                      {toolDisplayDesc(tool)}
                    </div>
                  </div>
                  <Switch
                    checked={deferredToolNames.includes(tool.name)}
                    onCheckedChange={(checked) => handleDeferredToolToggle(tool.name, checked)}
                  />
                </div>
              ))}
            </div>
          )}

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
              <DeferredNumberInput
                min={1}
                max={20}
                value={limits.maxImages}
                onValueCommit={(value) => updateLimit("maxImages", value)}
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
              <DeferredNumberInput
                min={1}
                max={10}
                value={limits.maxPdfs}
                onValueCommit={(value) => updateLimit("maxPdfs", value)}
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
              <DeferredNumberInput
                min={1}
                max={50}
                value={limits.maxVisionPages}
                onValueCommit={(value) => updateLimit("maxVisionPages", value)}
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

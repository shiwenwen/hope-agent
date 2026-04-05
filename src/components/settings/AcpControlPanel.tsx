import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"
import {
  Check,
  Loader2,
  RefreshCw,
  CircleCheck,
  CircleX,
  Cable,
  Plus,
  Trash2,
} from "lucide-react"

// ── Types ────────────────────────────────────────────────────────

interface AcpBackendConfig {
  id: string
  name: string
  binary: string
  acpArgs: string[]
  enabled: boolean
  defaultModel: string | null
  env: Record<string, string>
}

interface AcpControlConfig {
  enabled: boolean
  backends: AcpBackendConfig[]
  maxConcurrentSessions: number
  defaultTimeoutSecs: number
  runtimeTtlSecs: number
  autoDiscover: boolean
}

interface AcpHealthStatus {
  available: boolean
  binaryPath: string | null
  version: string | null
  error: string | null
  lastChecked: string
}

interface AcpBackendInfo {
  id: string
  name: string
  enabled: boolean
  health: AcpHealthStatus
}

// ── Component ────────────────────────────────────────────────────

export default function AcpControlPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<AcpControlConfig>({
    enabled: false,
    backends: [],
    maxConcurrentSessions: 5,
    defaultTimeoutSecs: 600,
    runtimeTtlSecs: 1800,
    autoDiscover: true,
  })
  const [savedSnapshot, setSavedSnapshot] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [backends, setBackends] = useState<AcpBackendInfo[]>([])
  const [checking, setChecking] = useState(false)

  const isDirty = JSON.stringify(config) !== savedSnapshot

  const loadConfig = useCallback(async () => {
    try {
      const cfg = await getTransport().call<AcpControlConfig>("acp_get_config")
      setConfig(cfg)
      setSavedSnapshot(JSON.stringify(cfg))
    } catch (e) {
      logger.error("settings", "AcpControlPanel", `Failed to load ACP config: ${e}`)
    }
  }, [])

  const loadBackends = useCallback(async () => {
    try {
      setChecking(true)
      const list = await getTransport().call<AcpBackendInfo[]>("acp_list_backends")
      setBackends(list)
    } catch (e) {
      logger.error("settings", "AcpControlPanel", `Failed to load ACP backends: ${e}`)
    } finally {
      setChecking(false)
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadBackends()
  }, [loadConfig, loadBackends])

  const handleSave = async () => {
    setSaving(true)
    setSaveStatus("idle")
    try {
      await getTransport().call("acp_set_config", { config })
      setSavedSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
      loadBackends()
    } catch (e) {
      logger.error("settings", "AcpControlPanel", `Failed to save ACP config: ${e}`)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const addBackend = () => {
    setConfig((prev) => ({
      ...prev,
      backends: [
        ...prev.backends,
        {
          id: `custom-${Date.now()}`,
          name: "Custom Agent",
          binary: "",
          acpArgs: [],
          enabled: true,
          defaultModel: null,
          env: {},
        },
      ],
    }))
  }

  const removeBackend = (index: number) => {
    setConfig((prev) => ({
      ...prev,
      backends: prev.backends.filter((_, i) => i !== index),
    }))
  }

  const updateBackend = (index: number, updates: Partial<AcpBackendConfig>) => {
    setConfig((prev) => ({
      ...prev,
      backends: prev.backends.map((b, i) =>
        i === index ? { ...b, ...updates } : b
      ),
    }))
  }

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-6">
      {/* Master switch */}
      <div className="flex items-center justify-between rounded-lg border p-4">
        <div>
          <Label className="text-sm font-medium">{t("settings.acpEnabled", "Enable ACP Control Plane")}</Label>
          <p className="text-xs text-muted-foreground mt-0.5">
            {t("settings.acpEnabledDesc", "Allow agents to spawn external ACP agents for task delegation")}
          </p>
        </div>
        <Switch
          checked={config.enabled}
          onCheckedChange={(checked) => setConfig((prev) => ({ ...prev, enabled: checked }))}
        />
      </div>

      {config.enabled && (
        <>
          {/* Backend list */}
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <Label className="text-sm font-medium">{t("settings.acpBackends", "ACP Backends")}</Label>
              <div className="flex gap-2">
                <Button variant="outline" size="sm" onClick={loadBackends} disabled={checking}>
                  <RefreshCw className={cn("h-3.5 w-3.5 mr-1", checking && "animate-spin")} />
                  {t("common.refresh", "Refresh")}
                </Button>
                <Button variant="outline" size="sm" onClick={addBackend}>
                  <Plus className="h-3.5 w-3.5 mr-1" />
                  {t("common.add", "Add")}
                </Button>
              </div>
            </div>

            {config.backends.map((backend, index) => {
              const info = backends.find((b) => b.id === backend.id)
              const health = info?.health

              return (
                <div key={backend.id} className="rounded-lg border p-3 space-y-2">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <Cable className="h-4 w-4 text-muted-foreground" />
                      <Input
                        className="h-7 w-40 text-sm"
                        value={backend.name}
                        onChange={(e) => updateBackend(index, { name: e.target.value })}
                      />
                      {health && (
                        <span className="flex items-center gap-1 text-xs">
                          {health.available ? (
                            <>
                              <CircleCheck className="h-3.5 w-3.5 text-green-500" />
                              <span className="text-green-600">{health.version || t("common.available", "Available")}</span>
                            </>
                          ) : (
                            <>
                              <CircleX className="h-3.5 w-3.5 text-red-500" />
                              <span className="text-red-500 max-w-48 truncate">{health.error || t("common.unavailable", "Unavailable")}</span>
                            </>
                          )}
                        </span>
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      <Switch
                        checked={backend.enabled}
                        onCheckedChange={(checked) => updateBackend(index, { enabled: checked })}
                      />
                      <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => removeBackend(index)}>
                        <Trash2 className="h-3.5 w-3.5 text-muted-foreground" />
                      </Button>
                    </div>
                  </div>

                  <div className="grid grid-cols-2 gap-2">
                    <div>
                      <Label className="text-xs text-muted-foreground">{t("settings.acpBackendId", "ID")}</Label>
                      <Input
                        className="h-7 text-xs"
                        value={backend.id}
                        onChange={(e) => updateBackend(index, { id: e.target.value })}
                      />
                    </div>
                    <div>
                      <Label className="text-xs text-muted-foreground">{t("settings.acpBinary", "Binary")}</Label>
                      <Input
                        className="h-7 text-xs"
                        value={backend.binary}
                        placeholder="e.g. claude, codex, gemini"
                        onChange={(e) => updateBackend(index, { binary: e.target.value })}
                      />
                    </div>
                  </div>
                </div>
              )
            })}
          </div>

          {/* Settings */}
          <div className="space-y-3">
            <Label className="text-sm font-medium">{t("settings.acpSettings", "Settings")}</Label>

            <div className="flex items-center justify-between rounded-lg border p-3">
              <div>
                <Label className="text-xs">{t("settings.acpAutoDiscover", "Auto-discover backends")}</Label>
                <p className="text-xs text-muted-foreground">{t("settings.acpAutoDiscoverDesc", "Scan $PATH for known ACP agent binaries on startup")}</p>
              </div>
              <Switch
                checked={config.autoDiscover}
                onCheckedChange={(checked) => setConfig((prev) => ({ ...prev, autoDiscover: checked }))}
              />
            </div>

            <div className="grid grid-cols-3 gap-3">
              <div>
                <Label className="text-xs text-muted-foreground">{t("settings.acpMaxConcurrent", "Max Concurrent")}</Label>
                <Input
                  type="number"
                  className="h-7 text-xs"
                  value={config.maxConcurrentSessions}
                  min={1}
                  max={20}
                  onChange={(e) =>
                    setConfig((prev) => ({ ...prev, maxConcurrentSessions: parseInt(e.target.value) || 5 }))
                  }
                />
              </div>
              <div>
                <Label className="text-xs text-muted-foreground">{t("settings.acpTimeout", "Timeout (s)")}</Label>
                <Input
                  type="number"
                  className="h-7 text-xs"
                  value={config.defaultTimeoutSecs}
                  min={60}
                  max={7200}
                  onChange={(e) =>
                    setConfig((prev) => ({ ...prev, defaultTimeoutSecs: parseInt(e.target.value) || 600 }))
                  }
                />
              </div>
              <div>
                <Label className="text-xs text-muted-foreground">{t("settings.acpIdleTtl", "Idle TTL (s)")}</Label>
                <Input
                  type="number"
                  className="h-7 text-xs"
                  value={config.runtimeTtlSecs}
                  min={60}
                  max={86400}
                  onChange={(e) =>
                    setConfig((prev) => ({ ...prev, runtimeTtlSecs: parseInt(e.target.value) || 1800 }))
                  }
                />
              </div>
            </div>
          </div>
        </>
      )}

      {/* Save button */}
      <Button
        onClick={handleSave}
        disabled={!isDirty || saving}
        className={cn(
          "w-full",
          saveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
          saveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20"
        )}
      >
        {saving ? (
          <>
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            {t("common.saving", "Saving...")}
          </>
        ) : saveStatus === "saved" ? (
          <>
            <Check className="mr-2 h-4 w-4" />
            {t("common.saved", "Saved")}
          </>
        ) : saveStatus === "failed" ? (
          t("common.saveFailed", "Save Failed")
        ) : (
          t("common.save", "Save")
        )}
      </Button>
    </div>
  )
}

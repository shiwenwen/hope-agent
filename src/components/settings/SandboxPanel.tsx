import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select"
import { cn } from "@/lib/utils"
import { Check, Loader2, CircleCheck, CircleX, ExternalLink, RefreshCw } from "lucide-react"

// ── Types ────────────────────────────────────────────────────────

interface DockerStatus {
  installed: boolean
  running: boolean
}

interface SandboxConfig {
  image: string
  memory_limit: number | null
  cpu_limit: number | null
  read_only: boolean
  network_mode: string
  cap_drop_all: boolean
  no_new_privileges: boolean
  pids_limit: number | null
  tmpfs: string[]
}

const DEFAULT_CONFIG: SandboxConfig = {
  image: "debian:bookworm-slim",
  memory_limit: 512 * 1024 * 1024,
  cpu_limit: 1.0,
  read_only: true,
  network_mode: "none",
  cap_drop_all: true,
  no_new_privileges: true,
  pids_limit: 256,
  tmpfs: ["/tmp:size=64M", "/var/tmp:size=32M", "/run:size=16M"],
}

export default function SandboxPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<SandboxConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState<string>("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [dockerStatus, setDockerStatus] = useState<DockerStatus | null>(null)

  const isDirty = JSON.stringify(config) !== savedSnapshot
  const dockerAvailable = dockerStatus?.installed && dockerStatus?.running

  const refreshDockerStatus = useCallback(async () => {
    try {
      const s = await invoke<DockerStatus>("check_sandbox_available")
      setDockerStatus(s)
    } catch {
      setDockerStatus({ installed: false, running: false })
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    invoke<SandboxConfig>("get_sandbox_config")
      .then((cfg) => {
        if (!cancelled) {
          setConfig(cfg)
          setSavedSnapshot(JSON.stringify(cfg))
        }
      })
      .catch((e) => {
        logger.error("settings", "SandboxPanel", `Failed to load sandbox config: ${e}`)
      })

    invoke<DockerStatus>("check_sandbox_available")
      .then((s) => {
        if (!cancelled) setDockerStatus(s)
      })
      .catch(() => {
        if (!cancelled) setDockerStatus({ installed: false, running: false })
      })

    return () => {
      cancelled = true
    }
  }, [])

  const save = async () => {
    setSaving(true)
    try {
      await invoke("set_sandbox_config", { config })
      setSavedSnapshot(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "SandboxPanel", `Failed to save sandbox config: ${e}`)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const bytesToMB = (bytes: number | null) => (bytes ? (bytes / (1024 * 1024)).toFixed(0) : "")
  const mbToBytes = (mb: string) => {
    const num = parseInt(mb, 10)
    if (!isNaN(num) && num > 0) return num * 1024 * 1024
    return config.memory_limit
  }

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-6">
        {/* Header + Docker Status */}
        <div className="flex items-center justify-between">
          <p className="text-xs text-muted-foreground">{t("settings.sandboxDesc")}</p>
          <div className="flex items-center gap-1.5 shrink-0">
            {dockerStatus === null ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
            ) : dockerAvailable ? (
              <CircleCheck className="h-3.5 w-3.5 text-green-500" />
            ) : (
              <CircleX className="h-3.5 w-3.5 text-destructive" />
            )}
            <span className="text-xs text-muted-foreground">
              {dockerStatus === null
                ? t("settings.sandboxDockerChecking")
                : dockerAvailable
                  ? t("settings.sandboxDockerAvailable")
                  : t("settings.sandboxDockerUnavailable")}
            </span>
          </div>
        </div>

        {/* Docker not available hint */}
        {dockerStatus && !dockerAvailable && (
          <div className="rounded-md border border-border/50 p-3 space-y-2">
            {!dockerStatus.installed ? (
              <>
                <p className="text-xs text-muted-foreground">
                  {t("settings.sandboxDockerNotInstalled")}
                </p>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 text-xs"
                  onClick={() =>
                    invoke("open_url", {
                      url: "https://www.docker.com/products/docker-desktop/",
                    })
                  }
                >
                  <ExternalLink className="h-3 w-3 mr-1" />
                  {t("settings.sandboxDockerInstall")}
                </Button>
              </>
            ) : (
              <>
                <p className="text-xs text-muted-foreground">
                  {t("settings.sandboxDockerNotRunning")}
                </p>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 text-xs"
                  onClick={refreshDockerStatus}
                >
                  <RefreshCw className="h-3 w-3 mr-1" />
                  {t("settings.sandboxDockerRefresh")}
                </Button>
              </>
            )}
          </div>
        )}

        {/* Container Image */}
        <div className="space-y-4">
          <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
            {t("settings.sandboxSectionContainer")}
          </h3>
          <div className="space-y-1.5">
            <span className="text-sm font-medium">{t("settings.sandboxImage")}</span>
            <Input
              value={config.image}
              onChange={(e) => setConfig((prev) => ({ ...prev, image: e.target.value }))}
              placeholder="debian:bookworm-slim"
            />
            <p className="text-xs text-muted-foreground">{t("settings.sandboxImageDesc")}</p>
          </div>
        </div>

        {/* Resource Limits */}
        <div className="space-y-4">
          <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
            {t("settings.sandboxSectionResources")}
          </h3>
          <div className="grid grid-cols-3 gap-4">
            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.sandboxMemoryLimit")}</span>
              <Input
                type="number"
                min={64}
                value={bytesToMB(config.memory_limit)}
                onChange={(e) =>
                  setConfig((prev) => ({ ...prev, memory_limit: mbToBytes(e.target.value) }))
                }
              />
              <p className="text-xs text-muted-foreground">MB</p>
            </div>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.sandboxCpuLimit")}</span>
              <Input
                type="number"
                min={0.1}
                step={0.1}
                value={config.cpu_limit ?? 1.0}
                onChange={(e) => {
                  const num = parseFloat(e.target.value)
                  if (!isNaN(num) && num > 0)
                    setConfig((prev) => ({ ...prev, cpu_limit: num }))
                }}
              />
              <p className="text-xs text-muted-foreground">{t("settings.sandboxCpuLimitDesc")}</p>
            </div>
            <div className="space-y-1.5">
              <span className="text-sm font-medium">{t("settings.sandboxPidsLimit")}</span>
              <Input
                type="number"
                min={16}
                value={config.pids_limit ?? 256}
                onChange={(e) => {
                  const num = parseInt(e.target.value, 10)
                  if (!isNaN(num) && num > 0)
                    setConfig((prev) => ({ ...prev, pids_limit: num }))
                }}
              />
            </div>
          </div>
        </div>

        {/* Security */}
        <div className="space-y-4">
          <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
            {t("settings.sandboxSectionSecurity")}
          </h3>

          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <div className="space-y-0.5">
                <span className="text-sm font-medium">{t("settings.sandboxReadOnly")}</span>
                <p className="text-xs text-muted-foreground">
                  {t("settings.sandboxReadOnlyDesc")}
                </p>
              </div>
              <Switch
                checked={config.read_only}
                onCheckedChange={(v) => setConfig((prev) => ({ ...prev, read_only: v }))}
              />
            </div>

            <div className="flex items-center justify-between">
              <div className="space-y-0.5">
                <span className="text-sm font-medium">{t("settings.sandboxCapDrop")}</span>
                <p className="text-xs text-muted-foreground">
                  {t("settings.sandboxCapDropDesc")}
                </p>
              </div>
              <Switch
                checked={config.cap_drop_all}
                onCheckedChange={(v) => setConfig((prev) => ({ ...prev, cap_drop_all: v }))}
              />
            </div>

            <div className="flex items-center justify-between">
              <div className="space-y-0.5">
                <span className="text-sm font-medium">{t("settings.sandboxNoNewPrivileges")}</span>
                <p className="text-xs text-muted-foreground">
                  {t("settings.sandboxNoNewPrivilegesDesc")}
                </p>
              </div>
              <Switch
                checked={config.no_new_privileges}
                onCheckedChange={(v) =>
                  setConfig((prev) => ({ ...prev, no_new_privileges: v }))
                }
              />
            </div>

            <div className="flex items-center justify-between">
              <div className="space-y-0.5">
                <span className="text-sm font-medium">{t("settings.sandboxNetworkMode")}</span>
                <p className="text-xs text-muted-foreground">
                  {t("settings.sandboxNetworkModeDesc")}
                </p>
              </div>
              <Select
                value={config.network_mode}
                onValueChange={(v) => setConfig((prev) => ({ ...prev, network_mode: v }))}
              >
                <SelectTrigger className="w-32">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">{t("settings.sandboxNetworkNone")}</SelectItem>
                  <SelectItem value="bridge">{t("settings.sandboxNetworkBridge")}</SelectItem>
                  <SelectItem value="host">{t("settings.sandboxNetworkHost")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
        </div>

        {/* Save button */}
        <div className="flex items-center gap-2 pt-2">
          <Button
            onClick={save}
            disabled={(!isDirty && saveStatus === "idle") || saving}
            className={cn(
              saveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
              saveStatus === "failed" &&
                "bg-destructive/10 text-destructive hover:bg-destructive/20",
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
    </div>
  )
}

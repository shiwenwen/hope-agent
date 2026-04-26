import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { withEventListener } from "@/lib/transport-events"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { Switch } from "@/components/ui/switch"
import {
  Circle,
  Download,
  ExternalLink,
  Loader2,
  Play,
  RefreshCw,
  RotateCw,
  Square,
  Trash2,
} from "lucide-react"
import type { SearxngDockerStatus } from "./types"

export function SearxngDockerSection({
  onUrlSet,
  useProxy,
  onUseProxyChange,
  saveConfig,
}: {
  onUrlSet: (url: string) => void
  useProxy: boolean
  onUseProxyChange: (enabled: boolean) => Promise<boolean>
  saveConfig: () => Promise<boolean>
}) {
  const { t } = useTranslation()
  const [status, setStatus] = useState<SearxngDockerStatus | null>(null)
  const [checking, setChecking] = useState(true)
  const [deploying, setDeploying] = useState(false)
  const [deployStep, setDeployStep] = useState<string | null>(null)
  const [deployLogs, setDeployLogs] = useState<string[]>([])
  const [actionLoading, setActionLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const refreshStatus = useCallback(async () => {
    setChecking(true)
    try {
      const s = await getTransport().call<SearxngDockerStatus>("searxng_docker_status")
      setStatus(s)
    } catch (e) {
      logger.error("settings", "SearxngDocker::status", "Failed to check Docker status", e)
    } finally {
      setChecking(false)
    }
  }, [])

  useEffect(() => {
    refreshStatus()
  }, [refreshStatus])

  // Poll status while container is starting or deploy is in progress
  useEffect(() => {
    const needsPoll = status?.deploying || (status?.containerRunning && !status?.healthOk)
    if (!needsPoll) return
    const timer = setInterval(async () => {
      try {
        const s = await getTransport().call<SearxngDockerStatus>("searxng_docker_status")
        setStatus(s)
        // Sync deploy state from backend when we're observing an external deploy
        if (s.deploying && !deploying) {
          setDeployStep(s.deployStep ?? null)
          setDeployLogs(s.deployLogs ?? [])
        }
        if (!s.deploying && !s.containerRunning) clearInterval(timer)
        if (s.healthOk) clearInterval(timer)
      } catch {
        /* ignore */
      }
    }, 1500)
    return () => clearInterval(timer)
  }, [status?.containerRunning, status?.healthOk, status?.deploying, deploying])

  const deployStepLabels: Record<string, string> = {
    checking_docker: t("settings.webSearchDockerStepCheckingDocker"),
    pulling_image: t("settings.webSearchDockerStepPullingImage"),
    removing_old: t("settings.webSearchDockerStepRemovingOld"),
    starting_container: t("settings.webSearchDockerStepStarting"),
    injecting_config: t("settings.webSearchDockerStepConfig"),
    restarting: t("settings.webSearchDockerStepRestarting"),
    health_check: t("settings.webSearchDockerStepHealthCheck"),
    done: t("settings.webSearchDockerStepDone"),
  }

  const waitForHealthy = useCallback(async () => {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 1500))
      const s = await getTransport().call<SearxngDockerStatus>("searxng_docker_status")
      setStatus(s)
      if (s.healthOk) break
    }
  }, [])

  const ensureConfigSaved = useCallback(async () => {
    const ok = await saveConfig()
    if (!ok) {
      setError(t("settings.webSearchDockerSaveConfigFailed"))
      return false
    }
    return true
  }, [saveConfig, t])

  const handleDeploy = useCallback(async () => {
    if (!(await ensureConfigSaved())) return
    setDeploying(true)
    setDeployStep(null)
    setDeployLogs([])
    setError(null)

    const handleProgress = (raw: unknown) => {
      const parsed = parsePayload<{ step?: string; log?: string }>(raw)
      if (parsed.step) setDeployStep(parsed.step)
      if (parsed.log) setDeployLogs((prev) => [...prev.slice(-50), parsed.log!])
    }

    try {
      const url = await withEventListener("searxng:deploy_progress", handleProgress, () =>
        getTransport().call<string>("searxng_docker_deploy"),
      )
      onUrlSet(url)
      await refreshStatus()
    } catch (e) {
      setError(String(e))
    } finally {
      setDeploying(false)
      setDeployStep(null)
    }
  }, [ensureConfigSaved, onUrlSet, refreshStatus])

  const handleRedeploy = useCallback(async () => {
    if (!(await ensureConfigSaved())) return
    setActionLoading(true)
    setError(null)
    try {
      // Remove existing container first
      await getTransport().call("searxng_docker_remove")
      await refreshStatus()
    } catch {
      // Ignore remove errors (container might not exist)
    }
    setActionLoading(false)
    // Then deploy fresh
    await handleDeploy()
  }, [ensureConfigSaved, handleDeploy, refreshStatus])

  const handleAction = useCallback(
    async (action: "start" | "stop" | "remove") => {
      if (action === "start" && !(await ensureConfigSaved())) return
      setActionLoading(true)
      setError(null)
      try {
        await getTransport().call(`searxng_docker_${action}`)
        await refreshStatus()
        // After start, poll until healthy (up to 15s)
        if (action === "start") {
          for (let i = 0; i < 10; i++) {
            await new Promise((r) => setTimeout(r, 1500))
            const s = await getTransport().call<SearxngDockerStatus>("searxng_docker_status")
            setStatus(s)
            if (s.healthOk) break
          }
        }
      } catch (e) {
        setError(String(e))
      } finally {
        setActionLoading(false)
      }
    },
    [ensureConfigSaved, refreshStatus],
  )

  const handleUseProxyToggle = useCallback(
    async (enabled: boolean) => {
      if (deploying || status?.deploying || actionLoading) return
      setError(null)

      const wasRunning = !!status?.containerRunning
      const ok = await onUseProxyChange(enabled)
      if (!ok) {
        setError(t("settings.webSearchDockerSaveConfigFailed"))
        return
      }

      if (!wasRunning) {
        await refreshStatus()
        return
      }

      setActionLoading(true)
      try {
        await getTransport().call("searxng_docker_stop")
        await getTransport().call("searxng_docker_start")
        await refreshStatus()
        await waitForHealthy()
      } catch (e) {
        setError(String(e))
      } finally {
        setActionLoading(false)
      }
    },
    [
      actionLoading,
      deploying,
      onUseProxyChange,
      refreshStatus,
      status?.containerRunning,
      status?.deploying,
      t,
      waitForHealthy,
    ],
  )

  if (checking && !status) {
    return (
      <div className="rounded-md border border-border/50 p-3 mt-1">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("settings.webSearchDockerChecking")}
        </div>
      </div>
    )
  }

  if (!status) return null

  if (!status.dockerInstalled) {
    const openExt = (url: string) => getTransport().call("open_url", { url })
    const alternatives: Array<{ label: string; url: string }> = [
      { label: "colima", url: "https://github.com/abiosoft/colima" },
      { label: "OrbStack", url: "https://orbstack.dev" },
      { label: "Rancher Desktop", url: "https://rancherdesktop.io" },
      { label: "Linux dockerd", url: "https://docs.docker.com/engine/install/" },
    ]
    return (
      <div className="rounded-md border border-border/50 p-3 mt-1 space-y-2">
        <div className="text-xs font-medium">{t("settings.webSearchDockerTitle")}</div>
        <p className="text-xs text-muted-foreground">{t("settings.webSearchDockerNotInstalled")}</p>
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs"
          onClick={() => openExt("https://www.docker.com/products/docker-desktop/")}
        >
          <ExternalLink className="h-3 w-3 mr-1" />
          {t("settings.webSearchDockerInstall")}
        </Button>
        <div className="text-[11px] text-muted-foreground leading-relaxed pt-0.5">
          {t("settings.webSearchDockerAlternatives")}{" "}
          {alternatives.map((item, idx) => (
            <span key={item.label}>
              {idx > 0 && <span className="mx-1 opacity-60">·</span>}
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="inline h-auto rounded-none px-0 py-0 text-[11px] font-normal align-baseline underline decoration-dotted underline-offset-2 hover:bg-transparent hover:text-primary"
                onClick={() => openExt(item.url)}
              >
                {item.label}
              </Button>
            </span>
          ))}
        </div>
      </div>
    )
  }

  if (status.dockerNotRunning) {
    return (
      <div className="rounded-md border border-border/50 p-3 mt-1 space-y-2">
        <div className="text-xs font-medium">{t("settings.webSearchDockerTitle")}</div>
        <p className="text-xs text-muted-foreground">{t("settings.webSearchDockerNotRunning")}</p>
        <Button size="sm" variant="outline" className="h-7 text-xs" onClick={refreshStatus}>
          <RefreshCw className="h-3 w-3 mr-1" />
          {t("settings.webSearchDockerRefresh")}
        </Button>
      </div>
    )
  }

  return (
    <div className="rounded-md border border-border/50 p-3 mt-1 space-y-2">
      <div className="text-xs font-medium">{t("settings.webSearchDockerTitle")}</div>

      <div className="rounded-md border border-border/40 bg-secondary/20 p-2.5">
        <div className="flex items-start justify-between gap-3">
          <div className="space-y-1">
            <div className="text-xs font-medium">{t("settings.webSearchDockerUseProxy")}</div>
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              {t("settings.webSearchDockerUseProxyDesc")}
            </p>
          </div>
          <Switch
            checked={useProxy}
            onCheckedChange={(checked) => void handleUseProxyToggle(checked)}
            disabled={deploying || status.deploying || actionLoading}
          />
        </div>
      </div>

      {status.containerExists && (
        <div className="space-y-1.5">
          <div className="flex items-center gap-2 text-xs">
            <Circle
              className={`h-2 w-2 fill-current ${
                status.containerRunning && status.searchOk
                  ? "text-green-500"
                  : status.containerRunning && status.healthOk
                    ? "text-yellow-500"
                    : status.containerRunning
                      ? "text-yellow-500"
                      : "text-muted-foreground"
              }`}
            />
            <span>
              {status.containerRunning
                ? status.searchOk
                  ? t("settings.webSearchDockerSearchOk", { count: status.searchResultCount })
                  : status.healthOk
                    ? t("settings.webSearchDockerSearchFail")
                    : t("settings.webSearchDockerStarting")
                : t("settings.webSearchDockerStopped")}
            </span>
            {status.port && status.containerRunning && (
              <IconTip label={t("settings.webSearchDockerFillUrl")}>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="inline h-auto rounded-none px-0 py-0 text-xs font-normal align-baseline text-muted-foreground underline decoration-dotted underline-offset-2 hover:bg-transparent hover:text-primary"
                  onClick={() => onUrlSet(`http://127.0.0.1:${status.port}`)}
                >
                  127.0.0.1:{status.port}
                </Button>
              </IconTip>
            )}
          </div>
          {status.containerRunning && status.healthOk && status.unresponsiveEngines.length > 0 && (
            <div className="text-xs text-yellow-600 dark:text-yellow-500 pl-4">
              {t("settings.webSearchDockerUnresponsive")}: {status.unresponsiveEngines.join(", ")}
            </div>
          )}
        </div>
      )}

      {error && <p className="text-xs text-destructive whitespace-pre-wrap break-all">{error}</p>}

      {(deploying || status.deploying) && (
        <div className="space-y-1.5">
          {(deployStep || status.deployStep) && (
            <p className="text-xs text-muted-foreground">
              <Loader2 className="h-3 w-3 animate-spin inline mr-1" />
              {deployStepLabels[deployStep || status.deployStep || ""] ||
                deployStep ||
                status.deployStep}
            </p>
          )}
          {(deployLogs.length > 0 || (status.deployLogs?.length ?? 0) > 0) && (
            <div className="rounded bg-muted/50 p-2 max-h-36 overflow-y-auto font-mono text-[11px] text-muted-foreground leading-relaxed">
              {(deployLogs.length > 0 ? deployLogs : (status.deployLogs ?? [])).map((line, i) => (
                <div key={i} className={line.startsWith("ERROR") ? "text-destructive" : ""}>
                  {line}
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      <div className="flex items-center gap-2">
        {!status.containerExists && !status.deploying && (
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={handleDeploy}
            disabled={deploying || status.deploying}
          >
            {deploying ? (
              <Loader2 className="h-3 w-3 animate-spin mr-1" />
            ) : (
              <Download className="h-3 w-3 mr-1" />
            )}
            {deploying
              ? t("settings.webSearchDockerDeploying")
              : t("settings.webSearchDockerDeploy")}
          </Button>
        )}
        {status.containerExists && !status.containerRunning && (
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={() => handleAction("start")}
            disabled={actionLoading}
          >
            {actionLoading ? (
              <Loader2 className="h-3 w-3 animate-spin mr-1" />
            ) : (
              <Play className="h-3 w-3 mr-1" />
            )}
            {t("settings.webSearchDockerStart")}
          </Button>
        )}
        {status.containerExists && status.containerRunning && (
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={() => handleAction("stop")}
            disabled={actionLoading}
          >
            {actionLoading ? (
              <Loader2 className="h-3 w-3 animate-spin mr-1" />
            ) : (
              <Square className="h-3 w-3 mr-1" />
            )}
            {t("settings.webSearchDockerStop")}
          </Button>
        )}
        {status.containerExists && !status.deploying && (
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={handleRedeploy}
            disabled={actionLoading || deploying}
          >
            {actionLoading ? (
              <Loader2 className="h-3 w-3 animate-spin mr-1" />
            ) : (
              <RotateCw className="h-3 w-3 mr-1" />
            )}
            {t("settings.webSearchDockerRedeploy")}
          </Button>
        )}
        {status.containerExists && (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 text-xs text-destructive hover:text-destructive"
            onClick={() => handleAction("remove")}
            disabled={actionLoading || deploying}
          >
            <Trash2 className="h-3 w-3 mr-1" />
            {t("settings.webSearchDockerRemove")}
          </Button>
        )}
      </div>
    </div>
  )
}

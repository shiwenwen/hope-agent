import { useState, useEffect, useCallback } from "react"
import { invoke, Channel } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import {
  Circle,
  Download,
  ExternalLink,
  Loader2,
  Play,
  RefreshCw,
  Square,
  Trash2,
} from "lucide-react"
import type { SearxngDockerStatus } from "./types"

export function SearxngDockerSection({ onUrlSet }: { onUrlSet: (url: string) => void }) {
  const { t } = useTranslation()
  const [status, setStatus] = useState<SearxngDockerStatus | null>(null)
  const [checking, setChecking] = useState(true)
  const [deploying, setDeploying] = useState(false)
  const [deployStep, setDeployStep] = useState<string | null>(null)
  const [actionLoading, setActionLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const refreshStatus = useCallback(async () => {
    setChecking(true)
    try {
      const s = await invoke<SearxngDockerStatus>("searxng_docker_status")
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

  // Poll status while container is running but not yet healthy
  useEffect(() => {
    if (!status?.containerRunning || status?.healthOk) return
    const timer = setInterval(async () => {
      try {
        const s = await invoke<SearxngDockerStatus>("searxng_docker_status")
        setStatus(s)
        if (s.healthOk) clearInterval(timer)
      } catch {
        /* ignore */
      }
    }, 3000)
    return () => clearInterval(timer)
  }, [status?.containerRunning, status?.healthOk])

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

  const handleDeploy = useCallback(async () => {
    setDeploying(true)
    setDeployStep(null)
    setError(null)
    try {
      const channel = new Channel<string>()
      channel.onmessage = (step) => {
        setDeployStep(step)
      }
      const url = await invoke<string>("searxng_docker_deploy", { channel })
      onUrlSet(url)
      await refreshStatus()
    } catch (e) {
      setError(String(e))
    } finally {
      setDeploying(false)
      setDeployStep(null)
    }
  }, [onUrlSet, refreshStatus])

  const handleAction = useCallback(
    async (action: "start" | "stop" | "remove") => {
      setActionLoading(true)
      setError(null)
      try {
        await invoke(`searxng_docker_${action}`)
        await refreshStatus()
        // After start, poll until healthy (up to 15s)
        if (action === "start") {
          for (let i = 0; i < 10; i++) {
            await new Promise((r) => setTimeout(r, 1500))
            const s = await invoke<SearxngDockerStatus>("searxng_docker_status")
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
    [refreshStatus],
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
    return (
      <div className="rounded-md border border-border/50 p-3 mt-1 space-y-2">
        <div className="text-xs font-medium">{t("settings.webSearchDockerTitle")}</div>
        <p className="text-xs text-muted-foreground">{t("settings.webSearchDockerNotInstalled")}</p>
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs"
          onClick={() =>
            invoke("open_url", { url: "https://www.docker.com/products/docker-desktop/" })
          }
        >
          <ExternalLink className="h-3 w-3 mr-1" />
          {t("settings.webSearchDockerInstall")}
        </Button>
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

      {status.containerExists && (
        <div className="flex items-center gap-2 text-xs">
          <Circle
            className={`h-2 w-2 fill-current ${
              status.containerRunning && status.healthOk
                ? "text-green-500"
                : status.containerRunning
                  ? "text-yellow-500"
                  : "text-muted-foreground"
            }`}
          />
          <span>
            {status.containerRunning
              ? status.healthOk
                ? t("settings.webSearchDockerRunning")
                : t("settings.webSearchDockerStarting")
              : t("settings.webSearchDockerStopped")}
          </span>
          {status.port && status.containerRunning && (
            <TooltipProvider>
              <IconTip label={t("settings.webSearchDockerFillUrl")}>
                <button
                  type="button"
                  className="text-muted-foreground hover:text-primary underline decoration-dotted underline-offset-2 transition-colors"
                  onClick={() => onUrlSet(`http://localhost:${status.port}`)}
                >
                  localhost:{status.port}
                </button>
              </IconTip>
            </TooltipProvider>
          )}
        </div>
      )}

      {error && <p className="text-xs text-destructive whitespace-pre-wrap break-all">{error}</p>}

      {deploying && deployStep && (
        <p className="text-xs text-muted-foreground">
          <Loader2 className="h-3 w-3 animate-spin inline mr-1" />
          {deployStepLabels[deployStep] || deployStep}
        </p>
      )}

      <div className="flex items-center gap-2">
        {!status.containerExists && (
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={handleDeploy}
            disabled={deploying}
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

import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Download, Loader2 } from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { logger } from "@/lib/logger"
import { parsePayload } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"

interface ChromiumRuntimeRequired {
  context?: string
  reason?: string
  installSupported?: boolean
  approxDownloadBytes?: number
}

interface ChromiumDownloadProgress {
  stage?: string
  percent?: number | null
}

interface ChromiumRuntimeDialogProps {
  onOpenBrowserSettings: () => void
}

/**
 * Process-wide remediation for user-triggered features that need Chromium.
 *
 * Core emits a transport-neutral event from the shared executable resolver;
 * keeping this dialog at App level means browser automation and Artifact PDF
 * export get the same install experience in both Tauri and HTTP modes.
 */
export default function ChromiumRuntimeDialog({
  onOpenBrowserSettings,
}: ChromiumRuntimeDialogProps) {
  const { t } = useTranslation()
  const [request, setRequest] = useState<ChromiumRuntimeRequired | null>(null)
  const [installing, setInstalling] = useState(false)
  const [percent, setPercent] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const unlistenRequired = getTransport().listen("browser:runtime_required", (raw) => {
      try {
        setRequest(parsePayload<ChromiumRuntimeRequired>(raw))
        setError(null)
        setPercent(null)
      } catch (e) {
        logger.warn(
          "browser",
          "ChromiumRuntimeDialog::runtime_required",
          "Failed to parse runtime-required event",
          e,
        )
      }
    })
    const unlistenProgress = getTransport().listen(
      "browser:chromium_download_progress",
      (raw) => {
        try {
          const progress = parsePayload<ChromiumDownloadProgress>(raw)
          if (!progress) return
          if (progress.stage === "ready") {
            setPercent(100)
          } else if (typeof progress.percent === "number") {
            setPercent(progress.percent)
          }
        } catch {
          // Ignore legacy or unrelated progress payloads.
        }
      },
    )
    return () => {
      unlistenRequired()
      unlistenProgress()
    }
  }, [])

  const install = useCallback(async () => {
    setInstalling(true)
    setError(null)
    setPercent(0)
    try {
      await getTransport().call("browser_install_chromium_runtime")
      toast.success(t("settings.browser.installRuntimeReady"), {
        description: t("settings.browser.installRuntimeRetry"),
      })
      setRequest(null)
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "browser",
        "ChromiumRuntimeDialog::install",
        `install-chromium-runtime failed: ${message}`,
      )
      setError(message)
    } finally {
      setInstalling(false)
    }
  }, [t])

  const openSettings = useCallback(() => {
    setRequest(null)
    onOpenBrowserSettings()
  }, [onOpenBrowserSettings])

  const descriptionKey =
    request?.context === "artifact_pdf"
      ? "settings.browser.runtimeRequiredArtifactPdfDescription"
      : "settings.browser.runtimeRequiredDescription"

  return (
    <Dialog
      open={request !== null}
      onOpenChange={(open) => {
        if (!open && !installing) setRequest(null)
      }}
    >
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("settings.browser.runtimeRequiredTitle")}</DialogTitle>
          <DialogDescription>{t(descriptionKey)}</DialogDescription>
        </DialogHeader>

        <div className="space-y-3 text-sm">
          <p className="text-muted-foreground">{t("settings.browser.runtimeRequiredHint")}</p>
          {installing && (
            <div className="space-y-1.5">
              <div className="text-xs text-muted-foreground">
                {t("settings.browser.installRuntimeRunning", { percent: percent ?? 0 })}
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-secondary/60">
                <div
                  className="h-full bg-primary transition-all"
                  style={{ width: `${Math.max(0, Math.min(100, percent ?? 0))}%` }}
                />
              </div>
            </div>
          )}
          {error && <div className="text-xs text-destructive">{error}</div>}
          {request?.installSupported === false && (
            <div className="text-xs text-muted-foreground">
              {t("settings.browser.runtimeInstallUnsupported")}
            </div>
          )}
        </div>

        <DialogFooter className="gap-2 sm:gap-0">
          <Button variant="outline" onClick={openSettings} disabled={installing}>
            {t("settings.browser.openBrowserSettings")}
          </Button>
          {request?.installSupported !== false && (
            <Button onClick={() => void install()} disabled={installing}>
              {installing ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Download className="h-4 w-4" />
              )}
              <span className="ml-1.5">{t("settings.browser.installRuntime")}</span>
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

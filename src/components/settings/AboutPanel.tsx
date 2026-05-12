import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import type { LucideIcon } from "lucide-react"
import {
  Brain,
  Download,
  ExternalLink,
  Globe,
  Loader2,
  Monitor,
  Power,
  RefreshCw,
} from "lucide-react"
import alphaLogoUrl from "@/assets/alpha-logo.png"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { HOPE_AGENT_URLS, useAppVersion } from "@/lib/appMeta"
import {
  checkForDesktopUpdate,
  isDesktopUpdaterAvailable,
  relaunchDesktopApp,
  setPendingUpdate as setGlobalPendingUpdate,
  subscribeManualCheckRequests,
  type DesktopUpdate,
} from "@/lib/desktopUpdater"
import { useDesktopUpdateStore } from "@/hooks/useDesktopUpdateStore"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"

interface HighlightItem {
  icon: LucideIcon
  title: string
  description: string
  cardClass: string
  iconClass: string
}

export default function AboutPanel() {
  const { t } = useTranslation()
  const appVersion = useAppVersion()
  const { pendingUpdate: globalPendingUpdate } = useDesktopUpdateStore()
  const [checkingUpdate, setCheckingUpdate] = useState(false)
  const [installingUpdate, setInstallingUpdate] = useState(false)
  const [pendingUpdate, setPendingUpdate] = useState<DesktopUpdate | null>(null)
  const [updateStatus, setUpdateStatus] = useState<string | null>(null)
  const [updateError, setUpdateError] = useState<string | null>(null)
  const [downloadPercent, setDownloadPercent] = useState<number | null>(null)
  const [restartDialogOpen, setRestartDialogOpen] = useState(false)
  const [restarting, setRestarting] = useState(false)
  const [restartError, setRestartError] = useState<string | null>(null)
  const desktopUpdaterAvailable = isDesktopUpdaterAvailable()

  function describeError(err: unknown): string {
    if (err instanceof Error) return err.message || err.toString()
    if (typeof err === "string") return err
    try {
      return JSON.stringify(err)
    } catch {
      return String(err)
    }
  }

  // Sync from global store: if auto-check found an update, reflect it here
  const syncedRef = useRef(false)
  useEffect(() => {
    if (globalPendingUpdate && !syncedRef.current) {
      syncedRef.current = true
      setPendingUpdate(globalPendingUpdate)
      setUpdateStatus(t("about.updateAvailable", { version: globalPendingUpdate.version }))
    }
  }, [globalPendingUpdate, t])

  // Subscribe to manual-check requests from the desktopUpdater store.
  // The store is fed by App.tsx's `desktop-update-check` listener (always
  // mounted) and queues a single pending request when no subscriber is
  // present, so requests fired before this panel mounts (e.g. from the
  // macOS app menu while the user is on the chat view) are replayed on
  // subscribe rather than dropped. `checkRef` keeps the subscription stable
  // while still calling the latest closure with current state.
  const checkRef = useRef<() => void>(() => {})
  useEffect(() => {
    const unsubscribe = subscribeManualCheckRequests(() => {
      checkRef.current()
    })
    return unsubscribe
  }, [])

  const highlights: HighlightItem[] = [
    {
      icon: Monitor,
      title: t("about.featureDailyTitle"),
      description: t("about.featureDailyDesc"),
      cardClass: "border-border/70 bg-sky-500/6",
      iconClass: "border border-sky-500/15 bg-sky-500/10 text-sky-600 dark:text-sky-300",
    },
    {
      icon: Brain,
      title: t("about.featureMemoryTitle"),
      description: t("about.featureMemoryDesc"),
      cardClass: "border-border/70 bg-emerald-500/6",
      iconClass:
        "border border-emerald-500/15 bg-emerald-500/10 text-emerald-600 dark:text-emerald-300",
    },
    {
      icon: Globe,
      title: t("about.featureReachTitle"),
      description: t("about.featureReachDesc"),
      cardClass: "border-border/70 bg-amber-500/6",
      iconClass: "border border-amber-500/15 bg-amber-500/10 text-amber-600 dark:text-amber-300",
    },
  ]

  async function openExternal(url: string) {
    try {
      await getTransport().call("open_url", { url })
    } catch {
      window.open(url, "_blank", "noopener,noreferrer")
    }
  }

  async function handleCheckForUpdates() {
    setCheckingUpdate(true)
    setUpdateStatus(t("about.updateChecking"))
    setUpdateError(null)
    setDownloadPercent(null)

    try {
      const update = await checkForDesktopUpdate()
      if (!update) {
        setPendingUpdate(null)
        void setGlobalPendingUpdate(null)
        setUpdateStatus(t("about.updateUpToDate", { version: appVersion }))
        return
      }

      setPendingUpdate(update)
      void setGlobalPendingUpdate(update)
      setUpdateStatus(t("about.updateAvailable", { version: update.version }))
    } catch (err) {
      const detail = describeError(err)
      logger.error("updater", "AboutPanel::handleCheckForUpdates", "check failed", {
        error: detail,
        currentVersion: appVersion,
      })
      setPendingUpdate(null)
      void setGlobalPendingUpdate(null)
      setUpdateStatus(t("about.updateCheckFailed"))
      setUpdateError(detail)
    } finally {
      setCheckingUpdate(false)
    }
  }

  // Keep `checkRef` pointing at the latest closure so the menu listener
  // (registered once, see the `desktop-update-check` effect) always invokes
  // the freshest version with current state.
  useEffect(() => {
    checkRef.current = () => {
      void handleCheckForUpdates()
    }
  })

  async function handleConfirmRestart() {
    setRestarting(true)
    setRestartError(null)
    logger.info("lifecycle", "AboutPanel::handleConfirmRestart", "user-initiated restart")
    try {
      await getTransport().call("request_app_restart", {})
      // Process is on its way out — desktop window will close (webview
      // tears down with it), HTTP clients will see their connection drop
      // and reconnect on the new instance. Keep the spinner on so the
      // user doesn't see a flash of "ready" state before the OS-level
      // handoff completes.
      setRestartDialogOpen(false)
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err)
      logger.error("lifecycle", "AboutPanel::handleConfirmRestart", "restart failed", {
        error: detail,
      })
      setRestartError(detail)
      setRestarting(false)
    }
  }

  async function handleInstallUpdate() {
    if (!pendingUpdate) return

    setInstallingUpdate(true)
    setDownloadPercent(0)
    setUpdateError(null)
    setUpdateStatus(t("about.updateInstalling", { version: pendingUpdate.version }))
    logger.info("updater", "AboutPanel::handleInstallUpdate", "install started", {
      fromVersion: appVersion,
      toVersion: pendingUpdate.version,
    })

    let downloaded = 0
    let contentLength = 0

    try {
      await pendingUpdate.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            contentLength = event.data.contentLength
            setDownloadPercent(0)
            break
          case "Progress":
            downloaded += event.data.chunkLength
            if (contentLength > 0) {
              setDownloadPercent(Math.min(100, Math.round((downloaded / contentLength) * 100)))
            }
            break
          case "Finished":
            setDownloadPercent(100)
            break
        }
      })

      await setPendingUpdate(null)
      void setGlobalPendingUpdate(null)
      setUpdateStatus(t("about.updateInstalled"))
      logger.info(
        "updater",
        "AboutPanel::handleInstallUpdate",
        "install completed, scheduling relaunch",
      )
      // Give the user a beat to see the "installed, restarting" message
      // before the process exits — without this the UI flips to a blank
      // pre-relaunch state and the whole flow feels like nothing happened.
      await new Promise((resolve) => setTimeout(resolve, 1500))
      try {
        await relaunchDesktopApp()
      } catch (err) {
        // relaunch() normally never returns (process exits). If it threw,
        // surface a manual-restart hint instead of silently leaving the
        // user staring at the "installed" message.
        const detail = describeError(err)
        logger.error("updater", "AboutPanel::handleInstallUpdate", "relaunch failed", {
          error: detail,
        })
        setUpdateStatus(t("about.updateRestartManually"))
        setUpdateError(detail)
      }
    } catch (err) {
      const detail = describeError(err)
      logger.error("updater", "AboutPanel::handleInstallUpdate", "install failed", {
        error: detail,
        fromVersion: appVersion,
        toVersion: pendingUpdate.version,
      })
      setUpdateStatus(t("about.updateInstallFailed"))
      setUpdateError(detail)
    } finally {
      setInstallingUpdate(false)
    }
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 p-6">
        <section className="rounded-[28px] border border-border/70 bg-card px-6 py-7 lg:px-8 lg:py-8">
          <div>
            <div className="inline-flex w-fit items-center gap-2 rounded-full border border-border/70 bg-secondary/40 px-3 py-1 text-[11px] font-medium uppercase tracking-[0.22em] text-primary/80">
              <span className="h-1.5 w-1.5 rounded-full bg-primary" />
              {t("about.badge")}
            </div>

            <div className="mt-5 flex items-center gap-4">
              <div className="flex h-[100px] w-[100px] items-center justify-center">
                <img
                  src={alphaLogoUrl}
                  alt="Hope Agent"
                  className="h-full w-full object-contain"
                  draggable={false}
                />
              </div>
              <div className="min-w-0 flex-1">
                <h2 className="text-3xl font-semibold tracking-tight text-foreground lg:text-4xl">
                  Hope Agent
                </h2>
                <div className="mt-2 flex flex-wrap items-center gap-2.5">
                  <span className="inline-flex items-center rounded-full border border-border/70 bg-secondary/40 px-3 py-1 text-sm font-medium text-muted-foreground">
                    v{appVersion}
                  </span>
                  {desktopUpdaterAvailable && (
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-auto gap-1.5 rounded-full border-border/50 bg-secondary/30 px-3 py-1 text-xs font-medium text-muted-foreground transition-all duration-200 hover:border-primary/30 hover:bg-primary/8 hover:text-foreground active:scale-[0.97]"
                      onClick={handleCheckForUpdates}
                      disabled={checkingUpdate || installingUpdate}
                    >
                      {checkingUpdate ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <RefreshCw className="h-3.5 w-3.5" />
                      )}
                      {checkingUpdate ? t("about.updateChecking") : t("about.updateCheck")}
                    </Button>
                  )}
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-auto gap-1.5 rounded-full border-border/50 bg-secondary/30 px-3 py-1 text-xs font-medium text-muted-foreground transition-all duration-200 hover:border-primary/30 hover:bg-primary/8 hover:text-foreground active:scale-[0.97]"
                    onClick={() => setRestartDialogOpen(true)}
                    disabled={installingUpdate}
                  >
                    <Power className="h-3.5 w-3.5" />
                    {t("about.restartApp")}
                  </Button>
                  {updateStatus && !pendingUpdate && (
                    <span className="text-xs text-muted-foreground/70">{updateStatus}</span>
                  )}
                </div>
              </div>
            </div>

            {pendingUpdate && (
              <div className="mt-5 overflow-hidden rounded-2xl border border-emerald-500/20 bg-gradient-to-r from-emerald-500/8 via-emerald-500/5 to-transparent">
                <div className="flex flex-wrap items-center gap-3 px-5 py-4">
                  <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-emerald-500/12">
                    <Download className="h-4.5 w-4.5 text-emerald-600 dark:text-emerald-400" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-semibold text-foreground">
                      {updateStatus}
                    </p>
                    {pendingUpdate.body && (
                      <p className="mt-0.5 line-clamp-2 text-xs leading-relaxed text-muted-foreground">
                        {pendingUpdate.body}
                      </p>
                    )}
                  </div>
                  <Button
                    size="sm"
                    className="shrink-0 gap-1.5 rounded-full bg-emerald-600 px-4 text-white shadow-sm transition-all hover:bg-emerald-700 hover:shadow-md active:scale-[0.97] dark:bg-emerald-500 dark:hover:bg-emerald-600"
                    onClick={handleInstallUpdate}
                    disabled={installingUpdate || checkingUpdate}
                  >
                    {installingUpdate ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Download className="h-3.5 w-3.5" />
                    )}
                    {installingUpdate
                      ? t("about.updateInstalling", { version: pendingUpdate.version })
                      : t("about.updateInstall", { version: pendingUpdate.version })}
                  </Button>
                </div>
                {installingUpdate && downloadPercent !== null && (
                  <div className="px-5 pb-4">
                    <div className="flex items-center justify-between text-xs text-muted-foreground">
                      <span>{t("about.updateDownloadProgress", { percent: downloadPercent })}</span>
                      <span className="tabular-nums">{downloadPercent}%</span>
                    </div>
                    <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-emerald-500/15">
                      <div
                        className="h-full rounded-full bg-gradient-to-r from-emerald-500 to-emerald-400 transition-all duration-300 ease-out"
                        style={{ width: `${downloadPercent}%` }}
                      />
                    </div>
                  </div>
                )}
              </div>
            )}

            {updateError && (
              <details className="mt-3 rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-2.5 text-xs">
                <summary className="cursor-pointer select-none font-medium text-destructive/90">
                  {t("about.updateErrorDetails")}
                </summary>
                <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed text-muted-foreground">
                  {updateError}
                </pre>
                <p className="mt-2 text-[11px] text-muted-foreground/80">
                  {t("about.updateErrorLogHint")}
                </p>
              </details>
            )}

            <p className="mt-6 max-w-4xl text-2xl font-semibold leading-tight tracking-tight text-foreground lg:text-4xl">
              {t("about.tagline")}
            </p>
            <p className="mt-4 max-w-3xl text-sm leading-7 text-muted-foreground lg:text-base">
              {t("about.description")}
            </p>

            <div className="mt-6 flex flex-wrap gap-3">
              <Button variant="outline" onClick={() => openExternal(HOPE_AGENT_URLS.github)}>
                {t("about.github")}
                <ExternalLink className="ml-1.5 h-4 w-4" />
              </Button>
              <Button variant="secondary" onClick={() => openExternal(HOPE_AGENT_URLS.releases)}>
                {t("about.releases")}
                <ExternalLink className="ml-1.5 h-4 w-4" />
              </Button>
              <Button variant="ghost" onClick={() => openExternal(HOPE_AGENT_URLS.feedback)}>
                {t("about.feedback")}
                <ExternalLink className="ml-1.5 h-4 w-4" />
              </Button>
            </div>
          </div>
        </section>

        <section className="grid gap-4 lg:grid-cols-3">
          {highlights.map((item) => {
            const Icon = item.icon
            return (
              <div key={item.title} className={`rounded-[24px] border p-5 ${item.cardClass}`}>
                <div
                  className={`flex h-11 w-11 items-center justify-center rounded-2xl ${item.iconClass}`}
                >
                  <Icon className="h-5 w-5" />
                </div>
                <h3 className="mt-5 text-lg font-semibold tracking-tight text-foreground">
                  {item.title}
                </h3>
                <p className="mt-2 text-sm leading-6 text-muted-foreground">{item.description}</p>
              </div>
            )
          })}
        </section>

        <section className="rounded-[24px] border border-border/70 bg-secondary/25 px-6 py-5">
          <p className="max-w-4xl text-sm leading-7 text-muted-foreground lg:text-base">
            {t("about.closing")}
          </p>
        </section>
      </div>

      <AlertDialog open={restartDialogOpen} onOpenChange={setRestartDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("about.restartConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("about.restartConfirmDesc")}</AlertDialogDescription>
          </AlertDialogHeader>
          {restartError && (
            <p className="text-xs text-destructive">
              {t("about.restartFailed", { error: restartError })}
            </p>
          )}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={restarting}>{t("about.restartCancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={(e) => {
                e.preventDefault()
                void handleConfirmRestart()
              }}
              disabled={restarting}
            >
              {restarting ? (
                <>
                  <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  {t("about.restarting")}
                </>
              ) : (
                t("about.restartConfirmAction")
              )}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

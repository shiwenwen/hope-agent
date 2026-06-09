import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import type { LucideIcon } from "lucide-react"
import {
  Brain,
  Check,
  Download,
  ExternalLink,
  Globe,
  History,
  Loader2,
  Monitor,
  RefreshCw,
  RotateCcw,
} from "lucide-react"
import alphaLogoUrl from "@/assets/alpha-logo.png"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { HOPE_AGENT_URLS, useAppVersion } from "@/lib/appMeta"
import {
  checkForDesktopUpdate,
  getAutoUpdateConfig,
  invalidateAutoUpdateConfig,
  isDesktopUpdaterAvailable,
  setPendingUpdate as setGlobalPendingUpdate,
  subscribeManualCheckRequests,
  type AutoUpdateConfig,
  type DesktopUpdate,
} from "@/lib/desktopUpdater"
import { useDesktopUpdateStore } from "@/hooks/useDesktopUpdateStore"
import { useDesktopUpdateInstall } from "@/hooks/useDesktopUpdateInstall"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"

interface HighlightItem {
  icon: LucideIcon
  title: string
  description: string
  cardClass: string
  iconClass: string
}

export default function AboutPanel({
  onOpenUpdateHistory,
}: {
  onOpenUpdateHistory?: () => void
}) {
  const { t } = useTranslation()
  const appVersion = useAppVersion()
  const { pendingUpdate: globalPendingUpdate } = useDesktopUpdateStore()
  const [checkingUpdate, setCheckingUpdate] = useState(false)
  const [pendingUpdate, setPendingUpdate] = useState<DesktopUpdate | null>(null)
  const [updateStatus, setUpdateStatus] = useState<string | null>(null)
  const [updateError, setUpdateError] = useState<string | null>(null)
  const desktopUpdaterAvailable = isDesktopUpdaterAvailable()

  // Shared install/restart lifecycle (same hook the App.tsx toast uses) so the
  // two surfaces stay in lockstep; failure + staged-restart states are handled
  // inside the hook.
  const {
    installing: installingUpdate,
    downloadPercent,
    awaitingRestart,
    install: runInstall,
    restartNow: runRestartNow,
  } = useDesktopUpdateInstall(pendingUpdate, {
    onError: (err) => {
      const detail = describeError(err)
      logger.error("updater", "AboutPanel::install", "install failed", {
        error: detail,
        fromVersion: appVersion,
        toVersion: pendingUpdate?.version,
      })
      setUpdateStatus(t("about.updateInstallFailed"))
      setUpdateError(detail)
    },
    beforeRelaunch: async () => {
      // Give the user a beat to see the "installed, restarting" message before
      // the process exits.
      setUpdateStatus(t("about.updateInstalled"))
      await new Promise((resolve) => setTimeout(resolve, 1500))
    },
  })

  // ── Auto-update preferences ──────────────────────────────────
  const [autoCfg, setAutoCfg] = useState<AutoUpdateConfig | null>(null)
  const [autoSaving, setAutoSaving] = useState(false)
  const [autoSaveStatus, setAutoSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  // Load auto-update config on both transports — the headless `hope-agent
  // server` runs the auto-update loop too, so its web UI must expose the same
  // toggles (the get/set_auto_update_config commands work over Tauri AND HTTP).
  useEffect(() => {
    let cancelled = false
    getAutoUpdateConfig(true)
      .then((cfg) => {
        if (!cancelled) setAutoCfg(cfg)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  async function saveAutoCfg(next: AutoUpdateConfig) {
    setAutoCfg(next)
    setAutoSaving(true)
    try {
      await getTransport().call("set_auto_update_config", { config: next })
      invalidateAutoUpdateConfig()
      setAutoSaveStatus("saved")
      setTimeout(() => setAutoSaveStatus("idle"), 2000)
    } catch (err) {
      logger.error("updater", "AboutPanel::saveAutoCfg", "save failed", {
        error: describeError(err),
      })
      setAutoSaveStatus("failed")
      setTimeout(() => setAutoSaveStatus("idle"), 2000)
    } finally {
      setAutoSaving(false)
    }
  }

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

  // Auto-check for updates once whenever the About page is opened, so the
  // version status is always fresh without the user clicking. Guarded to one
  // run per mount; a no-op off the desktop shell (and dev builds short-circuit
  // the real check). Manual re-checks still go through the button.
  const autoCheckedRef = useRef(false)
  useEffect(() => {
    if (!desktopUpdaterAvailable || autoCheckedRef.current) return
    autoCheckedRef.current = true
    checkRef.current()
  }, [desktopUpdaterAvailable])

  // `relaunchAfter` true ⇒ "更新并重启", false ⇒ "仅更新". The download / install
  // / staged-restart lifecycle (and failure handling) lives in the shared hook;
  // here we just set the "installing vX" status and delegate.
  function handleInstall(relaunchAfter: boolean) {
    if (!pendingUpdate) return
    setUpdateError(null)
    setUpdateStatus(t("about.updateInstalling", { version: pendingUpdate.version }))
    logger.info("updater", "AboutPanel::install", "install started", {
      fromVersion: appVersion,
      toVersion: pendingUpdate.version,
      relaunchAfter,
    })
    void runInstall(relaunchAfter)
  }

  async function handleRestartNow() {
    try {
      await runRestartNow()
    } catch (err) {
      setUpdateStatus(t("about.updateRestartManually"))
      setUpdateError(describeError(err))
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
                      className="h-auto gap-1.5 rounded-full border-border/50 bg-violet-100 px-3 py-1 text-xs font-medium text-violet-700 transition-all duration-200 hover:bg-violet-200 active:scale-[0.97] dark:bg-violet-500/15 dark:text-violet-300 dark:hover:bg-violet-500/25"
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
                  {updateStatus && !pendingUpdate && (
                    <span className="text-xs text-muted-foreground/70">{updateStatus}</span>
                  )}
                </div>
              </div>
            </div>

            {pendingUpdate && (
              <div className="mt-5 overflow-hidden rounded-2xl border border-emerald-500/20 bg-gradient-to-r from-emerald-500/8 via-emerald-500/5 to-transparent">
                <div className="flex flex-wrap items-start gap-3 px-5 py-4">
                  <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-emerald-500/12">
                    <Download className="h-4.5 w-4.5 text-emerald-600 dark:text-emerald-400" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-semibold text-foreground">
                      {awaitingRestart ? t("about.updateAwaitingRestart") : updateStatus}
                    </p>
                    {pendingUpdate.body && (
                      <div className="update-notes-markdown mt-1 max-h-48 overflow-auto text-xs leading-relaxed text-muted-foreground">
                        <MarkdownRenderer content={pendingUpdate.body} />
                      </div>
                    )}
                  </div>
                  {awaitingRestart ? (
                    <Button
                      size="sm"
                      className="mt-1 shrink-0 gap-1.5 rounded-full bg-emerald-600 px-4 text-white shadow-sm transition-all hover:bg-emerald-700 hover:shadow-md active:scale-[0.97] dark:bg-emerald-500 dark:hover:bg-emerald-600"
                      onClick={handleRestartNow}
                    >
                      <RotateCcw className="h-3.5 w-3.5" />
                      {t("about.restartNow")}
                    </Button>
                  ) : (
                    <div className="mt-1 flex shrink-0 items-center gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        className="gap-1.5 rounded-full"
                        onClick={() => handleInstall(false)}
                        disabled={installingUpdate || checkingUpdate}
                      >
                        {t("about.updateOnly")}
                      </Button>
                      <Button
                        size="sm"
                        className="gap-1.5 rounded-full bg-emerald-600 px-4 text-white shadow-sm transition-all hover:bg-emerald-700 hover:shadow-md active:scale-[0.97] dark:bg-emerald-500 dark:hover:bg-emerald-600"
                        onClick={() => handleInstall(true)}
                        disabled={installingUpdate || checkingUpdate}
                      >
                        {installingUpdate ? (
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Download className="h-3.5 w-3.5" />
                        )}
                        {installingUpdate
                          ? t("about.updateInstalling", { version: pendingUpdate.version })
                          : t("about.updateAndRestart")}
                      </Button>
                    </div>
                  )}
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
              {onOpenUpdateHistory && (
                <Button variant="outline" onClick={onOpenUpdateHistory}>
                  <History className="mr-1.5 h-4 w-4" />
                  {t("about.updateHistory")}
                </Button>
              )}
              <Button variant="ghost" onClick={() => openExternal(HOPE_AGENT_URLS.feedback)}>
                {t("about.feedback")}
                <ExternalLink className="ml-1.5 h-4 w-4" />
              </Button>
            </div>
          </div>
        </section>

        {autoCfg && (
          <section className="rounded-[24px] border border-border/70 bg-card px-6 py-5">
            <div className="flex items-center justify-between gap-4">
              <div>
                <h3 className="text-base font-semibold tracking-tight text-foreground">
                  {t("about.autoUpdateTitle")}
                </h3>
                <p className="mt-1 text-sm leading-6 text-muted-foreground">
                  {t("about.autoUpdateDesc")}
                </p>
              </div>
              {autoSaving ? (
                <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground" />
              ) : autoSaveStatus === "saved" ? (
                <Check className="h-4 w-4 shrink-0 text-green-500" />
              ) : null}
            </div>

            <div className="mt-4 flex flex-col divide-y divide-border/50">
              <div className="flex items-center justify-between gap-4 py-3">
                <span className="text-sm text-foreground">{t("about.autoUpdateCheck")}</span>
                <Switch
                  checked={autoCfg.checkEnabled}
                  disabled={autoSaving}
                  onCheckedChange={(v) => saveAutoCfg({ ...autoCfg, checkEnabled: v })}
                />
              </div>
              <div className="flex items-center justify-between gap-4 py-3">
                <span className="text-sm text-foreground">{t("about.autoUpdateInterval")}</span>
                <Input
                  type="number"
                  min={1}
                  max={168}
                  value={autoCfg.checkIntervalHours}
                  disabled={autoSaving || !autoCfg.checkEnabled}
                  className="h-8 w-24"
                  onChange={(e) =>
                    setAutoCfg({ ...autoCfg, checkIntervalHours: Number(e.target.value) || 1 })
                  }
                  onBlur={() => {
                    const clamped = Math.min(168, Math.max(1, autoCfg.checkIntervalHours || 12))
                    saveAutoCfg({ ...autoCfg, checkIntervalHours: clamped })
                  }}
                />
              </div>
              <div className="flex items-center justify-between gap-4 py-3">
                <span className="text-sm text-foreground">{t("about.autoUpdateDownload")}</span>
                <Switch
                  checked={autoCfg.autoDownload}
                  disabled={autoSaving || !autoCfg.checkEnabled}
                  onCheckedChange={(v) => saveAutoCfg({ ...autoCfg, autoDownload: v })}
                />
              </div>
              <div className="flex items-center justify-between gap-4 py-3">
                <span className="text-sm text-foreground">{t("about.autoUpdateNotify")}</span>
                <Switch
                  checked={autoCfg.notify}
                  disabled={autoSaving || !autoCfg.checkEnabled}
                  onCheckedChange={(v) => saveAutoCfg({ ...autoCfg, notify: v })}
                />
              </div>
            </div>
          </section>
        )}

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
    </div>
  )
}

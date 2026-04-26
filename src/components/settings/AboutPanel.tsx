import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import type { LucideIcon } from "lucide-react"
import { Brain, Download, ExternalLink, Globe, Loader2, Monitor, RefreshCw } from "lucide-react"
import logoUrl from "@/assets/logo.png"
import { Button } from "@/components/ui/button"
import { HOPE_AGENT_URLS, useAppVersion } from "@/lib/appMeta"
import {
  checkForDesktopUpdate,
  isDesktopUpdaterAvailable,
  relaunchDesktopApp,
  setPendingUpdate as setGlobalPendingUpdate,
  type DesktopUpdate,
} from "@/lib/desktopUpdater"
import { useDesktopUpdateStore } from "@/hooks/useDesktopUpdateStore"
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
  const [downloadPercent, setDownloadPercent] = useState<number | null>(null)
  const desktopUpdaterAvailable = isDesktopUpdaterAvailable()

  // Sync from global store: if auto-check found an update, reflect it here
  const syncedRef = useRef(false)
  useEffect(() => {
    if (globalPendingUpdate && !syncedRef.current) {
      syncedRef.current = true
      setPendingUpdate(globalPendingUpdate)
      setUpdateStatus(t("about.updateAvailable", { version: globalPendingUpdate.version }))
    }
  }, [globalPendingUpdate, t])

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
    } catch {
      setPendingUpdate(null)
      void setGlobalPendingUpdate(null)
      setUpdateStatus(t("about.updateCheckFailed"))
    } finally {
      setCheckingUpdate(false)
    }
  }

  async function handleInstallUpdate() {
    if (!pendingUpdate) return

    setInstallingUpdate(true)
    setDownloadPercent(0)
    setUpdateStatus(t("about.updateInstalling", { version: pendingUpdate.version }))

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
      await relaunchDesktopApp()
    } catch {
      setUpdateStatus(t("about.updateInstallFailed"))
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
              <div className="flex h-20 w-20 items-center justify-center rounded-[22px] border border-border/70 bg-secondary/30 p-2">
                <img
                  src={logoUrl}
                  alt="Hope Agent"
                  className="h-full w-full rounded-[18px] object-cover"
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
                  {desktopUpdaterAvailable && !pendingUpdate && (
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
    </div>
  )
}

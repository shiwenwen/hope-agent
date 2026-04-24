import { useState, useEffect, useCallback, useRef, lazy, Suspense } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { initLanguageFromConfig, listenLanguageConfigChange } from "@/i18n/i18n"
import { initThemeFromConfig, listenThemeConfigChange } from "@/hooks/useTheme"
import { listenNotificationConfigChange, notify } from "@/lib/notifications"
import {
  autoCheckForUpdate,
  relaunchDesktopApp,
  setPendingUpdate as setGlobalPendingUpdate,
  startPeriodicUpdateCheck,
} from "@/lib/desktopUpdater"
import { useDesktopUpdateStore } from "@/hooks/useDesktopUpdateStore"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { LightboxProvider } from "@/components/common/ImageLightbox"
import ErrorBoundary from "@/components/common/ErrorBoundary"
import ProviderSetup from "@/components/settings/ProviderSetup"
import SettingsView from "@/components/settings/SettingsView"
import type { SettingsSection } from "@/components/settings/types"
import OnboardingWizard from "@/components/onboarding"
import { CURRENT_ONBOARDING_VERSION } from "@/components/onboarding/version"
import IconSidebar from "@/components/common/IconSidebar"
import ChatScreen from "@/components/chat/ChatScreen"
import StarrySky from "@/components/common/StarrySky"
import DangerousModeBanner from "@/components/common/DangerousModeBanner"

// Lazy-loaded views (heavy dependencies: recharts, cron UI)
const DashboardView = lazy(() => import("@/components/dashboard/DashboardView"))
const CronCalendarView = lazy(() => import("@/components/cron/CronCalendarView"))

export default function App() {
  const { t, i18n } = useTranslation()
  const [view, setView] = useState<
    | "loading"
    | "onboarding"
    | "setup"
    | "chat"
    | "settings"
    | "skills"
    | "profile"
    | "agents"
    | "channels"
    | "calendar"
    | "dashboard"
  >("loading")
  const [agentIdForSettings, setAgentIdForSettings] = useState<string | undefined>(undefined)
  const [settingsInitialSection, setSettingsInitialSection] = useState<SettingsSection | undefined>(
    undefined,
  )
  const [userAvatar, setUserAvatar] = useState<string | null>(null)
  const [pendingSessionId, setPendingSessionId] = useState<string | undefined>(undefined)
  const [totalUnreadCount, setTotalUnreadCount] = useState(0)
  const [sessionsRefreshTrigger, setSessionsRefreshTrigger] = useState(0)
  const { pendingUpdate: globalPendingUpdate } = useDesktopUpdateStore()
  const [dismissedVersion, setDismissedVersion] = useState<string | null>(null)
  const [showIgnoreOptions, setShowIgnoreOptions] = useState(false)

  const ignoredVersion = localStorage.getItem("ignored_update_version")
  const shouldShowToast =
    globalPendingUpdate &&
    globalPendingUpdate.version !== dismissedVersion &&
    globalPendingUpdate.version !== ignoredVersion

  const [installingUpdate, setInstallingUpdate] = useState(false)
  const [downloadPercent, setDownloadPercent] = useState<number | null>(null)

  async function handleInstallUpdate() {
    if (!globalPendingUpdate) return

    setInstallingUpdate(true)
    setDownloadPercent(0)

    let downloaded = 0
    let contentLength = 0

    try {
      await globalPendingUpdate.downloadAndInstall((event) => {
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

      void setGlobalPendingUpdate(null)
      await relaunchDesktopApp()
    } catch (e) {
      logger.error("update", "App::handleInstallUpdate", "Failed to install update via toast", e)
      setInstallingUpdate(false)
      if (globalPendingUpdate?.version) {
        setDismissedVersion(globalPendingUpdate.version)
      }
    }
  }

  // Load user avatar
  async function fetchUserAvatar() {
    try {
      const config = await getTransport().call<{ avatar?: string | null }>("get_user_config")
      return config.avatar ?? null
    } catch {
      return null
    }
  }

  // Reload avatar when switching back to chat
  useEffect(() => {
    if (view === "chat") {
      let cancelled = false
      fetchUserAvatar().then((avatar) => {
        if (!cancelled) setUserAvatar(avatar)
      })
      return () => {
        cancelled = true
      }
    }
  }, [view])

  // Cmd+, on macOS, Ctrl+, on Windows/Linux — "preferences" convention.
  const handleOpenSettings = useCallback((section?: SettingsSection) => {
    setSettingsInitialSection(section)
    setView("settings")
  }, [])
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === ",") {
        e.preventDefault()
        handleOpenSettings()
      }
    }
    document.addEventListener("keydown", onKeyDown)
    return () => document.removeEventListener("keydown", onKeyDown)
  }, [handleOpenSettings])

  // Listen for system tray events + config hot-reload
  useEffect(() => {
    const unlistenSettings = getTransport().listen("open-settings", () => {
      setView("settings")
    })
    const unlistenNewSession = getTransport().listen("new-session", () => {
      setView("chat")
    })
    const unlistenLanguage = listenLanguageConfigChange()
    const unlistenTheme = listenThemeConfigChange()
    const unlistenNotification = listenNotificationConfigChange()
    return () => {
      unlistenSettings()
      unlistenNewSession()
      unlistenLanguage()
      unlistenTheme()
      unlistenNotification()
    }
  }, [])

  // Auto-check for desktop updates on startup
  const updateCheckRef = useRef(false)
  useEffect(() => {
    if (updateCheckRef.current) return
    if (view === "loading" || view === "onboarding" || view === "setup") return
    updateCheckRef.current = true

    autoCheckForUpdate()
      .then((update) => {
        if (update) {
          void notify("Hope Agent", t("about.updateAvailable", { version: update.version }))
        }
      })
      .catch(() => {})

    // Start background periodic check (e.g., every 12 hours)
    const cleanupPeriodic = startPeriodicUpdateCheck()

    return () => {
      cleanupPeriodic()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view])

  // Try to restore previous session on mount
  useEffect(() => {
    ;(async () => {
      try {
        // Load language preference from backend config.json
        await initLanguageFromConfig()
        await initThemeFromConfig()
        const avatar = await fetchUserAvatar()
        setUserAvatar(avatar)
        // Decide initial view in this order:
        //   1. Onboarding wizard outstanding → "onboarding"
        //   2. Prior session restorable → "chat"
        //   3. Has a provider configured (legacy users) → "chat"
        //   4. Otherwise → "setup" (the old provider-only fallback)
        const onboarding = await getTransport()
          .call<{ completedVersion?: number }>("get_onboarding_state")
          .catch(() => ({ completedVersion: 0 }))
        if ((onboarding.completedVersion ?? 0) < CURRENT_ONBOARDING_VERSION) {
          setView("onboarding")
          return
        }
        const restored = await getTransport().call<boolean>("try_restore_session")
        if (restored) {
          setView("chat")
        } else {
          const has = await getTransport().call<boolean>("has_providers")
          setView(has ? "chat" : "setup")
        }
      } catch (e) {
        logger.error("app", "App::init", "Failed to restore session", e)
        setView("setup")
      }
    })()
  }, [])

  // Codex OAuth — auth only, no view switch. Callers decide what to do
  // next (setup screen jumps to chat; onboarding advances to the next step).
  async function runCodexAuth(): Promise<void> {
    await getTransport().call("start_codex_auth")
    for (let i = 0; i < 300; i++) {
      await new Promise((r) => setTimeout(r, 1000))
      const status = await getTransport().call<{
        authenticated: boolean
        error: string | null
      }>("check_auth_status")
      if (status.authenticated) {
        await getTransport().call("finalize_codex_auth")
        return
      }
      if (status.error) {
        throw new Error(status.error)
      }
    }
    throw new Error("Login timed out")
  }

  async function handleCodexAuth() {
    await runCodexAuth()
    setView("chat")
  }

  if (view === "loading") {
    return (
      <div className="flex items-center justify-center h-screen">
        <StarrySky />
        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
      </div>
    )
  }

  if (view === "onboarding") {
    return (
      <TooltipProvider>
        <div className="flex flex-col h-screen overflow-y-auto">
          <StarrySky />
          <Toaster />
          <DangerousModeBanner />
          <OnboardingWizard
            onComplete={() => setView("chat")}
            onJumpToChannelsSettings={() => setView("channels")}
            onCodexAuth={runCodexAuth}
            initialLanguage={i18n.language || ""}
          />
        </div>
      </TooltipProvider>
    )
  }

  if (view === "setup") {
    return (
      <TooltipProvider>
        <div className="flex flex-col h-screen overflow-hidden">
          <StarrySky />
          <Toaster />
          <DangerousModeBanner />
          <div className="flex-1 min-h-0 overflow-hidden">
            <ProviderSetup onComplete={() => setView("chat")} onCodexAuth={handleCodexAuth} />
          </div>
        </div>
      </TooltipProvider>
    )
  }

  return (
    <ErrorBoundary>
      <TooltipProvider>
        <LightboxProvider>
          <div className="flex flex-col h-screen overflow-hidden bg-background">
            <StarrySky />
            <Toaster />
            <DangerousModeBanner />
            <div className="flex flex-1 min-h-0 overflow-hidden">
              <IconSidebar
                view={view === "loading" || view === "setup" ? "chat" : view}
                onOpenSettings={handleOpenSettings}
                onOpenChat={() => setView("chat")}
                onOpenAgents={() => {
                  setAgentIdForSettings(undefined)
                  setView("agents")
                }}
                onOpenSkills={() => setView("skills")}
                onOpenChannels={() => setView("channels")}
                onOpenProfile={() => {
                  setView("profile")
                }}
                onOpenCalendar={() => setView("calendar")}
                onOpenDashboard={() => setView("dashboard")}
                userAvatar={userAvatar}
                totalUnreadCount={totalUnreadCount}
                onMarkAllRead={() => setSessionsRefreshTrigger((n) => n + 1)}
              />
              {view === "settings" && (
                <SettingsView
                  onBack={() => setView("chat")}
                  onCodexAuth={handleCodexAuth}
                  onCodexReauth={handleCodexAuth}
                  initialSection={settingsInitialSection}
                />
              )}
              {view === "skills" && (
                <SettingsView
                  onBack={() => setView("chat")}
                  onCodexAuth={handleCodexAuth}
                  onCodexReauth={handleCodexAuth}
                  initialSection="skills"
                />
              )}
              {view === "profile" && (
                <SettingsView
                  onBack={() => setView("chat")}
                  onCodexAuth={handleCodexAuth}
                  onCodexReauth={handleCodexAuth}
                  initialSection="profile"
                  onProfileSaved={() => fetchUserAvatar().then(setUserAvatar)}
                />
              )}
              {view === "agents" && (
                <SettingsView
                  onBack={() => {
                    setView("chat")
                    setAgentIdForSettings(undefined)
                  }}
                  onCodexAuth={handleCodexAuth}
                  onCodexReauth={handleCodexAuth}
                  initialSection="agents"
                  initialAgentId={agentIdForSettings}
                />
              )}
              {view === "channels" && (
                <SettingsView
                  onBack={() => setView("chat")}
                  onCodexAuth={handleCodexAuth}
                  onCodexReauth={handleCodexAuth}
                  initialSection="channels"
                />
              )}
              {view === "calendar" && (
                <Suspense
                  fallback={
                    <div className="flex-1 flex items-center justify-center">
                      <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                    </div>
                  }
                >
                  <CronCalendarView
                    onBack={() => setView("chat")}
                    onNavigateToSession={(sessionId) => {
                      setPendingSessionId(sessionId)
                      setView("chat")
                    }}
                  />
                </Suspense>
              )}
              {view === "dashboard" && (
                <Suspense
                  fallback={
                    <div className="flex-1 flex items-center justify-center">
                      <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                    </div>
                  }
                >
                  <DashboardView onBack={() => setView("chat")} />
                </Suspense>
              )}
              <div className={view === "chat" ? "flex-1 flex overflow-hidden" : "hidden"}>
                <ChatScreen
                  onOpenAgentSettings={(agentId) => {
                    setAgentIdForSettings(agentId)
                    setView("agents")
                  }}
                  onCodexReauth={handleCodexAuth}
                  initialSessionId={pendingSessionId}
                  onSessionNavigated={() => setPendingSessionId(undefined)}
                  onUnreadCountChange={setTotalUnreadCount}
                  sessionsRefreshTrigger={sessionsRefreshTrigger}
                />
              </div>

              {/* In-app update toast */}
              {shouldShowToast && (
                <div className="fixed top-6 right-6 z-50 animate-in slide-in-from-top-5 fade-in duration-300">
                  <div className="relative flex flex-col gap-3 rounded-2xl border border-emerald-500/20 bg-card p-4 shadow-xl dark:bg-zinc-900/90 w-[380px]">
                    {/* Close / Ignore button */}
                    {!showIgnoreOptions && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation()
                          setShowIgnoreOptions(true)
                        }}
                        className="absolute top-3 right-3 p-1.5 text-muted-foreground hover:text-foreground hover:bg-secondary rounded-lg transition-colors z-10"
                      >
                        <svg
                          width="14"
                          height="14"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2.5"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                        >
                          <path d="M18 6 6 18" />
                          <path d="m6 6 12 12" />
                        </svg>
                      </button>
                    )}

                    {showIgnoreOptions ? (
                      <div className="flex flex-col gap-3 animate-in fade-in zoom-in-95 duration-200">
                        <p className="text-sm font-medium text-foreground text-center">
                          不再提醒 {globalPendingUpdate.version} 版本？
                        </p>
                        <div className="flex gap-2 justify-center">
                          <button
                            className="flex-1 text-xs font-medium text-muted-foreground bg-secondary hover:bg-secondary/80 px-3 py-2 rounded-lg transition-colors"
                            onClick={() => {
                              setDismissedVersion(globalPendingUpdate.version)
                              setShowIgnoreOptions(false)
                            }}
                          >
                            仅本次忽略
                          </button>
                          <button
                            className="flex-1 text-xs font-medium text-destructive bg-destructive/10 hover:bg-destructive/20 px-3 py-2 rounded-lg transition-colors"
                            onClick={() => {
                              localStorage.setItem(
                                "ignored_update_version",
                                globalPendingUpdate.version,
                              )
                              setDismissedVersion(globalPendingUpdate.version)
                              setShowIgnoreOptions(false)
                            }}
                          >
                            该版本不再提醒
                          </button>
                        </div>
                      </div>
                    ) : installingUpdate ? (
                      <div className="flex flex-col gap-2 mt-1">
                        <div className="flex items-center justify-between pr-6">
                          <p className="text-sm font-medium text-foreground">正在下载更新...</p>
                          <p className="text-sm font-medium text-emerald-500">
                            {downloadPercent ?? 0}%
                          </p>
                        </div>
                        <div className="h-1.5 w-full bg-secondary overflow-hidden rounded-full mt-1">
                          <div
                            className="h-full bg-emerald-500 transition-all duration-300 rounded-full"
                            style={{ width: `${downloadPercent ?? 0}%` }}
                          />
                        </div>
                      </div>
                    ) : (
                      <div
                        className="flex items-start gap-4 cursor-pointer group"
                        onClick={() => {
                          setDismissedVersion(globalPendingUpdate.version)
                          handleOpenSettings("about")
                        }}
                      >
                        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-emerald-500/10 text-emerald-500 group-hover:bg-emerald-500 group-hover:text-white transition-colors duration-300 mt-1">
                          <svg
                            width="20"
                            height="20"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="2"
                            strokeLinecap="round"
                            strokeLinejoin="round"
                          >
                            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                            <polyline points="7 10 12 15 17 10" />
                            <line x1="12" x2="12" y1="15" y2="3" />
                          </svg>
                        </div>
                        <div className="flex-1 min-w-0 pr-5">
                          <p className="text-sm font-semibold text-foreground group-hover:text-emerald-500 transition-colors truncate">
                            {i18n.language.startsWith("zh")
                              ? `发现新版本 v${globalPendingUpdate.version}`
                              : `Update v${globalPendingUpdate.version}`}
                          </p>
                          <div className="mt-2.5 max-h-[180px] overflow-y-auto pr-2 scrollbar-thin scrollbar-thumb-muted-foreground/20 hover:scrollbar-thumb-muted-foreground/40 scrollbar-track-transparent">
                            <p className="text-xs text-muted-foreground leading-relaxed whitespace-pre-wrap">
                              {globalPendingUpdate.body ||
                                t("about.updateAvailable", {
                                  version: globalPendingUpdate.version,
                                })}
                            </p>
                          </div>
                          <div className="mt-4 flex justify-end">
                            <button
                              onClick={(e) => {
                                e.stopPropagation()
                                handleInstallUpdate()
                              }}
                              className="px-4 py-2 text-xs font-semibold bg-emerald-500/10 text-emerald-600 hover:bg-emerald-500 hover:text-white rounded-lg transition-colors duration-200 dark:text-emerald-400 dark:hover:text-white"
                            >
                              {i18n.language.startsWith("zh") ? "立即更新" : "Update"}
                            </button>
                          </div>
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              )}
            </div>
          </div>
        </LightboxProvider>
      </TooltipProvider>
    </ErrorBoundary>
  )
}

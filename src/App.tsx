import { useState, useEffect, useCallback, lazy, Suspense } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { initLanguageFromConfig, listenLanguageConfigChange } from "@/i18n/i18n"
import { listenNotificationConfigChange } from "@/lib/notifications"
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
  const { i18n } = useTranslation()
  const [view, setView] = useState<
    "loading" | "onboarding" | "setup" | "chat" | "settings" | "skills" | "profile" | "agents" | "channels" | "calendar" | "dashboard"
  >("loading")
  const [agentIdForSettings, setAgentIdForSettings] = useState<string | undefined>(undefined)
  const [settingsInitialSection, setSettingsInitialSection] = useState<
    SettingsSection | undefined
  >(undefined)
  const [userAvatar, setUserAvatar] = useState<string | null>(null)
  const [pendingSessionId, setPendingSessionId] = useState<string | undefined>(undefined)
  const [totalUnreadCount, setTotalUnreadCount] = useState(0)
  const [sessionsRefreshTrigger, setSessionsRefreshTrigger] = useState(0)

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
    const unlistenNotification = listenNotificationConfigChange()
    return () => {
      unlistenSettings()
      unlistenNewSession()
      unlistenLanguage()
      unlistenNotification()
    }
  }, [])

  // Try to restore previous session on mount
  useEffect(() => {
    ;(async () => {
      try {
        // Load language preference from backend config.json
        await initLanguageFromConfig()
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
        <Suspense fallback={<div className="flex-1 flex items-center justify-center"><div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" /></div>}>
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
        <Suspense fallback={<div className="flex-1 flex items-center justify-center"><div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" /></div>}>
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
      </div>
    </div>
    </LightboxProvider>
    </TooltipProvider>
    </ErrorBoundary>
  )
}

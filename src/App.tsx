import { useState, useEffect, useCallback, useRef, lazy, Suspense } from "react"
import { useTranslation } from "react-i18next"
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"
import { getTransport, setDirtyTransportConfirmText } from "@/lib/transport-provider"
import { parsePayload, isTauriMode } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { MAIN_WINDOW_MIN_HEIGHT, MAIN_WINDOW_MIN_WIDTH } from "@/lib/mainWindowSize"
import { initLanguageFromConfig, listenLanguageConfigChange } from "@/i18n/i18n"
import { initThemeFromConfig, listenThemeConfigChange } from "@/hooks/useTheme"
import { initFocusTracking, listenNotificationConfigChange, notify } from "@/lib/notifications"
import { useDesktopAlerts } from "@/hooks/useDesktopAlerts"
import {
  autoCheckForUpdate,
  requestManualCheck,
  startPeriodicUpdateCheck,
} from "@/lib/desktopUpdater"
import { useDesktopUpdateStore } from "@/hooks/useDesktopUpdateStore"
import { useDesktopUpdateInstall } from "@/hooks/useDesktopUpdateInstall"
import { initDraftSkillsStore } from "@/hooks/useDraftSkillsStore"
import { initCronUnreadStore } from "@/hooks/useCronUnreadStore"
import { openExternalUrl } from "@/lib/openExternalUrl"
import { deliverAskAi, listenAskAi } from "@/lib/manual/askAi"
import { openHelpWindow } from "@/lib/manual/openHelpWindow"
import { SKILLS_EVENTS } from "@/types/skills"
import { Toaster } from "@/components/ui/sonner"
import { toast } from "sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { LightboxProvider } from "@/components/common/ImageLightbox"
import ErrorBoundary from "@/components/common/ErrorBoundary"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { AuthRequiredDialog } from "@/components/AuthRequiredDialog"
import ProviderSetup from "@/components/settings/ProviderSetup"
import type { SettingsSection } from "@/components/settings/types"
import type { AgentTab } from "@/components/settings/agent-panel/types"
import { parseOpenSettingsSection } from "@/components/settings/openSettingsEvent"
import OnboardingWizard from "@/components/onboarding"
import { CURRENT_ONBOARDING_VERSION } from "@/components/onboarding/version"
import ConfigRecoveryScreen, { type ConfigHealth } from "@/components/config/ConfigRecoveryScreen"
import IconSidebar from "@/components/common/IconSidebar"
import ChatScreen, { type ChatInsert } from "@/components/chat/ChatScreen"
import { subscribeChatFocus, type ChatFocusTarget } from "@/components/chat/chatFocus"
import {
  parseMemoryFocusFromLocation,
  requestMemoryFocus,
} from "@/components/settings/memory-panel/memoryFocus"
import {
  consumePendingMemoryScopeFocus,
  subscribeMemoryScopeFocus,
  type MemoryScopeFocusTarget,
} from "@/components/settings/memory-panel/scopeFocus"
import StarrySky from "@/components/common/StarrySky"
import DangerousModeBanner from "@/components/common/DangerousModeBanner"
import MissingModelDialog from "@/components/local-model/MissingModelDialog"
import ChromiumRuntimeDialog from "@/components/common/ChromiumRuntimeDialog"
import {
  LOCAL_MODEL_JOB_EVENTS,
  type LocalModelJobSnapshot,
} from "@/types/local-model-jobs"

// Lazy-loaded views (heavy dependencies: recharts, cron UI, settings 面板群)
const DashboardView = lazy(() => import("@/components/dashboard/DashboardView"))
const CronCalendarView = lazy(() => import("@/components/cron/CronCalendarView"))
const PlansView = lazy(() => import("@/components/plans/PlansView"))
const KnowledgeView = lazy(() => import("@/components/knowledge/KnowledgeView"))
const DesignView = lazy(() => import("@/components/design/DesignView"))
const ArtifactsView = lazy(() => import("@/components/artifacts/ArtifactsView"))
const SettingsView = lazy(() => import("@/components/settings/SettingsView"))

type AppView =
  | "loading"
  | "configRecovery"
  | "onboarding"
  | "setup"
  | "chat"
  | "settings"
  | "skills"
  | "profile"
  | "agents"
  | "modelConfig"
  | "memory"
  | "channels"
  | "calendar"
  | "dashboard"
  | "plans"
  | "knowledge"
  | "design"
  | "artifacts"

interface PendingChatFocus extends ChatFocusTarget {
  nonce: number
}

interface PendingProjectFocus {
  projectId: string
  nonce: number
}

export default function App() {
  const { t, i18n } = useTranslation()
  const [view, setView] = useState<AppView>("loading")
  const [agentIdForSettings, setAgentIdForSettings] = useState<string | undefined>(undefined)
  const [agentTabForSettings, setAgentTabForSettings] = useState<AgentTab | undefined>(undefined)
  const [settingsInitialSection, setSettingsInitialSection] = useState<SettingsSection | undefined>(
    undefined,
  )
  // `settings:navigate` 深链的 modelTab（如 embeddingModels / mediaModels）；
  // SettingsView 未挂载时事件监听不到，须经 prop 传入初值。
  const [settingsInitialModelTab, setSettingsInitialModelTab] = useState<string | undefined>(
    undefined,
  )
  const [settingsInitialSectionRequestKey, setSettingsInitialSectionRequestKey] = useState(0)
  // 记住进设置前所在的视图，返回时回到那里（而非硬编码回 chat）。
  const [settingsReturnView, setSettingsReturnView] = useState<AppView>("chat")
  const viewRef = useRef<AppView>(view)
  viewRef.current = view
  const [dashboardInitialTab, setDashboardInitialTab] = useState<string | undefined>(undefined)
  const [dashboardInitialReportId, setDashboardInitialReportId] = useState<string | null>(null)
  const [userAvatar, setUserAvatar] = useState<string | null>(null)
  const [pendingSessionId, setPendingSessionId] = useState<string | undefined>(undefined)
  const [currentChatProjectId, setCurrentChatProjectId] = useState<string | null>(null)
  const [configHealth, setConfigHealth] = useState<ConfigHealth | null>(null)
  // PlansView pushes `@plan:<short_id>:v<n>` tokens here; KnowledgeView pushes
  // `[[note]]` refs (with a KB to auto-attach). ChatScreen appends + clears.
  const [pendingChatInsert, setPendingChatInsert] = useState<ChatInsert | undefined>(undefined)
  // 设计空间「实现到代码」：跳到实现会话后把 handoff pack 作首条消息自动发送（一次性，nonce 防重放）。
  const [pendingAutoSend, setPendingAutoSend] = useState<
    { sessionId: string; message: string; nonce: number } | undefined
  >(undefined)
  const [pendingChatFocus, setPendingChatFocus] = useState<PendingChatFocus | null>(null)
  const [pendingProjectFocus, setPendingProjectFocus] = useState<PendingProjectFocus | null>(null)
  const [totalUnreadCount, setTotalUnreadCount] = useState(0)
  const [unreadFocusSignal, setUnreadFocusSignal] = useState(0)
  const [sessionsRefreshTrigger, setSessionsRefreshTrigger] = useState(0)
  const { pendingUpdate: globalPendingUpdate, downloadStatus } = useDesktopUpdateStore()
  const [dismissedVersion, setDismissedVersion] = useState<string | null>(null)
  const [showIgnoreOptions, setShowIgnoreOptions] = useState(false)
  const [forceShowUpdatePanel, setForceShowUpdatePanel] = useState(false)

  const completedLocalModelJobToasts = useRef<Set<string>>(new Set())
  const chatFocusNonceRef = useRef(0)
  const projectFocusNonceRef = useRef(0)
  const lastMemoryFocusHashRef = useRef<string | null>(null)

  useEffect(() => {
    setDirtyTransportConfirmText(
      t(
        "fileEditor.transportSwitchUnsaved",
        "You have unsaved file changes. Switch connection and discard them?",
      ),
    )
  }, [i18n.language, t])

  useEffect(() => {
    if (!isTauriMode()) return

    const enforceMainWindowMinSize = async () => {
      const win = getCurrentWindow()
      const minSize = new LogicalSize(MAIN_WINDOW_MIN_WIDTH, MAIN_WINDOW_MIN_HEIGHT)
      await win.setMinSize(minSize)

      const [innerSize, scaleFactor] = await Promise.all([win.innerSize(), win.scaleFactor()])
      const logicalSize = innerSize.toLogical(scaleFactor)
      const width = Math.max(logicalSize.width, MAIN_WINDOW_MIN_WIDTH)
      const height = Math.max(logicalSize.height, MAIN_WINDOW_MIN_HEIGHT)
      if (width !== logicalSize.width || height !== logicalSize.height) {
        await win.setSize(new LogicalSize(width, height))
      }
    }

    enforceMainWindowMinSize().catch((err) => {
      logger.warn("window", "App::enforceMainWindowMinSize", "Failed to enforce min size", err)
    })
  }, [])

  // Shared desktop-update install/restart lifecycle (also drives AboutPanel),
  // so the toast and the settings surface can't drift and the failure / staged
  // states are handled in one place.
  const {
    installing: installingUpdate,
    downloadPercent,
    awaitingRestart,
    install: runInstall,
    restartNow,
  } = useDesktopUpdateInstall(globalPendingUpdate, {
    onError: (e) => {
      logger.error("update", "App::install", "Failed to install update via toast", e)
      if (globalPendingUpdate?.version) setDismissedVersion(globalPendingUpdate.version)
      setForceShowUpdatePanel(false)
    },
  })

  const ignoredVersion = localStorage.getItem("ignored_update_version")
  const shouldAutoShowUpdatePanel =
    globalPendingUpdate &&
    globalPendingUpdate.version !== dismissedVersion &&
    globalPendingUpdate.version !== ignoredVersion
  const shouldShowUpdatePanel =
    !!globalPendingUpdate &&
    (forceShowUpdatePanel || installingUpdate || awaitingRestart || !!shouldAutoShowUpdatePanel)

  useEffect(() => {
    setForceShowUpdatePanel(false)
    setShowIgnoreOptions(false)
  }, [globalPendingUpdate?.version])

  const handleOpenUpdatePanel = useCallback(() => {
    if (!globalPendingUpdate) return
    setShowIgnoreOptions(false)
    setForceShowUpdatePanel(true)
  }, [globalPendingUpdate])

  // Mirror the authoritative regular unread-session total onto native surfaces:
  // Dock shows the exact count while the compact tray icon uses a boolean dot.
  // Desktop-only (no-op on HTTP/web). The total already excludes the active
  // session, Cron, Knowledge, IM, incognito, and sub-agents.
  useEffect(() => {
    if (!isTauriMode()) return
    void Promise.allSettled([
      getTransport().call("set_dock_badge_cmd", { count: totalUnreadCount }),
      getTransport().call("set_tray_unread_cmd", { hasUnread: totalUnreadCount > 0 }),
    ])
  }, [totalUnreadCount])

  // Load user avatar
  const fetchUserAvatar = useCallback(async () => {
    try {
      const config = await getTransport().call<{ avatar?: string | null }>("get_user_config")
      return config.avatar ?? null
    } catch {
      return null
    }
  }, [])

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
  }, [view, fetchUserAvatar])

  const keepConfigRecoveryView = useCallback(() => {
    if (configHealth?.ok === false) {
      setView("configRecovery")
      return true
    }
    return false
  }, [configHealth])

  // Cmd+, on macOS, Ctrl+, on Windows/Linux — "preferences" convention.
  const handleOpenSettings = useCallback(
    (section?: SettingsSection, modelTab?: string) => {
    if (keepConfigRecoveryView()) return
    // 记住来源视图（非 settings 本身），返回时回去。
    if (viewRef.current !== "settings") setSettingsReturnView(viewRef.current)
    setSettingsInitialSection(section)
    setSettingsInitialModelTab(modelTab)
    setSettingsInitialSectionRequestKey((n) => n + 1)
    setView("settings")
    },
    [keepConfigRecoveryView],
  )

  useEffect(() => {
    const handleNavigate = (event: Event) => {
      const detail = (
        event as CustomEvent<{ section?: SettingsSection; modelTab?: string }>
      ).detail
      handleOpenSettings(detail?.section, detail?.modelTab)
    }
    window.addEventListener("settings:navigate", handleNavigate)
    return () => window.removeEventListener("settings:navigate", handleNavigate)
  }, [handleOpenSettings])

  const handleMemoryFocusDeepLink = useCallback(() => {
    if (typeof window === "undefined") return false
    const target = parseMemoryFocusFromLocation()
    if (!target) {
      lastMemoryFocusHashRef.current = null
      return false
    }
    const hash = window.location.hash
    if (lastMemoryFocusHashRef.current === hash && view === "settings") return true
    lastMemoryFocusHashRef.current = hash
    requestMemoryFocus(target, { updateUrl: false })
    handleOpenSettings("memory")
    return true
  }, [handleOpenSettings, view])

  useEffect(() => {
    if (typeof window === "undefined") return
    const onHashChange = () => {
      handleMemoryFocusDeepLink()
    }
    window.addEventListener("hashchange", onHashChange)
    return () => window.removeEventListener("hashchange", onHashChange)
  }, [handleMemoryFocusDeepLink])

  useEffect(() => {
    if (
      view === "loading" ||
      view === "configRecovery" ||
      view === "onboarding" ||
      view === "setup"
    ) {
      return
    }
    handleMemoryFocusDeepLink()
  }, [handleMemoryFocusDeepLink, view])

  const handleOpenDashboard = useCallback(
    (tab?: string, reportId?: string | null) => {
    if (keepConfigRecoveryView()) return
    setDashboardInitialTab(tab)
    setDashboardInitialReportId(reportId ?? null)
    setView("dashboard")
    },
    [keepConfigRecoveryView],
  )
  const handleOpenKnowledge = useCallback(() => {
    if (keepConfigRecoveryView()) return
    setView("knowledge")
  }, [keepConfigRecoveryView])

  const handleOpenChat = useCallback(() => {
    if (keepConfigRecoveryView()) return
    if (view === "chat") {
      setUnreadFocusSignal((value) => value + 1)
    }
    setView("chat")
  }, [keepConfigRecoveryView, view])

  const handleChatFocus = useCallback(
    (target: ChatFocusTarget) => {
      if (keepConfigRecoveryView()) return
      const nonce = chatFocusNonceRef.current + 1
      chatFocusNonceRef.current = nonce
      setPendingChatFocus({ ...target, nonce })
      setView("chat")
    },
    [keepConfigRecoveryView],
  )

  useEffect(() => subscribeChatFocus(handleChatFocus), [handleChatFocus])

  const handleMemoryScopeFocus = useCallback(
    (target: MemoryScopeFocusTarget) => {
      if (keepConfigRecoveryView()) return
      if (target.kind === "agent") {
        setAgentIdForSettings(target.id)
        setAgentTabForSettings(target.agentTab)
        setView("agents")
        return
      }
      const nonce = projectFocusNonceRef.current + 1
      projectFocusNonceRef.current = nonce
      setPendingProjectFocus({ projectId: target.id, nonce })
      setView("chat")
    },
    [keepConfigRecoveryView],
  )

  useEffect(() => subscribeMemoryScopeFocus(handleMemoryScopeFocus), [handleMemoryScopeFocus])

  useEffect(() => {
    const pending = consumePendingMemoryScopeFocus()
    if (pending) handleMemoryScopeFocus(pending)
  }, [handleMemoryScopeFocus])

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
    const unlistenSettings = getTransport().listen("open-settings", (raw) => {
      handleOpenSettings(parseOpenSettingsSection(raw))
    })
    const unlistenNewSession = getTransport().listen("new-session", () => {
      if (keepConfigRecoveryView()) return
      setView("chat")
    })
    // macOS app menu's "Check for Updates..." emits this alongside
    // `open-settings`. Registered at App level (always mounted) so the
    // request isn't lost when AboutPanel hasn't mounted yet — the request
    // is queued in the desktopUpdater store and replayed on subscribe.
    const unlistenUpdateCheck = getTransport().listen("desktop-update-check", () => {
      requestManualCheck()
    })
    // Native menu / tray "Help" entries (Rust side can't create the webview
    // window with its full config; it asks the frontend to).
    const unlistenOpenHelp = getTransport().listen("open-help", () => {
      void openHelpWindow()
    })
    // Help window "Ask AI": switch to the chat view and stage the manual
    // excerpt as a message-quote chip (ChatScreen drains the queue on mount).
    const unlistenAskAi = listenAskAi(({ text }) => {
      if (keepConfigRecoveryView()) return
      setView("chat")
      deliverAskAi(text)
    })
    const unlistenLanguage = listenLanguageConfigChange()
    const unlistenTheme = listenThemeConfigChange()
    const unlistenNotification = listenNotificationConfigChange()
    return () => {
      unlistenSettings()
      unlistenNewSession()
      unlistenUpdateCheck()
      unlistenOpenHelp()
      unlistenAskAi()
      unlistenLanguage()
      unlistenTheme()
      unlistenNotification()
    }
  }, [handleOpenSettings, keepConfigRecoveryView])

  // Track window focus state for background-aware OS notifications.
  // App-level singleton — listeners stay registered for the process
  // lifetime; `initFocusTracking` is idempotent across StrictMode
  // double-invokes.
  useEffect(() => {
    initFocusTracking().catch(() => {})
  }, [])

  // Subscribe to "user action required" events and pop OS notifications
  // when the app is in the background.
  useDesktopAlerts()

  useEffect(() => {
    if (
      view === "loading" ||
      view === "configRecovery" ||
      view === "onboarding" ||
      view === "setup"
    )
      return

    const handleSnapshot = (raw: unknown) => {
      const job = parsePayload<LocalModelJobSnapshot>(raw)
      if (!job) return
      // Reembed / reindex jobs aren't installs — their progress + completion is
      // shown in the memory / knowledge panels. Skip the install-flavored global
      // toast ("{model} 已安装" / "安装失败" 等), which only fits model installs.
      if (job.kind === "memory_reembed" || job.kind === "knowledge_reembed") return
      if (completedLocalModelJobToasts.current.has(job.jobId)) return
      completedLocalModelJobToasts.current.add(job.jobId)
      if (job.status === "completed") {
        toast.success(t("localModelJobs.toast.completed", { model: job.displayName }))
      } else if (job.status === "paused") {
        toast.info(t("localModelJobs.toast.paused", { model: job.displayName }))
      } else if (job.status === "cancelled") {
        toast.info(t("localModelJobs.toast.cancelled", { model: job.displayName }))
      } else {
        const description = job.error?.trim() || undefined
        const isOllamaInstall = job.kind === "ollama_install"
        toast.error(t("localModelJobs.toast.failed", { model: job.displayName }), {
          description,
          duration: isOllamaInstall ? 15000 : undefined,
          action: isOllamaInstall
            ? {
                label: t("localModelJobs.toast.openDownload"),
                onClick: () => openExternalUrl("https://ollama.com/download"),
              }
            : undefined,
        })
      }
    }

    const unlistenCompleted = getTransport().listen(
      LOCAL_MODEL_JOB_EVENTS.completed,
      handleSnapshot,
    )
    return () => {
      unlistenCompleted()
    }
  }, [t, view])

  useEffect(() => {
    initDraftSkillsStore()
    initCronUnreadStore()
  }, [])

  useEffect(() => {
    if (
      view === "loading" ||
      view === "configRecovery" ||
      view === "onboarding" ||
      view === "setup"
    )
      return

    const handler = (raw: unknown) => {
      const report = parsePayload<{
        outcome?: string
        skill_id?: string | null
      }>(raw)
      if (!report) return
      if (report.outcome !== "created") return
      const name = report.skill_id || t("skills.toast.unnamedSkill")
      toast.info(t("skills.toast.draftCreated", { name }), {
        action: {
          label: t("skills.toast.review"),
          onClick: () => handleOpenSettings("skills"),
        },
      })
    }
    const unlisten = getTransport().listen(SKILLS_EVENTS.autoReviewComplete, handler)
    return () => {
      unlisten()
    }
  }, [t, view, handleOpenSettings])

  // Surface a hook's `statusMessage` as a toast while the handler runs.
  useEffect(() => {
    const handler = (raw: unknown) => {
      const payload = parsePayload<{ message?: string }>(raw)
      if (!payload) return
      if (payload.message) toast.info(payload.message)
    }
    const unlisten = getTransport().listen("hook:status", handler)
    return () => {
      unlisten()
    }
  }, [])

  // Auto-check for desktop updates on startup
  const updateCheckRef = useRef(false)
  useEffect(() => {
    if (updateCheckRef.current) return
    if (
      view === "loading" ||
      view === "configRecovery" ||
      view === "onboarding" ||
      view === "setup"
    )
      return
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

  const bootstrapApp = useCallback(async () => {
    setView("loading")
    try {
      const health = await getTransport().call<ConfigHealth | null | undefined>("get_config_health")
      if (health?.ok === false) {
        setConfigHealth(health)
        setView("configRecovery")
        return
      }
      setConfigHealth(null)

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
      let onboarding: { completedVersion?: number } | null | undefined
      try {
        onboarding = await getTransport().call<{ completedVersion?: number } | null | undefined>(
          "get_onboarding_state",
        )
      } catch (e) {
        const refreshed = await getTransport()
          .call<ConfigHealth>("get_config_health")
          .catch(() => null)
        if (refreshed && !refreshed.ok) {
          setConfigHealth(refreshed)
          setView("configRecovery")
          return
        }
        throw e
      }
      if ((onboarding?.completedVersion ?? 0) < CURRENT_ONBOARDING_VERSION) {
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
  }, [fetchUserAvatar])

  // Try to restore previous session on mount
  useEffect(() => {
    void bootstrapApp()
  }, [bootstrapApp])

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
    throw new Error(t("common.loginTimeout"))
  }

  async function handleCodexAuth() {
    await runCodexAuth()
    setView("chat")
  }

  // `AuthRequiredDialog` is mounted in every view branch — the first
  // protected API call from the boot effect commonly 401s while the
  // splash / onboarding / setup screens are visible, so the listener
  // has to be live before then. (The sticky flag in api-key-storage
  // backs this up if React commits the dialog after the 401 fires.)
  if (view === "loading") {
    return (
      <TooltipProvider>
        <div className="flex items-center justify-center h-screen">
          <StarrySky />
          <AuthRequiredDialog />
          <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
        </div>
      </TooltipProvider>
    )
  }

  if (view === "configRecovery") {
    return (
      <TooltipProvider>
        <div className="min-h-screen overflow-y-auto bg-surface-app">
          <StarrySky />
          <Toaster />
          <AuthRequiredDialog />
          <ConfigRecoveryScreen health={configHealth} onRecovered={bootstrapApp} />
        </div>
      </TooltipProvider>
    )
  }

  if (view === "onboarding") {
    return (
      <TooltipProvider>
        <div className="flex flex-col h-screen overflow-hidden">
          <StarrySky />
          <Toaster />
          <DangerousModeBanner />
          <AuthRequiredDialog />
          <div className="flex-1 min-h-0 overflow-y-auto overscroll-contain">
            <OnboardingWizard
              onComplete={() => setView("chat")}
              onJumpToChannelsSettings={() => setView("channels")}
              onCodexAuth={runCodexAuth}
              initialLanguage={i18n.language || ""}
            />
          </div>
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
          <AuthRequiredDialog />
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
          <div className="flex flex-col h-screen overflow-hidden bg-surface-app">
            <StarrySky />
            <Toaster />
            <DangerousModeBanner />
            <MissingModelDialog />
            <ChromiumRuntimeDialog onOpenBrowserSettings={() => handleOpenSettings("browser")} />
            <AuthRequiredDialog />
            <div className="flex flex-1 min-h-0 overflow-hidden">
              <IconSidebar
                view={view}
                onOpenSettings={handleOpenSettings}
                onOpenChat={handleOpenChat}
                onOpenAgents={() => {
                  setAgentIdForSettings(undefined)
                  setAgentTabForSettings(undefined)
                  setView("agents")
                }}
                onOpenModelConfig={() => setView("modelConfig")}
                onOpenSkills={() => setView("skills")}
                onOpenMemory={() => setView("memory")}
                onOpenChannels={() => setView("channels")}
                onOpenProfile={() => {
                  setView("profile")
                }}
                onOpenCalendar={() => setView("calendar")}
                onOpenDashboard={() => handleOpenDashboard()}
                onOpenPlans={() => setView("plans")}
                onOpenKnowledge={handleOpenKnowledge}
                onOpenDesign={() => setView("design")}
                onOpenArtifacts={() => setView("artifacts")}
                onOpenUpdatePanel={handleOpenUpdatePanel}
                userAvatar={userAvatar}
                totalUnreadCount={totalUnreadCount}
                onMarkAllRead={() => setSessionsRefreshTrigger((n) => n + 1)}
              />
              {/* SettingsView 现在懒加载；7 个互斥分支共用一个 Suspense 边界。 */}
              <Suspense
                fallback={
                  <div className="flex-1 flex items-center justify-center">
                    <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                  </div>
                }
              >
              {view === "settings" && (
                <SettingsView
                  key={settingsInitialSectionRequestKey}
                  onBack={() => setView(settingsReturnView)}
                  onCodexAuth={handleCodexAuth}
                  onCodexReauth={handleCodexAuth}
                  initialSection={settingsInitialSection}
                  initialModelConfigTab={settingsInitialModelTab}
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
              {view === "memory" && (
                <SettingsView
                  onBack={() => setView("chat")}
                  onCodexAuth={handleCodexAuth}
                  onCodexReauth={handleCodexAuth}
                  initialSection="memory"
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
                    setAgentTabForSettings(undefined)
                  }}
                  onCodexAuth={handleCodexAuth}
                  onCodexReauth={handleCodexAuth}
                  initialSection="agents"
                  initialAgentId={agentIdForSettings}
                  initialAgentTab={agentTabForSettings}
                />
              )}
              {view === "modelConfig" && (
                <SettingsView
                  onBack={() => setView("chat")}
                  onCodexAuth={handleCodexAuth}
                  onCodexReauth={handleCodexAuth}
                  initialSection="modelConfig"
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
              </Suspense>
              {view === "calendar" && (
                <Suspense
                  fallback={
                    <div className="flex-1 flex items-center justify-center">
                      <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                    </div>
                  }
                >
                  <CronCalendarView
                    defaultProjectId={currentChatProjectId}
                    onBack={() => setView("chat")}
                    onOpenSettings={handleOpenSettings}
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
                  <DashboardView
                    onBack={() => setView("chat")}
                    onOpenSettings={handleOpenSettings}
                    initialTab={dashboardInitialTab}
                    initialRecapReportId={dashboardInitialReportId}
                    onOpenPlanHistory={() => setView("plans")}
                    onOpenControlItem={(item) => {
                      handleChatFocus({
                        sessionId: item.sessionId,
                        controlTarget: {
                          kind: item.kind,
                          itemId: item.id,
                        },
                      })
                    }}
                  />
                </Suspense>
              )}
              {view === "plans" && (
                <Suspense
                  fallback={
                    <div className="flex-1 flex items-center justify-center">
                      <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                    </div>
                  }
                >
                  <PlansView
                    onBack={() => setView("chat")}
                    onJumpToSession={(sessionId) => {
                      setPendingSessionId(sessionId)
                      setView("chat")
                    }}
                    onInsertMention={(token) => {
                      setPendingChatInsert({ token })
                      setView("chat")
                    }}
                  />
                </Suspense>
              )}
              {view === "knowledge" && (
                <Suspense
                  fallback={
                    <div className="flex-1 flex items-center justify-center">
                      <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                    </div>
                  }
                >
                  <KnowledgeView
                    onBack={() => setView("chat")}
                    onOpenSettings={() => handleOpenSettings("knowledge")}
                  />
                </Suspense>
              )}
              {view === "design" && (
                <Suspense
                  fallback={
                    <div className="flex-1 flex items-center justify-center">
                      <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                    </div>
                  }
                >
                  <DesignView
                    onBack={() => setView("chat")}
                    onOpenSettings={() => handleOpenSettings("design")}
                    onImplementToCode={(sessionId, message) => {
                      // 不设 pendingSessionId：auto-send 的 sessionIdOverride 已原子切会话，
                      // 避免与导航半边竞争加载空历史（review F2）。
                      setPendingAutoSend({ sessionId, message, nonce: Date.now() })
                      setView("chat")
                    }}
                  />
                </Suspense>
              )}
              {view === "artifacts" && (
                <Suspense
                  fallback={
                    <div className="flex-1 flex items-center justify-center">
                      <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                    </div>
                  }
                >
                  <ArtifactsView onBack={() => setView("chat")} />
                </Suspense>
              )}
              <div className={view === "chat" ? "flex-1 flex overflow-hidden" : "hidden"}>
                <ChatScreen
                  isViewVisible={view === "chat"}
                  onOpenAgentSettings={(agentId) => {
                    setAgentIdForSettings(agentId)
                    setAgentTabForSettings(undefined)
                    setView("agents")
                  }}
                  onCodexReauth={handleCodexAuth}
                  initialSessionId={pendingSessionId}
                  onSessionNavigated={() => setPendingSessionId(undefined)}
                  onUnreadCountChange={setTotalUnreadCount}
                  unreadFocusSignal={unreadFocusSignal}
                  onOpenDashboardTab={handleOpenDashboard}
                  sessionsRefreshTrigger={sessionsRefreshTrigger}
                  onCurrentProjectChange={setCurrentChatProjectId}
                  externalChatFocus={pendingChatFocus}
                  onExternalChatFocusHandled={(nonce) => {
                    setPendingChatFocus((prev) => (prev?.nonce === nonce ? null : prev))
                  }}
                  externalProjectFocus={pendingProjectFocus}
                  onExternalProjectFocusHandled={(nonce) => {
                    setPendingProjectFocus((prev) => (prev?.nonce === nonce ? null : prev))
                  }}
                  pendingChatInsert={pendingChatInsert}
                  onChatInsertConsumed={() => setPendingChatInsert(undefined)}
                  pendingAutoSend={pendingAutoSend}
                  onAutoSendConsumed={(nonce) =>
                    setPendingAutoSend((prev) => (prev?.nonce === nonce ? undefined : prev))
                  }
                  onOpenSettings={handleOpenSettings}
                  onOpenKnowledge={handleOpenKnowledge}
                />
              </div>

              {/* In-app update panel */}
              {globalPendingUpdate && shouldShowUpdatePanel && (
                <div className="fixed top-6 right-6 z-50 animate-in slide-in-from-top-5 fade-in duration-300">
                  <div className="relative flex flex-col gap-3 rounded-2xl border border-emerald-500/20 bg-card p-4 shadow-xl dark:bg-zinc-900/90 w-[380px]">
                    {/* Close / Ignore button */}
                    {!showIgnoreOptions && !installingUpdate && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation()
                          if (awaitingRestart) {
                            // "稍后重启": dismiss; the staged binary applies on
                            // the next launch.
                            setDismissedVersion(globalPendingUpdate.version)
                            setForceShowUpdatePanel(false)
                          } else {
                            setShowIgnoreOptions(true)
                          }
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
                          {t("about.updateToast.notRemindVersion", {
                            version: globalPendingUpdate.version,
                          })}
                        </p>
                        <div className="flex gap-2 justify-center">
                          <button
                            className="flex-1 text-xs font-medium text-muted-foreground bg-secondary hover:bg-secondary/80 px-3 py-2 rounded-lg transition-colors"
                            onClick={() => {
                              setDismissedVersion(globalPendingUpdate.version)
                              setForceShowUpdatePanel(false)
                              setShowIgnoreOptions(false)
                            }}
                          >
                            {t("about.updateToast.ignoreOnce")}
                          </button>
                          <button
                            className="flex-1 text-xs font-medium text-destructive bg-destructive/10 hover:bg-destructive/20 px-3 py-2 rounded-lg transition-colors"
                            onClick={() => {
                              localStorage.setItem(
                                "ignored_update_version",
                                globalPendingUpdate.version,
                              )
                              setDismissedVersion(globalPendingUpdate.version)
                              setForceShowUpdatePanel(false)
                              setShowIgnoreOptions(false)
                            }}
                          >
                            {t("about.updateToast.neverRemindVersion")}
                          </button>
                        </div>
                      </div>
                    ) : installingUpdate ? (
                      <div className="flex flex-col gap-2 mt-1">
                        <div className="flex items-center justify-between pr-6">
                          <p className="text-sm font-medium text-foreground">
                            {t("about.updateToast.updating")}
                          </p>
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
                    ) : awaitingRestart ? (
                      <div className="flex items-start gap-4">
                        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-emerald-500/10 text-emerald-500 mt-1">
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
                            <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
                            <path d="M3 3v5h5" />
                          </svg>
                        </div>
                        <div className="flex-1 min-w-0 pr-5">
                          <p className="text-sm font-semibold text-foreground truncate">
                            {t("about.updateToast.versionReady", {
                              version: globalPendingUpdate.version,
                            })}
                          </p>
                          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                            {t("about.updateToast.restartDescription")}
                          </p>
                          <div className="mt-4 flex justify-end">
                            <button
                              onClick={(e) => {
                                e.stopPropagation()
                                void restartNow()
                              }}
                              className="px-4 py-2 text-xs font-semibold bg-emerald-500 text-white hover:bg-emerald-600 rounded-lg transition-colors duration-200"
                            >
                              {t("about.restartNow")}
                            </button>
                          </div>
                        </div>
                      </div>
                    ) : (
                      <div
                        className="flex items-start gap-4 cursor-pointer group"
                        onClick={() => {
                          setDismissedVersion(globalPendingUpdate.version)
                          setForceShowUpdatePanel(false)
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
                            {t("about.updateToast.newVersionTitle", {
                              version: globalPendingUpdate.version,
                            })}
                          </p>
                          <div className="update-notes-markdown mt-2.5 max-h-[180px] overflow-y-auto pr-2 text-xs leading-relaxed text-muted-foreground scrollbar-thin scrollbar-thumb-muted-foreground/20 hover:scrollbar-thumb-muted-foreground/40 scrollbar-track-transparent">
                            {globalPendingUpdate.body ? (
                              <MarkdownRenderer content={globalPendingUpdate.body} />
                            ) : (
                              <p>
                                {t("about.updateAvailable", {
                                  version: globalPendingUpdate.version,
                                })}
                              </p>
                            )}
                          </div>
                          {downloadStatus === "downloaded" && (
                            <p className="mt-2 text-[11px] font-medium text-emerald-600 dark:text-emerald-400">
                              {t("about.updateToast.downloadedReady")}
                            </p>
                          )}
                          <div className="mt-4 flex justify-end gap-2">
                            <button
                              onClick={(e) => {
                                e.stopPropagation()
                                void runInstall(false)
                              }}
                              className="px-3 py-2 text-xs font-medium text-muted-foreground bg-secondary hover:bg-secondary/80 rounded-lg transition-colors duration-200"
                            >
                              {t("about.updateOnly")}
                            </button>
                            <button
                              onClick={(e) => {
                                e.stopPropagation()
                                void runInstall(true)
                              }}
                              className="px-4 py-2 text-xs font-semibold bg-emerald-500/10 text-emerald-600 hover:bg-emerald-500 hover:text-white rounded-lg transition-colors duration-200 dark:text-emerald-400 dark:hover:text-white"
                            >
                              {t("about.updateAndRestart")}
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

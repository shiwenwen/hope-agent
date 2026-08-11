import { useState, useEffect, useCallback, useRef, lazy, Suspense, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"
import { listen } from "@tauri-apps/api/event"
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
import { PortalScopeProvider } from "@/components/ui/portal-scope"
import { LightboxProvider } from "@/components/common/ImageLightbox"
import ErrorBoundary from "@/components/common/ErrorBoundary"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import ServerUpdateNotice from "@/components/common/ServerUpdateNotice"
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
import type { KnowledgeFocusTarget } from "@/components/knowledge/knowledgeFocus"
import {
  clearMemoryFocusUrl,
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
import { LOCAL_MODEL_JOB_EVENTS, type LocalModelJobSnapshot } from "@/types/local-model-jobs"
import type { PetNavigationTarget } from "@/types/pet"
import {
  SPACE_WINDOW_IMPLEMENT_EVENT,
  SPACE_WINDOW_OPEN_SETTINGS_EVENT,
  SPACE_WINDOW_REATTACH_EVENT,
  focusDetachedSpaceWindow,
  navigateDetachedSpaceWindow,
  openDetachedSpaceWindow,
  type DesignSpaceLocation,
  type DetachableSpace,
  type KnowledgeSpaceLocation,
  type SpaceNavigationRequest,
  type SpaceKnowledgeFocusRequest,
  type SpaceWindowAction,
  type SpaceWindowActionRequest,
  type SpaceWindowImplementRequest,
  type SpaceWindowLocation,
  type SpaceWindowSettingsRequest,
} from "@/lib/spaceWindow"

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

const PERSISTENT_APP_VIEWS: ReadonlySet<AppView> = new Set([
  "chat",
  "calendar",
  "dashboard",
  "plans",
  "knowledge",
  "design",
  "artifacts",
])

const SETTINGS_APP_VIEWS: ReadonlySet<AppView> = new Set([
  "settings",
  "skills",
  "profile",
  "agents",
  "modelConfig",
  "memory",
  "channels",
])

function PersistentViewSurface({ active, children }: { active: boolean; children: ReactNode }) {
  const [portalContainer, setPortalContainer] = useState<HTMLDivElement | null>(null)
  return (
    <div
      ref={setPortalContainer}
      aria-hidden={!active}
      inert={active ? undefined : true}
      className={active ? "flex min-h-0 min-w-0 flex-1 overflow-hidden" : "hidden"}
    >
      <PortalScopeProvider active={active} container={portalContainer}>
        {children}
      </PortalScopeProvider>
    </div>
  )
}

interface PendingChatFocus extends ChatFocusTarget {
  nonce: number
}

interface PendingProjectFocus {
  projectId: string
  nonce: number
}

type PendingKnowledgePetFocus = {
  target: Extract<PetNavigationTarget, { kind: "knowledge" }>
  nonce: number
}

type PendingDesignPetFocus = {
  target: Extract<PetNavigationTarget, { kind: "design" }>
  nonce: number
}

type PetInstallLinkEvent = {
  link: string
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
  const [pendingPetInstallLink, setPendingPetInstallLink] = useState<string | null>(null)
  // 记住进设置前所在的视图，返回时回到那里（而非硬编码回 chat）。
  const [settingsReturnView, setSettingsReturnView] = useState<AppView>("chat")
  const viewRef = useRef<AppView>(view)
  viewRef.current = view
  const [dashboardInitialTab, setDashboardInitialTab] = useState<string | undefined>(undefined)
  const [dashboardInitialReportId, setDashboardInitialReportId] = useState<string | null>(null)
  const [dashboardNavigationRequestKey, setDashboardNavigationRequestKey] = useState(0)
  const [userAvatar, setUserAvatar] = useState<string | null>(null)
  const [pendingSessionId, setPendingSessionId] = useState<string | undefined>(undefined)
  const [currentChatProjectId, setCurrentChatProjectId] = useState<string | null>(null)
  const [configHealth, setConfigHealth] = useState<ConfigHealth | null>(null)
  // Cross-space "reference in chat" actions land here with any typed
  // provenance/KB attachment metadata. ChatScreen appends once and clears.
  const [pendingChatInsert, setPendingChatInsert] = useState<ChatInsert | undefined>(undefined)
  // 设计空间「实现到代码」：跳到实现会话后把 handoff pack 作首条消息自动发送（一次性，nonce 防重放）。
  const [pendingAutoSend, setPendingAutoSend] = useState<
    { sessionId: string; message: string; nonce: number } | undefined
  >(undefined)
  const [pendingChatFocus, setPendingChatFocus] = useState<PendingChatFocus | null>(null)
  const [pendingProjectFocus, setPendingProjectFocus] = useState<PendingProjectFocus | null>(null)
  const [pendingKnowledgePetFocus, setPendingKnowledgePetFocus] =
    useState<PendingKnowledgePetFocus | null>(null)
  const [pendingDesignPetFocus, setPendingDesignPetFocus] = useState<PendingDesignPetFocus | null>(
    null,
  )
  const [detachedSpaces, setDetachedSpaces] = useState<Record<DetachableSpace, boolean>>({
    knowledge: false,
    design: false,
  })
  const detachedSpacesRef = useRef(detachedSpaces)
  const detachedSpaceGenerationRef = useRef<Record<DetachableSpace, number>>({
    knowledge: 0,
    design: 0,
  })
  const knowledgeLocationRef = useRef<KnowledgeSpaceLocation>({ kbId: null, path: null })
  const designLocationRef = useRef<DesignSpaceLocation>({ projectId: null, artifactId: null })
  const spaceNavigationNonceRef = useRef(0)
  const knowledgeWindowActionNonceRef = useRef(0)
  const [knowledgeWindowNavigation, setKnowledgeWindowNavigation] =
    useState<SpaceNavigationRequest<KnowledgeSpaceLocation> | null>(null)
  const [pendingKnowledgeFocus, setPendingKnowledgeFocus] =
    useState<SpaceKnowledgeFocusRequest | null>(null)
  const [knowledgeWindowActionRequest, setKnowledgeWindowActionRequest] =
    useState<SpaceWindowActionRequest | null>(null)
  const [designWindowNavigation, setDesignWindowNavigation] =
    useState<SpaceNavigationRequest<DesignSpaceLocation> | null>(null)
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
  const petFocusNonceRef = useRef(0)
  const knowledgeFocusNonceRef = useRef(0)
  const lastMemoryFocusHashRef = useRef<string | null>(null)
  const previousViewRef = useRef<AppView>(view)
  // 侧边栏工作区首次访问时才挂载；之后只隐藏顶层容器，不销毁组件树与 Effects。
  // 需要区分可见性的行为（快捷键、轮询、已读回执）由各工作区的 isViewVisible 明确门控。
  const [mountedViews, setMountedViews] = useState<Set<AppView>>(() => new Set(["chat"]))

  const setSpaceDetached = useCallback((space: DetachableSpace, detached: boolean) => {
    detachedSpacesRef.current = { ...detachedSpacesRef.current, [space]: detached }
    setDetachedSpaces(detachedSpacesRef.current)
  }, [])

  useEffect(() => {
    if (!PERSISTENT_APP_VIEWS.has(view)) return
    setMountedViews((current) => {
      if (current.has(view)) return current
      const next = new Set(current)
      next.add(view)
      return next
    })
  }, [view])

  const shouldMountView = useCallback(
    (candidate: AppView) => view === candidate || mountedViews.has(candidate),
    [mountedViews, view],
  )

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
      const detail = (event as CustomEvent<{ section?: SettingsSection; modelTab?: string }>).detail
      handleOpenSettings(detail?.section, detail?.modelTab)
    }
    window.addEventListener("settings:navigate", handleNavigate)
    return () => window.removeEventListener("settings:navigate", handleNavigate)
  }, [handleOpenSettings])

  // Memory panels keep their current subview in the hash with replaceState.
  // Consume that internal URL state before the view-change deep-link effect below,
  // otherwise an intentional top-level navigation is interpreted as a fresh deep link.
  useEffect(() => {
    const previousView = previousViewRef.current
    previousViewRef.current = view
    if (previousView === view || !SETTINGS_APP_VIEWS.has(previousView)) return
    if (clearMemoryFocusUrl()) lastMemoryFocusHashRef.current = null
  }, [view])

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
      if (tab !== undefined || reportId !== undefined) {
        setDashboardNavigationRequestKey((n) => n + 1)
      }
      setView("dashboard")
    },
    [keepConfigRecoveryView],
  )

  const handleOpenDetachedSpace = useCallback(
    async (target: SpaceWindowLocation) => {
      if (keepConfigRecoveryView()) return
      if (detachedSpacesRef.current[target.space]) {
        const focused = await focusDetachedSpaceWindow(target.space)
        if (focused) return
        detachedSpaceGenerationRef.current[target.space] += 1
        setSpaceDetached(target.space, false)
      }
      const title =
        target.space === "knowledge"
          ? t("knowledge.title", "Knowledge Space")
          : t("design.title", "Design Space")
      const webview = await openDetachedSpaceWindow(target, title)
      if (!webview) return
      const generation = ++detachedSpaceGenerationRef.current[target.space]
      setSpaceDetached(target.space, true)
      void webview.once("tauri://destroyed", () => {
        if (detachedSpaceGenerationRef.current[target.space] === generation) {
          setSpaceDetached(target.space, false)
        }
      })
      if (viewRef.current === target.space) setView("chat")
    },
    [keepConfigRecoveryView, setSpaceDetached, t],
  )

  const handleOpenKnowledgeWindow = useCallback(() => {
    if (keepConfigRecoveryView()) return

    const requestDetach = () => {
      const nonce = ++knowledgeWindowActionNonceRef.current
      setKnowledgeWindowActionRequest({ nonce, action: "detach" })
      setView("knowledge")
    }

    if (detachedSpacesRef.current.knowledge) {
      void focusDetachedSpaceWindow("knowledge").then((focused) => {
        if (focused) return
        detachedSpaceGenerationRef.current.knowledge += 1
        setSpaceDetached("knowledge", false)
        requestDetach()
      })
      return
    }

    requestDetach()
  }, [keepConfigRecoveryView, setSpaceDetached])

  const handleKnowledgeWindowActionReady = useCallback(
    (action: SpaceWindowAction, location: KnowledgeSpaceLocation) => {
      if (action === "detach") {
        void handleOpenDetachedSpace({ space: "knowledge", location })
      }
    },
    [handleOpenDetachedSpace],
  )

  const handleOpenKnowledge = useCallback(
    (target?: KnowledgeFocusTarget) => {
      if (keepConfigRecoveryView()) return
      const focusRequest = target ? { nonce: ++knowledgeFocusNonceRef.current, target } : null
      if (detachedSpacesRef.current.knowledge) {
        const openDetached = target
          ? navigateDetachedSpaceWindow({ space: "knowledge", knowledgeFocus: target })
          : focusDetachedSpaceWindow("knowledge")
        void openDetached.then((focused) => {
          if (!focused) {
            detachedSpaceGenerationRef.current.knowledge += 1
            setSpaceDetached("knowledge", false)
            if (focusRequest) setPendingKnowledgeFocus(focusRequest)
            setView("knowledge")
          }
        })
        return
      }
      if (focusRequest) setPendingKnowledgeFocus(focusRequest)
      setView("knowledge")
    },
    [keepConfigRecoveryView, setSpaceDetached],
  )

  const handleOpenDesign = useCallback(() => {
    if (keepConfigRecoveryView()) return
    if (detachedSpacesRef.current.design) {
      void focusDetachedSpaceWindow("design").then((focused) => {
        if (!focused) {
          detachedSpaceGenerationRef.current.design += 1
          setSpaceDetached("design", false)
          setView("design")
        }
      })
      return
    }
    setView("design")
  }, [keepConfigRecoveryView, setSpaceDetached])

  const handleDesignImplementToCode = useCallback((sessionId: string, message: string) => {
    // 不设 pendingSessionId：auto-send 的 sessionIdOverride 已原子切会话，
    // 避免与导航半边竞争加载空历史（review F2）。
    setPendingAutoSend({ sessionId, message, nonce: Date.now() })
    setView("chat")
  }, [])

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

  useEffect(() => {
    if (!isTauriMode()) return
    let cancelled = false
    let unlisteners: Array<() => void> = []

    void Promise.all([
      listen<SpaceWindowLocation>(SPACE_WINDOW_REATTACH_EVENT, (event) => {
        const payload = event.payload
        if (!payload || (payload.space !== "knowledge" && payload.space !== "design")) return
        detachedSpaceGenerationRef.current[payload.space] += 1
        setSpaceDetached(payload.space, false)
        const nonce = ++spaceNavigationNonceRef.current
        if (payload.space === "knowledge") {
          knowledgeLocationRef.current = payload.location
          setKnowledgeWindowNavigation({ nonce, location: payload.location })
          if (!keepConfigRecoveryView()) setView("knowledge")
        } else {
          designLocationRef.current = payload.location
          setDesignWindowNavigation({ nonce, location: payload.location })
          if (!keepConfigRecoveryView()) setView("design")
        }
      }),
      listen<SpaceWindowSettingsRequest>(SPACE_WINDOW_OPEN_SETTINGS_EVENT, (event) => {
        if (event.payload?.section) handleOpenSettings(event.payload.section)
      }),
      listen<SpaceWindowImplementRequest>(SPACE_WINDOW_IMPLEMENT_EVENT, (event) => {
        const payload = event.payload
        if (!payload?.sessionId || !payload.message) return
        handleDesignImplementToCode(payload.sessionId, payload.message)
      }),
    ]).then((stops) => {
      if (cancelled) stops.forEach((stop) => stop())
      else unlisteners = stops
    })

    return () => {
      cancelled = true
      unlisteners.forEach((stop) => stop())
      unlisteners = []
    }
  }, [handleDesignImplementToCode, handleOpenSettings, keepConfigRecoveryView, setSpaceDetached])

  useEffect(() => {
    const unlisten = getTransport().listen("pet:navigate", (raw) => {
      const target = parsePayload<PetNavigationTarget>(raw)
      if (!target || keepConfigRecoveryView()) return
      if (target.kind === "regular") {
        handleChatFocus({ sessionId: target.sessionId })
        return
      }
      const nonce = ++petFocusNonceRef.current
      if (target.kind === "knowledge") {
        if (detachedSpacesRef.current.knowledge) {
          void navigateDetachedSpaceWindow({
            space: "knowledge",
            petFocus: { target, nonce },
          }).then((navigated) => {
            if (navigated) return
            detachedSpaceGenerationRef.current.knowledge += 1
            setSpaceDetached("knowledge", false)
            setPendingKnowledgePetFocus({ target, nonce })
            setView("knowledge")
          })
          return
        }
        setPendingKnowledgePetFocus({ target, nonce })
        setView("knowledge")
        return
      }
      if (detachedSpacesRef.current.design) {
        void navigateDetachedSpaceWindow({ space: "design", petFocus: { target, nonce } }).then(
          (navigated) => {
            if (navigated) return
            detachedSpaceGenerationRef.current.design += 1
            setSpaceDetached("design", false)
            setPendingDesignPetFocus({ target, nonce })
            setView("design")
          },
        )
        return
      }
      setPendingDesignPetFocus({ target, nonce })
      setView("design")
    })
    return unlisten
  }, [handleChatFocus, keepConfigRecoveryView, setSpaceDetached])

  useEffect(() => {
    if (!isTauriMode()) return
    const openInstall = (link: string) => {
      if (!link.startsWith("hope-agent://pets/install?")) return
      setPendingPetInstallLink(link)
      handleOpenSettings("pets")
    }
    const unlisten = getTransport().listen("pet:install_link", (raw) => {
      const payload = parsePayload<PetInstallLinkEvent>(raw)
      if (!payload || typeof payload.link !== "string") return
      openInstall(payload.link)
      void getTransport()
        .call("pet_take_install_link_cmd")
        .catch(() => undefined)
    })
    void getTransport()
      .call<string | null>("pet_take_install_link_cmd")
      .then((link) => {
        if (link) openInstall(link)
      })
      .catch(() => undefined)
    return unlisten
  }, [handleOpenSettings])

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

  // `ha-settings` can wake/tuck the pet without going through the Settings
  // panel.  Core emits the config invalidation; the desktop shell owns the
  // native window lifecycle, so the always-mounted main renderer bridges it.
  useEffect(() => {
    if (!isTauriMode()) return
    let syncTimer: ReturnType<typeof setTimeout> | null = null
    const unlisten = getTransport().listen("pet:config_changed", (payload) => {
      if (syncTimer) clearTimeout(syncTimer)
      const source =
        typeof payload === "object" && payload !== null && "source" in payload
          ? String((payload as { source?: unknown }).source ?? "")
          : ""
      // Give PetWindow's own tuck command enough time to deliver its invoke
      // response before destroying that webview. Other config paths sync on
      // the next task and repeated events are coalesced.
      syncTimer = setTimeout(
        () => {
          syncTimer = null
          void getTransport()
            .call("pet_sync_window_cmd")
            .catch((error) => {
              logger.warn("pet", "App::syncPetWindow", "Failed to sync pet window", error)
            })
        },
        source === "pet-window" ? 120 : 0,
      )
    })
    return () => {
      if (syncTimer) clearTimeout(syncTimer)
      unlisten()
    }
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

  if (view === "loading") {
    return (
      <TooltipProvider>
        <div className="flex items-center justify-center h-screen">
          <StarrySky />
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
                onOpenKnowledgeWindow={handleOpenKnowledgeWindow}
                onOpenDesign={handleOpenDesign}
                onOpenDesignWindow={() =>
                  void handleOpenDetachedSpace({
                    space: "design",
                    location: designLocationRef.current,
                  })
                }
                knowledgeDetached={detachedSpaces.knowledge}
                designDetached={detachedSpaces.design}
                onOpenArtifacts={() => setView("artifacts")}
                onOpenUpdatePanel={handleOpenUpdatePanel}
                userAvatar={userAvatar}
                totalUnreadCount={totalUnreadCount}
                onMarkAllRead={() => setSessionsRefreshTrigger((n) => n + 1)}
              />
              {/* 侧边栏工作区按需首次挂载，之后仅切换顶层可见性，保留完整运行状态。 */}
              <Suspense
                fallback={
                  <div className="flex-1 flex items-center justify-center">
                    <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                  </div>
                }
              >
                {shouldMountView("settings") && (
                  <PersistentViewSurface active={view === "settings"}>
                    <SettingsView
                      key={settingsInitialSectionRequestKey}
                      onBack={() => setView(settingsReturnView)}
                      onCodexAuth={handleCodexAuth}
                      onCodexReauth={handleCodexAuth}
                      initialSection={settingsInitialSection}
                      initialModelConfigTab={settingsInitialModelTab}
                      initialPetInstallLink={pendingPetInstallLink}
                      onPetInstallLinkConsumed={() => setPendingPetInstallLink(null)}
                    />
                  </PersistentViewSurface>
                )}
                {shouldMountView("skills") && (
                  <PersistentViewSurface active={view === "skills"}>
                    <SettingsView
                      onBack={() => setView("chat")}
                      onCodexAuth={handleCodexAuth}
                      onCodexReauth={handleCodexAuth}
                      initialSection="skills"
                    />
                  </PersistentViewSurface>
                )}
                {shouldMountView("memory") && (
                  <PersistentViewSurface active={view === "memory"}>
                    <SettingsView
                      onBack={() => setView("chat")}
                      onCodexAuth={handleCodexAuth}
                      onCodexReauth={handleCodexAuth}
                      initialSection="memory"
                    />
                  </PersistentViewSurface>
                )}
                {shouldMountView("profile") && (
                  <PersistentViewSurface active={view === "profile"}>
                    <SettingsView
                      onBack={() => setView("chat")}
                      onCodexAuth={handleCodexAuth}
                      onCodexReauth={handleCodexAuth}
                      initialSection="profile"
                      onProfileSaved={() => fetchUserAvatar().then(setUserAvatar)}
                    />
                  </PersistentViewSurface>
                )}
                {shouldMountView("agents") && (
                  <PersistentViewSurface active={view === "agents"}>
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
                  </PersistentViewSurface>
                )}
                {shouldMountView("modelConfig") && (
                  <PersistentViewSurface active={view === "modelConfig"}>
                    <SettingsView
                      onBack={() => setView("chat")}
                      onCodexAuth={handleCodexAuth}
                      onCodexReauth={handleCodexAuth}
                      initialSection="modelConfig"
                    />
                  </PersistentViewSurface>
                )}
                {shouldMountView("channels") && (
                  <PersistentViewSurface active={view === "channels"}>
                    <SettingsView
                      onBack={() => setView("chat")}
                      onCodexAuth={handleCodexAuth}
                      onCodexReauth={handleCodexAuth}
                      initialSection="channels"
                    />
                  </PersistentViewSurface>
                )}
              </Suspense>
              {shouldMountView("calendar") && (
                <PersistentViewSurface active={view === "calendar"}>
                  <Suspense
                    fallback={
                      <div className="flex-1 flex items-center justify-center">
                        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                      </div>
                    }
                  >
                    <CronCalendarView
                      isViewVisible={view === "calendar"}
                      defaultProjectId={currentChatProjectId}
                      onOpenSettings={handleOpenSettings}
                    />
                  </Suspense>
                </PersistentViewSurface>
              )}
              {shouldMountView("dashboard") && (
                <PersistentViewSurface active={view === "dashboard"}>
                  <Suspense
                    fallback={
                      <div className="flex-1 flex items-center justify-center">
                        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                      </div>
                    }
                  >
                    <DashboardView
                      key={dashboardNavigationRequestKey}
                      isViewVisible={view === "dashboard"}
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
                </PersistentViewSurface>
              )}
              {shouldMountView("plans") && (
                <PersistentViewSurface active={view === "plans"}>
                  <Suspense
                    fallback={
                      <div className="flex-1 flex items-center justify-center">
                        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                      </div>
                    }
                  >
                    <PlansView
                      isViewVisible={view === "plans"}
                      onJumpToSession={(sessionId) => {
                        setPendingSessionId(sessionId)
                        setView("chat")
                      }}
                      onInsertMention={(token) => {
                        const targetId = token.slice("@plan:".length)
                        setPendingChatInsert({
                          token,
                          mention: {
                            kind: "plan",
                            targetId,
                            displayLabel: token,
                          },
                        })
                        setView("chat")
                      }}
                    />
                  </Suspense>
                </PersistentViewSurface>
              )}
              {shouldMountView("knowledge") && (
                <PersistentViewSurface active={view === "knowledge"}>
                  <Suspense
                    fallback={
                      <div className="flex-1 flex items-center justify-center">
                        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                      </div>
                    }
                  >
                    <KnowledgeView
                      isViewVisible={view === "knowledge"}
                      windowNavigation={knowledgeWindowNavigation}
                      knowledgeFocus={pendingKnowledgeFocus}
                      onKnowledgeFocusHandled={(nonce) =>
                        setPendingKnowledgeFocus((current) =>
                          current?.nonce === nonce ? null : current,
                        )
                      }
                      windowActionRequest={knowledgeWindowActionRequest}
                      onWindowLocationChange={(location) => {
                        knowledgeLocationRef.current = location
                      }}
                      onWindowActionReady={handleKnowledgeWindowActionReady}
                      onToggleWindowMode={
                        isTauriMode()
                          ? (location) =>
                              void handleOpenDetachedSpace({ space: "knowledge", location })
                          : undefined
                      }
                      onOpenSettings={() => handleOpenSettings("knowledge")}
                      petFocus={pendingKnowledgePetFocus}
                      onPetFocusHandled={(nonce) =>
                        setPendingKnowledgePetFocus((current) =>
                          current?.nonce === nonce ? null : current,
                        )
                      }
                    />
                  </Suspense>
                </PersistentViewSurface>
              )}
              {shouldMountView("design") && (
                <PersistentViewSurface active={view === "design"}>
                  <Suspense
                    fallback={
                      <div className="flex-1 flex items-center justify-center">
                        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                      </div>
                    }
                  >
                    <DesignView
                      isViewVisible={view === "design"}
                      windowNavigation={designWindowNavigation}
                      onWindowLocationChange={(location) => {
                        designLocationRef.current = location
                      }}
                      onToggleWindowMode={
                        isTauriMode()
                          ? (location) =>
                              void handleOpenDetachedSpace({ space: "design", location })
                          : undefined
                      }
                      onOpenSettings={() => handleOpenSettings("design")}
                      petFocus={pendingDesignPetFocus}
                      onPetFocusHandled={(nonce) =>
                        setPendingDesignPetFocus((current) =>
                          current?.nonce === nonce ? null : current,
                        )
                      }
                      onImplementToCode={handleDesignImplementToCode}
                    />
                  </Suspense>
                </PersistentViewSurface>
              )}
              {shouldMountView("artifacts") && (
                <PersistentViewSurface active={view === "artifacts"}>
                  <Suspense
                    fallback={
                      <div className="flex-1 flex items-center justify-center">
                        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
                      </div>
                    }
                  >
                    <ArtifactsView isViewVisible={view === "artifacts"} />
                  </Suspense>
                </PersistentViewSurface>
              )}
              <PersistentViewSurface active={view === "chat"}>
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
              </PersistentViewSurface>

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
                          {downloadPercent !== null && (
                            <p className="text-sm font-medium text-emerald-500">
                              {downloadPercent}%
                            </p>
                          )}
                        </div>
                        <div className="h-1.5 w-full bg-secondary overflow-hidden rounded-full mt-1">
                          <div
                            className={
                              downloadPercent === null
                                ? "h-full w-1/3 animate-pulse rounded-full bg-gradient-to-r from-transparent via-emerald-500 to-transparent"
                                : "h-full rounded-full bg-emerald-500 transition-all duration-300"
                            }
                            style={
                              downloadPercent === null
                                ? undefined
                                : { width: `${downloadPercent}%` }
                            }
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
              <ServerUpdateNotice />
            </div>
          </div>
        </LightboxProvider>
      </TooltipProvider>
    </ErrorBoundary>
  )
}

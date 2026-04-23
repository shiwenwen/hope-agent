import { useState, useRef, useEffect, useCallback, useMemo } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { notify } from "@/lib/notifications"
import { parseSessionMessages } from "../chatUtils"
import { useSessionPagination } from "./useSessionPagination"
import { useChannelStreaming } from "./useChannelStreaming"
import { PAGE_SIZE } from "./constants"
import type {
  Message,
  AvailableModel,
  ActiveModel,
  SessionMeta,
  SessionMessage,
  AgentSummaryForSidebar,
  SubagentEvent,
} from "@/types/chat"
import type { AgentConfig } from "@/components/settings/types"

export { PAGE_SIZE, SESSION_PAGE_SIZE } from "./constants"

export interface UseChatSessionReturn {
  // State
  messages: Message[]
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  currentSessionId: string | null
  setCurrentSessionId: React.Dispatch<React.SetStateAction<string | null>>
  currentSessionIdRef: React.MutableRefObject<string | null>
  sessions: SessionMeta[]
  agents: AgentSummaryForSidebar[]
  currentAgentId: string
  setCurrentAgentId: React.Dispatch<React.SetStateAction<string>>
  agentName: string
  setAgentName: React.Dispatch<React.SetStateAction<string>>
  loading: boolean
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionIds: Set<string>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  hasMore: boolean
  loadingMore: boolean
  hasMoreSessions: boolean
  loadingMoreSessions: boolean
  /**
   * When set (from a search result click), MessageList should scroll to the
   * message with this `id` and briefly highlight it. Must be reset to `null`
   * by the consumer after the scroll has been applied.
   */
  pendingScrollTarget: number | null
  clearPendingScrollTarget: () => void
  /**
   * Scroll the current session to a specific message and briefly highlight
   * it. If the target is not in the currently loaded window, reloads a
   * window of messages centred on the target first. Used by the in-chat
   * "find in page" search bar.
   */
  jumpToMessage: (messageId: number) => Promise<void>

  // Refs
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  hasMoreRef: React.MutableRefObject<Map<string, boolean>>
  oldestDbIdRef: React.MutableRefObject<Map<string, number>>

  // Handlers
  reloadSessions: () => Promise<void>
  reloadAgents: () => Promise<void>
  handleSwitchSession: (sessionId: string, opts?: { targetMessageId?: number }) => Promise<void>
  handleNewChat: (agentId: string) => Promise<void>
  /**
   * Create a new session inside a Project. Pre-materializes the session via
   * `create_session_cmd` so project context (memories, files, instructions)
   * is wired in immediately — the subsequent first message reuses the
   * existing sessionId instead of auto-creating an unassigned one.
   */
  handleNewChatInProject: (
    projectId: string,
    defaultAgentId?: string | null,
    incognito?: boolean,
  ) => Promise<void>
  handleDeleteSession: (sessionId: string) => Promise<void>
  handleLoadMore: () => Promise<void>
  handleLoadMoreSessions: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  updateSessionMeta: (sessionId: string, updater: (prev: SessionMeta) => SessionMeta) => void
}

interface UseChatSessionOptions {
  availableModels: AvailableModel[]
  setActiveModel: React.Dispatch<React.SetStateAction<ActiveModel | null>>
  globalActiveModelRef: React.MutableRefObject<ActiveModel | null>
  handleModelChange: (key: string) => void
  applyModelForDisplay: (key: string) => void
  initialSessionId?: string
  onSessionNavigated?: () => void
  onUnreadCountChange?: (count: number) => void
}

export function useChatSession({
  availableModels,
  setActiveModel,
  globalActiveModelRef,
  handleModelChange,
  applyModelForDisplay,
  initialSessionId,
  onSessionNavigated,
  onUnreadCountChange,
}: UseChatSessionOptions): UseChatSessionReturn {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null)
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [currentAgentId, setCurrentAgentId] = useState("default")
  const [agentName, setAgentName] = useState("")
  const [loading, setLoading] = useState(false)
  const [loadingSessionIds, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const [pendingScrollTarget, setPendingScrollTarget] = useState<number | null>(null)
  const clearPendingScrollTarget = useCallback(() => setPendingScrollTarget(null), [])

  const currentSessionIdRef = useRef<string | null>(null)
  const switchVersionRef = useRef(0)
  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const hasMoreRef = useRef<Map<string, boolean>>(new Map())
  const oldestDbIdRef = useRef<Map<string, number>>(new Map())
  // Mirror of `messages` so `jumpToMessage` can synchronously check whether
  // a target message is already loaded without stale-closure hazards.
  const messagesRef = useRef<Message[]>([])
  // Mirror of `sessions` so callbacks reading session metadata don't have to
  // list `sessions` in their deps (which would invalidate them on every
  // streaming meta tick and cascade re-renders into the sidebar tree).
  const sessionsRef = useRef<SessionMeta[]>([])
  // Tracks the previous `currentSessionId` so the effect below can fire
  // `purge_session_if_incognito` exactly once per swap.
  const previousSessionIdRef = useRef<string | null>(null)

  // Keep ref in sync with state
  useEffect(() => {
    currentSessionIdRef.current = currentSessionId
  }, [currentSessionId])

  useEffect(() => {
    messagesRef.current = messages
  }, [messages])

  useEffect(() => {
    sessionsRef.current = sessions
  }, [sessions])

  // --- Session pagination sub-hook ---
  const {
    hasMore,
    setHasMore,
    loadingMore,
    hasMoreSessions,
    // setHasMoreSessions not needed at this level
    loadingMoreSessions,
    handleLoadMore,
    handleLoadMoreSessions,
    reloadSessions,
  } = useSessionPagination({
    currentSessionIdRef,
    sessionCacheRef,
    hasMoreRef,
    oldestDbIdRef,
    setSessions,
    setMessages,
    sessionsLength: sessions.length,
  })

  // --- Channel streaming sub-hook ---
  useChannelStreaming({
    currentSessionIdRef,
    sessionCacheRef,
    loadingSessionsRef,
    setMessages,
    setLoading,
    setLoadingSessionIds,
    reloadSessions,
  })

  /** Update messages for a specific session. If it's the current session, also update state. */
  const updateSessionMessages = useCallback(
    (sessionId: string, updater: (prev: Message[]) => Message[]) => {
      const prev = sessionCacheRef.current.get(sessionId) || []
      const next = updater(prev)
      sessionCacheRef.current.set(sessionId, next)
      if (currentSessionIdRef.current === sessionId) {
        setMessages(next)
      }
    },
    [],
  )

  const updateSessionMeta = useCallback(
    (sessionId: string, updater: (prev: SessionMeta) => SessionMeta) => {
      setSessions((prev) => {
        let changed = false
        const next = prev.map((session) => {
          if (session.id !== sessionId) return session
          const updated = updater(session)
          if (updated !== session) changed = true
          return updated
        })
        return changed ? next : prev
      })
    },
    [],
  )

  // Drop every locally-cached trace of a session — used both by explicit
  // delete and by the incognito close-on-leave purge so the two paths stay
  // in lockstep and we don't leak entries in any of the per-session refs.
  const evictSessionLocal = useCallback((sessionId: string) => {
    sessionCacheRef.current.delete(sessionId)
    loadingSessionsRef.current.delete(sessionId)
    hasMoreRef.current.delete(sessionId)
    oldestDbIdRef.current.delete(sessionId)
    setLoadingSessionIds((prev) => {
      if (!prev.has(sessionId)) return prev
      const next = new Set(prev)
      next.delete(sessionId)
      return next
    })
    setSessions((prev) => {
      const next = prev.filter((s) => s.id !== sessionId)
      return next.length === prev.length ? prev : next
    })
  }, [])

  const purgeIncognitoSession = useCallback(
    (sessionIdToLeave: string | null) => {
      if (!sessionIdToLeave) return
      const previousMeta = sessionsRef.current.find((s) => s.id === sessionIdToLeave)
      if (!previousMeta?.incognito) return
      evictSessionLocal(sessionIdToLeave)
      void getTransport()
        .call("purge_session_if_incognito", { sessionId: sessionIdToLeave })
        .catch((err) => {
          logger.warn(
            "chat",
            "useChatSession::purgeIncognito",
            `purge failed for ${sessionIdToLeave}`,
            err,
          )
        })
    },
    [evictSessionLocal],
  )

  // Centralized close-on-leave: any path that mutates `currentSessionId`
  // (sidebar click, new chat, project new chat, deep-link nav, jumpToMessage,
  // delete-session-while-active) reaches this effect and the previous session
  // is purged exactly once. Beats open-coding the call at every navigation
  // entry point.
  useEffect(() => {
    const previous = previousSessionIdRef.current
    previousSessionIdRef.current = currentSessionId
    if (previous && previous !== currentSessionId) {
      purgeIncognitoSession(previous)
    }
  }, [currentSessionId, purgeIncognitoSession])

  // Load agent list
  const reloadAgents = useCallback(async () => {
    try {
      const list = await getTransport().call<AgentSummaryForSidebar[]>("list_agents")
      setAgents(list)
    } catch (e) {
      logger.error("ui", "ChatScreen::loadAgents", "Failed to load agents", e)
    }
  }, [])

  useEffect(() => {
    reloadSessions()
    reloadAgents()
  }, [reloadSessions, reloadAgents])

  // Refresh agent list when agents are created/saved/deleted in settings panel
  useEffect(() => {
    const handler = () => {
      reloadAgents()
    }
    window.addEventListener("agents-changed", handler)
    return () => window.removeEventListener("agents-changed", handler)
  }, [reloadAgents])

  // Listen for cron job completions to refresh unread counts + send notification
  useEffect(() => {
    return getTransport().listen("cron:run_completed", (raw) => {
      reloadSessions()
      const payload = raw as {
        job_id: string
        job_name: string
        status: string
        notify: boolean
      }
      if (payload.notify && payload.job_name) {
        const title =
          payload.status === "success" ? t("notification.cronSuccess") : t("notification.cronError")
        notify(title, payload.job_name)
      }
    })
  }, [reloadSessions, t])

  // Listen for pending-interaction lifecycle events so the sidebar refreshes
  // its `pendingInteractionCount` for non-active sessions in near-real-time.
  // Coalesce bursts via a 300ms trailing debounce — the underlying query is
  // cheap but we don't need to thrash the list.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null
    const schedule = () => {
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => {
        timer = null
        reloadSessions()
      }, 300)
    }
    const offApproval = getTransport().listen("approval_required", schedule)
    const offAskUser = getTransport().listen("ask_user_request", schedule)
    const offChanged = getTransport().listen("session_pending_interactions_changed", schedule)
    return () => {
      if (timer) clearTimeout(timer)
      offApproval()
      offAskUser()
      offChanged()
    }
  }, [reloadSessions])

  // Listen for sub-agent events — manage loading state + refresh sidebar
  useEffect(() => {
    return getTransport().listen("subagent_event", (raw) => {
      const payload = raw as SubagentEvent
      const childSid = payload.childSessionId
      if (childSid) {
        if (["spawning", "running"].includes(payload.status)) {
          loadingSessionsRef.current.add(childSid)
          setLoadingSessionIds(new Set(loadingSessionsRef.current))
        } else {
          loadingSessionsRef.current.delete(childSid)
          setLoadingSessionIds(new Set(loadingSessionsRef.current))
        }
      }
      if (["completed", "error", "timeout", "killed", "spawning"].includes(payload.status)) {
        reloadSessions()
      }
    })
  }, [reloadSessions])

  // Compute total unread count — exclude channel sessions (IM messages don't count as unread)
  const totalUnreadCount = useMemo(
    () =>
      sessions.reduce((sum, s) => {
        if (s.channelInfo || s.id === currentSessionId) return sum
        return sum + s.unreadCount
      }, 0),
    [sessions, currentSessionId],
  )

  useEffect(() => {
    onUnreadCountChange?.(totalUnreadCount)
  }, [totalUnreadCount, onUnreadCountChange])

  // Switch to an existing session
  const handleSwitchSession = useCallback(
    async (sessionId: string, opts?: { targetMessageId?: number }) => {
      const targetMessageId = opts?.targetMessageId
      // Always reload when jumping to a specific message; otherwise skip if
      // already viewing the same session.
      if (!sessionId) return
      if (targetMessageId === undefined && sessionId === currentSessionIdRef.current) {
        return
      }

      const version = ++switchVersionRef.current

      // If target session is in cache and we don't need to jump to a specific
      // message, restore immediately.
      const cached = sessionCacheRef.current.get(sessionId)
      if (targetMessageId === undefined && cached) {
        setMessages(cached)
        setHasMore(hasMoreRef.current.get(sessionId) ?? false)
        setLoading(loadingSessionsRef.current.has(sessionId))
        setCurrentSessionId(sessionId)
      } else {
        try {
          let msgs: SessionMessage[]
          let hasMoreBefore: boolean
          if (targetMessageId !== undefined) {
            // `[messages, total, hasMoreBefore, hasMoreAfter]` — hasMoreAfter unused.
            const [m, , hasMoreB] = await getTransport().call<
              [SessionMessage[], number, boolean, boolean]
            >("load_session_messages_around_cmd", {
              sessionId,
              targetMessageId,
              before: 40,
              after: 20,
            })
            msgs = m
            hasMoreBefore = hasMoreB
          } else {
            // hasMore is authoritative from DB; don't infer from msgs.length
            // since user-boundary alignment may extend beyond the requested limit.
            const [m, , hasMore] = await getTransport().call<[SessionMessage[], number, boolean]>(
              "load_session_messages_latest_cmd",
              { sessionId, limit: PAGE_SIZE },
            )
            msgs = m
            hasMoreBefore = hasMore
          }
          const [currentSessions] = await getTransport().call<[SessionMeta[], number]>(
            "list_sessions_cmd",
            {},
          )
          const sessionMeta = currentSessions.find((s) => s.id === sessionId)
          const parentSession = sessionMeta?.parentSessionId
            ? currentSessions.find((s) => s.id === sessionMeta.parentSessionId)
            : undefined
          if (switchVersionRef.current !== version) return // stale switch
          const displayMessages = parseSessionMessages(msgs, parentSession?.agentId)
          sessionCacheRef.current.set(sessionId, displayMessages)
          hasMoreRef.current.set(sessionId, hasMoreBefore)
          if (msgs.length > 0) {
            oldestDbIdRef.current.set(sessionId, msgs[0].id)
          }
          setMessages(displayMessages)
          setHasMore(hasMoreBefore)
          setLoading(loadingSessionsRef.current.has(sessionId))
          setCurrentSessionId(sessionId)
        } catch (e) {
          logger.error("session", "ChatScreen::switchSession", "Failed to load session", {
            sessionId,
            error: e,
          })
          return
        }
      }

      if (targetMessageId !== undefined) {
        setPendingScrollTarget(targetMessageId)
      }

      if (switchVersionRef.current !== version) return // stale switch

      // Use fresh sessions list for session lookup
      const [currentSessions] = await getTransport()
        .call<[SessionMeta[], number]>("list_sessions_cmd", {})
        .catch(() => [[] as SessionMeta[], 0] as [SessionMeta[], number])
      const currentAgents = await getTransport()
        .call<AgentSummaryForSidebar[]>("list_agents")
        .catch(() => [] as AgentSummaryForSidebar[])
      const session = currentSessions.find((s) => s.id === sessionId)
      if (session) {
        setCurrentAgentId(session.agentId)
        const agent = currentAgents.find((a) => a.id === session.agentId)
        if (agent) setAgentName(agent.name)

        // Restore the model used in this session (if still available)
        if (session.providerId && session.modelId) {
          const modelExists = availableModels.some(
            (m) => m.providerId === session.providerId && m.modelId === session.modelId,
          )
          if (modelExists) {
            handleModelChange(`${session.providerId}::${session.modelId}`)
          }
        } else {
          // Session has no model info, fallback to agent's configured model or global default
          try {
            const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", {
              id: session.agentId,
            })
            if (agentConfig.model.primary) {
              const modelExists = availableModels.some(
                (m) => `${m.providerId}::${m.modelId}` === agentConfig.model.primary,
              )
              if (modelExists) {
                applyModelForDisplay(agentConfig.model.primary)
                // Mark session as read and refresh
                getTransport()
                  .call("mark_session_read_cmd", { sessionId })
                  .catch(() => {})
                reloadSessions()
                return
              }
            }
          } catch {
            // ignore
          }
          // No agent model or unavailable — restore global default
          if (globalActiveModelRef.current) {
            setActiveModel(globalActiveModelRef.current)
          }
        }
      }

      // Mark session as read and refresh unread counts
      getTransport()
        .call("mark_session_read_cmd", { sessionId })
        .catch(() => {})
      reloadSessions()
    },
    [
      availableModels,
      handleModelChange,
      applyModelForDisplay,
      globalActiveModelRef,
      setActiveModel,
      reloadSessions,
      setHasMore,
    ],
  )

  // Jump to a specific message within the *current* session. If the target
  // is already in the loaded window, just sets `pendingScrollTarget` to let
  // MessageList scroll & pulse. Otherwise reloads a window of messages
  // centred on the target (delegating to handleSwitchSession).
  const jumpToMessage = useCallback(
    async (messageId: number) => {
      const sid = currentSessionIdRef.current
      if (!sid) return
      const exists = messagesRef.current.some((m) => m.dbId === messageId)
      if (exists) {
        setPendingScrollTarget(messageId)
        return
      }
      await handleSwitchSession(sid, { targetMessageId: messageId })
    },
    [handleSwitchSession],
  )

  // Navigate to a specific session when initialSessionId changes
  useEffect(() => {
    if (!initialSessionId) return
    ;(async () => {
      await reloadSessions()
      await handleSwitchSession(initialSessionId)
      onSessionNavigated?.()
    })()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialSessionId])

  // Create a new chat with a specific agent
  const handleNewChat = useCallback(
    async (agentId: string) => {
      // Save current session to cache
      // (cache is already maintained by updateSessionMessages)

      const currentAgents = await getTransport()
        .call<AgentSummaryForSidebar[]>("list_agents")
        .catch(() => [] as AgentSummaryForSidebar[])
      const agent = currentAgents.find((a) => a.id === agentId)
      setMessages([])
      setCurrentSessionId(null)
      setLoading(false)
      setHasMore(false)
      setCurrentAgentId(agentId)
      if (agent) {
        setAgentName(agent.name)
      }

      // Apply agent's configured model, or restore global default
      try {
        const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", {
          id: agentId,
        })
        if (agentConfig.model.primary) {
          const modelExists = availableModels.some(
            (m) => `${m.providerId}::${m.modelId}` === agentConfig.model.primary,
          )
          if (modelExists) {
            applyModelForDisplay(agentConfig.model.primary)
            return
          }
        }
      } catch {
        // ignore
      }
      // No agent model configured or unavailable — restore global default
      if (globalActiveModelRef.current) {
        setActiveModel(globalActiveModelRef.current)
      }
    },
    [availableModels, applyModelForDisplay, globalActiveModelRef, setActiveModel, setHasMore],
  )

  // Create a new session inside a Project and materialize it immediately
  // so project context is active for the first message.
  const handleNewChatInProject = useCallback(
    async (projectId: string, defaultAgentId?: string | null, incognito = false) => {
      try {
        const agentId =
          defaultAgentId && defaultAgentId.length > 0 ? defaultAgentId : currentAgentId
        const created = await getTransport().call<SessionMeta>("create_session_cmd", {
          agentId,
          projectId,
          // Project + incognito are mutually exclusive; backend coerces but
          // we strip here too so the optimistic UI stays consistent.
          incognito: projectId ? false : incognito,
        })
        setMessages([])
        setCurrentSessionId(created.id)
        setLoading(false)
        setHasMore(false)
        setCurrentAgentId(created.agentId)
        const currentAgents = await getTransport()
          .call<AgentSummaryForSidebar[]>("list_agents")
          .catch(() => [] as AgentSummaryForSidebar[])
        const agent = currentAgents.find((a) => a.id === created.agentId)
        if (agent) {
          setAgentName(agent.name)
        }
        // Apply the agent's configured model (best-effort).
        try {
          const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", {
            id: created.agentId,
          })
          if (agentConfig.model.primary) {
            const modelExists = availableModels.some(
              (m) => `${m.providerId}::${m.modelId}` === agentConfig.model.primary,
            )
            if (modelExists) {
              applyModelForDisplay(agentConfig.model.primary)
            }
          }
        } catch {
          // ignore
        }
        if (globalActiveModelRef.current) {
          setActiveModel(globalActiveModelRef.current)
        }
      } catch (e) {
        logger.warn("useChatSession", "handleNewChatInProject failed", e)
        notify({ title: t("common.saveFailed"), body: String(e) })
      }
    },
    [
      currentAgentId,
      availableModels,
      applyModelForDisplay,
      globalActiveModelRef,
      setActiveModel,
      setHasMore,
      t,
    ],
  )

  // Delete a session
  const handleDeleteSession = useCallback(
    async (sessionId: string) => {
      const sessionTitle =
        sessions.find((s) => s.id === sessionId)?.title || t("chat.untitledSession")
      try {
        await getTransport().call("delete_session_cmd", { sessionId })
        evictSessionLocal(sessionId)
        if (currentSessionIdRef.current === sessionId) {
          setMessages([])
          setCurrentSessionId(null)
          setLoading(false)
          setHasMore(false)
        }
        reloadSessions()
        toast.success(t("common.deleted"), {
          description: sessionTitle,
        })
      } catch (err) {
        logger.error("session", "ChatScreen::deleteSession", "Failed to delete session", err)
        toast.error(t("common.deleteFailed"), {
          description: sessionTitle,
        })
      }
    },
    [reloadSessions, setHasMore, evictSessionLocal, sessions, t],
  )

  return {
    messages,
    setMessages,
    currentSessionId,
    setCurrentSessionId,
    currentSessionIdRef,
    sessions,
    agents,
    currentAgentId,
    setCurrentAgentId,
    agentName,
    setAgentName,
    loading,
    setLoading,
    loadingSessionIds,
    setLoadingSessionIds,
    hasMore,
    loadingMore,
    hasMoreSessions,
    loadingMoreSessions,
    pendingScrollTarget,
    clearPendingScrollTarget,
    jumpToMessage,
    sessionCacheRef,
    loadingSessionsRef,
    hasMoreRef,
    oldestDbIdRef,
    reloadSessions,
    reloadAgents,
    handleSwitchSession,
    handleNewChat,
    handleNewChatInProject,
    handleDeleteSession,
    handleLoadMore,
    handleLoadMoreSessions,
    updateSessionMessages,
    updateSessionMeta,
  }
}

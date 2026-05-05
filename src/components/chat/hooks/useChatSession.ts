import { useState, useRef, useEffect, useCallback, useMemo } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { notify } from "@/lib/notifications"
import { materializeMessages } from "../chatUtils"
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
  hasMoreAfter: boolean
  loadingMoreAfter: boolean
  hasMoreSessions: boolean
  loadingMoreSessions: boolean
  /**
   * Search-jump intent for MessageList: which message to scroll to + which
   * literal substrings to inline-highlight inside it. `null` between jumps.
   * Consumer calls `clearPendingScrollIntent` once the scroll has been
   * applied. Single object so the (target, terms) invariant can't drift —
   * terms are always tied to a specific scroll target.
   */
  pendingScrollIntent: { messageId: number; highlightTerms: string[] | null } | null
  clearPendingScrollIntent: () => void
  /**
   * Scroll the current session to a specific message and briefly highlight
   * it. If the target is not in the currently loaded window, reloads a
   * window of messages centred on the target first. Used by the in-chat
   * "find in page" search bar. Optional `highlightTerms` are painted inline
   * inside the target bubble via the CSS Custom Highlight API.
   */
  jumpToMessage: (messageId: number, highlightTerms?: string[]) => Promise<void>

  // Refs
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  hasMoreRef: React.MutableRefObject<Map<string, boolean>>
  oldestDbIdRef: React.MutableRefObject<Map<string, number>>

  // Handlers
  reloadSessions: () => Promise<void>
  reloadAgents: () => Promise<void>
  handleSwitchSession: (
    sessionId: string,
    opts?: { targetMessageId?: number; highlightTerms?: string[] },
  ) => Promise<void>
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
  handleLoadMoreAfter: () => Promise<void>
  /** Drop the partial around-window and reload the latest page. */
  resetToLatest: () => Promise<void>
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
  onSidebarAggregatesChanged?: () => void
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
  onSidebarAggregatesChanged,
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
  const [pendingScrollIntent, setPendingScrollIntent] = useState<
    { messageId: number; highlightTerms: string[] | null } | null
  >(null)
  const clearPendingScrollIntent = useCallback(() => setPendingScrollIntent(null), [])

  const currentSessionIdRef = useRef<string | null>(null)
  const switchVersionRef = useRef(0)
  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const hasMoreRef = useRef<Map<string, boolean>>(new Map())
  const hasMoreAfterRef = useRef<Map<string, boolean>>(new Map())
  const oldestDbIdRef = useRef<Map<string, number>>(new Map())
  const newestDbIdRef = useRef<Map<string, number>>(new Map())
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
    hasMoreAfter,
    setHasMoreAfter,
    loadingMoreAfter,
    hasMoreSessions,
    // setHasMoreSessions not needed at this level
    loadingMoreSessions,
    handleLoadMore,
    handleLoadMoreAfter,
    resetToLatest,
    handleLoadMoreSessions,
    reloadSessions,
  } = useSessionPagination({
    currentSessionIdRef,
    sessionCacheRef,
    hasMoreRef,
    hasMoreAfterRef,
    oldestDbIdRef,
    newestDbIdRef,
    sessionsRef,
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
      const hasCached = sessionCacheRef.current.has(sessionId)
      const prev = sessionCacheRef.current.get(sessionId) || []
      const next = updater(prev)
      if (
        !hasCached &&
        currentSessionIdRef.current !== sessionId &&
        next === prev &&
        next.length === 0
      ) {
        return
      }
      sessionCacheRef.current.set(sessionId, next)
      // Track the tail dbId so handleLoadMoreAfter has a fresh anchor even
      // after streaming appends. Otherwise the after-pagination cursor stays
      // pinned to whatever was loaded at switch time and the second
      // handleLoadMoreAfter call would re-fetch already-displayed rows.
      const tail = next.length > 0 ? next[next.length - 1] : null
      if (tail && typeof tail.dbId === "number") {
        const prevNewest = newestDbIdRef.current.get(sessionId)
        if (prevNewest === undefined || tail.dbId > prevNewest) {
          newestDbIdRef.current.set(sessionId, tail.dbId)
        }
      }
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
    hasMoreAfterRef.current.delete(sessionId)
    oldestDbIdRef.current.delete(sessionId)
    newestDbIdRef.current.delete(sessionId)
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

  // Load agent list. Also pulls the global `default_agent_id` so the
  // implicit "current agent" state matches what the user configured in
  // settings — without this, `currentAgentId` is stuck at the hardcoded
  // "default" until the user switches manually, defeating the setting.
  const reloadAgents = useCallback(async () => {
    try {
      const [list, defaultId] = await Promise.all([
        getTransport().call<AgentSummaryForSidebar[]>("list_agents"),
        getTransport()
          .call<string | null>("get_default_agent_id")
          .catch(() => null),
      ])
      setAgents(list)
      // Only reseed the implicit selection while no concrete session is
      // active. Once the user is inside a session we already track its
      // agent_id and don't want to clobber it.
      if (!currentSessionIdRef.current) {
        const id =
          typeof defaultId === "string" && defaultId.trim().length > 0
            ? defaultId
            : "default"
        setCurrentAgentId(id)
        const match = list.find((a) => a.id === id)
        if (match) setAgentName(match.name)
      }
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

  // Pick up changes to the global default agent from the settings panel.
  useEffect(() => {
    return getTransport().listen("config:changed", () => {
      void reloadAgents()
    })
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

  // Compute total unread count — channel and sub-agent sessions don't surface
  // global unread indicators in the primary chat entry.
  const totalUnreadCount = useMemo(
    () =>
      sessions.reduce((sum, s) => {
        if (s.channelInfo || s.parentSessionId || s.id === currentSessionId) return sum
        return sum + s.unreadCount
      }, 0),
    [sessions, currentSessionId],
  )

  useEffect(() => {
    onUnreadCountChange?.(totalUnreadCount)
  }, [totalUnreadCount, onUnreadCountChange])

  // Switch to an existing session
  const handleSwitchSession = useCallback(
    async (
      sessionId: string,
      opts?: { targetMessageId?: number; highlightTerms?: string[] },
    ) => {
      const targetMessageId = opts?.targetMessageId
      const highlightTerms = opts?.highlightTerms
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
        setHasMoreAfter(hasMoreAfterRef.current.get(sessionId) ?? false)
        setLoading(loadingSessionsRef.current.has(sessionId))
        setCurrentSessionId(sessionId)
      } else {
        try {
          let msgs: SessionMessage[]
          let hasMoreBefore: boolean
          let hasMoreAfterFlag = false
          if (targetMessageId !== undefined) {
            // Symmetric 40/40 around-window so a hit shows enough context
            // both ways for handleLoadMoreAfter to take over naturally.
            const [m, , hasMoreB, hasMoreA] = await getTransport().call<
              [SessionMessage[], number, boolean, boolean]
            >("load_session_messages_around_cmd", {
              sessionId,
              targetMessageId,
              before: 40,
              after: 40,
            })
            msgs = m
            hasMoreBefore = hasMoreB
            hasMoreAfterFlag = hasMoreA
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
          const displayMessages = await materializeMessages(sessionId, msgs, sessionsRef)
          if (switchVersionRef.current !== version) return // stale switch
          sessionCacheRef.current.set(sessionId, displayMessages)
          hasMoreRef.current.set(sessionId, hasMoreBefore)
          hasMoreAfterRef.current.set(sessionId, hasMoreAfterFlag)
          if (msgs.length > 0) {
            oldestDbIdRef.current.set(sessionId, msgs[0].id)
            newestDbIdRef.current.set(sessionId, msgs[msgs.length - 1].id)
          }
          setMessages(displayMessages)
          setHasMore(hasMoreBefore)
          setHasMoreAfter(hasMoreAfterFlag)
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
        setPendingScrollIntent({
          messageId: targetMessageId,
          highlightTerms: highlightTerms ?? null,
        })
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
                  .finally(() => {
                    reloadSessions()
                    onSidebarAggregatesChanged?.()
                  })
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
        .finally(() => {
          reloadSessions()
          onSidebarAggregatesChanged?.()
        })
    },
    [
      availableModels,
      handleModelChange,
      applyModelForDisplay,
      globalActiveModelRef,
      setActiveModel,
      reloadSessions,
      onSidebarAggregatesChanged,
      setHasMore,
      setHasMoreAfter,
    ],
  )

  // Jump to a specific message within the *current* session. If the target
  // is already in the loaded window, just sets `pendingScrollIntent` to let
  // MessageList scroll & pulse. Otherwise reloads a window of messages
  // centred on the target (delegating to handleSwitchSession).
  const jumpToMessage = useCallback(
    async (messageId: number, highlightTerms?: string[]) => {
      const sid = currentSessionIdRef.current
      if (!sid) return
      const exists = messagesRef.current.some((m) => m.dbId === messageId)
      if (exists) {
        setPendingScrollIntent({ messageId, highlightTerms: highlightTerms ?? null })
        return
      }
      await handleSwitchSession(sid, { targetMessageId: messageId, highlightTerms })
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
      setHasMoreAfter(false)
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
    [
      availableModels,
      applyModelForDisplay,
      globalActiveModelRef,
      setActiveModel,
      setHasMore,
      setHasMoreAfter,
    ],
  )

  // Create a new session inside a Project and materialize it immediately
  // so project context is active for the first message.
  const handleNewChatInProject = useCallback(
    async (projectId: string, defaultAgentId?: string | null, incognito = false) => {
      try {
        // When the caller does not supply an explicit agentId, leave it
        // undefined so `create_session_cmd` runs the resolver chain
        // (project.default_agent_id → AppConfig.default_agent_id →
        // hardcoded "default"). Falling back to the in-UI `currentAgentId`
        // here would pin the new session to whichever agent the user happened
        // to be chatting with last, bypassing project / global defaults.
        const explicitAgent =
          defaultAgentId && defaultAgentId.length > 0 ? defaultAgentId : undefined
        const created = await getTransport().call<SessionMeta>("create_session_cmd", {
          agentId: explicitAgent,
          projectId,
          // Project + incognito are mutually exclusive; backend coerces but
          // we strip here too so the optimistic UI stays consistent.
          incognito: projectId ? false : incognito,
        })
        setMessages([])
        setCurrentSessionId(created.id)
        setSessions((prev) =>
          prev.some((s) => s.id === created.id) ? prev : [created, ...prev],
        )
        onSidebarAggregatesChanged?.()
        setLoading(false)
        setHasMore(false)
        setHasMoreAfter(false)
        setCurrentAgentId(created.agentId)
        const agent = agents.find((a) => a.id === created.agentId)
        if (agent) setAgentName(agent.name)
        // Apply the agent's configured model. If the agent has no preferred
        // model (or it's not currently available), fall back to the global
        // active model so the title bar still shows something sensible.
        let appliedAgentModel = false
        try {
          const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", {
            id: created.agentId,
          })
          const primary = agentConfig.model.primary
          if (primary) {
            const modelExists = availableModels.some(
              (m) => `${m.providerId}::${m.modelId}` === primary,
            )
            if (modelExists) {
              applyModelForDisplay(primary)
              appliedAgentModel = true
            }
          }
        } catch {
          // ignore
        }
        if (!appliedAgentModel && globalActiveModelRef.current) {
          setActiveModel(globalActiveModelRef.current)
        }
      } catch (e) {
        logger.warn("chat", "useChatSession", "handleNewChatInProject failed", e)
        notify(t("common.saveFailed"), String(e))
      }
    },
    [
      agents,
      availableModels,
      applyModelForDisplay,
      globalActiveModelRef,
      onSidebarAggregatesChanged,
      setActiveModel,
      setHasMore,
      setHasMoreAfter,
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
          setHasMoreAfter(false)
        }
        reloadSessions()
        onSidebarAggregatesChanged?.()
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
    [
      reloadSessions,
      setHasMore,
      setHasMoreAfter,
      evictSessionLocal,
      sessions,
      t,
      onSidebarAggregatesChanged,
    ],
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
    hasMoreAfter,
    loadingMoreAfter,
    hasMoreSessions,
    loadingMoreSessions,
    pendingScrollIntent,
    clearPendingScrollIntent,
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
    handleLoadMoreAfter,
    resetToLatest,
    handleLoadMoreSessions,
    updateSessionMessages,
    updateSessionMeta,
  }
}

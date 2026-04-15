import { useState, useRef, useEffect, useCallback, useMemo } from "react"
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
  handleSwitchSession: (
    sessionId: string,
    opts?: { targetMessageId?: number },
  ) => Promise<void>
  handleNewChat: (agentId: string) => Promise<void>
  handleDeleteSession: (sessionId: string) => Promise<void>
  handleLoadMore: () => Promise<void>
  handleLoadMoreSessions: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
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

  // Keep ref in sync with state
  useEffect(() => {
    currentSessionIdRef.current = currentSessionId
  }, [currentSessionId])

  useEffect(() => {
    messagesRef.current = messages
  }, [messages])

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
    const offChanged = getTransport().listen(
      "session_pending_interactions_changed",
      schedule,
    )
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
    () => sessions.reduce((sum, s) => {
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
          let total: number
          if (targetMessageId !== undefined) {
            // Load a window around the target message for jump-to-message.
            ;[msgs, total] = await getTransport().call<[SessionMessage[], number]>(
              "load_session_messages_around_cmd",
              {
                sessionId,
                targetMessageId,
                before: 40,
                after: 20,
              },
            )
          } else {
            ;[msgs, total] = await getTransport().call<[SessionMessage[], number]>(
              "load_session_messages_latest_cmd",
              { sessionId, limit: PAGE_SIZE },
            )
          }
          const [currentSessions] = await getTransport().call<[SessionMeta[], number]>("list_sessions_cmd", {})
          const sessionMeta = currentSessions.find((s) => s.id === sessionId)
          const parentSession = sessionMeta?.parentSessionId
            ? currentSessions.find((s) => s.id === sessionMeta.parentSessionId)
            : undefined
          if (switchVersionRef.current !== version) return // stale switch
          const displayMessages = parseSessionMessages(msgs, parentSession?.agentId)
          sessionCacheRef.current.set(sessionId, displayMessages)
          const moreAvailable = msgs.length < total
          hasMoreRef.current.set(sessionId, moreAvailable)
          if (msgs.length > 0) {
            oldestDbIdRef.current.set(sessionId, msgs[0].id)
          }
          setMessages(displayMessages)
          setHasMore(moreAvailable)
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
      const [currentSessions] = await getTransport().call<[SessionMeta[], number]>("list_sessions_cmd", {}).catch(
        () => [[] as SessionMeta[], 0] as [SessionMeta[], number],
      )
      const currentAgents = await getTransport().call<AgentSummaryForSidebar[]>("list_agents").catch(
        () => [] as AgentSummaryForSidebar[],
      )
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
                getTransport().call("mark_session_read_cmd", { sessionId }).catch(() => {})
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
      getTransport().call("mark_session_read_cmd", { sessionId }).catch(() => {})
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

      const currentAgents = await getTransport().call<AgentSummaryForSidebar[]>("list_agents").catch(
        () => [] as AgentSummaryForSidebar[],
      )
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

  // Delete a session
  const handleDeleteSession = useCallback(
    async (sessionId: string) => {
      try {
        await getTransport().call("delete_session_cmd", { sessionId })
        sessionCacheRef.current.delete(sessionId)
        loadingSessionsRef.current.delete(sessionId)
        hasMoreRef.current.delete(sessionId)
        oldestDbIdRef.current.delete(sessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        if (currentSessionIdRef.current === sessionId) {
          setMessages([])
          setCurrentSessionId(null)
          setLoading(false)
          setHasMore(false)
        }
        reloadSessions()
      } catch (err) {
        logger.error("session", "ChatScreen::deleteSession", "Failed to delete session", err)
      }
    },
    [reloadSessions, setHasMore],
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
    handleDeleteSession,
    handleLoadMore,
    handleLoadMoreSessions,
    updateSessionMessages,
  }
}

import { useState, useRef, useEffect, useCallback, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { notify } from "@/lib/notifications"
import { parseSessionMessages } from "./chatUtils"
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

export const PAGE_SIZE = 30
export const SESSION_PAGE_SIZE = 50

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

  // Refs
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  hasMoreRef: React.MutableRefObject<Map<string, boolean>>
  oldestDbIdRef: React.MutableRefObject<Map<string, number>>

  // Handlers
  reloadSessions: () => Promise<void>
  reloadAgents: () => Promise<void>
  handleSwitchSession: (sessionId: string) => Promise<void>
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
  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [hasMoreSessions, setHasMoreSessions] = useState(false)
  const [loadingMoreSessions, setLoadingMoreSessions] = useState(false)

  const currentSessionIdRef = useRef<string | null>(null)
  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const hasMoreRef = useRef<Map<string, boolean>>(new Map())
  const oldestDbIdRef = useRef<Map<string, number>>(new Map())

  // Keep ref in sync with state
  useEffect(() => {
    currentSessionIdRef.current = currentSessionId
  }, [currentSessionId])

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

  // Load session list and agent list (paginated)
  const reloadSessions = useCallback(async () => {
    try {
      const [list, total] = await invoke<[SessionMeta[], number]>("list_sessions_cmd", {
        limit: SESSION_PAGE_SIZE,
        offset: 0,
      })
      setSessions(list)
      setHasMoreSessions(list.length < total)
    } catch (e) {
      logger.error("ui", "ChatScreen::loadSessions", "Failed to load sessions", e)
    }
  }, [])

  const handleLoadMoreSessions = useCallback(async () => {
    if (loadingMoreSessions || !hasMoreSessions) return
    setLoadingMoreSessions(true)
    try {
      const [more, total] = await invoke<[SessionMeta[], number]>("list_sessions_cmd", {
        limit: SESSION_PAGE_SIZE,
        offset: sessions.length,
      })
      if (more.length === 0) {
        setHasMoreSessions(false)
        return
      }
      setSessions((prev) => {
        // Deduplicate by id in case new sessions appeared while paginating
        const existingIds = new Set(prev.map((s) => s.id))
        const newItems = more.filter((s) => !existingIds.has(s.id))
        const merged = [...prev, ...newItems]
        setHasMoreSessions(merged.length < total)
        return merged
      })
    } catch (e) {
      logger.error("ui", "ChatScreen::loadMoreSessions", "Failed to load more sessions", e)
    } finally {
      setLoadingMoreSessions(false)
    }
  }, [loadingMoreSessions, hasMoreSessions, sessions.length])

  const reloadAgents = useCallback(async () => {
    try {
      const list = await invoke<AgentSummaryForSidebar[]>("list_agents")
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
    let unlisten: UnlistenFn | undefined
    listen("cron:run_completed", (event) => {
      reloadSessions()
      const payload = event.payload as {
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
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [reloadSessions, t])

  // Listen for sub-agent events — manage loading state + refresh sidebar
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen("subagent_event", (event) => {
      const payload = event.payload as SubagentEvent
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
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [reloadSessions])

  // Listen for channel stream lifecycle — loading state management
  useEffect(() => {
    const unlisteners: UnlistenFn[] = []

    listen("channel:stream_start", (event) => {
      const payload = event.payload as { sessionId: string }
      if (!payload.sessionId) return
      // Mark session as loading
      loadingSessionsRef.current.add(payload.sessionId)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))
      if (payload.sessionId === currentSessionIdRef.current) {
        setLoading(true)
      }
    }).then((fn) => unlisteners.push(fn))

    listen("channel:stream_end", (event) => {
      const payload = event.payload as { sessionId: string }
      if (!payload.sessionId) return
      loadingSessionsRef.current.delete(payload.sessionId)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))
      if (payload.sessionId === currentSessionIdRef.current) {
        setLoading(false)
      }
    }).then((fn) => unlisteners.push(fn))

    return () => { unlisteners.forEach((fn) => fn()) }
  }, [setLoading, setLoadingSessionIds])

  // Listen for channel streaming events — real-time display for active channel session
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen("channel:stream_delta", (event) => {
      const payload = event.payload as { sessionId: string; event: string }
      if (!payload.sessionId || payload.sessionId !== currentSessionIdRef.current) return

      // Parse the raw event from chat engine
      let parsed: { type?: string; text?: string } | null = null
      try { parsed = JSON.parse(payload.event) } catch { return }
      if (!parsed || parsed.type !== "text_delta" || !parsed.text) return

      const delta = parsed.text
      setMessages((prev) => {
        const last = prev[prev.length - 1]
        if (last && last.role === "assistant" && last.isStreaming) {
          const updated = [...prev]
          updated[updated.length - 1] = { ...last, content: last.content + delta }
          return updated
        }
        return [
          ...prev,
          { role: "assistant" as const, content: delta, isStreaming: true, timestamp: new Date().toISOString() },
        ]
      })
    }).then((fn) => { unlisten = fn })
    return () => { unlisten?.() }
  }, [])

  // Listen for channel message updates — refresh sessions + reload current session messages
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen("channel:message_update", (event) => {
      const payload = event.payload as { sessionId: string }
      reloadSessions()
      // If the updated session is currently active, reload its messages from DB
      // (replaces the streaming placeholder with the final persisted message)
      if (payload.sessionId && payload.sessionId === currentSessionIdRef.current) {
        invoke<[SessionMessage[], number]>("load_session_messages_latest_cmd", {
          sessionId: payload.sessionId,
          limit: 50,
        }).then(([msgs]) => {
          const parsed = parseSessionMessages(msgs)
          setMessages(parsed)
          // Update cache
          sessionCacheRef.current.set(payload.sessionId, parsed)
        }).catch(() => {})
      }
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
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
    async (sessionId: string) => {
      if (!sessionId || sessionId === currentSessionIdRef.current) return

      // Save current session's messages to cache
      const curSid = currentSessionIdRef.current
      if (curSid) {
        // Read latest messages from state via a trick: we use the ref-based cache
        // (already kept in sync by updateSessionMessages)
      }

      // If target session is in cache (e.g. still loading), restore from cache
      const cached = sessionCacheRef.current.get(sessionId)
      if (cached) {
        setMessages(cached)
        setHasMore(hasMoreRef.current.get(sessionId) ?? false)
        setLoading(loadingSessionsRef.current.has(sessionId))
        setCurrentSessionId(sessionId)
      } else {
        // Load latest PAGE_SIZE messages from DB
        try {
          const [msgs, total] = await invoke<[SessionMessage[], number]>(
            "load_session_messages_latest_cmd",
            { sessionId, limit: PAGE_SIZE },
          )
          const [currentSessions] = await invoke<[SessionMeta[], number]>("list_sessions_cmd", {})
          const sessionMeta = currentSessions.find((s) => s.id === sessionId)
          const parentSession = sessionMeta?.parentSessionId
            ? currentSessions.find((s) => s.id === sessionMeta.parentSessionId)
            : undefined
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

      // Use fresh sessions list for session lookup
      const [currentSessions] = await invoke<[SessionMeta[], number]>("list_sessions_cmd", {}).catch(
        () => [[] as SessionMeta[], 0] as [SessionMeta[], number],
      )
      const currentAgents = await invoke<AgentSummaryForSidebar[]>("list_agents").catch(
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
            const agentConfig = await invoke<AgentConfig>("get_agent_config", {
              id: session.agentId,
            })
            if (agentConfig.model.primary) {
              const modelExists = availableModels.some(
                (m) => `${m.providerId}::${m.modelId}` === agentConfig.model.primary,
              )
              if (modelExists) {
                applyModelForDisplay(agentConfig.model.primary)
                // Mark session as read and refresh
                invoke("mark_session_read_cmd", { sessionId }).catch(() => {})
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
      invoke("mark_session_read_cmd", { sessionId }).catch(() => {})
      reloadSessions()
    },
    [
      availableModels,
      handleModelChange,
      applyModelForDisplay,
      globalActiveModelRef,
      setActiveModel,
      reloadSessions,
    ],
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
      if (currentSessionIdRef.current) {
        // cache is already maintained by updateSessionMessages
      }

      const currentAgents = await invoke<AgentSummaryForSidebar[]>("list_agents").catch(
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
        const agentConfig = await invoke<AgentConfig>("get_agent_config", {
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
    [availableModels, applyModelForDisplay, globalActiveModelRef, setActiveModel],
  )

  // Delete a session
  const handleDeleteSession = useCallback(
    async (sessionId: string) => {
      try {
        await invoke("delete_session_cmd", { sessionId })
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
    [reloadSessions],
  )

  // Load older messages when user scrolls to top
  const handleLoadMore = useCallback(async () => {
    const curSid = currentSessionIdRef.current
    if (!curSid || loadingMore || !hasMore) return
    const oldestId = oldestDbIdRef.current.get(curSid)
    if (oldestId === undefined) return

    setLoadingMore(true)
    try {
      const olderMsgs = await invoke<SessionMessage[]>("load_session_messages_before_cmd", {
        sessionId: curSid,
        beforeId: oldestId,
        limit: PAGE_SIZE,
      })
      if (olderMsgs.length === 0) {
        hasMoreRef.current.set(curSid, false)
        setHasMore(false)
        return
      }
      const [currentSessions] = await invoke<[SessionMeta[], number]>("list_sessions_cmd", {}).catch(
        () => [[] as SessionMeta[], 0] as [SessionMeta[], number],
      )
      const sessionMeta = currentSessions.find((s) => s.id === curSid)
      const parentSession = sessionMeta?.parentSessionId
        ? currentSessions.find((s) => s.id === sessionMeta.parentSessionId)
        : undefined
      const olderDisplay = parseSessionMessages(olderMsgs, parentSession?.agentId)
      oldestDbIdRef.current.set(curSid, olderMsgs[0].id)
      if (olderMsgs.length < PAGE_SIZE) {
        hasMoreRef.current.set(curSid, false)
        setHasMore(false)
      }

      setMessages((prev) => {
        const merged = [...olderDisplay, ...prev]
        sessionCacheRef.current.set(curSid, merged)
        return merged
      })
    } catch (e) {
      logger.error("session", "ChatScreen::loadMore", "Failed to load older messages", { error: e })
    } finally {
      setLoadingMore(false)
    }
  }, [loadingMore, hasMore])

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

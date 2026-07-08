import { useState, useRef, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { parseSessionMessages, reloadAndMergeSessionMessages } from "@/components/chat/chatUtils"
import { normalizeEffortForModel } from "@/types/chat"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import type {
  Message,
  AvailableModel,
  ActiveModel,
  SessionMeta,
  SessionMessage,
  AgentSummaryForSidebar,
} from "@/types/chat"
import type { AgentConfig } from "@/components/settings/types"
import type { DesignChatThread } from "@/types/design"

const PAGE_SIZE = 30
/** Thread-history page size (separate from the per-thread message page). */
const THREADS_PAGE = 30

type ModelSnapshot = {
  models: AvailableModel[]
  displayModel: ActiveModel | null
  defaultEffort: string
}

export interface UseDesignChatReturn {
  // useChatStream-compatible state
  messages: Message[]
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  currentSessionId: string | null
  setCurrentSessionId: React.Dispatch<React.SetStateAction<string | null>>
  currentSessionIdRef: React.MutableRefObject<string | null>
  currentAgentId: string
  agents: AgentSummaryForSidebar[]
  loading: boolean
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  sessions: SessionMeta[]
  reloadSessions: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void

  // Pagination
  hasMore: boolean
  loadingMore: boolean
  handleLoadMore: () => Promise<void>

  // Model state
  availableModels: AvailableModel[]
  activeModel: ActiveModel | null
  reasoningEffort: string
  handleModelChange: (key: string) => void
  handleEffortChange: (effort: string) => void

  // Agent
  handleSwitchAgent: (agentId: string) => void

  // Design chat threads
  threads: DesignChatThread[]
  reloadThreads: (query?: string) => Promise<void>
  /** More history pages exist beyond what's loaded (drives infinite scroll). */
  threadsHasMore: boolean
  /** Append the next history page (same query as the last `reloadThreads`). */
  loadMoreThreads: () => Promise<void>
  handleNewThread: () => void
  switchThread: (sessionId: string) => Promise<void>
  /** Reconcile the current thread with DB truth after a turn ends (HTTP has no
   *  live reattach here, so this fills in the final answer). Merge-based +
   *  session-guarded: never blanks the view on a transient error and never
   *  clobbers a thread the user has since switched to. */
  reconcileThread: (sessionId: string) => Promise<void>
}

/**
 * Session manager for the design-space per-project chat. Mirrors
 * `useKnowledgeChat` but threads are anchored to a design PROJECT (not a KB +
 * note): opening a project default-loads its most recent conversation, "new"
 * clears to a draft that the first send auto-creates as a design thread (via the
 * `chat` command's `toolScope: "design"` branch — no empty sessions), and the
 * history picker lists every thread in the project. Streaming/sending is driven
 * by `useChatStream` in the panel; this hook only owns session lifecycle +
 * model/agent state.
 */
export function useDesignChat(projectId: string | null, active: boolean): UseDesignChatReturn {
  const [messages, setMessages] = useState<Message[]>([])
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null)
  const currentSessionIdRef = useRef<string | null>(null)
  const [currentAgentId, setCurrentAgentId] = useState<string>(DEFAULT_AGENT_ID)
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [loading, setLoading] = useState(false)
  const [, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [threads, setThreads] = useState<DesignChatThread[]>([])
  const [threadsHasMore, setThreadsHasMore] = useState(false)
  // Pagination cursor for the history list (query + offset in refs so
  // `loadMoreThreads` keeps the active filter without re-arming each render).
  const threadsQueryRef = useRef<string | undefined>(undefined)
  const threadsOffsetRef = useRef(0)
  const threadsLoadingRef = useRef(false)
  const threadsLoadVersionRef = useRef(0)

  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const manualModelOverrideRef = useRef<ActiveModel | null>(null)
  // Monotonic guards: a late-resolving messages/model fetch must not clobber a
  // newer thread switch (last-writer-by-intent, not by RTT).
  const switchVersionRef = useRef(0)
  const modelLoadVersionRef = useRef(0)

  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [oldestDbId, setOldestDbId] = useState<number | null>(null)

  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null)
  const [reasoningEffort, setReasoningEffort] = useState("medium")

  useEffect(() => {
    currentSessionIdRef.current = currentSessionId
  }, [currentSessionId])

  const loadAgents = useCallback(async () => {
    try {
      const list = await getTransport().call<AgentSummaryForSidebar[]>("list_agents")
      setAgents(list)
      return list
    } catch (e) {
      logger.error("ui", "DesignChat::loadAgents", "Failed to load agents", e)
      return []
    }
  }, [])

  const loadModels = useCallback(
    async (agentId: string): Promise<ModelSnapshot | null> => {
      const version = ++modelLoadVersionRef.current
      try {
        const [models, active, settings, agentConfig] = await Promise.all([
          getTransport().call<AvailableModel[]>("get_available_models"),
          getTransport().call<ActiveModel | null>("get_active_model"),
          getTransport().call<{ reasoning_effort: string }>("get_current_settings"),
          getTransport()
            .call<AgentConfig>("get_agent_config", { id: agentId })
            .catch(() => null),
        ])
        if (modelLoadVersionRef.current !== version) return null
        setAvailableModels(models)
        let displayModel = active
        const manualOverride = manualModelOverrideRef.current
        const manualModel = manualOverride
          ? models.find(
              (m) =>
                m.providerId === manualOverride.providerId && m.modelId === manualOverride.modelId,
            )
          : undefined
        if (manualOverride && !manualModel) manualModelOverrideRef.current = null
        if (manualModel && manualOverride) {
          displayModel = manualOverride
        } else if (agentConfig?.model.primary) {
          const [providerId, modelId] = agentConfig.model.primary.split("::")
          const agentModel = models.find((m) => m.providerId === providerId && m.modelId === modelId)
          if (agentModel) displayModel = { providerId, modelId }
        }
        setActiveModel(displayModel)
        const currentModel = displayModel
          ? models.find(
              (m) => m.providerId === displayModel!.providerId && m.modelId === displayModel!.modelId,
            )
          : undefined
        const effort = agentConfig?.model?.reasoningEffort ?? settings.reasoning_effort
        setReasoningEffort(normalizeEffortForModel(currentModel, effort, (key) => key))
        return { models, displayModel, defaultEffort: effort }
      } catch (e) {
        logger.error("ui", "DesignChat::loadModels", "Failed to load models", e)
        return null
      }
    },
    [],
  )

  // Replace-load for SWITCHING to a thread (clears + repopulates). Version-guarded
  // so a slow A→B→A switch can't let the late A load overwrite B.
  const loadThreadMessages = useCallback(async (sessionId: string): Promise<boolean> => {
    const version = ++switchVersionRef.current
    try {
      const [rawMsgs, , hasMoreFromApi] = await getTransport().call<
        [SessionMessage[], number, boolean]
      >("load_session_messages_latest_cmd", { sessionId, limit: PAGE_SIZE })
      if (switchVersionRef.current !== version) return false
      const parsed = parseSessionMessages(rawMsgs)
      setMessages(parsed)
      sessionCacheRef.current.set(sessionId, parsed)
      setHasMore(hasMoreFromApi)
      setOldestDbId(rawMsgs[0]?.id ?? null)
      setLoadingMore(false)
      return true
    } catch (e) {
      if (switchVersionRef.current !== version) return false
      logger.error("ui", "DesignChat::loadMessages", "Failed to load messages", e)
      setMessages([])
      setHasMore(false)
      setOldestDbId(null)
      return false
    }
  }, [])

  // Reconcile the CURRENT thread with DB truth after a turn ends. Merge-based +
  // session-guarded (no blank-on-error, no cross-thread clobber).
  const reconcileThread = useCallback(async (sessionId: string) => {
    await reloadAndMergeSessionMessages({
      sessionId,
      pageSize: PAGE_SIZE,
      sessionCacheRef,
      setMessages: (msgs) => {
        if (currentSessionIdRef.current === sessionId) setMessages(msgs)
      },
    })
  }, [])

  // `query === undefined` (no arg) = refresh in place, keeping the active filter.
  // A string arg sets the filter (`""` clears it).
  const reloadThreads = useCallback(
    async (query?: string) => {
      if (!projectId) {
        setThreads([])
        setThreadsHasMore(false)
        return
      }
      const q = query === undefined ? threadsQueryRef.current : query.trim() || undefined
      threadsQueryRef.current = q
      threadsOffsetRef.current = 0
      const v = ++threadsLoadVersionRef.current
      try {
        const list = await getTransport().call<DesignChatThread[]>("design_chat_threads_list_cmd", {
          projectId,
          query: q,
          limit: THREADS_PAGE,
          offset: 0,
        })
        if (v !== threadsLoadVersionRef.current) return
        setThreads(list)
        setThreadsHasMore(list.length >= THREADS_PAGE)
        threadsOffsetRef.current = list.length
      } catch (e) {
        if (v !== threadsLoadVersionRef.current) return
        logger.error("ui", "DesignChat::reloadThreads", "Failed to list threads", e)
        setThreads([])
        setThreadsHasMore(false)
      }
    },
    [projectId],
  )

  // Append the next history page. Dedup by sessionId on merge; guarded by the
  // load version so a page resolving after a reload / search-reset is discarded.
  const loadMoreThreads = useCallback(async () => {
    if (!projectId || threadsLoadingRef.current || !threadsHasMore) return
    threadsLoadingRef.current = true
    const v = threadsLoadVersionRef.current
    try {
      const offset = threadsOffsetRef.current
      const list = await getTransport().call<DesignChatThread[]>("design_chat_threads_list_cmd", {
        projectId,
        query: threadsQueryRef.current,
        limit: THREADS_PAGE,
        offset,
      })
      if (v !== threadsLoadVersionRef.current) return
      setThreads((prev) => {
        const seen = new Set(prev.map((t) => t.sessionId))
        return [...prev, ...list.filter((t) => !seen.has(t.sessionId))]
      })
      setThreadsHasMore(list.length >= THREADS_PAGE)
      threadsOffsetRef.current = offset + list.length
    } catch (e) {
      if (v !== threadsLoadVersionRef.current) return
      logger.error("ui", "DesignChat::loadMoreThreads", "Failed to page threads", e)
    } finally {
      threadsLoadingRef.current = false
    }
  }, [projectId, threadsHasMore])

  // reloadSessions for useChatStream compat — refresh the thread list so a newly
  // auto-created session surfaces in history without a manual reload.
  const reloadSessions = useCallback(async () => {
    await reloadThreads()
  }, [reloadThreads])

  const updateSessionMessages = useCallback(
    (sessionId: string, updater: (prev: Message[]) => Message[]) => {
      if (sessionId === currentSessionIdRef.current) {
        setMessages((prev) => {
          const next = updater(prev)
          sessionCacheRef.current.set(sessionId, next)
          return next
        })
      }
    },
    [],
  )

  // Bootstrap agents + models when the panel becomes active.
  useEffect(() => {
    if (!active) return
    void loadAgents()
    void loadModels(currentAgentId)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active])

  // Default-load the current project's most recent conversation. Switching
  // projects swaps the loaded thread; a project with no prior conversation lands
  // on a draft (currentSessionId = null) that the first send auto-creates.
  useEffect(() => {
    if (!active || !projectId) return
    let cancelled = false
    void (async () => {
      try {
        const meta = await getTransport().call<SessionMeta | null>("design_chat_thread_get_cmd", {
          projectId,
        })
        if (cancelled) return
        if (meta) {
          const agentId = meta.agentId || DEFAULT_AGENT_ID
          setCurrentSessionId(meta.id)
          manualModelOverrideRef.current = null
          setCurrentAgentId(agentId)
          void loadModels(agentId)
          setSessions([meta])
          const streaming = loadingSessionsRef.current.has(meta.id)
          setLoading(streaming)
          const cached = sessionCacheRef.current.get(meta.id)
          if (streaming && cached) {
            setMessages(cached)
            setHasMore(false)
            setOldestDbId(null)
          } else {
            await loadThreadMessages(meta.id)
          }
        } else {
          setCurrentSessionId(null)
          setMessages([])
          setHasMore(false)
          setOldestDbId(null)
          setLoading(false)
        }
      } catch (e) {
        if (!cancelled) logger.error("ui", "DesignChat::defaultLoad", "Failed", e)
      }
      if (!cancelled) void reloadThreads("")
    })()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, projectId])

  const handleLoadMore = useCallback(async () => {
    const sid = currentSessionIdRef.current
    if (!sid || !hasMore || loadingMore || oldestDbId == null) return
    setLoadingMore(true)
    try {
      const [older, more] = await getTransport().call<[SessionMessage[], boolean]>(
        "load_session_messages_before_cmd",
        { sessionId: sid, beforeId: oldestDbId, limit: PAGE_SIZE },
      )
      const olderParsed = parseSessionMessages(older)
      setMessages((prev) => {
        const merged = [...olderParsed, ...prev]
        sessionCacheRef.current.set(sid, merged)
        return merged
      })
      setHasMore(more)
      setOldestDbId(older[0]?.id ?? oldestDbId)
    } catch (e) {
      logger.error("ui", "DesignChat::loadMore", "Failed", e)
    } finally {
      setLoadingMore(false)
    }
  }, [hasMore, loadingMore, oldestDbId])

  const handleModelChange = useCallback((key: string) => {
    const [providerId, modelId] = key.split("::")
    if (!providerId || !modelId) return
    const next = { providerId, modelId }
    manualModelOverrideRef.current = next
    setActiveModel(next)
  }, [])

  const handleEffortChange = useCallback((effort: string) => {
    setReasoningEffort(effort)
  }, [])

  const handleSwitchAgent = useCallback(
    (agentId: string) => {
      if (agentId === currentAgentId) return
      // An agent is baked into a session once it has messages, so switching
      // mid-conversation auto-creates a fresh draft thread (anchored to the same
      // project); the old thread stays retrievable in history.
      if (currentSessionIdRef.current) {
        setCurrentSessionId(null)
        setMessages([])
        setHasMore(false)
        setOldestDbId(null)
      }
      manualModelOverrideRef.current = null
      setCurrentAgentId(agentId)
      void loadModels(agentId)
    },
    [currentAgentId, loadModels],
  )

  const handleNewThread = useCallback(() => {
    setCurrentSessionId(null)
    setMessages([])
    setHasMore(false)
    setOldestDbId(null)
    manualModelOverrideRef.current = null
  }, [])

  const switchThread = useCallback(
    async (sessionId: string) => {
      if (sessionId === currentSessionIdRef.current) return
      const meta = threads.find((t) => t.sessionId === sessionId)
      setCurrentSessionId(sessionId)
      if (meta) {
        setSessions([{ id: meta.sessionId } as SessionMeta])
        const agentId = meta.agentId || DEFAULT_AGENT_ID
        manualModelOverrideRef.current = null
        setCurrentAgentId(agentId)
        void loadModels(agentId)
      }
      const streaming = loadingSessionsRef.current.has(sessionId)
      setLoading(streaming)
      const cached = sessionCacheRef.current.get(sessionId)
      if (streaming && cached) {
        setMessages(cached)
        setHasMore(false)
        setOldestDbId(null)
      } else {
        await loadThreadMessages(sessionId)
      }
    },
    [threads, loadThreadMessages, loadModels],
  )

  return {
    messages,
    setMessages,
    currentSessionId,
    setCurrentSessionId,
    currentSessionIdRef,
    currentAgentId,
    agents,
    loading,
    setLoading,
    setLoadingSessionIds,
    sessionCacheRef,
    loadingSessionsRef,
    sessions,
    reloadSessions,
    updateSessionMessages,
    hasMore,
    loadingMore,
    handleLoadMore,
    availableModels,
    activeModel,
    reasoningEffort,
    handleModelChange,
    handleEffortChange,
    handleSwitchAgent,
    threads,
    reloadThreads,
    threadsHasMore,
    loadMoreThreads,
    handleNewThread,
    switchThread,
    reconcileThread,
  }
}

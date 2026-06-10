import { useState, useRef, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { parseSessionMessages } from "@/components/chat/chatUtils"
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
import type { KbChatThread } from "@/types/knowledge"

const PAGE_SIZE = 30

type ModelSnapshot = {
  models: AvailableModel[]
  displayModel: ActiveModel | null
  defaultEffort: string
}

export interface UseKnowledgeChatReturn {
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

  // KB chat threads
  threads: KbChatThread[]
  reloadThreads: (query?: string) => Promise<void>
  handleNewThread: () => void
  switchThread: (sessionId: string) => Promise<void>
}

/**
 * Session manager for the knowledge-space sidebar chat. Mirrors
 * `useQuickChatSession` but threads are anchored to a (KB, note): opening a note
 * default-loads its most recent conversation, "new" clears to a draft that the
 * first send auto-creates as a knowledge thread (via the `chat` command's
 * `toolScope: "knowledge"` branch — no empty sessions), and the history picker
 * lists every thread in the KB. Streaming/sending is driven by `useChatStream`
 * in the panel; this hook only owns session lifecycle + model/agent state.
 */
export function useKnowledgeChat(
  kbId: string | null,
  notePath: string | null,
  active: boolean,
): UseKnowledgeChatReturn {
  const [messages, setMessages] = useState<Message[]>([])
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null)
  const currentSessionIdRef = useRef<string | null>(null)
  const [currentAgentId, setCurrentAgentId] = useState<string>(DEFAULT_AGENT_ID)
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [loading, setLoading] = useState(false)
  const [, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [threads, setThreads] = useState<KbChatThread[]>([])

  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const manualModelOverrideRef = useRef<ActiveModel | null>(null)

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
      logger.error("ui", "KnowledgeChat::loadAgents", "Failed to load agents", e)
      return []
    }
  }, [])

  const loadModels = useCallback(
    async (agentId: string): Promise<ModelSnapshot | null> => {
      try {
        const [models, active, settings, agentConfig] = await Promise.all([
          getTransport().call<AvailableModel[]>("get_available_models"),
          getTransport().call<ActiveModel | null>("get_active_model"),
          getTransport().call<{ reasoning_effort: string }>("get_current_settings"),
          getTransport()
            .call<AgentConfig>("get_agent_config", { id: agentId })
            .catch(() => null),
        ])
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
        logger.error("ui", "KnowledgeChat::loadModels", "Failed to load models", e)
        return null
      }
    },
    [],
  )

  const loadThreadMessages = useCallback(async (sessionId: string): Promise<boolean> => {
    try {
      const [rawMsgs, , hasMoreFromApi] = await getTransport().call<
        [SessionMessage[], number, boolean]
      >("load_session_messages_latest_cmd", { sessionId, limit: PAGE_SIZE })
      const parsed = parseSessionMessages(rawMsgs)
      setMessages(parsed)
      sessionCacheRef.current.set(sessionId, parsed)
      setHasMore(hasMoreFromApi)
      setOldestDbId(rawMsgs[0]?.id ?? null)
      setLoadingMore(false)
      return true
    } catch (e) {
      logger.error("ui", "KnowledgeChat::loadMessages", "Failed to load messages", e)
      setMessages([])
      setHasMore(false)
      setOldestDbId(null)
      return false
    }
  }, [])

  const reloadThreads = useCallback(
    async (query?: string) => {
      if (!kbId) {
        setThreads([])
        return
      }
      try {
        const list = await getTransport().call<KbChatThread[]>("kb_chat_threads_list_cmd", {
          kbId,
          query: query?.trim() || undefined,
        })
        setThreads(list)
      } catch (e) {
        logger.error("ui", "KnowledgeChat::reloadThreads", "Failed to list threads", e)
        setThreads([])
      }
    },
    [kbId],
  )

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

  // Default-load the current note's most recent conversation. Switching notes
  // swaps the loaded thread; a note with no prior conversation lands on a draft
  // (currentSessionId = null) that the first send auto-creates.
  useEffect(() => {
    if (!active || !kbId) return
    let cancelled = false
    void (async () => {
      try {
        const meta = await getTransport().call<SessionMeta | null>("kb_chat_thread_get_cmd", {
          kbId,
          note: notePath || undefined,
        })
        if (cancelled) return
        if (meta) {
          setCurrentSessionId(meta.id)
          setCurrentAgentId(meta.agentId || DEFAULT_AGENT_ID)
          setSessions([meta])
          await loadThreadMessages(meta.id)
        } else {
          setCurrentSessionId(null)
          setMessages([])
          setHasMore(false)
          setOldestDbId(null)
        }
      } catch (e) {
        if (!cancelled) logger.error("ui", "KnowledgeChat::defaultLoad", "Failed", e)
      }
      if (!cancelled) void reloadThreads()
    })()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, kbId, notePath])

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
      logger.error("ui", "KnowledgeChat::loadMore", "Failed", e)
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
      // An agent is baked into a session's prompt + history once it has
      // messages, so switching mid-conversation auto-creates a fresh draft
      // thread (anchored to the same note); the old thread stays retrievable
      // in history. On a blank draft we just swap the pending agent in place.
      if (currentSessionIdRef.current) {
        setCurrentSessionId(null)
        setMessages([])
        setHasMore(false)
        setOldestDbId(null)
      }
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
  }, [])

  const switchThread = useCallback(
    async (sessionId: string) => {
      if (sessionId === currentSessionIdRef.current) return
      const meta = threads.find((t) => t.sessionId === sessionId)
      setCurrentSessionId(sessionId)
      if (meta) setSessions([{ id: meta.sessionId } as SessionMeta])
      await loadThreadMessages(sessionId)
    },
    [threads, loadThreadMessages],
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
    handleNewThread,
    switchThread,
  }
}

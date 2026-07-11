import { useState, useRef, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { parseSessionMessages } from "./chatUtils"
import type {
  Message,
  AvailableModel,
  ActiveModel,
  SessionMeta,
  SessionMessage,
  AgentSummaryForSidebar,
  ChatRuntimeDefaults,
} from "@/types/chat"
import { normalizeEffortForModel } from "@/types/chat"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import { toast } from "sonner"
import { useTranslation } from "react-i18next"

const STORAGE_PREFIX = "quickchat:lastSession:"
const QUICK_CHAT_PAGE_SIZE = 20

type QuickModelSnapshot = {
  models: AvailableModel[]
}

function getLastSessionId(agentId: string): string | null {
  try {
    return localStorage.getItem(STORAGE_PREFIX + agentId)
  } catch {
    return null
  }
}

function setLastSessionId(agentId: string, sessionId: string) {
  try {
    localStorage.setItem(STORAGE_PREFIX + agentId, sessionId)
  } catch {
    // localStorage might not be available
  }
}

export interface UseQuickChatSessionReturn {
  // State
  messages: Message[]
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  currentSessionId: string | null
  setCurrentSessionId: React.Dispatch<React.SetStateAction<string | null>>
  currentSessionIdRef: React.MutableRefObject<string | null>
  currentAgentId: string
  agentName: string
  agents: AgentSummaryForSidebar[]
  loading: boolean
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionIds: Set<string>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>

  // Refs for useChatStream compatibility
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  manualModelOverrideRef: React.MutableRefObject<ActiveModel | null>

  // Draft-state incognito flag (only meaningful when `currentSessionId` is
  // null — once a session materializes, sessions[].incognito is the truth).
  // Mirrors `ChatScreen.draftIncognito` so the IncognitoToggle in the quick
  // chat header / dialog header behaves identically to the main chat.
  draftIncognito: boolean
  setDraftIncognito: React.Dispatch<React.SetStateAction<boolean>>

  // Pagination
  hasMore: boolean
  loadingMore: boolean
  handleLoadMore: () => Promise<void>

  // Model state
  availableModels: AvailableModel[]
  activeModel: ActiveModel | null
  reasoningEffort: string
  setReasoningEffort: React.Dispatch<React.SetStateAction<string>>
  sessionTemperature: number | null
  unavailableModelPreference: string | null

  // Handlers
  handleNewChat: () => Promise<void>
  handleSwitchAgent: (agentId: string) => Promise<void>
  handleModelChange: (key: string, options?: { applyToAgentDefault?: boolean }) => Promise<void>
  handleEffortChange: (effort: string, options?: { applyToAgentDefault?: boolean }) => Promise<void>
  resetEffort: () => Promise<void>
  handleTemperatureChange: (
    temperature: number | null,
    options?: { applyToAgentDefault?: boolean },
  ) => Promise<void>
  reloadSessions: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  initSession: () => Promise<void>
  sessions: SessionMeta[]
}

export function useQuickChatSession(open: boolean): UseQuickChatSessionReturn {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  const [currentSessionId, setCurrentSessionIdState] = useState<string | null>(null)
  const currentSessionIdRef = useRef<string | null>(null)
  const setCurrentSessionId = useCallback<React.Dispatch<React.SetStateAction<string | null>>>(
    (next) => {
      const resolved = typeof next === "function" ? next(currentSessionIdRef.current) : next
      currentSessionIdRef.current = resolved
      setCurrentSessionIdState(resolved)
    },
    [],
  )
  const [currentAgentId, setCurrentAgentId] = useState<string>(DEFAULT_AGENT_ID)
  const [agentName, setAgentName] = useState("")
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [loading, setLoading] = useState(false)
  const [loadingSessionIds, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const [sessions, setSessions] = useState<SessionMeta[]>([])

  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const manualModelOverrideRef = useRef<ActiveModel | null>(null)

  const [draftIncognito, setDraftIncognito] = useState(false)

  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [oldestDbId, setOldestDbId] = useState<number | null>(null)
  const resetPagination = useCallback(() => {
    setHasMore(false)
    setOldestDbId(null)
    setLoadingMore(false)
  }, [])

  // Model state
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null)
  const [reasoningEffort, setReasoningEffort] = useState("medium")
  const [sessionTemperature, setSessionTemperature] = useState<number | null>(null)
  const [unavailableModelPreference, setUnavailableModelPreference] = useState<string | null>(null)

  // Load agents list
  const loadAgents = useCallback(async () => {
    try {
      const list = await getTransport().call<AgentSummaryForSidebar[]>("list_agents")
      setAgents(list)
      return list
    } catch (e) {
      logger.error("ui", "QuickChat::loadAgents", "Failed to load agents", e)
      return []
    }
  }, [])

  // Load models and settings
  const loadModels = useCallback(
    async (
      agentId = currentAgentId,
      sessionId?: string | null,
    ): Promise<QuickModelSnapshot | null> => {
      try {
        const [models, runtimeDefaults] = await Promise.all([
          getTransport().call<AvailableModel[]>("get_available_models"),
          getTransport().call<ChatRuntimeDefaults>("get_chat_runtime_defaults", {
            ...(sessionId ? { sessionId } : {}),
            agentId,
          }),
        ])
        setAvailableModels(models)
        const manualOverride = manualModelOverrideRef.current
        const manualModel = manualOverride
          ? models.find(
              (m) =>
                m.providerId === manualOverride.providerId && m.modelId === manualOverride.modelId,
            )
          : undefined
        if (manualOverride && !manualModel) {
          manualModelOverrideRef.current = null
        }
        const displayModel =
          manualModel && manualOverride
            ? manualOverride
            : (runtimeDefaults.model ?? null)
        setActiveModel(displayModel)
        const currentModel = displayModel
          ? models.find(
              (m) => m.providerId === displayModel.providerId && m.modelId === displayModel.modelId,
            )
          : undefined
        const effort = runtimeDefaults.reasoningEffort
        setReasoningEffort(normalizeEffortForModel(currentModel, effort, (key) => key))
        setSessionTemperature(runtimeDefaults.temperature ?? null)
        setUnavailableModelPreference(
          !runtimeDefaults.preferredModelAvailable && runtimeDefaults.preferredModel
            ? `${runtimeDefaults.preferredModel.providerId}::${runtimeDefaults.preferredModel.modelId}`
            : null,
        )
        return { models }
      } catch (e) {
        logger.error("ui", "QuickChat::loadModels", "Failed to load models", e)
        return null
      }
    },
    [currentAgentId],
  )

  const loadSessionMessages = useCallback(
    async (sessionId: string): Promise<boolean> => {
      try {
        const [rawMsgs, , hasMoreFromApi] = await getTransport().call<
          [SessionMessage[], number, boolean]
        >("load_session_messages_latest_cmd", {
          sessionId,
          limit: QUICK_CHAT_PAGE_SIZE,
        })
        const parsed = parseSessionMessages(rawMsgs)
        setMessages(parsed)
        sessionCacheRef.current.set(sessionId, parsed)
        setHasMore(hasMoreFromApi)
        setOldestDbId(rawMsgs[0]?.id ?? null)
        setLoadingMore(false)
        return true
      } catch (e) {
        logger.error("ui", "QuickChat::loadMessages", "Failed to load messages", e)
        setMessages([])
        resetPagination()
        return false
      }
    },
    [resetPagination],
  )

  const loadSessionsForAgent = useCallback(async (agentId: string): Promise<SessionMeta[]> => {
    try {
      const [list] = await getTransport().call<[SessionMeta[], number]>("list_sessions_cmd", {
        agentId: agentId === DEFAULT_AGENT_ID ? null : agentId,
      })
      setSessions(list)
      return list
    } catch {
      return []
    }
  }, [])

  // Reload sessions list (for useChatStream compatibility)
  const reloadSessions = useCallback(async () => {
    await loadSessionsForAgent(currentAgentId)
  }, [currentAgentId, loadSessionsForAgent])

  const applySessionRuntimeState = useCallback(
    async (session: SessionMeta, snapshot: QuickModelSnapshot | null) => {
      if (!snapshot) return
      manualModelOverrideRef.current = null
      const runtimeDefaults = await getTransport().call<ChatRuntimeDefaults>(
        "get_chat_runtime_defaults",
        { sessionId: session.id, agentId: session.agentId },
      )
      const displayModel = runtimeDefaults.model ?? null
      setActiveModel(displayModel)
      const modelInfo = displayModel
        ? snapshot.models.find(
            (m) => m.providerId === displayModel.providerId && m.modelId === displayModel.modelId,
          )
        : undefined
      const effort = runtimeDefaults.reasoningEffort
      setReasoningEffort(normalizeEffortForModel(modelInfo, effort, (key) => key))
      setSessionTemperature(runtimeDefaults.temperature ?? null)
      setUnavailableModelPreference(
        !runtimeDefaults.preferredModelAvailable && runtimeDefaults.preferredModel
          ? `${runtimeDefaults.preferredModel.providerId}::${runtimeDefaults.preferredModel.modelId}`
          : null,
      )
    },
    [],
  )

  // Update session messages helper (for useChatStream compatibility)
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

  // Initialize/restore session for current agent
  const initSession = useCallback(async () => {
    const agentList = await loadAgents()
    const modelSnapshot = await loadModels()

    // Try to find the agent name
    const agent = agentList.find((a) => a.id === currentAgentId)
    if (agent) setAgentName(agent.name)

    const lastSid = getLastSessionId(currentAgentId)
    if (lastSid && (await loadSessionMessages(lastSid))) {
      const sessionList = await loadSessionsForAgent(currentAgentId)
      const restoredSession = sessionList.find((s) => s.id === lastSid)
      if (restoredSession) {
        await applySessionRuntimeState(restoredSession, modelSnapshot)
      }
      setCurrentSessionId(lastSid)
      return
    }

    // No previous session or it was deleted — start empty (session created on first send)
    setCurrentSessionId(null)
    setMessages([])
    resetPagination()
  }, [
    applySessionRuntimeState,
    currentAgentId,
    loadAgents,
    loadModels,
    loadSessionMessages,
    loadSessionsForAgent,
    resetPagination,
    setCurrentSessionId,
  ])

  // Re-init when dialog opens
  useEffect(() => {
    if (open) {
      queueMicrotask(() => {
        initSession()
      })
    }
  }, [open, initSession])

  useEffect(() => {
    manualModelOverrideRef.current = null
  }, [currentAgentId])

  useEffect(() => {
    const offConfig = getTransport().listen("config:changed", () => {
      void loadModels(currentAgentId, currentSessionIdRef.current)
    })
    const offAgents = getTransport().listen("agents:changed", () => {
      void loadAgents()
      void loadModels(currentAgentId, currentSessionIdRef.current)
    })
    const onWindowAgentsChanged = () => {
      void loadAgents()
      void loadModels(currentAgentId, currentSessionIdRef.current)
    }
    window.addEventListener("agents-changed", onWindowAgentsChanged)
    return () => {
      offConfig()
      offAgents()
      window.removeEventListener("agents-changed", onWindowAgentsChanged)
    }
  }, [currentAgentId, loadAgents, loadModels])

  const handleNewChat = useCallback(async () => {
    manualModelOverrideRef.current = null
    setActiveModel(null)
    setCurrentSessionId(null)
    setMessages([])
    sessionCacheRef.current.clear()
    resetPagination()
    setDraftIncognito(false)
    await loadModels(currentAgentId)
  }, [currentAgentId, loadModels, resetPagination, setCurrentSessionId])

  // Switch agent
  const handleSwitchAgent = useCallback(
    async (agentId: string) => {
      // Save current session ID for current agent before switching
      if (currentSessionIdRef.current) {
        setLastSessionId(currentAgentId, currentSessionIdRef.current)
      }
      manualModelOverrideRef.current = null
      setActiveModel(null)
      setCurrentSessionId(null)

      setCurrentAgentId(agentId)
      const agent = agents.find((a) => a.id === agentId)
      if (agent) setAgentName(agent.name)

      const modelSnapshot = await loadModels(agentId)

      // Try to restore last session for new agent
      const lastSid = getLastSessionId(agentId)
      if (lastSid) {
        if (await loadSessionMessages(lastSid)) {
          const sessionList = await loadSessionsForAgent(agentId)
          const restoredSession = sessionList.find((s) => s.id === lastSid)
          if (restoredSession) {
            await applySessionRuntimeState(restoredSession, modelSnapshot)
          }
          setCurrentSessionId(lastSid)
          return
        }
      }

      // No previous session
      setMessages([])
      resetPagination()
      setDraftIncognito(false)
    },
    [
      applySessionRuntimeState,
      currentAgentId,
      agents,
      loadModels,
      loadSessionMessages,
      loadSessionsForAgent,
      resetPagination,
      setCurrentSessionId,
    ],
  )

  const handleModelChange = useCallback(
    async (key: string, options?: { applyToAgentDefault?: boolean }) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      const previousModel = activeModel
      const previousManualModel = manualModelOverrideRef.current
      setUnavailableModelPreference(null)
      const sessionId = currentSessionIdRef.current
      manualModelOverrideRef.current = sessionId ? null : { providerId, modelId }
      setActiveModel({ providerId, modelId })
      if (sessionId) {
        try {
          await getTransport().call("set_session_model", { sessionId, providerId, modelId })
        } catch (e) {
          manualModelOverrideRef.current = previousManualModel
          setActiveModel(previousModel)
          logger.error("ui", "QuickChat::modelChange", "Failed to set session model", e)
          toast.error(t("common.saveFailed", "保存失败"))
          return
        }
      }
      if (options?.applyToAgentDefault) {
        try {
          await getTransport().call("patch_agent_model_defaults", {
            id: currentAgentId,
            patch: { primaryModel: { providerId, modelId } },
          })
        } catch (e) {
          logger.error("ui", "QuickChat::modelAgentDefault", "Failed to set Agent model", e)
          toast.error(t("chat.modelPicker.agentDefaultFailed", "当前会话已更新，但 Agent 默认保存失败"))
        }
      }

      const newModel = availableModels.find(
        (m) => m.providerId === providerId && m.modelId === modelId,
      )
      if (newModel) {
        const normalized = normalizeEffortForModel(newModel, reasoningEffort, (k) => k)
        if (normalized !== reasoningEffort) {
          setReasoningEffort(normalized)
          if (sessionId) {
            setSessions((prev) =>
              prev.map((s) => (s.id === sessionId ? { ...s, reasoningEffort: normalized } : s)),
            )
          }
          if (sessionId) {
            await getTransport().call("set_session_reasoning_effort", {
              sessionId,
              mode: "value",
              value: normalized,
            })
          }
        }
      }
    },
    [activeModel, availableModels, currentAgentId, reasoningEffort, t],
  )

  // Effort change
  const handleEffortChange = useCallback(
    async (effort: string, options?: { applyToAgentDefault?: boolean }) => {
      const sessionId = currentSessionIdRef.current
      const previous = reasoningEffort
      setReasoningEffort(effort)
      if (sessionId) {
        setSessions((prev) =>
          prev.map((s) => (s.id === sessionId ? { ...s, reasoningEffort: effort } : s)),
        )
      }
      try {
        if (sessionId) {
          await getTransport().call("set_session_reasoning_effort", {
            sessionId,
            mode: "value",
            value: effort,
          })
        }
      } catch (e) {
        setReasoningEffort(previous)
        if (sessionId) {
          setSessions((prev) =>
            prev.map((s) => (s.id === sessionId ? { ...s, reasoningEffort: previous } : s)),
          )
        }
        logger.error("ui", "QuickChat::effortChange", "Failed to set effort", e)
        toast.error(t("common.saveFailed", "保存失败"))
        return
      }
      if (options?.applyToAgentDefault) {
        try {
          await getTransport().call("patch_agent_model_defaults", {
            id: currentAgentId,
            patch: { reasoningEffort: effort },
          })
        } catch (e) {
          logger.error("ui", "QuickChat::effortAgentDefault", "Failed to set Agent effort", e)
          toast.error(t("chat.modelPicker.agentDefaultFailed", "当前会话已更新，但 Agent 默认保存失败"))
        }
      }
    },
    [currentAgentId, reasoningEffort, t],
  )

  const handleTemperatureChange = useCallback(
    async (temperature: number | null, options?: { applyToAgentDefault?: boolean }) => {
      const sessionId = currentSessionIdRef.current
      const previous = sessionTemperature
      try {
        if (temperature == null && sessionId) {
          const reset = await getTransport().call<number | null>("set_session_temperature", {
            sessionId,
            mode: "agentDefault",
            value: null,
          })
          setSessionTemperature(reset)
        } else if (temperature == null) {
          setSessionTemperature(null)
        } else {
          setSessionTemperature(temperature)
          if (sessionId) {
            await getTransport().call("set_session_temperature", {
              sessionId,
              mode: "value",
              value: temperature,
            })
          }
        }
      } catch (e) {
        setSessionTemperature(previous)
        logger.error("ui", "QuickChat::temperatureChange", "Failed to set temperature", e)
        toast.error(t("common.saveFailed", "保存失败"))
        return
      }
      if (options?.applyToAgentDefault) {
        try {
          await getTransport().call("patch_agent_model_defaults", {
            id: currentAgentId,
            patch: { temperature },
          })
        } catch (e) {
          logger.error("ui", "QuickChat::temperatureAgentDefault", "Failed to set Agent temperature", e)
          toast.error(t("chat.modelPicker.agentDefaultFailed", "当前会话已更新，但 Agent 默认保存失败"))
        }
      }
    },
    [currentAgentId, sessionTemperature, t],
  )

  const resetEffort = useCallback(async () => {
    const sessionId = currentSessionIdRef.current
    if (sessionId) {
      const effort = await getTransport().call<string>("set_session_reasoning_effort", {
        sessionId,
        mode: "agentDefault",
        value: null,
      })
      setReasoningEffort(effort)
      return
    }
    const defaults = await getTransport().call<ChatRuntimeDefaults>("get_chat_runtime_defaults", {
      agentId: currentAgentId,
    })
    setReasoningEffort(defaults.reasoningEffort)
  }, [currentAgentId])

  const handleLoadMore = useCallback(async () => {
    const curSid = currentSessionIdRef.current
    if (!curSid || loadingMore || !hasMore || oldestDbId === null) return
    setLoadingMore(true)
    try {
      const [olderMsgs, hasMoreBefore] = await getTransport().call<[SessionMessage[], boolean]>(
        "load_session_messages_before_cmd",
        {
          sessionId: curSid,
          beforeId: oldestDbId,
          limit: QUICK_CHAT_PAGE_SIZE,
        },
      )
      if (olderMsgs.length === 0) {
        setHasMore(false)
        return
      }
      const olderDisplay = parseSessionMessages(olderMsgs)
      setOldestDbId(olderMsgs[0].id)
      setHasMore(hasMoreBefore)
      setMessages((prev) => {
        const merged = [...olderDisplay, ...prev]
        sessionCacheRef.current.set(curSid, merged)
        return merged
      })
    } catch (e) {
      logger.error("ui", "QuickChat::loadMore", "Failed to load older messages", e)
    } finally {
      setLoadingMore(false)
    }
  }, [loadingMore, hasMore, oldestDbId])

  // Save session ID when it changes (e.g. after first message creates a session)
  useEffect(() => {
    if (currentSessionId) {
      setLastSessionId(currentAgentId, currentSessionId)
    }
  }, [currentSessionId, currentAgentId])

  return {
    messages,
    setMessages,
    currentSessionId,
    setCurrentSessionId,
    currentSessionIdRef,
    currentAgentId,
    agentName,
    agents,
    loading,
    setLoading,
    loadingSessionIds,
    setLoadingSessionIds,
    sessionCacheRef,
    loadingSessionsRef,
    manualModelOverrideRef,
    draftIncognito,
    setDraftIncognito,
    hasMore,
    loadingMore,
    handleLoadMore,
    availableModels,
    activeModel,
    reasoningEffort,
    setReasoningEffort,
    sessionTemperature,
    unavailableModelPreference,
    handleNewChat,
    handleSwitchAgent,
    handleModelChange,
    handleEffortChange,
    resetEffort,
    handleTemperatureChange,
    reloadSessions,
    updateSessionMessages,
    initSession,
    sessions,
  }
}

import { useState, useRef, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { parseSessionMessages } from "./chatUtils"
import type {
  Message,
  AvailableModel,
  ActiveModel,
  SessionMeta,
  AgentSummaryForSidebar,
} from "@/types/chat"
import { normalizeEffortForModel } from "@/types/chat"
import type { AgentConfig } from "@/components/settings/types"

const STORAGE_PREFIX = "quickchat:lastSession:"
const QUICK_CHAT_PAGE_SIZE = 20

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

  // Model state
  availableModels: AvailableModel[]
  activeModel: ActiveModel | null
  reasoningEffort: string
  setReasoningEffort: React.Dispatch<React.SetStateAction<string>>

  // Handlers
  handleNewChat: () => Promise<void>
  handleSwitchAgent: (agentId: string) => Promise<void>
  handleModelChange: (key: string) => Promise<void>
  handleEffortChange: (effort: string) => Promise<void>
  reloadSessions: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  initSession: () => Promise<void>
  sessions: SessionMeta[]
}

export function useQuickChatSession(open: boolean): UseQuickChatSessionReturn {
  const [messages, setMessages] = useState<Message[]>([])
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null)
  const currentSessionIdRef = useRef<string | null>(null)
  const [currentAgentId, setCurrentAgentId] = useState("default")
  const [agentName, setAgentName] = useState("")
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [loading, setLoading] = useState(false)
  const [loadingSessionIds, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const [sessions, setSessions] = useState<SessionMeta[]>([])

  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const manualModelOverrideRef = useRef<ActiveModel | null>(null)

  // Model state
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null)
  const [reasoningEffort, setReasoningEffort] = useState("medium")

  // Keep ref in sync
  useEffect(() => {
    currentSessionIdRef.current = currentSessionId
  }, [currentSessionId])

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
  const loadModels = useCallback(async () => {
    try {
      const [models, active, settings, agentConfig] = await Promise.all([
        getTransport().call<AvailableModel[]>("get_available_models"),
        getTransport().call<ActiveModel | null>("get_active_model"),
        getTransport().call<{ reasoning_effort: string }>("get_current_settings"),
        getTransport()
          .call<AgentConfig>("get_agent_config", { id: currentAgentId })
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
      if (manualOverride && !manualModel) {
        manualModelOverrideRef.current = null
      }
      if (manualModel && manualOverride) {
        displayModel = manualOverride
      } else if (agentConfig?.model.primary) {
        const [providerId, modelId] = agentConfig.model.primary.split("::")
        const agentModel = models.find((m) => m.providerId === providerId && m.modelId === modelId)
        if (agentModel) {
          displayModel = { providerId, modelId }
        }
      }
      setActiveModel(displayModel)
      const currentModel = displayModel
        ? models.find(
            (m) => m.providerId === displayModel.providerId && m.modelId === displayModel.modelId,
          )
        : undefined
      setReasoningEffort(normalizeEffortForModel(currentModel, settings.reasoning_effort, (key) => key))
    } catch (e) {
      logger.error("ui", "QuickChat::loadModels", "Failed to load models", e)
    }
  }, [currentAgentId])

  // Load session messages
  const loadSessionMessages = useCallback(async (sessionId: string) => {
    try {
      const [rawMsgs] = await getTransport().call<[unknown[], number]>(
        "load_session_messages_latest_cmd",
        { sessionId, limit: QUICK_CHAT_PAGE_SIZE },
      )
      const parsed = parseSessionMessages(
        rawMsgs as import("@/types/chat").SessionMessage[],
      )
      setMessages(parsed)
      sessionCacheRef.current.set(sessionId, parsed)
    } catch (e) {
      logger.error("ui", "QuickChat::loadMessages", "Failed to load messages", e)
      setMessages([])
    }
  }, [])

  // Reload sessions list (for useChatStream compatibility)
  const reloadSessions = useCallback(async () => {
    try {
      const [list] = await getTransport().call<[SessionMeta[], number]>("list_sessions_cmd", {
        agentId: currentAgentId === "default" ? null : currentAgentId,
      })
      setSessions(list)
    } catch {
      // ignore
    }
  }, [currentAgentId])

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
    await loadModels()

    // Try to find the agent name
    const agent = agentList.find((a) => a.id === currentAgentId)
    if (agent) setAgentName(agent.name)

    // Try to restore last session
    const lastSid = getLastSessionId(currentAgentId)
    if (lastSid) {
      try {
        // Verify session still exists by loading messages
        const [rawMsgs] = await getTransport().call<[unknown[], number, boolean]>(
          "load_session_messages_latest_cmd",
          { sessionId: lastSid, limit: QUICK_CHAT_PAGE_SIZE },
        )
        const parsed = parseSessionMessages(
          rawMsgs as import("@/types/chat").SessionMessage[],
        )
        setCurrentSessionId(lastSid)
        setMessages(parsed)
        sessionCacheRef.current.set(lastSid, parsed)
        return
      } catch {
        // Session may have been deleted, create new
      }
    }

    // No previous session or it was deleted — start empty (session created on first send)
    setCurrentSessionId(null)
    setMessages([])
  }, [currentAgentId, loadAgents, loadModels])

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
      void loadModels()
    })
    const offAgents = getTransport().listen("agents:changed", () => {
      void loadAgents()
      void loadModels()
    })
    const onWindowAgentsChanged = () => {
      void loadAgents()
      void loadModels()
    }
    window.addEventListener("agents-changed", onWindowAgentsChanged)
    return () => {
      offConfig()
      offAgents()
      window.removeEventListener("agents-changed", onWindowAgentsChanged)
    }
  }, [loadAgents, loadModels])

  // Create new chat session
  const handleNewChat = useCallback(async () => {
    setCurrentSessionId(null)
    setMessages([])
    sessionCacheRef.current.clear()
  }, [])

  // Switch agent
  const handleSwitchAgent = useCallback(
    async (agentId: string) => {
      // Save current session ID for current agent before switching
      if (currentSessionIdRef.current) {
        setLastSessionId(currentAgentId, currentSessionIdRef.current)
      }

      setCurrentAgentId(agentId)
      const agent = agents.find((a) => a.id === agentId)
      if (agent) setAgentName(agent.name)

      // Try to load the agent's specific model config
      try {
        const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", { id: agentId })
        if (agentConfig.model.primary) {
          const [pId, mId] = agentConfig.model.primary.split("::")
          if (pId && mId) {
            setActiveModel({ providerId: pId, modelId: mId })
          }
        }
      } catch {
        // Fall back to global active model
      }

      // Try to restore last session for new agent
      const lastSid = getLastSessionId(agentId)
      if (lastSid) {
        try {
          await loadSessionMessages(lastSid)
          setCurrentSessionId(lastSid)
          return
        } catch {
          // Session deleted, create new
        }
      }

      // No previous session
      setCurrentSessionId(null)
      setMessages([])
    },
    [currentAgentId, agents, loadSessionMessages],
  )

  // Model change
  const handleModelChange = useCallback(
    async (key: string) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      manualModelOverrideRef.current = { providerId, modelId }
      setActiveModel({ providerId, modelId })
      const newModel = availableModels.find(
        (m) => m.providerId === providerId && m.modelId === modelId,
      )
      if (newModel) {
        setReasoningEffort((prev) => normalizeEffortForModel(newModel, prev, (k) => k))
      }
      try {
        await getTransport().call("set_active_model", { providerId, modelId })
      } catch (e) {
        logger.error("ui", "QuickChat::modelChange", "Failed to set model", e)
      }
    },
    [availableModels],
  )

  // Effort change
  const handleEffortChange = useCallback(async (effort: string) => {
    setReasoningEffort(effort)
    try {
      await getTransport().call("set_reasoning_effort", { effort })
    } catch (e) {
      logger.error("ui", "QuickChat::effortChange", "Failed to set effort", e)
    }
  }, [])

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
    availableModels,
    activeModel,
    reasoningEffort,
    setReasoningEffort,
    handleNewChat,
    handleSwitchAgent,
    handleModelChange,
    handleEffortChange,
    reloadSessions,
    updateSessionMessages,
    initSession,
    sessions,
  }
}

import { useState, useRef, useEffect, useCallback, useLayoutEffect } from "react"
import { invoke, Channel } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { Settings, Copy, Check, Info, BarChart3, AlertCircle } from "lucide-react"
import type {
  Message,
  MessageUsage,
  ContentBlock,
  ToolCall,
  AvailableModel,
  ActiveModel,
  SessionMeta,
  SessionMessage,
  AgentSummaryForSidebar,
  FallbackEvent,
} from "@/types/chat"
import { getEffortOptionsForType } from "@/types/chat"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import ApprovalDialog, { type ApprovalRequest } from "@/components/chat/ApprovalDialog"
import ToolCallBlock from "@/components/chat/ToolCallBlock"
import ThinkingBlock from "@/components/chat/ThinkingBlock"
import ChatSidebar from "@/components/chat/ChatSidebar"
import ChatInput from "@/components/chat/ChatInput"
import FallbackDetailsPopover from "@/components/chat/FallbackDetailsPopover"

/** Inline banner that mimics the original blockquote style, with a clickable ⚠️ icon for details */
function FallbackBanner({ event }: { event: FallbackEvent }) {
  const [showPopover, setShowPopover] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  // Close popover on outside click
  useEffect(() => {
    if (!showPopover) return
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setShowPopover(false)
      }
    }
    document.addEventListener("mousedown", handler)
    return () => document.removeEventListener("mousedown", handler)
  }, [showPopover])

  const from = event.from_model ? ` ← ${event.from_model}` : ""
  const attempt = event.attempt && event.total ? ` [${event.attempt}/${event.total}]` : ""

  return (
    <div className="mb-2 border-l-2 border-muted-foreground/30 pl-3 py-0.5 text-sm text-muted-foreground italic" ref={ref}>
      <span className="relative inline-block">
        <button
          onClick={() => setShowPopover((v) => !v)}
          className="not-italic cursor-pointer hover:scale-110 transition-transform inline-block"
          title="Details"
        >
          <AlertCircle className="inline h-4 w-4 text-amber-500 -mt-0.5" />
        </button>
        <FallbackDetailsPopover event={event} open={showPopover} />
      </span>
      {` Fallback: ${event.model}${from}${attempt}`}
    </div>
  )
}

/** Format token count: ≥10000 → "12.3k tokens", else "1,234 tokens" */
function formatTokens(n: number): string {
  if (n >= 10000) return `${(n / 1000).toFixed(1)}k tokens`
  return `${n.toLocaleString()} tokens`
}

interface ChatScreenProps {
  onOpenAgentSettings?: (agentId: string) => void
}

/** Format message timestamp to HH:mm */
function formatMessageTime(timestamp?: string): string {
  if (!timestamp) return ""
  try {
    const date = new Date(timestamp)
    if (isNaN(date.getTime())) return ""
    const now = new Date()
    const isToday = date.toDateString() === now.toDateString()
    const yesterday = new Date(now)
    yesterday.setDate(yesterday.getDate() - 1)
    const isYesterday = date.toDateString() === yesterday.toDateString()
    const hours = date.getHours().toString().padStart(2, "0")
    const minutes = date.getMinutes().toString().padStart(2, "0")
    const time = `${hours}:${minutes}`
    if (isToday) return time
    if (isYesterday) return `昨天 ${time}`
    const month = date.getMonth() + 1
    const day = date.getDate()
    if (date.getFullYear() === now.getFullYear()) return `${month}/${day} ${time}`
    return `${date.getFullYear()}/${month}/${day} ${time}`
  } catch {
    return ""
  }
}

/** Parse DB SessionMessage[] into display Message[] */
function parseSessionMessages(msgs: SessionMessage[]): Message[] {
  const displayMessages: Message[] = []
  const pendingTools: ToolCall[] = []
  const pendingBlocks: ContentBlock[] = []
  for (const msg of msgs) {
    if (msg.role === "user") {
      displayMessages.push({ role: "user", content: msg.content, timestamp: msg.timestamp })
    } else if (msg.role === "tool" && msg.toolCallId) {
      const tool: ToolCall = {
        callId: msg.toolCallId,
        name: msg.toolName || "",
        arguments: msg.toolArguments || "",
        result: msg.toolResult || undefined,
      }
      // Check if already exists in pendingTools (merge result)
      const existing = pendingTools.find(c => c.callId === msg.toolCallId)
      if (existing) {
        if (msg.toolResult) existing.result = msg.toolResult
        if (msg.toolName && !existing.name) existing.name = msg.toolName
        if (msg.toolArguments && !existing.arguments) existing.arguments = msg.toolArguments
        // Update matching block too
        const blockIdx = pendingBlocks.findIndex(b => b.type === "tool_call" && b.tool.callId === msg.toolCallId)
        if (blockIdx >= 0) {
          pendingBlocks[blockIdx] = { type: "tool_call", tool: { ...existing } }
        }
      } else {
        pendingTools.push(tool)
        pendingBlocks.push({ type: "tool_call", tool })
      }
    } else if (msg.role === "assistant") {
      const toolCalls = pendingTools.length > 0 ? [...pendingTools] : undefined
      // Build contentBlocks: tool_call blocks first, then text block
      const blocks: ContentBlock[] = [...pendingBlocks]
      if (msg.content) {
        blocks.push({ type: "text", content: msg.content })
      }
      pendingTools.length = 0
      pendingBlocks.length = 0
      const hasUsage = msg.toolDurationMs || msg.tokensIn || msg.tokensOut
      const usage = hasUsage ? {
        durationMs: msg.toolDurationMs || undefined,
        inputTokens: msg.tokensIn || undefined,
        outputTokens: msg.tokensOut || undefined,
      } : undefined
      displayMessages.push({
        role: "assistant",
        content: msg.content,
        contentBlocks: blocks.length > 0 ? blocks : undefined,
        toolCalls,
        timestamp: msg.timestamp,
        usage,
        model: msg.model || undefined,
      })
    } else if (msg.role === "event") {
      displayMessages.push({ role: "event", content: msg.content, timestamp: msg.timestamp })
    }
  }
  return displayMessages
}


export default function ChatScreen({ onOpenAgentSettings }: ChatScreenProps) {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState("")
  const [loading, setLoading] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null)

  // Pending message queue: when user sends while loading, message is queued here
  const [pendingMessage, setPendingMessage] = useState<string | null>(null)
  const pendingMessageRef = useRef<string | null>(null)
  // Keep ref in sync
  useEffect(() => {
    pendingMessageRef.current = pendingMessage
  }, [pendingMessage])

  // Auto-send pending messages setting (loaded from user config)
  const autoSendPendingRef = useRef(true)
  useEffect(() => {
    invoke<{ autoSendPending?: boolean }>("get_user_config").then((cfg) => {
      autoSendPendingRef.current = cfg.autoSendPending !== false
    }).catch(() => {})
  }, [])

  // Auto-send flag: when set, triggers handleSend after input state is flushed
  const autoSendRef = useRef(false)

  // Per-session message cache & loading tracking
  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const [loadingSessionIds, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const currentSessionIdRef = useRef<string | null>(null)

  // Keep ref in sync with state
  useEffect(() => {
    currentSessionIdRef.current = currentSessionId
  }, [currentSessionId])

  // Session & Agent list state
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])

  // Resizable panel
  const [panelWidth, setPanelWidth] = useState(256)

  // Current agent info
  const [agentName, setAgentName] = useState("")
  const [currentAgentId, setCurrentAgentId] = useState("default")

  // Model state
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null)
  const [reasoningEffort, setReasoningEffort] = useState("medium")

  // Command approval queue
  const [approvalRequests, setApprovalRequests] = useState<ApprovalRequest[]>([])

  // Attached files
  const [attachedFiles, setAttachedFiles] = useState<File[]>([])

  // Copied message feedback
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null)
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [hoveredMsgIndex, setHoveredMsgIndex] = useState<number | null>(null)
  // Details popover state
  const [detailsIndex, setDetailsIndex] = useState<number | null>(null)
  // Session status popover
  const [showStatus, setShowStatus] = useState(false)
  const statusRef = useRef<HTMLDivElement>(null)

  // Close status popover on outside click
  useEffect(() => {
    if (!showStatus) return
    const handler = (e: MouseEvent) => {
      if (statusRef.current && !statusRef.current.contains(e.target as Node)) {
        setShowStatus(false)
      }
    }
    document.addEventListener("mousedown", handler)
    return () => document.removeEventListener("mousedown", handler)
  }, [showStatus])

  function handleCopyMessage(content: string, index: number) {
    navigator.clipboard.writeText(content).then(() => {
      if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
      setCopiedIndex(index)
      copiedTimerRef.current = setTimeout(() => setCopiedIndex(null), 1500)
    }).catch(() => {})
  }

  /** Format duration in ms to human-readable string */
  function formatDuration(ms: number): string {
    if (ms < 1000) return `${ms}ms`
    const seconds = ms / 1000
    if (seconds < 60) return `${seconds.toFixed(1)}s`
    const minutes = Math.floor(seconds / 60)
    const remainingSeconds = Math.round(seconds % 60)
    return `${minutes}m ${remainingSeconds}s`
  }

  const scrollContainerRef = useRef<HTMLDivElement>(null)

  // --- Smooth auto-scroll during streaming ---
  const isUserScrolledUpRef = useRef(false)
  const rafIdRef = useRef<number | null>(null)
  const prevScrollHeightRef = useRef(0)

  // Delta 批量合并缓冲区：累积 text_delta / thinking_delta，每帧刷新一次
  const deltaBufferRef = useRef({ text: "", thinking: "", sid: "" })
  const deltaFlushRafRef = useRef<number | null>(null)

  // Detect user scrolling up to pause auto-scroll
  useEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return
    const handleScroll = () => {
      const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
      isUserScrolledUpRef.current = distanceFromBottom > 150
    }
    el.addEventListener("scroll", handleScroll, { passive: true })
    return () => el.removeEventListener("scroll", handleScroll)
  }, [])

  // rAF loop: smoothly follow content growth during streaming
  useEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return

    if (loading) {
      // Reset scroll-up detection when new message starts
      isUserScrolledUpRef.current = false
      prevScrollHeightRef.current = el.scrollHeight

      const tick = () => {
        if (!isUserScrolledUpRef.current) {
          // 直接设置 scrollTop，每帧跟随内容增长，避免 smooth scroll 冲突
          el.scrollTop = el.scrollHeight
        }
        rafIdRef.current = requestAnimationFrame(tick)
      }
      rafIdRef.current = requestAnimationFrame(tick)

      return () => {
        if (rafIdRef.current !== null) {
          cancelAnimationFrame(rafIdRef.current)
          rafIdRef.current = null
        }
      }
    } else {
      // Streaming ended — do a final smooth scroll to bottom
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" })
    }
  }, [loading])

  // When user sends a new message, immediately scroll to bottom
  useLayoutEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return
    // Only trigger on user messages being added
    const lastMsg = messages[messages.length - 1]
    if (lastMsg?.role === "user") {
      isUserScrolledUpRef.current = false
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" })
    }
  }, [messages.length]) // eslint-disable-line react-hooks/exhaustive-deps

  // Listen for command approval events
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<string>("approval_required", (event) => {
      try {
        const request: ApprovalRequest = JSON.parse(event.payload)
        setApprovalRequests((prev) => [...prev, request])
      } catch (e) {
        logger.error("ui", "ChatScreen::approval", "Failed to parse approval request", e)
      }
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [])

  async function handleApprovalResponse(
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) {
    setApprovalRequests((prev) =>
      prev.filter((r) => r.request_id !== requestId),
    )
    try {
      await invoke("respond_to_approval", { requestId, response })
    } catch (e) {
      logger.error("ui", "ChatScreen::approval", "Failed to respond to approval", e)
    }
  }

  // Fetch models and current settings on mount
  useEffect(() => {
    ;(async () => {
      try {
        const [models, active, settings, agentConfig] = await Promise.all([
          invoke<AvailableModel[]>("get_available_models"),
          invoke<ActiveModel | null>("get_active_model"),
          invoke<{ model: string; reasoning_effort: string }>(
            "get_current_settings",
          ),
          invoke<{ name: string; emoji?: string | null; avatar?: string | null }>("get_agent_config", { id: "default" }).catch(() => null),
        ])
        setAvailableModels(models)
        setActiveModel(active)
        setReasoningEffort(settings.reasoning_effort)
        if (agentConfig) {
          setAgentName(agentConfig.name)
        }
      } catch (e) {
        logger.error("ui", "ChatScreen::loadSettings", "Failed to load settings", e)
      }
    })()
  }, [])

  // Load session list and agent list
  const reloadSessions = useCallback(async () => {
    try {
      const list = await invoke<SessionMeta[]>("list_sessions_cmd", {})
      setSessions(list)
    } catch (e) {
      logger.error("ui", "ChatScreen::loadSessions", "Failed to load sessions", e)
    }
  }, [])

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

  /** Update messages for a specific session. If it's the current session, also update state. */
  function updateSessionMessages(sessionId: string, updater: (prev: Message[]) => Message[]) {
    const prev = sessionCacheRef.current.get(sessionId) || []
    const next = updater(prev)
    sessionCacheRef.current.set(sessionId, next)
    if (currentSessionIdRef.current === sessionId) {
      setMessages(next)
    }
  }

  // Switch to an existing session
  async function handleSwitchSession(sessionId: string) {
    if (sessionId === currentSessionId) return

    // Save current session's messages to cache
    if (currentSessionId) {
      sessionCacheRef.current.set(currentSessionId, messages)
    }

    // If target session is in cache (e.g. still loading), restore from cache
    const cached = sessionCacheRef.current.get(sessionId)
    if (cached) {
      setMessages(cached)
      setLoading(loadingSessionsRef.current.has(sessionId))
      setCurrentSessionId(sessionId)
    } else {
      // Load from DB
      try {
        const msgs = await invoke<SessionMessage[]>("load_session_messages_cmd", { sessionId })
        const displayMessages = parseSessionMessages(msgs)
        sessionCacheRef.current.set(sessionId, displayMessages)
        setMessages(displayMessages)
        setLoading(loadingSessionsRef.current.has(sessionId))
        setCurrentSessionId(sessionId)
      } catch (e) {
        logger.error("session", "ChatScreen::switchSession", "Failed to load session", { sessionId, error: e })
        return
      }
    }

    const session = sessions.find(s => s.id === sessionId)
    if (session) {
      setCurrentAgentId(session.agentId)
      const agent = agents.find(a => a.id === session.agentId)
      if (agent) setAgentName(agent.name)

      // Restore the model used in this session (if still available)
      if (session.providerId && session.modelId) {
        const modelExists = availableModels.some(
          (m) => m.providerId === session.providerId && m.modelId === session.modelId
        )
        if (modelExists) {
          handleModelChange(`${session.providerId}::${session.modelId}`)
        }
      }
    }
  }

  // Create a new chat with a specific agent
  async function handleNewChat(agentId: string) {
    // Save current session to cache
    if (currentSessionId) {
      sessionCacheRef.current.set(currentSessionId, messages)
    }

    const agent = agents.find(a => a.id === agentId)
    setMessages([])
    setCurrentSessionId(null)
    setLoading(false)
    setCurrentAgentId(agentId)
    if (agent) {
      setAgentName(agent.name)
    }
  }

  // Delete a session
  async function handleDeleteSession(sessionId: string) {
    try {
      await invoke("delete_session_cmd", { sessionId })
      sessionCacheRef.current.delete(sessionId)
      loadingSessionsRef.current.delete(sessionId)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))
      if (currentSessionId === sessionId) {
        setMessages([])
        setCurrentSessionId(null)
        setLoading(false)
      }
      reloadSessions()
    } catch (err) {
      logger.error("session", "ChatScreen::deleteSession", "Failed to delete session", err)
    }
  }

  async function handleModelChange(key: string) {
    const [providerId, modelId] = key.split("::")
    if (!providerId || !modelId) return

    setActiveModel({ providerId, modelId })
    try {
      await invoke("set_active_model", { providerId, modelId })
    } catch (e) {
      logger.error("ui", "ChatScreen::modelChange", "Failed to set model", e)
    }

    const newModel = availableModels.find(
      (m) => m.providerId === providerId && m.modelId === modelId,
    )
    if (newModel) {
      const validOptions = getEffortOptionsForType(newModel.apiType, t)
      const isValid = validOptions.some((opt) => opt.value === reasoningEffort)
      if (!isValid) {
        const fallback = validOptions.some((o) => o.value === "medium")
          ? "medium"
          : "none"
        handleEffortChange(fallback)
      }
    }
  }

  async function handleEffortChange(effort: string) {
    setReasoningEffort(effort)
    try {
      await invoke("set_reasoning_effort", { effort })
    } catch (e) {
      logger.error("ui", "ChatScreen::effortChange", "Failed to set reasoning effort", e)
    }
  }

  async function handleStop() {
    try {
      await invoke("stop_chat")
    } catch (e) {
      logger.error("ui", "ChatScreen::stop", "Failed to stop chat", e)
    }
  }

  async function handleSend() {
    if (!input.trim()) return

    // If currently loading, queue the message as pending
    if (loading) {
      setPendingMessage(input.trim())
      setInput("")
      return
    }

    const text = input.trim()
    const filesToSend = [...attachedFiles]
    setInput("")
    setAttachedFiles([])
    const now = new Date().toISOString()
    setMessages((prev) => [...prev, { role: "user", content: text, timestamp: now }])
    setLoading(true)

    // Process attached files: images → base64 data, non-images → save to disk via Rust
    const attachments: { name: string; mime_type: string; data?: string; file_path?: string }[] = []
    for (const file of filesToSend) {
      try {
        const mimeType = file.type || "application/octet-stream"
        const arrayBuffer = await file.arrayBuffer()

        if (mimeType.startsWith("image/")) {
          // Images: encode as base64 and pass directly (needed for LLM API)
          const bytes = new Uint8Array(arrayBuffer)
          // Use chunked approach to avoid stack overflow on large files
          let binary = ""
          const chunkSize = 8192
          for (let i = 0; i < bytes.length; i += chunkSize) {
            binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize))
          }
          attachments.push({
            name: file.name,
            mime_type: mimeType,
            data: btoa(binary),
          })
        } else {
          // Non-image files: save to disk via Rust backend, pass file path
          const bytes = Array.from(new Uint8Array(arrayBuffer))
          const filePath = await invoke<string>("save_attachment", {
            sessionId: currentSessionId,
            fileName: file.name,
            mimeType,
            data: bytes,
          })
          attachments.push({
            name: file.name,
            mime_type: mimeType,
            file_path: filePath,
          })
        }
      } catch (err) {
        logger.error("ui", "ChatScreen::attachment", "Failed to process attachment", { fileName: file.name, error: err })
      }
    }

    // Add empty assistant message that we'll stream into
    setMessages((prev) => [...prev, { role: "assistant", content: "", timestamp: new Date().toISOString() }])

    // Capture the session ID for this request (may be null for new chats, will be set by session_created event)
    let targetSessionId = currentSessionId

    try {
      const onEvent = new Channel<string>()
      onEvent.onmessage = (raw) => {
        try {
          const event = JSON.parse(raw)

          // Handle session_created first — sets targetSessionId for the rest of this chat
          if (event.type === "session_created" && event.session_id) {
            targetSessionId = event.session_id
            // Move cached messages from null to the new session ID
            const current = sessionCacheRef.current.get("__pending__")
            if (current) {
              sessionCacheRef.current.delete("__pending__")
              sessionCacheRef.current.set(event.session_id, current)
            }
            // Transfer loading state to the new session ID
            loadingSessionsRef.current.add(event.session_id)
            setLoadingSessionIds(new Set(loadingSessionsRef.current))
            setCurrentSessionId(event.session_id)
            // Immediately refresh session list so the new session appears in sidebar
            reloadSessions()
            return
          }

          const sid = targetSessionId || "__pending__"

          // text_delta 和 thinking_delta 累积到 buffer，rAF 批量刷新
          if (event.type === "text_delta" || event.type === "thinking_delta") {
            if (event.type === "text_delta") {
              deltaBufferRef.current.text += (event.content || "")
            } else {
              deltaBufferRef.current.thinking += (event.content || "")
            }
            deltaBufferRef.current.sid = sid
            // 调度 rAF flush（如果还没调度的话）
            if (deltaFlushRafRef.current === null) {
              deltaFlushRafRef.current = requestAnimationFrame(() => {
                deltaFlushRafRef.current = null
                const buf = deltaBufferRef.current
                const textChunk = buf.text
                const thinkingChunk = buf.thinking
                const flushSid = buf.sid
                buf.text = ""
                buf.thinking = ""
                if (!textChunk && !thinkingChunk) return
                updateSessionMessages(flushSid, (prev) => {
                  const updated = [...prev]
                  const last = updated[updated.length - 1]
                  if (!last || last.role !== "assistant") return updated
                  // Build new contentBlocks
                  const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
                  if (thinkingChunk) {
                    const lastBlock = blocks[blocks.length - 1]
                    if (lastBlock && lastBlock.type === "thinking") {
                      blocks[blocks.length - 1] = { type: "thinking", content: lastBlock.content + thinkingChunk }
                    } else {
                      blocks.push({ type: "thinking", content: thinkingChunk })
                    }
                  }
                  if (textChunk) {
                    const lastBlock = blocks[blocks.length - 1]
                    if (lastBlock && lastBlock.type === "text") {
                      blocks[blocks.length - 1] = { type: "text", content: lastBlock.content + textChunk }
                    } else {
                      blocks.push({ type: "text", content: textChunk })
                    }
                  }
                  updated[updated.length - 1] = {
                    ...last,
                    contentBlocks: blocks,
                    ...(textChunk ? { content: last.content + textChunk } : {}),
                    ...(thinkingChunk ? { thinking: (last.thinking || "") + thinkingChunk } : {}),
                  }
                  return updated
                })
              })
            }
            return
          }

          // Handle usage event — store on last assistant message (may receive multiple: tokens then duration)
          if (event.type === "usage") {
            updateSessionMessages(sid, (prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (!last || last.role !== "assistant") return updated
              const prevUsage = last.usage || {}
              const usage: MessageUsage = {
                ...prevUsage,
                ...(event.duration_ms != null ? { durationMs: event.duration_ms } : {}),
                ...(event.input_tokens != null ? { inputTokens: event.input_tokens } : {}),
                ...(event.output_tokens != null ? { outputTokens: event.output_tokens } : {}),
                ...(event.cache_creation_input_tokens != null ? { cacheCreationInputTokens: event.cache_creation_input_tokens } : {}),
                ...(event.cache_read_input_tokens != null ? { cacheReadInputTokens: event.cache_read_input_tokens } : {}),
              }
              const model = event.model ? String(event.model) : last.model
              updated[updated.length - 1] = { ...last, usage, model }
              return updated
            })
            return
          }

          updateSessionMessages(sid, (prev) => {
            const updated = [...prev]
            const last = updated[updated.length - 1]
            if (!last || last.role !== "assistant") return updated

            switch (event.type) {
              case "tool_call": {
                const calls = [...(last.toolCalls || [])]
                const newTool = {
                  callId: event.call_id,
                  name: event.name,
                  arguments: event.arguments,
                }
                calls.push(newTool)
                const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
                blocks.push({ type: "tool_call", tool: { ...newTool } })
                updated[updated.length - 1] = { ...last, toolCalls: calls, contentBlocks: blocks }
                break
              }
              case "tool_result": {
                const calls = [...(last.toolCalls || [])]
                const idx = calls.findIndex(
                  (c) => c.callId === event.call_id,
                )
                if (idx >= 0) {
                  calls[idx] = { ...calls[idx], result: event.result }
                }
                // Also update the matching tool_call block in contentBlocks
                const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
                const blockIdx = blocks.findIndex(
                  (b) => b.type === "tool_call" && b.tool.callId === event.call_id,
                )
                if (blockIdx >= 0) {
                  const block = blocks[blockIdx] as { type: "tool_call"; tool: ToolCall }
                  blocks[blockIdx] = { type: "tool_call", tool: { ...block.tool, result: event.result } }
                }
                updated[updated.length - 1] = { ...last, toolCalls: calls, contentBlocks: blocks }
                break
              }
              case "model_fallback": {
                updated[updated.length - 1] = {
                  ...last,
                  fallbackEvent: event,
                }
                break
              }
            }
            return updated
          })
        } catch {
          const sid = targetSessionId || "__pending__"
          updateSessionMessages(sid, (prev) => {
            const updated = [...prev]
            const last = updated[updated.length - 1]
            if (last && last.role === "assistant") {
              updated[updated.length - 1] = {
                ...last,
                content: last.content + raw,
              }
            }
            return updated
          })
        }
      }

      // Track loading state for this session
      // Use the up-to-date messages: old messages + new user message + empty assistant
      const freshMessages = [...messages, { role: "user" as const, content: text, timestamp: now }, { role: "assistant" as const, content: "", timestamp: new Date().toISOString() }]
      if (targetSessionId) {
        loadingSessionsRef.current.add(targetSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        sessionCacheRef.current.set(targetSessionId, freshMessages)
      } else {
        sessionCacheRef.current.set("__pending__", freshMessages)
      }

      await invoke<string>("chat", { message: text, attachments, sessionId: currentSessionId, onEvent })
    } catch (e) {
      const sid = targetSessionId || "__pending__"
      updateSessionMessages(sid, (prev) => {
        const updated = [...prev]
        const last = updated[updated.length - 1]
        if (last && last.role === "assistant" && last.content === "" && !last.toolCalls?.length) {
          updated.pop()
        }
        updated.push({ role: "event", content: `${e}` })
        return updated
      })
    } finally {
      const sid = targetSessionId || "__pending__"
      // Clean up empty assistant message if chat was stopped before any response arrived
      updateSessionMessages(sid, (prev) => {
        const updated = [...prev]
        const last = updated[updated.length - 1]
        if (
          last &&
          last.role === "assistant" &&
          !last.content &&
          !last.toolCalls?.length &&
          !last.contentBlocks?.length
        ) {
          updated.pop()
        }
        return updated
      })
      loadingSessionsRef.current.delete(sid)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))
      if (currentSessionIdRef.current === sid) {
        setLoading(false)
      }
      reloadSessions()

      // Handle pending message after loading finishes
      if (pendingMessageRef.current) {
        const pending = pendingMessageRef.current
        setPendingMessage(null)
        setInput(pending)
        if (autoSendPendingRef.current) {
          autoSendRef.current = true
        }
      }
    }
  }

  // Auto-send: fires after React flushes the input state + loading=false
  useEffect(() => {
    if (autoSendRef.current && input.trim() && !loading) {
      autoSendRef.current = false
      handleSend()
    }
  }, [input, loading]) // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <>
      {/* Sidebar: Agents + Sessions */}
      <ChatSidebar
        sessions={sessions}
        agents={agents}
        currentSessionId={currentSessionId}
        loadingSessionIds={loadingSessionIds}
        panelWidth={panelWidth}
        onPanelWidthChange={setPanelWidth}
        onSwitchSession={handleSwitchSession}
        onNewChat={handleNewChat}
        onDeleteSession={handleDeleteSession}
      />

      {/* Command Approval Dialog */}
      <ApprovalDialog
        requests={approvalRequests}
        onRespond={handleApprovalResponse}
      />

      {/* Chat Area */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Title bar */}
        <div className="h-10 flex items-end justify-between px-4 bg-background shrink-0" data-tauri-drag-region>
          <span className="text-sm font-medium text-foreground shrink-0 pb-1.5">
            {agentName || t("chat.mainAgent")}
          </span>
          <div className="flex items-end gap-1">
            {/* Session Status Button */}
            <div className="relative" ref={statusRef}>
              <button
                className={cn(
                  "pb-1.5 text-muted-foreground hover:text-foreground transition-colors",
                  showStatus && "text-foreground"
                )}
                onClick={() => setShowStatus((v) => !v)}
                title={t("chat.sessionStatus")}
              >
                <BarChart3 className="h-4 w-4" />
              </button>
              {showStatus && (
                <div
                  className="absolute top-full right-0 mt-1.5 z-50 min-w-[260px] rounded-xl border border-border bg-popover p-3.5 shadow-xl"
                  onClick={(e) => e.stopPropagation()}
                >
                  <div className="space-y-2 text-xs">
                    {/* App version */}
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">🖥️ OpenComputer</span>
                      <span className="font-medium text-foreground tabular-nums">v0.1.0</span>
                    </div>
                    <div className="border-t border-border" />
                    {/* Model + Auth */}
                    {(() => {
                      const m = activeModel
                        ? availableModels.find(
                            (x) => x.providerId === activeModel.providerId && x.modelId === activeModel.modelId
                          )
                        : null
                      const modelLabel = m
                        ? `${m.providerName}/${m.modelId}`
                        : activeModel?.modelId || "—"
                      const apiType = m?.apiType || "—"
                      const authLabel = apiType === "codex" ? "oauth" : "api-key"
                      return (
                        <>
                          <div className="flex items-start gap-2">
                            <span className="text-muted-foreground shrink-0">🧠 {t("chat.statusModel")}</span>
                            <span className="font-medium text-foreground text-right ml-auto">
                              {modelLabel}
                            </span>
                          </div>
                          <div className="flex items-center justify-between gap-2">
                            <span className="text-muted-foreground">🔑 {t("chat.statusAuth")}</span>
                            <span className="font-medium text-foreground">{authLabel}</span>
                          </div>
                        </>
                      )
                    })()}
                    {/* Context window usage */}
                    {(() => {
                      const m = activeModel
                        ? availableModels.find(
                            (x) => x.providerId === activeModel.providerId && x.modelId === activeModel.modelId
                          )
                        : null
                      if (!m) return null
                      const ctxK = Math.round(m.contextWindow / 1000)
                      // Find the latest assistant message with usage to show context consumption
                      const lastAssistantWithUsage = [...messages].reverse().find(
                        (msg) => msg.role === "assistant" && msg.usage?.inputTokens
                      )
                      const usedTokens = lastAssistantWithUsage?.usage?.inputTokens || 0
                      const usedK = Math.round(usedTokens / 1000)
                      const pct = m.contextWindow > 0 ? Math.round((usedTokens / m.contextWindow) * 100) : 0
                      return (
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-muted-foreground">📚 {t("chat.statusContext")}</span>
                          <span className="font-medium text-foreground tabular-nums">{usedK}/{ctxK}k ({pct}%)</span>
                        </div>
                      )
                    })()}
                    {/* Cache info (Anthropic) */}
                    {(() => {
                      const lastAssistantWithUsage = [...messages].reverse().find(
                        (msg) => msg.role === "assistant" && msg.usage
                      )
                      const u = lastAssistantWithUsage?.usage
                      if (!u || (u.cacheCreationInputTokens == null && u.cacheReadInputTokens == null)) return null
                      const created = u.cacheCreationInputTokens || 0
                      const read = u.cacheReadInputTokens || 0
                      return (
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-muted-foreground">🗄️ {t("chat.statusCache")}</span>
                          <span className="font-medium text-foreground tabular-nums">
                            +{created > 1000 ? `${(created / 1000).toFixed(1)}k` : created}
                            {" / "}
                            ⚡{read > 1000 ? `${(read / 1000).toFixed(1)}k` : read}
                          </span>
                        </div>
                      )
                    })()}
                    <div className="border-t border-border" />
                    {/* Agent */}
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">🤖 {t("chat.statusAgent")}</span>
                      <span className="font-medium text-foreground">{agentName || t("chat.mainAgent")}</span>
                    </div>
                    {/* Session */}
                    <div className="flex items-start gap-2">
                      <span className="text-muted-foreground shrink-0">🧵 {t("chat.statusSession")}</span>
                      <span className="font-medium text-foreground text-right ml-auto truncate max-w-[160px]">
                        {currentSessionId
                          ? (() => {
                              const sess = sessions.find((s) => s.id === currentSessionId)
                              return sess?.title || currentSessionId.slice(0, 8)
                            })()
                          : t("chat.statusNewSession")}
                      </span>
                    </div>
                    {/* Message count */}
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">📊 {t("chat.statusMessages", { count: messages.length })}</span>
                    </div>
                    <div className="border-t border-border" />
                    {/* Runtime: Thinking */}
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">⚙️ {t("chat.statusThinking")}</span>
                      <span className="font-medium text-foreground">
                        {t(`effort.${reasoningEffort}`)}
                      </span>
                    </div>
                    {/* Updated */}
                    {currentSessionId && (() => {
                      const sess = sessions.find((s) => s.id === currentSessionId)
                      if (!sess) return null
                      return (
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-muted-foreground">🕒 {t("chat.statusUpdated")}</span>
                          <span className="font-medium text-foreground tabular-nums">
                            {formatMessageTime(sess.updatedAt)}
                          </span>
                        </div>
                      )
                    })()}
                  </div>
                </div>
              )}
            </div>
            {/* Settings Button */}
            {onOpenAgentSettings && (
              <button
                className="pb-1.5 text-muted-foreground hover:text-foreground transition-colors"
                onClick={() => onOpenAgentSettings(currentAgentId)}
                title={t("settings.agents")}
              >
                <Settings className="h-4 w-4" />
              </button>
            )}
          </div>
        </div>

        {/* Messages */}
        <div ref={scrollContainerRef} className="flex-1 overflow-y-auto px-4 py-6 space-y-4">
          {messages.length === 0 && (
            <div className="flex items-center justify-center h-full">
              <p className="text-muted-foreground text-sm">
                {t("chat.howCanIHelp")}
              </p>
            </div>
          )}
          {messages.map((msg, i) => (
            <div
              key={i}
              className={cn(
                "flex",
                msg.role === "event" ? "justify-center" : msg.role === "user" ? "justify-end" : "justify-start",
              )}
            >
              {msg.role === "event" ? (
                <div className="max-w-[80%] px-3 py-1.5 rounded-lg text-xs text-muted-foreground bg-muted/50 border border-border/50 text-center">
                  {msg.content}
                </div>
              ) : (
              <div
                className="relative max-w-[95%]"
                onMouseEnter={() => setHoveredMsgIndex(i)}
                onMouseLeave={() => {
                  setHoveredMsgIndex((prev) => prev === i ? null : prev)
                  setDetailsIndex((prev) => prev === i ? null : prev)
                }}
              >
                {msg.role === "assistant" && msg.fallbackEvent && (
                  <FallbackBanner event={msg.fallbackEvent} />
                )}
                <div
                  className={cn(
                    "px-4 py-2.5 rounded-xl text-sm leading-relaxed overflow-hidden break-words select-text",
                    msg.role === "user"
                      ? "bg-[var(--color-user-bubble)] text-foreground whitespace-pre-wrap"
                      : "bg-card text-foreground/80",
                    msg.role === "assistant" && !msg.content && !msg.toolCalls?.length && !msg.contentBlocks?.length && "animate-pulse",
                    msg.role === "assistant" && loading && i === messages.length - 1 && "streaming-bubble"
                  )}
                >
                  {msg.role === "assistant" && msg.contentBlocks && msg.contentBlocks.length > 0 ? (
                    // New path: render content blocks in order (thinking → tool → text)
                    msg.contentBlocks.map((block, blockIdx) => {
                      if (block.type === "thinking") {
                        // isStreaming only for the last block of the actively streaming message
                        const isLast = blockIdx === msg.contentBlocks!.length - 1
                        return (
                          <ThinkingBlock
                            key={blockIdx}
                            content={block.content}
                            isStreaming={loading && i === messages.length - 1 && isLast && !msg.content.trim()}
                          />
                        )
                      }
                      if (block.type === "tool_call") {
                        return <ToolCallBlock key={block.tool.callId} tool={block.tool} />
                      }
                      if (block.type === "text") {
                        return (
                          <MarkdownRenderer
                            key={blockIdx}
                            content={block.content}
                            isStreaming={loading && i === messages.length - 1 && blockIdx === msg.contentBlocks!.length - 1}
                          />
                        )
                      }
                      return null
                    }).concat(
                      // Show loading dots between tool rounds: last block is a completed tool_call
                      (loading && i === messages.length - 1) ? (() => {
                        const lastBlock = msg.contentBlocks![msg.contentBlocks!.length - 1]
                        const waitingForNextRound = lastBlock.type === "tool_call" && lastBlock.tool.result !== undefined
                        if (!waitingForNextRound) return null
                        return (
                          <div key="__loading__" className="flex items-center gap-1 py-1 px-2">
                            <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse" />
                            <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse [animation-delay:300ms]" />
                            <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse [animation-delay:600ms]" />
                          </div>
                        )
                      })() : null
                    )
                  ) : msg.role === "assistant" ? (
                    // Legacy fallback path for old messages without contentBlocks
                    <>
                      {msg.thinking && (
                        <ThinkingBlock
                          content={msg.thinking}
                          isStreaming={loading && i === messages.length - 1 && !msg.content}
                        />
                      )}
                      {msg.toolCalls?.map((tool) => (
                        <ToolCallBlock key={tool.callId} tool={tool} />
                      ))}
                      {msg.content ? (
                        <MarkdownRenderer
                          content={msg.content}
                          isStreaming={loading && i === messages.length - 1}
                        />
                      ) : (
                        !msg.toolCalls?.length && (
                          <div className="flex items-center gap-1.5 h-6 px-2 relative top-1">
                            <span className="block w-2 h-2 aspect-square rounded-full bg-foreground animate-bounce-pulse" />
                            <span className="block w-2 h-2 aspect-square rounded-full bg-foreground animate-bounce-pulse [animation-delay:200ms]" />
                            <span className="block w-2 h-2 aspect-square rounded-full bg-foreground animate-bounce-pulse [animation-delay:400ms]" />
                          </div>
                        )
                      )}
                    </>
                  ) : (
                    // User message content
                    msg.content
                  )}
                  {msg.timestamp && (
                    <div className={cn(
                      "mt-1 text-[10px] leading-none select-none",
                      msg.role === "user" ? "text-foreground/40 text-right" : "text-muted-foreground/60"
                    )}>
                      {formatMessageTime(msg.timestamp)}
                    </div>
                  )}
                </div>
                {/* Hover toolbar — below the bubble, always reserves space */}
                {msg.content && (
                  <div className={cn(
                    "flex items-center gap-0.5 mt-0.5 h-6",
                    msg.role === "user" ? "justify-end" : "justify-start",
                    !(hoveredMsgIndex === i || copiedIndex === i || detailsIndex === i) && "invisible"
                  )}>
                    <button
                      onClick={() => handleCopyMessage(msg.content, i)}
                      className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors"
                      title={t("chat.copy")}
                    >
                      {copiedIndex === i ? (
                        <Check className="h-3.5 w-3.5 text-green-500" />
                      ) : (
                        <Copy className="h-3.5 w-3.5" />
                      )}
                    </button>
                    {msg.role === "assistant" && (msg.usage || msg.model) && (
                      <div className="relative">
                        <button
                          onClick={() => setDetailsIndex(detailsIndex === i ? null : i)}
                          className={cn(
                            "p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors",
                            detailsIndex === i && "text-foreground bg-muted/80"
                          )}
                          title={t("chat.details")}
                        >
                          <Info className="h-3.5 w-3.5" />
                        </button>
                        {detailsIndex === i && (
                          <div
                            className="absolute bottom-full mb-1 z-50 min-w-[180px] rounded-lg border border-border bg-popover p-2.5 shadow-lg left-0"
                          >
                            <div className="space-y-1.5 text-xs">
                              {msg.model && (
                                <div className="flex items-center justify-between gap-3">
                                  <span className="text-muted-foreground">{t("chat.statusModel")}</span>
                                  <span className="font-medium text-foreground truncate max-w-[160px]" title={msg.model}>
                                    {msg.model}
                                  </span>
                                </div>
                              )}
                              {msg.model && msg.usage?.inputTokens != null && (
                                <div className="border-t border-border" />
                              )}
                              {msg.usage?.inputTokens != null && (
                                <div className="flex items-center justify-between gap-3">
                                  <span className="text-muted-foreground">{t("chat.inputTokens")}</span>
                                  <span className="font-medium text-foreground tabular-nums">
                                    {formatTokens(msg.usage.inputTokens)}
                                  </span>
                                </div>
                              )}
                              {msg.usage?.outputTokens != null && (
                                <div className="flex items-center justify-between gap-3">
                                  <span className="text-muted-foreground">{t("chat.outputTokens")}</span>
                                  <span className="font-medium text-foreground tabular-nums">
                                    {formatTokens(msg.usage.outputTokens)}
                                  </span>
                                </div>
                              )}
                              {msg.usage?.inputTokens != null && msg.usage?.outputTokens != null && (
                                <>
                                  <div className="border-t border-border" />
                                  <div className="flex items-center justify-between gap-3">
                                    <span className="text-muted-foreground">{t("chat.totalTokens")}</span>
                                    <span className="font-medium text-foreground tabular-nums">
                                      {formatTokens(msg.usage.inputTokens + msg.usage.outputTokens)}
                                    </span>
                                  </div>
                                </>
                              )}
                              {msg.usage?.durationMs != null && (
                                <div className="flex items-center justify-between gap-3">
                                  <span className="text-muted-foreground">{t("chat.duration")}</span>
                                  <span className="font-medium text-foreground tabular-nums">
                                    {formatDuration(msg.usage.durationMs)}
                                  </span>
                                </div>
                              )}
                            </div>
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                )}
              </div>
              )}
            </div>
          ))}

          <div ref={bottomRef} />
        </div>

        {/* Bottom Input Area */}
        <ChatInput
          input={input}
          onInputChange={setInput}
          onSend={handleSend}
          loading={loading}
          availableModels={availableModels}
          activeModel={activeModel}
          reasoningEffort={reasoningEffort}
          onModelChange={handleModelChange}
          onEffortChange={handleEffortChange}
          attachedFiles={attachedFiles}
          onAttachFiles={(files) => setAttachedFiles((prev) => [...prev, ...files])}
          onRemoveFile={(index) => setAttachedFiles((prev) => prev.filter((_, i) => i !== index))}
          pendingMessage={pendingMessage}
          onCancelPending={() => {
            setInput(pendingMessage || "")
            setPendingMessage(null)
          }}
          onStop={handleStop}
        />
      </div>
    </>
  )
}

import { useState, useRef, useEffect } from "react"
import { invoke, Channel } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { notify, loadNotificationConfig, isAgentNotifyEnabled } from "@/lib/notifications"
import { parseSessionMessages } from "./chatUtils"
import { PAGE_SIZE } from "./useChatSession"
import type {
  Message,
  ContentBlock,
  ActiveModel,
  SessionMessage,
  MessageUsage,
  AgentSummaryForSidebar,
  ParentAgentStreamEvent,
} from "@/types/chat"
import type { ApprovalRequest } from "@/components/chat/ApprovalDialog"

export interface UseChatStreamOptions {
  messages: Message[]
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  currentSessionId: string | null
  setCurrentSessionId: React.Dispatch<React.SetStateAction<string | null>>
  currentSessionIdRef: React.MutableRefObject<string | null>
  currentAgentId: string
  agentName: string
  loading: boolean
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  sessions: { id: string; title?: string | null }[]
  agents: AgentSummaryForSidebar[]
  activeModel: ActiveModel | null
  reloadSessions: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
}

export interface UseChatStreamReturn {
  input: string
  setInput: React.Dispatch<React.SetStateAction<string>>
  attachedFiles: File[]
  setAttachedFiles: React.Dispatch<React.SetStateAction<File[]>>
  pendingMessage: string | null
  setPendingMessage: React.Dispatch<React.SetStateAction<string | null>>
  approvalRequests: ApprovalRequest[]
  showCodexAuthExpired: boolean
  setShowCodexAuthExpired: React.Dispatch<React.SetStateAction<boolean>>
  handleSend: () => Promise<void>
  handleStop: () => Promise<void>
  handleApprovalResponse: (
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) => Promise<void>
}

export function useChatStream({
  messages,
  setMessages,
  currentSessionId,
  setCurrentSessionId,
  currentSessionIdRef,
  currentAgentId,
  agentName,
  loading,
  setLoading,
  loadingSessionsRef,
  setLoadingSessionIds,
  sessionCacheRef,
  sessions,
  agents,
  activeModel,
  reloadSessions,
  updateSessionMessages,
}: UseChatStreamOptions): UseChatStreamReturn {
  const { t } = useTranslation()
  const [input, setInput] = useState("")
  const [attachedFiles, setAttachedFiles] = useState<File[]>([])
  const [pendingMessage, setPendingMessage] = useState<string | null>(null)
  const pendingMessageRef = useRef<string | null>(null)
  const [approvalRequests, setApprovalRequests] = useState<ApprovalRequest[]>([])
  const [showCodexAuthExpired, setShowCodexAuthExpired] = useState(false)

  // Auto-send pending messages setting
  const autoSendPendingRef = useRef(true)
  const autoSendRef = useRef(false)

  // Delta batch buffer
  const deltaBufferRef = useRef({ text: "", thinking: "", sid: "" })
  const deltaFlushRafRef = useRef<number | null>(null)

  // Keep ref in sync
  useEffect(() => {
    pendingMessageRef.current = pendingMessage
  }, [pendingMessage])

  // Load config on mount
  useEffect(() => {
    invoke<{ autoSendPending?: boolean }>("get_user_config")
      .then((cfg) => {
        autoSendPendingRef.current = cfg.autoSendPending !== false
      })
      .catch(() => {})
    loadNotificationConfig().catch(() => {})
  }, [])

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

  // Listen for agent-initiated notification events
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen("agent:send_notification", (event) => {
      const { title, body } = event.payload as { title: string; body: string }
      notify(title || "OpenComputer", body)
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [])

  // Listen for backend-driven parent agent streaming (sub-agent result injection)
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<ParentAgentStreamEvent>("parent_agent_stream", (event) => {
      const payload = event.payload
      const { eventType, parentSessionId, delta } = payload
      const isCurrentSession = currentSessionIdRef.current === parentSessionId

      if (eventType === "started") {
        if (isCurrentSession) {
          setMessages((prev) => {
            const next = [
              ...prev,
              {
                role: "assistant" as const,
                content: "",
                timestamp: new Date().toISOString(),
              },
            ]
            sessionCacheRef.current.set(parentSessionId, next)
            return next
          })
        }
        setLoading(true)
        loadingSessionsRef.current.add(parentSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
      } else if (eventType === "delta" && delta && isCurrentSession) {
        try {
          const ev = JSON.parse(delta)
          const sid = parentSessionId
          if (ev.type === "text_delta" && ev.text) {
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === "assistant") {
                updated[updated.length - 1] = {
                  ...last,
                  content: last.content + ev.text,
                }
                sessionCacheRef.current.set(sid, updated)
              }
              return updated
            })
          } else if (ev.type === "tool_call") {
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === "assistant") {
                const toolCalls = [
                  ...(last.toolCalls || []),
                  {
                    callId: ev.call_id,
                    name: ev.name,
                    arguments: ev.arguments || "",
                  },
                ]
                const blocks = [...(last.contentBlocks || [])]
                if (last.content) blocks.push({ type: "text" as const, content: last.content })
                blocks.push({
                  type: "tool_call" as const,
                  tool: toolCalls[toolCalls.length - 1],
                })
                updated[updated.length - 1] = {
                  ...last,
                  toolCalls,
                  contentBlocks: blocks,
                }
                sessionCacheRef.current.set(sid, updated)
              }
              return updated
            })
          } else if (ev.type === "tool_result") {
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === "assistant" && last.toolCalls) {
                const mediaUrls: string[] | undefined = ev.media_urls?.length ? ev.media_urls : undefined
                const toolCalls = last.toolCalls.map((tc) =>
                  tc.callId === ev.call_id ? { ...tc, result: ev.result, ...(mediaUrls && { mediaUrls }) } : tc,
                )
                const blocks = (last.contentBlocks || []).map((b) =>
                  b.type === "tool_call" && b.tool?.callId === ev.call_id
                    ? { ...b, tool: { ...b.tool!, result: ev.result, ...(mediaUrls && { mediaUrls }) } }
                    : b,
                )
                updated[updated.length - 1] = {
                  ...last,
                  toolCalls,
                  contentBlocks: blocks,
                }
                sessionCacheRef.current.set(sid, updated)
              }
              return updated
            })
          } else if (ev.type === "usage") {
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === "assistant") {
                updated[updated.length - 1] = {
                  ...last,
                  usage: ev,
                  model: ev.model,
                }
                sessionCacheRef.current.set(sid, updated)
              }
              return updated
            })
          }
        } catch {
          /* ignore parse errors */
        }
      } else if (eventType === "done" || eventType === "error") {
        if (eventType === "error") {
          logger.error("subagent", "inject", "Backend parent agent injection failed", payload.error)
        }
        setLoading(false)
        loadingSessionsRef.current.delete(parentSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        reloadSessions()
        // Reload messages from DB so subagent result message renders with correct type
        if (currentSessionIdRef.current === parentSessionId) {
          invoke<[SessionMessage[], number]>("load_session_messages_latest_cmd", {
            sessionId: parentSessionId,
            limit: PAGE_SIZE,
          })
            .then(([msgs]) => {
              const displayMessages = parseSessionMessages(msgs)
              sessionCacheRef.current.set(parentSessionId, displayMessages)
              setMessages(displayMessages)
            })
            .catch(() => {})
        } else {
          // Not current session — clear cache so next visit loads fresh from DB
          sessionCacheRef.current.delete(parentSessionId)
        }
      }
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [reloadSessions]) // eslint-disable-line react-hooks/exhaustive-deps

  async function handleApprovalResponse(
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) {
    setApprovalRequests((prev) => prev.filter((r) => r.request_id !== requestId))
    try {
      await invoke("respond_to_approval", { requestId, response })
    } catch (e) {
      logger.error("ui", "ChatScreen::approval", "Failed to respond to approval", e)
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
    const attachments: {
      name: string
      mime_type: string
      data?: string
      file_path?: string
    }[] = []
    for (const file of filesToSend) {
      try {
        const mimeType = file.type || "application/octet-stream"
        const arrayBuffer = await file.arrayBuffer()

        if (mimeType.startsWith("image/")) {
          const bytes = new Uint8Array(arrayBuffer)
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
        logger.error("ui", "ChatScreen::attachment", "Failed to process attachment", {
          fileName: file.name,
          error: err,
        })
      }
    }

    // Add empty assistant message that we'll stream into
    setMessages((prev) => [
      ...prev,
      { role: "assistant", content: "", timestamp: new Date().toISOString() },
    ])

    let targetSessionId = currentSessionId

    try {
      const onEvent = new Channel<string>()
      onEvent.onmessage = (raw) => {
        try {
          const event = JSON.parse(raw)

          // Handle session_created first
          if (event.type === "session_created" && event.session_id) {
            targetSessionId = event.session_id
            const current = sessionCacheRef.current.get("__pending__")
            if (current) {
              sessionCacheRef.current.delete("__pending__")
              sessionCacheRef.current.set(event.session_id, current)
            }
            loadingSessionsRef.current.add(event.session_id)
            setLoadingSessionIds(new Set(loadingSessionsRef.current))
            setCurrentSessionId(event.session_id)
            reloadSessions()
            return
          }

          const sid = targetSessionId || "__pending__"

          // text_delta and thinking_delta: buffer and flush via rAF
          if (event.type === "text_delta" || event.type === "thinking_delta") {
            if (event.type === "text_delta") {
              deltaBufferRef.current.text += event.content || ""
            } else {
              deltaBufferRef.current.thinking += event.content || ""
            }
            deltaBufferRef.current.sid = sid
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
                  const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
                  if (thinkingChunk) {
                    const lastBlock = blocks[blocks.length - 1]
                    if (lastBlock && lastBlock.type === "thinking") {
                      blocks[blocks.length - 1] = {
                        type: "thinking",
                        content: lastBlock.content + thinkingChunk,
                      }
                    } else {
                      blocks.push({ type: "thinking", content: thinkingChunk })
                    }
                  }
                  if (textChunk) {
                    const lastBlock = blocks[blocks.length - 1]
                    if (lastBlock && lastBlock.type === "text") {
                      blocks[blocks.length - 1] = {
                        type: "text",
                        content: lastBlock.content + textChunk,
                      }
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

          // Handle usage event
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
                ...(event.cache_creation_input_tokens != null
                  ? { cacheCreationInputTokens: event.cache_creation_input_tokens }
                  : {}),
                ...(event.cache_read_input_tokens != null
                  ? { cacheReadInputTokens: event.cache_read_input_tokens }
                  : {}),
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
                updated[updated.length - 1] = {
                  ...last,
                  toolCalls: calls,
                  contentBlocks: blocks,
                }
                break
              }
              case "tool_result": {
                const mediaUrls: string[] | undefined = event.media_urls?.length ? event.media_urls : undefined
                const calls = [...(last.toolCalls || [])]
                const idx = calls.findIndex((c) => c.callId === event.call_id)
                if (idx >= 0) {
                  calls[idx] = { ...calls[idx], result: event.result, ...(mediaUrls && { mediaUrls }) }
                }
                const blocks: ContentBlock[] = [...(last.contentBlocks || [])]
                const blockIdx = blocks.findIndex(
                  (b) => b.type === "tool_call" && b.tool.callId === event.call_id,
                )
                if (blockIdx >= 0) {
                  const block = blocks[blockIdx] as {
                    type: "tool_call"
                    tool: { callId: string; name: string; arguments: string; result?: string; mediaUrls?: string[] }
                  }
                  blocks[blockIdx] = {
                    type: "tool_call",
                    tool: { ...block.tool, result: event.result, ...(mediaUrls && { mediaUrls }) },
                  }
                }
                updated[updated.length - 1] = {
                  ...last,
                  toolCalls: calls,
                  contentBlocks: blocks,
                }
                break
              }
              case "model_fallback": {
                updated[updated.length - 1] = {
                  ...last,
                  fallbackEvent: event,
                }
                break
              }
              case "codex_auth_expired": {
                setShowCodexAuthExpired(true)
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
      const freshMessages = [
        ...messages,
        { role: "user" as const, content: text, timestamp: now },
        {
          role: "assistant" as const,
          content: "",
          timestamp: new Date().toISOString(),
        },
      ]
      if (targetSessionId) {
        loadingSessionsRef.current.add(targetSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        sessionCacheRef.current.set(targetSessionId, freshMessages)
      } else {
        sessionCacheRef.current.set("__pending__", freshMessages)
      }

      const modelOverride = activeModel
        ? `${activeModel.providerId}::${activeModel.modelId}`
        : undefined
      await invoke<string>("chat", {
        message: text,
        attachments,
        sessionId: currentSessionId,
        modelOverride,
        agentId: currentAgentId,
        onEvent,
      })
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
      // Notify on error for non-current sessions
      if (targetSessionId && currentSessionIdRef.current !== targetSessionId) {
        const agent = agents.find((a) => a.id === currentAgentId)
        if (isAgentNotifyEnabled(agent?.notifyOnComplete)) {
          const sessionTitle =
            sessions.find((s) => s.id === targetSessionId)?.title || t("notification.chatError")
          notify(t("notification.chatError"), sessionTitle)
        }
      }
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
      // Notify on completion for non-current sessions
      if (targetSessionId && currentSessionIdRef.current !== targetSessionId) {
        const agent = agents.find((a) => a.id === currentAgentId)
        if (isAgentNotifyEnabled(agent?.notifyOnComplete)) {
          const sessionTitle = sessions.find((s) => s.id === targetSessionId)?.title || agentName
          notify(t("notification.chatCompleted"), sessionTitle)
        }
      }
      // Mark current session as read so unread count stays 0 for active session
      if (targetSessionId) {
        invoke("mark_session_read_cmd", { sessionId: targetSessionId }).catch(() => {})
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

  return {
    input,
    setInput,
    attachedFiles,
    setAttachedFiles,
    pendingMessage,
    setPendingMessage,
    approvalRequests,
    showCodexAuthExpired,
    setShowCodexAuthExpired,
    handleSend,
    handleStop,
    handleApprovalResponse,
  }
}

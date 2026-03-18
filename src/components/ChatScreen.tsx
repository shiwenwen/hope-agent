import { useState, useRef, useEffect, useCallback, useLayoutEffect } from "react"
import { invoke, Channel } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Settings } from "lucide-react"
import type {
  Message,
  AvailableModel,
  ActiveModel,
  SessionMeta,
  SessionMessage,
  AgentSummaryForSidebar,
} from "@/types/chat"
import { getEffortOptionsForType } from "@/types/chat"
import MarkdownRenderer from "@/components/MarkdownRenderer"
import ApprovalDialog, { type ApprovalRequest } from "@/components/ApprovalDialog"
import ToolCallBlock from "@/components/ToolCallBlock"
import ChatSidebar from "@/components/ChatSidebar"
import ChatInput from "@/components/ChatInput"

interface ChatScreenProps {
  onOpenAgentSettings?: (agentId: string) => void
}

export default function ChatScreen({ onOpenAgentSettings }: ChatScreenProps) {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState("")
  const [loading, setLoading] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null)

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

  const scrollContainerRef = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return
    if (loading) {
      el.scrollTop = el.scrollHeight
    } else {
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" })
    }
  }, [messages, loading])

  // Listen for command approval events
  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<string>("approval_required", (event) => {
      try {
        const request: ApprovalRequest = JSON.parse(event.payload)
        setApprovalRequests((prev) => [...prev, request])
      } catch (e) {
        console.error("Failed to parse approval request:", e)
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
      console.error("Failed to respond to approval:", e)
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
        console.error("Failed to load settings:", e)
      }
    })()
  }, [])

  // Load session list and agent list
  const reloadSessions = useCallback(async () => {
    try {
      const list = await invoke<SessionMeta[]>("list_sessions_cmd", {})
      setSessions(list)
    } catch (e) {
      console.error("Failed to load sessions:", e)
    }
  }, [])

  const reloadAgents = useCallback(async () => {
    try {
      const list = await invoke<AgentSummaryForSidebar[]>("list_agents")
      setAgents(list)
    } catch (e) {
      console.error("Failed to load agents:", e)
    }
  }, [])

  useEffect(() => {
    reloadSessions()
    reloadAgents()
  }, [reloadSessions, reloadAgents])

  // Switch to an existing session
  async function handleSwitchSession(sessionId: string) {
    if (sessionId === currentSessionId) return
    try {
      const msgs = await invoke<SessionMessage[]>("load_session_messages_cmd", { sessionId })
      const displayMessages: Message[] = []
      for (const msg of msgs) {
        if (msg.role === "user") {
          displayMessages.push({ role: "user", content: msg.content })
        } else if (msg.role === "assistant") {
          displayMessages.push({ role: "assistant", content: msg.content })
        }
      }
      setMessages(displayMessages)
      setCurrentSessionId(sessionId)
      const session = sessions.find(s => s.id === sessionId)
      if (session) {
        setCurrentAgentId(session.agentId)
        const agent = agents.find(a => a.id === session.agentId)
        if (agent) setAgentName(agent.name)
      }
    } catch (e) {
      console.error("Failed to load session:", e)
    }
  }

  // Create a new chat with a specific agent
  async function handleNewChat(agentId: string) {
    const agent = agents.find(a => a.id === agentId)
    setMessages([])
    setCurrentSessionId(null)
    setCurrentAgentId(agentId)
    if (agent) {
      setAgentName(agent.name)
    }
  }

  // Delete a session
  async function handleDeleteSession(sessionId: string) {
    try {
      await invoke("delete_session_cmd", { sessionId })
      if (currentSessionId === sessionId) {
        setMessages([])
        setCurrentSessionId(null)
      }
      reloadSessions()
    } catch (err) {
      console.error("Failed to delete session:", err)
    }
  }

  async function handleModelChange(key: string) {
    const [providerId, modelId] = key.split("::")
    if (!providerId || !modelId) return

    setActiveModel({ providerId, modelId })
    try {
      await invoke("set_active_model", { providerId, modelId })
    } catch (e) {
      console.error("Failed to set model:", e)
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
      console.error("Failed to set reasoning effort:", e)
    }
  }

  async function handleSend() {
    if (!input.trim() || loading) return
    const text = input.trim()
    const filesToSend = [...attachedFiles]
    setInput("")
    setAttachedFiles([])
    setMessages((prev) => [...prev, { role: "user", content: text }])
    setLoading(true)

    // Read attached files as base64
    const attachments: { name: string; mime_type: string; data: string }[] = []
    for (const file of filesToSend) {
      try {
        const arrayBuffer = await file.arrayBuffer()
        const bytes = new Uint8Array(arrayBuffer)
        let binary = ""
        for (let i = 0; i < bytes.length; i++) {
          binary += String.fromCharCode(bytes[i])
        }
        const base64 = btoa(binary)
        attachments.push({
          name: file.name,
          mime_type: file.type || "application/octet-stream",
          data: base64,
        })
      } catch (err) {
        console.error("Failed to read file:", file.name, err)
      }
    }

    // Add empty assistant message that we'll stream into
    setMessages((prev) => [...prev, { role: "assistant", content: "" }])

    try {
      const onEvent = new Channel<string>()
      onEvent.onmessage = (raw) => {
        try {
          const event = JSON.parse(raw)
          setMessages((prev) => {
            const updated = [...prev]
            const last = updated[updated.length - 1]
            if (!last || last.role !== "assistant") return updated

            switch (event.type) {
              case "text_delta": {
                updated[updated.length - 1] = {
                  ...last,
                  content: last.content + (event.content || ""),
                }
                break
              }
              case "tool_call": {
                const calls = [...(last.toolCalls || [])]
                calls.push({
                  callId: event.call_id,
                  name: event.name,
                  arguments: event.arguments,
                })
                updated[updated.length - 1] = { ...last, toolCalls: calls }
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
                updated[updated.length - 1] = { ...last, toolCalls: calls }
                break
              }
              case "session_created": {
                if (event.session_id) {
                  setCurrentSessionId(event.session_id)
                }
                break
              }
              case "model_fallback": {
                const from = event.from_model ? ` ← ${event.from_model}` : ""
                const reason = event.reason && event.reason !== "unknown" ? ` (${event.reason})` : ""
                const attempt = event.attempt && event.total ? ` [${event.attempt}/${event.total}]` : ""
                const notice = `⚠️ Fallback → ${event.model}${from}${reason}${attempt}`
                updated[updated.length - 1] = {
                  ...last,
                  content: `> _${notice}_\n\n` + last.content,
                }
                break
              }
            }
            return updated
          })
        } catch {
          setMessages((prev) => {
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

      await invoke<string>("chat", { message: text, attachments, sessionId: currentSessionId, onEvent })
    } catch (e) {
      setMessages((prev) => {
        const updated = [...prev]
        const last = updated[updated.length - 1]
        if (last && last.role === "assistant" && last.content === "") {
          updated[updated.length - 1] = {
            ...last,
            content: `Error: ${e}`,
          }
        } else {
          updated.push({ role: "assistant", content: `Error: ${e}` })
        }
        return updated
      })
    } finally {
      setLoading(false)
      reloadSessions()
    }
  }

  return (
    <>
      {/* Sidebar: Agents + Sessions */}
      <ChatSidebar
        sessions={sessions}
        agents={agents}
        currentSessionId={currentSessionId}
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
                msg.role === "user" ? "justify-end" : "justify-start",
              )}
            >
              <div
                className={cn(
                  "max-w-[70%] px-4 py-2.5 rounded-xl text-sm leading-relaxed overflow-hidden break-words select-text",
                  msg.role === "user"
                    ? "bg-secondary text-foreground whitespace-pre-wrap"
                    : "bg-card text-foreground/80",
                  msg.role === "assistant" && !msg.content && !msg.toolCalls?.length && "animate-pulse"
                )}
              >
                {msg.role === "assistant" &&
                  msg.toolCalls?.map((tool) => (
                    <ToolCallBlock key={tool.callId} tool={tool} />
                  ))}
                {msg.content ? (
                  <MarkdownRenderer
                    content={msg.content}
                    isStreaming={msg.role === "assistant" && loading && i === messages.length - 1}
                  />
                ) : (
                  msg.role === "assistant" &&
                  !msg.toolCalls?.length && (
                    <div className="flex items-center gap-1.5 h-6 px-2 relative top-1">
                      <span className="w-2 h-2 rounded-full bg-foreground animate-bounce-pulse" />
                      <span className="w-2 h-2 rounded-full bg-foreground animate-bounce-pulse [animation-delay:200ms]" />
                      <span className="w-2 h-2 rounded-full bg-foreground animate-bounce-pulse [animation-delay:400ms]" />
                    </div>
                  )
                )}
              </div>
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
        />
      </div>
    </>
  )
}

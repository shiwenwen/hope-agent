import { useState, useRef, useEffect, useCallback, useLayoutEffect } from "react"
import { invoke, Channel, convertFileSrc } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"
import {
  Send,
  Brain,
  ChevronDown,
  ChevronRight,
  Terminal,
  MessageSquare,
  Bot,
  Settings,
  Languages,
  ImagePlus,
  Paperclip,
  Puzzle,
  X,
  Sun,
  Moon,
  Monitor,
  User,
  Trash2,
  MessageSquarePlus,
} from "lucide-react"
import { useTheme } from "@/hooks/useTheme"
import ProviderSetup from "@/components/ProviderSetup"
import SettingsView from "@/components/SettingsView"
import MarkdownRenderer from "@/components/MarkdownRenderer"
import ApprovalDialog, { type ApprovalRequest } from "@/components/ApprovalDialog"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage } from "@/i18n/i18n"

// ── Icon Sidebar (shared across chat & settings) ──────────────────

function IconSidebar({
  view,
  onOpenSettings,
  onOpenChat,
  onOpenSkills,
  onOpenProfile,
}: {
  view: "chat" | "settings" | "skills" | "profile"
  onOpenSettings: () => void
  onOpenChat: () => void
  onOpenSkills: () => void
  onOpenProfile: () => void
}) {
  const { t, i18n } = useTranslation()
  const { theme, cycleTheme } = useTheme()
  const [showLangMenu, setShowLangMenu] = useState(false)

  return (
    <div className="w-[72px] shrink-0 border-r border-border bg-secondary/30 flex flex-col items-center">
      {/* Drag region for window movement — covers traffic light area */}
      <div className="w-full pt-10 flex justify-center" data-tauri-drag-region>
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            "rounded-xl h-8 w-8",
            view === "chat"
              ? "bg-primary/10 text-primary hover:bg-primary/20"
              : "text-muted-foreground hover:text-foreground"
          )}
          onClick={onOpenChat}
          title={t("chat.conversations")}
        >
          <MessageSquare className="h-4 w-4" />
        </Button>
      </div>

      {/* Skills entry */}
      <div className="w-full flex justify-center mt-1">
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            "rounded-xl h-8 w-8",
            view === "skills"
              ? "bg-primary/10 text-primary hover:bg-primary/20"
              : "text-muted-foreground hover:text-foreground"
          )}
          onClick={onOpenSkills}
          title={t("settings.skills")}
        >
          <Puzzle className="h-4 w-4" />
        </Button>
      </div>

      <div className="flex-1" />

      <div className="py-3 flex flex-col gap-2">
        {/* Profile */}
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            "rounded-xl h-8 w-8",
            view === "profile"
              ? "bg-primary/10 text-primary hover:bg-primary/20"
              : "text-muted-foreground hover:text-foreground"
          )}
          onClick={onOpenProfile}
          title={t("settings.profile")}
        >
          <User className="h-4 w-4" />
        </Button>

        {/* Theme Toggle */}
        <Button
          variant="ghost"
          size="icon"
          className="rounded-xl text-muted-foreground hover:text-foreground h-8 w-8"
          onClick={cycleTheme}
          title={`${t("theme.title")}: ${t(`theme.${theme}`)}`}
        >
          {theme === "auto" ? (
            <Monitor className="h-4 w-4" />
          ) : theme === "light" ? (
            <Sun className="h-4 w-4" />
          ) : (
            <Moon className="h-4 w-4" />
          )}
        </Button>

        {/* Language Selector */}
        <div className="relative">
          <Button
            variant="ghost"
            size="icon"
            className="rounded-xl text-muted-foreground hover:text-foreground h-8 w-8"
            onClick={() => setShowLangMenu(!showLangMenu)}
            title={t("language.title")}
          >
            <Languages className="h-4 w-4" />
          </Button>
          {showLangMenu && (
            <>
              <div className="fixed inset-0 z-40" onClick={() => setShowLangMenu(false)} />
              <div className="absolute left-12 bottom-0 z-50 bg-card border border-border rounded-lg shadow-lg py-1 min-w-[160px] max-h-[400px] overflow-y-auto">
                {/* Follow System option */}
                <button
                  className={`flex items-center gap-2.5 w-full px-3 py-1.5 text-xs transition-colors hover:bg-secondary ${
                    isFollowingSystem()
                      ? "text-primary font-medium"
                      : "text-foreground"
                  }`}
                  onClick={() => {
                    setFollowSystemLanguage()
                    setShowLangMenu(false)
                  }}
                >
                  <Monitor className="h-3.5 w-3.5 text-primary/70" />
                  <span>{t("language.system")}</span>
                  {isFollowingSystem() && (
                    <span className="ml-auto text-primary">●</span>
                  )}
                </button>
                <div className="border-t border-border/50 my-0.5" />
                {SUPPORTED_LANGUAGES.map((lang) => (
                  <button
                    key={lang.code}
                    className={`flex items-center gap-2.5 w-full px-3 py-1.5 text-xs transition-colors hover:bg-secondary ${
                      !isFollowingSystem() && (i18n.language === lang.code || (i18n.language.startsWith(lang.code + "-") && lang.code !== "zh"))
                        ? "text-primary font-medium"
                        : "text-foreground"
                    }`}
                    onClick={() => {
                      i18n.changeLanguage(lang.code)
                      setShowLangMenu(false)
                    }}
                  >
                    <span className="text-[10px] font-bold w-5 text-primary/70">{lang.shortLabel}</span>
                    <span>{lang.label}</span>
                    {!isFollowingSystem() && (i18n.language === lang.code || (i18n.language.startsWith(lang.code + "-") && lang.code !== "zh")) && (
                      <span className="ml-auto text-primary">●</span>
                    )}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>
        {/* Settings */}
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            "rounded-xl h-8 w-8",
            view === "settings"
              ? "bg-primary/10 text-primary hover:bg-primary/20"
              : "text-muted-foreground hover:text-foreground"
          )}
          onClick={onOpenSettings}
          title={t("chat.settings")}
        >
          <Settings className="h-4 w-4" />
        </Button>
      </div>
    </div>
  )
}

interface ToolCall {
  callId: string
  name: string
  arguments: string
  result?: string
}

interface Message {
  role: "user" | "assistant"
  content: string
  toolCalls?: ToolCall[]
}

interface AvailableModel {
  providerId: string
  providerName: string
  apiType: string
  modelId: string
  modelName: string
  inputTypes: string[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
}

interface ActiveModel {
  providerId: string
  modelId: string
}

function getEffortOptionsForType(apiType: string | undefined, t: (key: string) => string) {
  const off = t("effort.off")
  const on = t("effort.on")
  const low = t("effort.low")
  const medium = t("effort.medium")
  const high = t("effort.high")
  const xhigh = t("effort.xhigh")
  switch (apiType) {
    case "openai-responses":
    case "codex":
      return [
        { value: "none", label: off },
        { value: "low", label: low },
        { value: "medium", label: medium },
        { value: "high", label: high },
        { value: "xhigh", label: xhigh },
      ]
    case "anthropic":
    case "openai-chat":
      return [
        { value: "none", label: off },
        { value: "low", label: low },
        { value: "medium", label: medium },
        { value: "high", label: high },
      ]
    default:
      return [
        { value: "none", label: off },
        { value: "medium", label: on },
      ]
  }
}

// removed — merged into getEffortOptionsForType above

function ToolCallBlock({ tool }: { tool: ToolCall }) {
  const [expanded, setExpanded] = useState(false)
  const isRunning = tool.result === undefined
  const displayArgs = (() => {
    try {
      const parsed = JSON.parse(tool.arguments)
      if (tool.name === "exec") return parsed.command
      if (tool.name === "read_file" || tool.name === "list_dir")
        return parsed.path || "."
      if (tool.name === "write_file") return parsed.path
      return tool.arguments
    } catch {
      return tool.arguments
    }
  })()

  return (
    <div className="my-1.5 rounded-lg border border-border bg-secondary/50 text-xs">
      <button
        className="flex items-center gap-1.5 w-full px-2.5 py-1.5 text-left hover:bg-secondary/80 rounded-lg transition-colors"
        onClick={() => !isRunning && setExpanded(!expanded)}
      >
        {isRunning ? (
          <span className="animate-spin h-3 w-3 border border-current border-t-transparent rounded-full shrink-0" />
        ) : expanded ? (
          <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
        )}
        <Terminal className="h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="font-medium text-foreground">{tool.name}</span>
        <span className="text-muted-foreground truncate">{displayArgs}</span>
      </button>
      {expanded && tool.result && (
        <div className="px-2.5 pb-2 pt-0.5">
          <pre className="whitespace-pre-wrap text-muted-foreground bg-background rounded p-2 max-h-48 overflow-y-auto text-[11px] leading-relaxed">
            {tool.result}
          </pre>
        </div>
      )}
    </div>
  )
}

interface SessionMeta {
  id: string
  title?: string | null
  agentId: string
  providerName?: string | null
  modelId?: string | null
  createdAt: string
  updatedAt: string
  messageCount: number
}

interface SessionMessage {
  id: number
  sessionId: string
  role: string
  content: string
  timestamp: string
  attachmentsMeta?: string | null
  model?: string | null
  tokensIn?: number | null
  tokensOut?: number | null
  toolCallId?: string | null
  toolName?: string | null
  toolArguments?: string | null
  toolResult?: string | null
  toolDurationMs?: number | null
  isError?: boolean | null
}

interface AgentSummaryForSidebar {
  id: string
  name: string
  description?: string | null
  emoji?: string | null
  avatar?: string | null
}

function ChatScreen({ onOpenAgentSettings }: { onOpenAgentSettings?: () => void }) {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState("")
  const [loading, setLoading] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null)

  // Session & Agent list state
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [agentsExpanded, setAgentsExpanded] = useState(true)
  const [showNewChatMenu, setShowNewChatMenu] = useState(false)
  const newChatMenuRef = useRef<HTMLDivElement>(null)

  // Agent filter state: selected agent IDs for filtering sessions
  const [selectedAgentIds, setSelectedAgentIds] = useState<Set<string>>(new Set())

  // Filtered sessions based on selected agents
  const filteredSessions = selectedAgentIds.size === 0
    ? sessions
    : sessions.filter(s => selectedAgentIds.has(s.agentId))

  // Toggle agent selection for filtering
  const toggleAgentFilter = useCallback((agentId: string) => {
    setSelectedAgentIds(prev => {
      const next = new Set(prev)
      if (next.has(agentId)) {
        next.delete(agentId)
      } else {
        next.add(agentId)
      }
      return next
    })
  }, [])

  // Resizable agent list panel
  const [panelWidth, setPanelWidth] = useState(256)
  const isDragging = useRef(false)

  // Current agent info
  const [agentName, setAgentName] = useState("")

  // Model state (new provider-based)
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null)
  const [reasoningEffort, setReasoningEffort] = useState("medium")


  // Model selector popup state
  const [showModelMenu, setShowModelMenu] = useState(false)
  const [menuProvider, setMenuProvider] = useState<string | null>(null)
  const modelMenuRef = useRef<HTMLDivElement>(null)
  const [showThinkMenu, setShowThinkMenu] = useState(false)
  const thinkMenuRef = useRef<HTMLDivElement>(null)

  // Textarea
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  // Command approval queue
  const [approvalRequests, setApprovalRequests] = useState<ApprovalRequest[]>([])

  // Attached files (images & files)
  const [attachedFiles, setAttachedFiles] = useState<File[]>([])
  const imageInputRef = useRef<HTMLInputElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const scrollContainerRef = useRef<HTMLDivElement>(null)

  // Use useLayoutEffect to update scroll position synchronously BEFORE the browser paints.
  // This completely eliminates the 1-frame "jitter" (一抖一抖) when text expands the container.
  useLayoutEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return
    
    // Only force auto-scroll if loading (streaming) or at the bottom already
    // When loading, we snap to the bottom immediately to avoid jumping animations.
    if (loading) {
      el.scrollTop = el.scrollHeight
    } else {
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" })
    }
  }, [messages, loading])

  // Close model menu on outside click
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (modelMenuRef.current && !modelMenuRef.current.contains(e.target as Node)) {
        setShowModelMenu(false)
        setMenuProvider(null)
      }
      if (thinkMenuRef.current && !thinkMenuRef.current.contains(e.target as Node)) {
        setShowThinkMenu(false)
      }
    }
    if (showModelMenu || showThinkMenu) {
      document.addEventListener("mousedown", handleClickOutside)
      return () => document.removeEventListener("mousedown", handleClickOutside)
    }
  }, [showModelMenu, showThinkMenu])

  // Listen for command approval events from backend
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

  // Drag handler for resizable panel
  const handleDragStart = (e: React.MouseEvent) => {
    e.preventDefault()
    isDragging.current = true
    const startX = e.clientX
    const startWidth = panelWidth

    const onMouseMove = (ev: MouseEvent) => {
      if (!isDragging.current) return
      const delta = ev.clientX - startX
      const newWidth = Math.min(400, Math.max(180, startWidth + delta))
      setPanelWidth(newWidth)
    }

    const onMouseUp = () => {
      isDragging.current = false
      document.removeEventListener("mousemove", onMouseMove)
      document.removeEventListener("mouseup", onMouseUp)
      document.body.style.cursor = ""
      document.body.style.userSelect = ""
    }

    document.addEventListener("mousemove", onMouseMove)
    document.addEventListener("mouseup", onMouseUp)
    document.body.style.cursor = "col-resize"
    document.body.style.userSelect = "none"
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

  // Close new-chat menu on outside click
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (newChatMenuRef.current && !newChatMenuRef.current.contains(e.target as Node)) {
        setShowNewChatMenu(false)
      }
    }
    if (showNewChatMenu) {
      document.addEventListener("mousedown", handleClickOutside)
      return () => document.removeEventListener("mousedown", handleClickOutside)
    }
  }, [showNewChatMenu])

  // Switch to an existing session
  async function handleSwitchSession(sessionId: string) {
    if (sessionId === currentSessionId) return
    try {
      const msgs = await invoke<SessionMessage[]>("load_session_messages_cmd", { sessionId })
      // Convert SessionMessage[] to Message[] for display
      const displayMessages: Message[] = []
      for (const msg of msgs) {
        if (msg.role === "user") {
          displayMessages.push({ role: "user", content: msg.content })
        } else if (msg.role === "assistant") {
          displayMessages.push({ role: "assistant", content: msg.content })
        }
        // tool messages are shown as part of assistant's toolCalls — skip for now
      }
      setMessages(displayMessages)
      setCurrentSessionId(sessionId)
    } catch (e) {
      console.error("Failed to load session:", e)
    }
  }

  // Create a new chat with a specific agent
  async function handleNewChat(agentId: string) {
    const agent = agents.find(a => a.id === agentId)
    setMessages([])
    setCurrentSessionId(null)
    setShowNewChatMenu(false)
    if (agent) {
      setAgentName(agent.name)
    }
    // TODO: set current_agent_id via invoke
  }

  // Delete a session
  async function handleDeleteSession(sessionId: string, e: React.MouseEvent) {
    e.stopPropagation()
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

  // Helper: find agent info for a session
  const getAgentInfo = (agentId: string) => {
    return agents.find(a => a.id === agentId)
  }

  // Helper: format relative time
  const formatRelativeTime = (dateStr: string) => {
    const date = new Date(dateStr)
    const now = new Date()
    const diff = now.getTime() - date.getTime()
    const minutes = Math.floor(diff / 60000)
    if (minutes < 1) return t("chat.justNow") || "刚刚"
    if (minutes < 60) return `${minutes}m`
    const hours = Math.floor(minutes / 60)
    if (hours < 24) return `${hours}h`
    const days = Math.floor(hours / 24)
    if (days < 7) return `${days}d`
    return date.toLocaleDateString()
  }

  async function handleModelChange(key: string) {
    // key format: "providerId::modelId"
    const [providerId, modelId] = key.split("::")
    if (!providerId || !modelId) return

    setActiveModel({ providerId, modelId })
    try {
      await invoke("set_active_model", { providerId, modelId })
    } catch (e) {
      console.error("Failed to set model:", e)
    }

    // Auto-clamp effort if it's not valid for the new model's API type
    const newModel = availableModels.find(
      (m) => m.providerId === providerId && m.modelId === modelId,
    )
    if (newModel) {
      const validOptions = getEffortOptionsForType(newModel.apiType, t)
      const isValid = validOptions.some((opt) => opt.value === reasoningEffort)
      if (!isValid) {
        // Reset to "medium" if available, otherwise "none"
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


  // File attachment handlers
  const handleFileSelect = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files
    if (files) {
      setAttachedFiles((prev) => [...prev, ...Array.from(files)])
    }
    // Reset the input so the same file can be selected again
    e.target.value = ""
  }, [])

  const handleRemoveFile = useCallback((index: number) => {
    setAttachedFiles((prev) => prev.filter((_, i) => i !== index))
  }, [])

  // Paste handler for images/files
  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    const items = e.clipboardData?.items
    if (!items) return
    const files: File[] = []
    for (let i = 0; i < items.length; i++) {
      const item = items[i]
      if (item.kind === "file") {
        const file = item.getAsFile()
        if (file) files.push(file)
      }
    }
    if (files.length > 0) {
      e.preventDefault()
      setAttachedFiles((prev) => [...prev, ...files])
    }
  }, [])

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
                // Backend auto-created a session — store the ID
                if (event.session_id) {
                  setCurrentSessionId(event.session_id)
                }
                break
              }
              case "model_fallback": {
                // Insert a system notice showing fallback details
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

  // Keyboard handler: Enter to send, Shift+Enter to newline
  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.nativeEvent.isComposing || e.keyCode === 229) return
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  // Current model display info
  const currentModelInfo = availableModels.find(
    (m) =>
      m.providerId === activeModel?.providerId &&
      m.modelId === activeModel?.modelId,
  )

  return (
    <>
      {/* Sidebar: Agents + Sessions */}
      <div
        style={{ width: panelWidth }}
        className="shrink-0 border-r border-border bg-background flex flex-col"
      >
        {/* Title bar */}
        <div className="h-10 flex items-end px-4 shrink-0" data-tauri-drag-region>
          <h2 className="text-sm font-semibold text-foreground pb-1.5">{t("chat.conversations")}</h2>
          {/* New Chat button */}
          <div className="ml-auto relative" ref={newChatMenuRef}>
            <button
              className="text-muted-foreground hover:text-foreground transition-colors pb-1.5"
              onClick={() => setShowNewChatMenu(!showNewChatMenu)}
              title={t("chat.newChat") || "New Chat"}
            >
              <MessageSquarePlus className="h-4 w-4" />
            </button>
            {/* Agent selector popup */}
            {showNewChatMenu && (
              <div className="absolute right-0 top-full mt-1 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-lg z-50 min-w-[180px] p-1.5">
                {agents.map((agent) => (
                  <button
                    key={agent.id}
                    className="flex items-center gap-2 w-full px-2.5 py-1.5 text-[13px] rounded-md text-foreground/80 hover:bg-secondary/60 hover:text-foreground transition-colors"
                    onClick={() => handleNewChat(agent.id)}
                  >
                    <div className="w-5 h-5 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden">
                      {agent.avatar ? (
                        <img src={agent.avatar.startsWith("/") ? convertFileSrc(agent.avatar) : agent.avatar} className="w-full h-full object-cover" alt="" />
                      ) : agent.emoji ? (
                        <span>{agent.emoji}</span>
                      ) : (
                        <Bot className="h-3 w-3" />
                      )}
                    </div>
                    <span className="truncate">{agent.name}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>

        <div className="flex-1 overflow-y-auto">
          {/* Collapsible Agents section */}
          <div className="border-b border-border/50">
            <div className="flex items-center">
              <button
                className="flex items-center gap-1.5 flex-1 px-4 py-2 text-[11px] font-semibold text-muted-foreground uppercase tracking-wider hover:text-foreground transition-colors"
                onClick={() => setAgentsExpanded(!agentsExpanded)}
              >
                {agentsExpanded ? (
                  <ChevronDown className="h-3 w-3" />
                ) : (
                  <ChevronRight className="h-3 w-3" />
                )}
                <span>Agents</span>
                <span className="font-normal normal-case text-muted-foreground/60 ml-0.5">({agents.length})</span>
              </button>
              {/* Clear all agent filters */}
              {selectedAgentIds.size > 0 && (
                <button
                  className="mr-3 flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[10px] text-primary bg-primary/10 hover:bg-primary/20 transition-colors"
                  onClick={() => setSelectedAgentIds(new Set())}
                  title={t("chat.clearFilter") || "Clear filter"}
                >
                  <X className="h-2.5 w-2.5" />
                  <span>{selectedAgentIds.size}</span>
                </button>
              )}
            </div>
            {agentsExpanded && (
              <div className={cn("px-2 pb-2 grid gap-1", panelWidth >= 280 ? "grid-cols-2" : "grid-cols-1")}>
                {agents.map((agent) => {
                  const isSelected = selectedAgentIds.has(agent.id)
                  return (
                    <div
                      key={agent.id}
                      className={cn(
                        "flex items-center gap-2 px-2 py-1.5 rounded-lg text-xs transition-colors truncate group/agent",
                        isSelected
                          ? "bg-primary/10 ring-1 ring-primary/30"
                          : "hover:bg-secondary/60"
                      )}
                      title={agent.description || agent.name}
                    >
                      {/* Clickable area: toggle filter */}
                      <button
                        className="flex items-center gap-2 flex-1 min-w-0"
                        onClick={() => toggleAgentFilter(agent.id)}
                      >
                        <div className={cn(
                          "w-6 h-6 rounded-full flex items-center justify-center shrink-0 text-[10px] overflow-hidden",
                          isSelected ? "bg-primary/25 text-primary" : "bg-primary/15 text-primary"
                        )}>
                          {agent.avatar ? (
                            <img src={agent.avatar.startsWith("/") ? convertFileSrc(agent.avatar) : agent.avatar} className="w-full h-full object-cover" alt="" />
                          ) : agent.emoji ? (
                            <span>{agent.emoji}</span>
                          ) : (
                            <Bot className="h-3 w-3" />
                          )}
                        </div>
                        <span className={cn("truncate", isSelected ? "text-primary font-medium" : "text-foreground/80")}>
                          {agent.name}{agent.emoji ? ` ${agent.emoji}` : ""}
                        </span>
                      </button>
                      {/* New chat button */}
                      <button
                        className="shrink-0 p-0.5 rounded text-muted-foreground/0 group-hover/agent:text-muted-foreground/60 hover:!text-primary transition-colors"
                        onClick={(e) => {
                          e.stopPropagation()
                          handleNewChat(agent.id)
                        }}
                        title={t("chat.newChat") || "New Chat"}
                      >
                        <MessageSquarePlus className="h-3 w-3" />
                      </button>
                    </div>
                  )
                })}
              </div>
            )}
          </div>

          {/* Session list, ordered by updatedAt DESC (already from backend) */}
          <div className="p-2 space-y-0.5">
            {filteredSessions.length === 0 ? (
              <div className="text-center py-8">
                <MessageSquare className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
                <p className="text-xs text-muted-foreground/60">
                  {selectedAgentIds.size > 0
                    ? (t("chat.noMatchingSessions") || "No matching sessions")
                    : t("chat.startConversation")}
                </p>
              </div>
            ) : (
              filteredSessions.map((session) => {
                const agent = getAgentInfo(session.agentId)
                const isActive = session.id === currentSessionId
                return (
                  <button
                    key={session.id}
                    className={cn(
                      "flex items-center gap-2.5 w-full px-2.5 py-2 rounded-lg text-left transition-colors group",
                      isActive
                        ? "bg-secondary/70 border border-border/50"
                        : "hover:bg-secondary/40"
                    )}
                    onClick={() => handleSwitchSession(session.id)}
                  >
                    {/* Agent avatar (small) */}
                    <div className="w-7 h-7 rounded-full bg-primary/10 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden">
                      {agent?.avatar ? (
                        <img src={agent.avatar.startsWith("/") ? convertFileSrc(agent.avatar) : agent.avatar} className="w-full h-full object-cover" alt="" />
                      ) : agent?.emoji ? (
                        <span>{agent.emoji}</span>
                      ) : (
                        <Bot className="h-3.5 w-3.5" />
                      )}
                    </div>

                    {/* Title + meta */}
                    <div className="flex-1 min-w-0">
                      <div className="text-[13px] font-medium text-foreground truncate">
                        {session.title || t("chat.newChat") || "New Chat"}
                      </div>
                      <div className="text-[11px] text-muted-foreground truncate">
                        {agent?.name || session.agentId}
                        <span className="mx-1">·</span>
                        {formatRelativeTime(session.updatedAt)}
                      </div>
                    </div>

                    {/* Delete button (hover) */}
                    <button
                      className="shrink-0 text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-destructive transition-colors p-0.5"
                      onClick={(e) => handleDeleteSession(session.id, e)}
                      title="Delete"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </button>
                )
              })
            )}
          </div>
        </div>
      </div>

      {/* Drag Handle */}
      <div
        className="w-1 shrink-0 cursor-col-resize hover:bg-primary/30 active:bg-primary/50 transition-colors"
        onMouseDown={handleDragStart}
      />

      {/* Command Approval Dialog */}
      <ApprovalDialog
        requests={approvalRequests}
        onRespond={handleApprovalResponse}
      />

      {/* Column 3: Chat Area */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Drag region for window movement */}
        <div className="h-10 flex items-end justify-between px-4 bg-background shrink-0" data-tauri-drag-region>
          <span className="text-sm font-medium text-foreground shrink-0 pb-1.5">
            {agentName || t("chat.mainAgent")}
          </span>
          {onOpenAgentSettings && (
            <button
              className="pb-1.5 text-muted-foreground hover:text-foreground transition-colors"
              onClick={onOpenAgentSettings}
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

        {/* Bottom Input Area — ChatGPT-style container */}
        <div className="px-3 pb-3 pt-2">
          <div className="rounded-2xl border border-border bg-card">
            {/* Attached files preview — above textarea */}
            {attachedFiles.length > 0 && (
              <div className="flex gap-2 px-3 pt-3 pb-1 flex-wrap">
                {attachedFiles.map((file, index) => (
                  <div
                    key={`${file.name}-${index}`}
                    className="group relative flex items-center gap-1.5 bg-secondary rounded-lg px-2 py-1 text-xs text-foreground/80 border border-border/50"
                  >
                    {file.type.startsWith("image/") ? (
                      <img
                        src={URL.createObjectURL(file)}
                        alt={file.name}
                        className="h-8 w-8 rounded object-cover"
                      />
                    ) : (
                      <Paperclip className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                    )}
                    <span className="max-w-[120px] truncate">{file.name}</span>
                    <button
                      className="ml-0.5 text-muted-foreground hover:text-foreground transition-colors"
                      onClick={() => handleRemoveFile(index)}
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            )}

            {/* Textarea */}
            <Textarea
              ref={textareaRef}
              placeholder={t("chat.askAnything")}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              onPaste={handlePaste}
              rows={2}
              className="border-0 shadow-none bg-transparent px-4 pt-3 pb-1 text-sm text-foreground placeholder:text-muted-foreground focus-visible:ring-0 resize-none min-h-[52px] max-h-[200px]"
            />

            {/* Toolbar: left = attach + model + thinking | right = send */}
            <div className="flex items-center gap-1 px-2 pb-2">
              {/* Attach buttons */}
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 rounded-lg text-muted-foreground hover:text-foreground"
                onClick={() => imageInputRef.current?.click()}
                title={t("chat.attachImage")}
              >
                <ImagePlus className="h-4 w-4" />
              </Button>
              <input
                ref={imageInputRef}
                type="file"
                accept="image/*"
                multiple
                className="hidden"
                onChange={handleFileSelect}
              />
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 rounded-lg text-muted-foreground hover:text-foreground"
                onClick={() => fileInputRef.current?.click()}
                title={t("chat.attachFile")}
              >
                <Paperclip className="h-4 w-4" />
              </Button>
              <input
                ref={fileInputRef}
                type="file"
                multiple
                className="hidden"
                onChange={handleFileSelect}
              />

              {/* Model Selector — two-level popup */}
              {availableModels.length > 0 && (
                <div className="relative" ref={modelMenuRef}>
                  <button
                    onClick={() => {
                      setShowModelMenu(!showModelMenu)
                      setMenuProvider(null)
                    }}
                    className="flex items-center gap-1 bg-transparent text-muted-foreground hover:text-foreground text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary"
                  >
                    <span className="truncate">
                      {currentModelInfo
                        ? `${currentModelInfo.providerName} / ${currentModelInfo.modelName}`
                        : t("chat.selectModel")}
                    </span>
                  </button>

                  {/* Cascading menu — opens upward, submenu to the right */}
                  {showModelMenu && (
                    <div className="absolute bottom-full left-0 mb-2 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 min-w-[160px] max-w-[220px] p-1.5">
                      <div className="flex flex-col gap-0.5">
                        {Array.from(
                          new Map(
                            availableModels.map((m) => [m.providerId, m.providerName])
                          )
                        ).map(([pid, pname]) => {
                          const models = availableModels.filter((m) => m.providerId === pid)
                          const hasMultiple = models.length > 1
                          return (
                            <div key={pid} className="relative">
                              <button
                                className={cn(
                                  "w-full text-left px-2.5 py-1.5 text-[13px] rounded-md transition-all duration-150 flex items-center justify-between gap-3",
                                  menuProvider === pid 
                                    ? "bg-secondary text-foreground shadow-sm" 
                                    : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground"
                                )}
                                onMouseEnter={() => setMenuProvider(hasMultiple ? pid : null)}
                                onClick={() => {
                                  if (!hasMultiple) {
                                    handleModelChange(`${models[0].providerId}::${models[0].modelId}`)
                                    setShowModelMenu(false)
                                    setMenuProvider(null)
                                  }
                                }}
                              >
                                <span className="truncate">{pname}</span>
                                {hasMultiple && (
                                  <ChevronRight className="h-3.5 w-3.5 shrink-0 opacity-50" />
                                )}
                              </button>

                              {/* Submenu — appears to the right, anchored to bottom to grow upwards */}
                              {hasMultiple && menuProvider === pid && (
                                <div className="absolute left-full bottom-[-6px] ml-1.5 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 min-w-[160px] max-w-[260px] p-1.5">
                                  <div className="flex flex-col gap-0.5 max-h-[50vh] overflow-y-auto overscroll-contain">
                                    {models.map((m) => (
                                      <button
                                        key={m.modelId}
                                        className={cn(
                                          "w-full text-left px-2.5 py-1.5 text-[13px] rounded-md transition-all duration-150 truncate",
                                          activeModel?.providerId === m.providerId && activeModel?.modelId === m.modelId
                                            ? "bg-secondary text-foreground font-medium shadow-sm"
                                            : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground"
                                        )}
                                        onClick={() => {
                                          handleModelChange(`${m.providerId}::${m.modelId}`)
                                          setShowModelMenu(false)
                                          setMenuProvider(null)
                                        }}
                                      >
                                        {m.modelName}
                                      </button>
                                    ))}
                                  </div>
                                </div>
                              )}
                            </div>
                          )
                        })}
                      </div>
                    </div>
                  )}
                </div>
              )}

              {/* Think Mode Toggle — popup style */}
              {(currentModelInfo?.reasoning ?? true) && (
                <div className="relative" ref={thinkMenuRef}>
                  <button
                    onClick={() => setShowThinkMenu(!showThinkMenu)}
                    className="flex items-center gap-1 bg-transparent text-muted-foreground hover:text-foreground text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary"
                  >
                    <Brain className="h-3.5 w-3.5 shrink-0" />
                    <span>{getEffortOptionsForType(currentModelInfo?.apiType, t).find((o) => o.value === reasoningEffort)?.label ?? reasoningEffort}</span>
                  </button>

                  {showThinkMenu && (
                    <div className="absolute bottom-full left-0 mb-2 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 min-w-[120px] p-1.5">
                      <div className="flex flex-col gap-0.5">
                        {getEffortOptionsForType(currentModelInfo?.apiType, t).map((opt) => (
                          <button
                            key={opt.value}
                            className={cn(
                              "w-full text-left px-2.5 py-1.5 text-[13px] rounded-md transition-all duration-150",
                              reasoningEffort === opt.value
                                ? "bg-secondary text-foreground font-medium shadow-sm"
                                : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground"
                            )}
                            onClick={() => {
                              handleEffortChange(opt.value)
                              setShowThinkMenu(false)
                            }}
                          >
                            {opt.label}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              )}

              <div className="flex-1" />

              {/* Send Button */}
              <Button
                size="icon"
                className="h-8 w-8 rounded-full shrink-0"
                onClick={handleSend}
                disabled={loading || !input.trim()}
              >
                <Send className="h-4 w-4" />
              </Button>
            </div>
          </div>
        </div>
      </div>
    </>
  )
}

export default function App() {
  const [view, setView] = useState<
    "loading" | "setup" | "chat" | "settings" | "skills" | "profile" | "agents"
  >("loading")

  // Try to restore previous session on mount
  useEffect(() => {
    ;(async () => {
      try {
        const restored = await invoke<boolean>("try_restore_session")
        if (restored) {
          setView("chat")
        } else {
          // Check if there are any providers configured
          const has = await invoke<boolean>("has_providers")
          setView(has ? "chat" : "setup")
        }
      } catch (e) {
        console.error("Failed to restore session:", e)
        setView("setup")
      }
    })()
  }, [])

  async function handleCodexAuth() {
    // Start the OAuth flow (opens browser)
    await invoke("start_codex_auth")

    // Poll for auth completion
    const poll = async (): Promise<void> => {
      for (let i = 0; i < 300; i++) {
        await new Promise((r) => setTimeout(r, 1000))
        const status = await invoke<{
          authenticated: boolean
          error: string | null
        }>("check_auth_status")
        if (status.authenticated) {
          await invoke("finalize_codex_auth")
          setView("chat")
          return
        }
        if (status.error) {
          throw new Error(status.error)
        }
      }
      throw new Error("Login timed out")
    }

    await poll()
  }



  if (view === "loading") {
    return (
      <div className="flex items-center justify-center h-screen">
        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
      </div>
    )
  }

  if (view === "setup") {
    return (
      <ProviderSetup
        onComplete={() => setView("chat")}
        onCodexAuth={handleCodexAuth}
      />
    )
  }


  return (
    <div className="flex h-screen overflow-hidden bg-background">
      <IconSidebar
        view={view === "settings" ? "settings" : view === "skills" ? "skills" : view === "profile" ? "profile" : view === "agents" ? "settings" : "chat"}
        onOpenSettings={() => setView("settings")}
        onOpenChat={() => setView("chat")}
        onOpenSkills={() => setView("skills")}
        onOpenProfile={() => setView("profile")}
      />
      {view === "settings" ? (
        <SettingsView
          onBack={() => setView("chat")}
          onCodexAuth={handleCodexAuth}
          onCodexReauth={handleCodexAuth}
        />
      ) : view === "skills" ? (
        <SettingsView
          onBack={() => setView("chat")}
          onCodexAuth={handleCodexAuth}
          onCodexReauth={handleCodexAuth}
          initialSection="skills"
        />
      ) : view === "profile" ? (
        <SettingsView
          onBack={() => setView("chat")}
          onCodexAuth={handleCodexAuth}
          onCodexReauth={handleCodexAuth}
          initialSection="profile"
        />
      ) : view === "agents" ? (
        <SettingsView
          onBack={() => setView("chat")}
          onCodexAuth={handleCodexAuth}
          onCodexReauth={handleCodexAuth}
          initialSection="agents"
        />
      ) : (
        <ChatScreen onOpenAgentSettings={() => setView("agents")} />
      )}
    </div>
  )
}

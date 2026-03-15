import { useState, useRef, useEffect, useCallback, useLayoutEffect } from "react"
import { invoke, Channel } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
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
  X,
} from "lucide-react"
import ProviderSetup from "@/components/ProviderSetup"
import ProviderSettings from "@/components/ProviderSettings"
import MarkdownRenderer from "@/components/MarkdownRenderer"
import ApprovalDialog, { type ApprovalRequest } from "@/components/ApprovalDialog"
import { SUPPORTED_LANGUAGES } from "@/i18n/i18n"

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

function ChatScreen({
  onOpenSettings,
}: {
  onOpenSettings: () => void
}) {
  const { t, i18n } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState("")
  const [loading, setLoading] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)

  // Resizable agent list panel
  const [panelWidth, setPanelWidth] = useState(256)
  const isDragging = useRef(false)

  // Model state (new provider-based)
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null)
  const [reasoningEffort, setReasoningEffort] = useState("medium")
  const [showLangMenu, setShowLangMenu] = useState(false)

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
        const [models, active, settings] = await Promise.all([
          invoke<AvailableModel[]>("get_available_models"),
          invoke<ActiveModel | null>("get_active_model"),
          invoke<{ model: string; reasoning_effort: string }>(
            "get_current_settings",
          ),
        ])
        setAvailableModels(models)
        setActiveModel(active)
        setReasoningEffort(settings.reasoning_effort)
      } catch (e) {
        console.error("Failed to load settings:", e)
      }
    })()
  }, [])

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

      await invoke<string>("chat", { message: text, attachments, onEvent })
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
    }
  }

  // Keyboard handler: Enter to send, Shift+Enter to newline
  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
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
    <div className="flex h-screen overflow-hidden bg-background">
      {/* Column 1: Icon Sidebar */}
      <div className="w-14 shrink-0 border-r border-border bg-secondary/30 flex flex-col items-center">
        <div className="h-11 flex items-center justify-center border-b border-border w-full">
          <Button
            variant="ghost"
            size="icon"
            className="rounded-xl bg-primary/10 text-primary hover:bg-primary/20 h-8 w-8"
            title={t("chat.conversations")}
          >
            <MessageSquare className="h-4 w-4" />
          </Button>
        </div>

        <div className="flex-1" />

        <div className="py-3 flex flex-col gap-2">
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
                  {SUPPORTED_LANGUAGES.map((lang) => (
                    <button
                      key={lang.code}
                      className={`flex items-center gap-2.5 w-full px-3 py-1.5 text-xs transition-colors hover:bg-secondary ${
                        i18n.language === lang.code || i18n.language.startsWith(lang.code + "-") && lang.code !== "zh"
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
                      {(i18n.language === lang.code || (i18n.language.startsWith(lang.code + "-") && lang.code !== "zh")) && (
                        <span className="ml-auto text-primary">●</span>
                      )}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>
          <Button
            variant="ghost"
            size="icon"
            className="rounded-xl text-muted-foreground hover:text-foreground h-8 w-8"
            onClick={onOpenSettings}
            title={t("chat.settings")}
          >
            <Settings className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {/* Column 2: Agent List */}
      <div
        style={{ width: panelWidth }}
        className="shrink-0 border-r border-border bg-background flex flex-col"
      >
        <div className="h-11 flex items-center px-4 border-b border-border">
          <h2 className="text-sm font-semibold text-foreground">{t("chat.conversations")}</h2>
        </div>
        <div className="flex-1 overflow-y-auto p-2">
          {/* Main Agent — active */}
          <div className="flex items-center gap-3 px-3 py-2.5 rounded-lg bg-secondary/60 cursor-pointer border border-border/50 transition-colors">
            <div className="w-9 h-9 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0">
              <Bot className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="font-medium text-sm text-foreground truncate">
                Main Agent
              </div>
              <div className="text-xs text-muted-foreground truncate">
                {messages.length > 0
                  ? messages[messages.length - 1].content.slice(0, 30) ||
                    t("chat.toolCalling")
                  : t("chat.startConversation")}
              </div>
            </div>
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
        {/* Header — simplified, only agent name */}
        <div className="h-11 flex items-center justify-between px-4 border-b border-border bg-background gap-2">
          <div className="flex items-center gap-2 min-w-0">
            <span className="text-sm font-medium text-foreground shrink-0">
              Main Agent
            </span>
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
                msg.role === "user" ? "justify-end" : "justify-start",
              )}
            >
              <div
                className={cn(
                  "max-w-[70%] px-4 py-2.5 rounded-xl text-sm leading-relaxed overflow-hidden break-words",
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
                      <style>{`
                        @keyframes customBouncePulse {
                          0%, 100% { transform: translateY(0) scale(1); opacity: 0.3; }
                          50% { transform: translateY(-6px) scale(1.1); opacity: 1; }
                        }
                      `}</style>
                      <span className="w-2 h-2 rounded-full bg-foreground" style={{ animation: "customBouncePulse 1.2s cubic-bezier(0.4, 0, 0.6, 1) infinite", animationDelay: "0ms" }} />
                      <span className="w-2 h-2 rounded-full bg-foreground" style={{ animation: "customBouncePulse 1.2s cubic-bezier(0.4, 0, 0.6, 1) infinite", animationDelay: "200ms" }} />
                      <span className="w-2 h-2 rounded-full bg-foreground" style={{ animation: "customBouncePulse 1.2s cubic-bezier(0.4, 0, 0.6, 1) infinite", animationDelay: "400ms" }} />
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
            <textarea
              ref={textareaRef}
              placeholder={t("chat.askAnything")}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              onPaste={handlePaste}
              rows={2}
              className="w-full bg-transparent px-4 pt-3 pb-1 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none resize-none min-h-[52px] max-h-[200px]"
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
    </div>
  )
}

export default function App() {
  const [view, setView] = useState<
    "loading" | "setup" | "chat" | "settings" | "add-provider"
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

  if (view === "add-provider") {
    return (
      <ProviderSetup
        onComplete={() => setView("settings")}
        onCodexAuth={handleCodexAuth}
        onCancel={() => setView("settings")}
      />
    )
  }

  if (view === "settings") {
    return (
      <ProviderSettings
        onBack={() => setView("chat")}
        onAddProvider={() => setView("add-provider")}
        onCodexReauth={handleCodexAuth}
      />
    )
  }

  return (
    <ChatScreen
      onOpenSettings={() => setView("settings")}
    />
  )
}

import { useState, useRef, useEffect } from "react"
import { invoke, Channel } from "@tauri-apps/api/core"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import { Send, LogOut, Brain, ChevronDown, ChevronRight, Terminal, MessageSquare, Bot } from "lucide-react"

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

type AuthMethod = "none" | "api-key" | "codex-oauth"

interface CodexModel {
  id: string
  name: string
}

interface CurrentSettings {
  model: string
  reasoning_effort: string
}

const EFFORT_OPTIONS = [
  { value: "none", label: "关闭" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "XHigh" },
] as const

function SetupScreen({ onApiKeyInit, onCodexAuth }: {
  onApiKeyInit: (key: string) => Promise<void>
  onCodexAuth: () => Promise<void>
}) {
  const [apiKey, setApiKey] = useState("")
  const [loading, setLoading] = useState(false)
  const [authMethod, setAuthMethod] = useState<AuthMethod>("none")
  const [error, setError] = useState("")

  async function handleApiKeyInit() {
    if (!apiKey.trim()) return
    setLoading(true)
    setError("")
    try {
      await onApiKeyInit(apiKey.trim())
    } catch (e) {
      setError(String(e))
      setLoading(false)
    }
  }

  async function handleCodexAuth() {
    setLoading(true)
    setError("")
    setAuthMethod("codex-oauth")
    try {
      await onCodexAuth()
    } catch (e) {
      setError(String(e))
      setLoading(false)
      setAuthMethod("none")
    }
  }

  return (
    <div className="flex flex-col items-center justify-center h-screen gap-6 px-8">
      <div className="text-center">
        <h1 className="text-3xl font-semibold tracking-tight text-foreground">
          OpenComputer
        </h1>
        <p className="text-sm text-muted-foreground mt-1">Your personal AI assistant</p>
      </div>

      {/* Codex OAuth Button */}
      <div className="flex flex-col gap-3 w-80">
        <Button
          onClick={handleCodexAuth}
          disabled={loading}
          className="w-full h-11 text-sm font-medium bg-primary hover:bg-primary/90"
        >
          {loading && authMethod === "codex-oauth" ? (
            <span className="flex items-center gap-2">
              <span className="animate-spin h-4 w-4 border-2 border-current border-t-transparent rounded-full" />
              等待浏览器登录...
            </span>
          ) : (
            "Sign in with ChatGPT"
          )}
        </Button>

        {/* Divider */}
        <div className="flex items-center gap-3">
          <div className="flex-1 h-px bg-border" />
          <span className="text-xs text-muted-foreground">或使用 API Key</span>
          <div className="flex-1 h-px bg-border" />
        </div>

        {/* API Key Input */}
        <Input
          type="password"
          placeholder="Anthropic API key"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleApiKeyInit()}
          className="bg-card"
          disabled={loading}
        />
        <Button
          variant="secondary"
          onClick={handleApiKeyInit}
          disabled={loading || !apiKey.trim()}
        >
          {loading && authMethod === "api-key" ? "连接中..." : "使用 API Key 登录"}
        </Button>

        {error && (
          <p className="text-xs text-red-400 text-center">{error}</p>
        )}
      </div>
    </div>
  )
}

function ToolCallBlock({ tool }: { tool: ToolCall }) {
  const [expanded, setExpanded] = useState(false)
  const isRunning = tool.result === undefined
  const displayArgs = (() => {
    try {
      const parsed = JSON.parse(tool.arguments)
      if (tool.name === "exec") return parsed.command
      if (tool.name === "read_file" || tool.name === "list_dir") return parsed.path || "."
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

function ChatScreen({ onLogout }: { onLogout: () => void }) {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState("")
  const [loading, setLoading] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)

  // Model & reasoning state
  const [models, setModels] = useState<CodexModel[]>([])
  const [currentModel, setCurrentModel] = useState("gpt-5.4")
  const [reasoningEffort, setReasoningEffort] = useState("medium")

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [messages, loading])

  // Fetch models and current settings on mount
  useEffect(() => {
    (async () => {
      try {
        const [modelList, settings] = await Promise.all([
          invoke<CodexModel[]>("get_codex_models"),
          invoke<CurrentSettings>("get_current_settings"),
        ])
        setModels(modelList)
        setCurrentModel(settings.model)
        setReasoningEffort(settings.reasoning_effort)
      } catch (e) {
        console.error("Failed to load settings:", e)
      }
    })()
  }, [])

  async function handleModelChange(modelId: string) {
    setCurrentModel(modelId)
    try {
      await invoke("set_codex_model", { model: modelId })
    } catch (e) {
      console.error("Failed to set model:", e)
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
    setInput("")
    setMessages((prev) => [...prev, { role: "user", content: text }])
    setLoading(true)

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
                updated[updated.length - 1] = { ...last, content: last.content + (event.content || "") }
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
                const idx = calls.findIndex((c) => c.callId === event.call_id)
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
          // Fallback: treat as plain text delta (shouldn't happen)
          setMessages((prev) => {
            const updated = [...prev]
            const last = updated[updated.length - 1]
            if (last && last.role === "assistant") {
              updated[updated.length - 1] = { ...last, content: last.content + raw }
            }
            return updated
          })
        }
      }

      await invoke<string>("chat", { message: text, onEvent })
    } catch (e) {
      setMessages((prev) => {
        const updated = [...prev]
        const last = updated[updated.length - 1]
        if (last && last.role === "assistant" && last.content === "") {
          // Replace empty streaming message with error
          updated[updated.length - 1] = { ...last, content: `Error: ${e}` }
        } else {
          updated.push({ role: "assistant", content: `Error: ${e}` })
        }
        return updated
      })
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex h-screen overflow-hidden bg-background">
      {/* Column 1: Icon Sidebar */}
      <div className="w-14 shrink-0 border-r border-border bg-secondary/30 flex flex-col items-center">
        <div className="h-11 flex items-center justify-center border-b border-border w-full">
          <Button
            variant="ghost"
            size="icon"
            className="rounded-xl bg-primary/10 text-primary hover:bg-primary/20 h-8 w-8"
            title="会话"
          >
            <MessageSquare className="h-4 w-4" />
          </Button>
        </div>

        <div className="flex-1" />

        <div className="py-3">
          <Button
            variant="ghost"
            size="icon"
            className="rounded-xl text-muted-foreground hover:text-foreground h-8 w-8"
            onClick={onLogout}
            title="登出"
          >
            <LogOut className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {/* Column 2: Agent List */}
      <div className="w-64 shrink-0 border-r border-border bg-background flex flex-col">
        <div className="h-11 flex items-center px-4 border-b border-border">
          <h2 className="text-sm font-semibold text-foreground">会话</h2>
        </div>
        <div className="flex-1 overflow-y-auto p-2">
          {/* Main Agent — active */}
          <div className="flex items-center gap-3 px-3 py-2.5 rounded-lg bg-secondary/60 cursor-pointer border border-border/50 transition-colors">
            <div className="w-9 h-9 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0">
              <Bot className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="font-medium text-sm text-foreground truncate">Main Agent</div>
              <div className="text-xs text-muted-foreground truncate">
                {messages.length > 0
                  ? messages[messages.length - 1].content.slice(0, 30) || "工具调用中..."
                  : "开始对话"}
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Column 3: Chat Area */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Header */}
        <div className="h-11 flex items-center justify-between px-4 border-b border-border bg-background gap-2">
          <div className="flex items-center gap-2 min-w-0">
            <span className="text-sm font-medium text-foreground shrink-0">Main Agent</span>

            {/* Model Selector */}
            {models.length > 0 && (
              <div className="relative">
                <select
                  value={currentModel}
                  onChange={(e) => handleModelChange(e.target.value)}
                  className="appearance-none bg-secondary text-foreground text-xs font-medium pl-2 pr-6 py-1 rounded-md border border-border cursor-pointer hover:bg-secondary/80 transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                >
                  {models.map((m) => (
                    <option key={m.id} value={m.id}>{m.name}</option>
                  ))}
                </select>
                <ChevronDown className="absolute right-1.5 top-1/2 -translate-y-1/2 h-3 w-3 text-muted-foreground pointer-events-none" />
              </div>
            )}
          </div>

          <div className="flex items-center gap-1">
            {/* Think Mode Toggle */}
            <div className="relative">
              <div className="flex items-center gap-1">
                <Brain className="h-3.5 w-3.5 text-muted-foreground" />
                <select
                  value={reasoningEffort}
                  onChange={(e) => handleEffortChange(e.target.value)}
                  className="appearance-none bg-secondary text-foreground text-xs font-medium pl-1 pr-5 py-1 rounded-md border border-border cursor-pointer hover:bg-secondary/80 transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                >
                  {EFFORT_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                  ))}
                </select>
                <ChevronDown className="absolute right-1 top-1/2 -translate-y-1/2 h-3 w-3 text-muted-foreground pointer-events-none" />
              </div>
            </div>
          </div>
        </div>

        {/* Messages */}
        <div className="flex-1 overflow-y-auto px-4 py-6 space-y-4">
          {messages.length === 0 && (
            <div className="flex items-center justify-center h-full">
              <p className="text-muted-foreground text-sm">How can I help you today?</p>
            </div>
          )}
          {messages.map((msg, i) => (
            <div
              key={i}
              className={cn("flex", msg.role === "user" ? "justify-end" : "justify-start")}
            >
              <div
                className={cn(
                  "max-w-[70%] px-4 py-2.5 rounded-xl text-sm leading-relaxed",
                  msg.role === "user"
                    ? "bg-secondary text-foreground whitespace-pre-wrap"
                    : "bg-card text-foreground/80"
                )}
              >
                {msg.role === "assistant" && msg.toolCalls?.map((tool) => (
                  <ToolCallBlock key={tool.callId} tool={tool} />
                ))}
                {msg.content && (
                  <div className="whitespace-pre-wrap">{msg.content}</div>
                )}
              </div>
            </div>
          ))}
          {loading && (
            <div className="flex justify-start">
              <div className="bg-card px-4 py-2.5 rounded-xl text-muted-foreground text-sm tracking-widest">
                ...
              </div>
            </div>
          )}
          <div ref={bottomRef} />
        </div>

        {/* Input */}
        <div className="border-t border-border px-4 py-3 flex gap-2 bg-background">
          <Input
            placeholder="Ask anything..."
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSend()}
            className="bg-card"
          />
          <Button
            size="icon"
            onClick={handleSend}
            disabled={loading || !input.trim()}
          >
            <Send className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </div>
  )
}

export default function App() {
  const [initialized, setInitialized] = useState(false)
  const [restoring, setRestoring] = useState(true)

  // Try to restore previous session on mount
  useEffect(() => {
    (async () => {
      try {
        const restored = await invoke<boolean>("try_restore_session")
        if (restored) {
          setInitialized(true)
        }
      } catch (e) {
        console.error("Failed to restore session:", e)
      } finally {
        setRestoring(false)
      }
    })()
  }, [])

  async function handleApiKeyInit(apiKey: string) {
    await invoke("initialize_agent", { apiKey })
    setInitialized(true)
  }

  async function handleCodexAuth() {
    // Start the OAuth flow (opens browser)
    await invoke("start_codex_auth")

    // Poll for auth completion
    const poll = async (): Promise<void> => {
      for (let i = 0; i < 300; i++) {
        await new Promise((r) => setTimeout(r, 1000))
        const status = await invoke<{ authenticated: boolean; error: string | null }>("check_auth_status")
        if (status.authenticated) {
          await invoke("finalize_codex_auth")
          setInitialized(true)
          return
        }
        if (status.error) {
          throw new Error(status.error)
        }
      }
      throw new Error("登录超时，请重试")
    }

    await poll()
  }

  async function handleLogout() {
    try {
      await invoke("logout_codex")
    } catch (e) {
      console.error("Logout error:", e)
    }
    setInitialized(false)
  }

  if (restoring) {
    return (
      <div className="flex items-center justify-center h-screen">
        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
      </div>
    )
  }

  return initialized ? (
    <ChatScreen onLogout={handleLogout} />
  ) : (
    <SetupScreen onApiKeyInit={handleApiKeyInit} onCodexAuth={handleCodexAuth} />
  )
}

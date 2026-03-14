import { useState, useRef, useEffect } from "react"
import { invoke, Channel } from "@tauri-apps/api/core"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import { Send, LogOut, Brain, ChevronDown } from "lucide-react"

interface Message {
  role: "user" | "assistant"
  content: string
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
      onEvent.onmessage = (delta) => {
        setMessages((prev) => {
          const updated = [...prev]
          const last = updated[updated.length - 1]
          if (last && last.role === "assistant") {
            updated[updated.length - 1] = { ...last, content: last.content + delta }
          }
          return updated
        })
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
    <div className="flex flex-col h-screen">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-border bg-background gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-sm font-medium text-foreground shrink-0">OpenComputer</span>

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

          <Button variant="ghost" size="icon" onClick={onLogout} title="登出">
            <LogOut className="h-4 w-4" />
          </Button>
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
                "max-w-[70%] px-4 py-2.5 rounded-xl text-sm leading-relaxed whitespace-pre-wrap",
                msg.role === "user"
                  ? "bg-secondary text-foreground"
                  : "bg-card text-foreground/80"
              )}
            >
              {msg.content}
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

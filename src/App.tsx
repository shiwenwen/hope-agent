import { useState, useRef, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import { Send } from "lucide-react"

interface Message {
  role: "user" | "assistant"
  content: string
}

function SetupScreen({ onInit }: { onInit: (key: string) => Promise<void> }) {
  const [apiKey, setApiKey] = useState("")
  const [loading, setLoading] = useState(false)

  async function handleInit() {
    if (!apiKey.trim()) return
    setLoading(true)
    await onInit(apiKey.trim())
    setLoading(false)
  }

  return (
    <div className="flex flex-col items-center justify-center h-screen gap-4 px-8">
      <h1 className="text-3xl font-semibold tracking-tight text-foreground">
        OpenComputer
      </h1>
      <p className="text-sm text-muted-foreground">Your personal AI assistant</p>
      <div className="flex flex-col gap-2 w-72 mt-2">
        <Input
          type="password"
          placeholder="Anthropic API key"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleInit()}
          className="bg-card"
        />
        <Button onClick={handleInit} disabled={loading || !apiKey.trim()}>
          {loading ? "Connecting..." : "Get Started"}
        </Button>
      </div>
    </div>
  )
}

function ChatScreen() {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState("")
  const [loading, setLoading] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [messages, loading])

  async function handleSend() {
    if (!input.trim() || loading) return
    const text = input.trim()
    setInput("")
    setMessages((prev) => [...prev, { role: "user", content: text }])
    setLoading(true)
    try {
      const response = await invoke<string>("chat", { message: text })
      setMessages((prev) => [...prev, { role: "assistant", content: response }])
    } catch (e) {
      setMessages((prev) => [
        ...prev,
        { role: "assistant", content: `Error: ${e}` },
      ])
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex flex-col h-screen">
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

  async function handleInit(apiKey: string) {
    try {
      await invoke("initialize_agent", { apiKey })
      setInitialized(true)
    } catch (e) {
      console.error(e)
    }
  }

  return initialized ? <ChatScreen /> : <SetupScreen onInit={handleInit} />
}

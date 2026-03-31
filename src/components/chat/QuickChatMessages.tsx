import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { ExternalLink } from "lucide-react"
import type { Message } from "@/types/chat"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"

interface QuickChatMessagesProps {
  messages: Message[]
  loading: boolean
  sessionId: string | null
  onNavigateToSession?: (sessionId: string) => void
}

export default function QuickChatMessages({
  messages,
  loading,
  sessionId,
  onNavigateToSession,
}: QuickChatMessagesProps) {
  const { t } = useTranslation()
  const bottomRef = useRef<HTMLDivElement>(null)
  const lastMessageContent = messages[messages.length - 1]?.content ?? ""

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [messages.length, lastMessageContent])

  if (messages.length === 0) {
    return null
  }

  return (
    <div className="flex-1 overflow-y-auto min-h-0 px-4 py-3 space-y-3">
      {/* View full conversation link */}
      {sessionId && onNavigateToSession && (
        <button
          onClick={() => onNavigateToSession(sessionId)}
          className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors mx-auto"
        >
          <ExternalLink className="h-3 w-3" />
          {t("quickChat.viewFullChat")}
        </button>
      )}

      {messages.map((msg, i) => {
        if (msg.role === "event") {
          return (
            <div key={i} className="text-xs text-center text-muted-foreground py-1">
              {msg.content}
            </div>
          )
        }

        if (msg.role === "user") {
          return (
            <div key={i} className="flex justify-end">
              <div className="max-w-[80%] rounded-2xl rounded-br-md bg-primary text-primary-foreground px-3.5 py-2 text-sm">
                {msg.content}
              </div>
            </div>
          )
        }

        // Assistant message
        const isLastAssistant = i === messages.length - 1 && msg.role === "assistant"
        const isStreaming = isLastAssistant && loading

        return (
          <div key={i} className="flex justify-start">
            <div className="max-w-[85%] rounded-2xl rounded-bl-md bg-muted px-3.5 py-2 text-sm">
              {msg.content ? (
                <div className="prose prose-sm dark:prose-invert max-w-none [&_pre]:max-h-[200px] [&_pre]:overflow-auto">
                  <MarkdownRenderer content={msg.content} isStreaming={isStreaming} />
                </div>
              ) : isStreaming ? (
                <div className="flex items-center gap-1.5 text-muted-foreground">
                  <div className="h-1.5 w-1.5 rounded-full bg-current animate-pulse" />
                  <div className="h-1.5 w-1.5 rounded-full bg-current animate-pulse [animation-delay:0.15s]" />
                  <div className="h-1.5 w-1.5 rounded-full bg-current animate-pulse [animation-delay:0.3s]" />
                </div>
              ) : null}

              {/* Tool calls summary */}
              {msg.toolCalls && msg.toolCalls.length > 0 && (
                <div className="mt-1.5 text-xs text-muted-foreground">
                  {msg.toolCalls.map((tc, j) => (
                    <span key={j} className="inline-flex items-center gap-1 mr-2">
                      <span className="opacity-60">⚙</span>
                      {tc.name}
                    </span>
                  ))}
                </div>
              )}
            </div>
          </div>
        )
      })}

      <div ref={bottomRef} />
    </div>
  )
}

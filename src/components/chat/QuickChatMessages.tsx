import { useCallback, useMemo } from "react"
import { useTranslation } from "react-i18next"
import { ExternalLink } from "lucide-react"
import type { Message } from "@/types/chat"
import { useVirtualFeed } from "@/components/common/useVirtualFeed"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"

interface QuickChatMessagesProps {
  messages: Message[]
  loading: boolean
  sessionId: string | null
  onNavigateToSession?: (sessionId: string) => void
}

type QuickChatRow =
  | { type: "viewFullChat"; key: "view-full-chat" }
  | { type: "message"; key: string; msg: Message; originalIndex: number }

function getMessageRowKey(msg: Message, originalIndex: number): string {
  if (typeof msg.dbId === "number") return `message:${msg.dbId}`
  return `message:${msg.role}:${msg.timestamp ?? "pending"}:${originalIndex}`
}

export default function QuickChatMessages({
  messages,
  loading,
  sessionId,
  onNavigateToSession,
}: QuickChatMessagesProps) {
  const { t } = useTranslation()

  const rows = useMemo<QuickChatRow[]>(() => {
    const next: QuickChatRow[] = []
    if (sessionId && onNavigateToSession) {
      next.push({ type: "viewFullChat", key: "view-full-chat" })
    }
    messages.forEach((msg, originalIndex) => {
      next.push({
        type: "message",
        key: getMessageRowKey(msg, originalIndex),
        msg,
        originalIndex,
      })
    })
    return next
  }, [messages, onNavigateToSession, sessionId])

  const getRowKey = useCallback((row: QuickChatRow) => row.key, [])
  const estimateSize = useCallback((index: number) => {
    const row = rows[index]
    if (!row) return 72
    if (row.type === "viewFullChat") return 28
    if (row.msg.role === "event") return 28
    if (row.msg.role === "user") return 58
    return 72
  }, [rows])

  const lastMsg = messages[messages.length - 1]
  const followKey = `${rows.length}:${lastMsg?.role ?? ""}:${lastMsg?.content.length ?? 0}:${lastMsg?.toolCalls?.length ?? 0}`
  const { scrollRef, virtualizer, virtualItems, totalSize } = useVirtualFeed({
    rows,
    getRowKey,
    estimateSize,
    overscan: 6,
    gap: 12,
    followOutput: loading,
    followKey,
    resetKey: sessionId ?? "quick-chat",
  })

  if (messages.length === 0) {
    return null
  }

  const renderRow = (row: QuickChatRow) => {
    if (row.type === "viewFullChat") {
      return (
        <button
          onClick={() => sessionId && onNavigateToSession?.(sessionId)}
          className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors mx-auto"
        >
          <ExternalLink className="h-3 w-3" />
          {t("quickChat.viewFullChat")}
        </button>
      )
    }

    const { msg, originalIndex } = row
    if (msg.role === "event") {
      return (
        <div className="text-xs text-center text-muted-foreground py-1">
          {msg.content}
        </div>
      )
    }

    if (msg.role === "user") {
      return (
        <div className="flex justify-end">
          <div className="max-w-[80%] rounded-2xl rounded-br-md bg-primary text-primary-foreground px-3.5 py-2 text-sm">
            {msg.content}
          </div>
        </div>
      )
    }

    const isLastAssistant = originalIndex === messages.length - 1 && msg.role === "assistant"
    const isStreaming = isLastAssistant && loading

    return (
      <div className="flex justify-start">
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

          {msg.toolCalls && msg.toolCalls.length > 0 && (
            <div className="mt-1.5 text-xs text-muted-foreground">
              {msg.toolCalls.map((tc) => (
                <span key={tc.callId} className="inline-flex items-center gap-1 mr-2">
                  <span className="opacity-60">⚙</span>
                  {tc.name}
                </span>
              ))}
            </div>
          )}
        </div>
      </div>
    )
  }

  return (
    <div ref={scrollRef} className="flex-1 overflow-y-auto min-h-0 px-4 py-3">
      <div className="relative w-full" style={{ height: totalSize }}>
        {virtualItems.map((virtualRow) => {
          const row = rows[virtualRow.index]
          if (!row) return null
          return (
            <div
              key={virtualRow.key}
              ref={virtualizer.measureElement}
              data-index={virtualRow.index}
              className="absolute left-0 top-0 w-full"
              style={{ transform: `translateY(${virtualRow.start}px)` }}
            >
              {renderRow(row)}
            </div>
          )
        })}
      </div>
    </div>
  )
}

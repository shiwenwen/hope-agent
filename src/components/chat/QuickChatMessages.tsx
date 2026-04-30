import { useCallback, useMemo } from "react"
import { useTranslation } from "react-i18next"
import { ArrowDown, ExternalLink } from "lucide-react"
import type { Message } from "@/types/chat"
import { useVirtualFeed } from "@/components/common/useVirtualFeed"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { IconTip } from "@/components/ui/tooltip"
import LoadMoreRow from "./LoadMoreRow"
import { getLatestUserTurnKey } from "./chatScrollKeys"

interface QuickChatMessagesProps {
  messages: Message[]
  loading: boolean
  sessionId: string | null
  onNavigateToSession?: (sessionId: string) => void
  hasMore?: boolean
  loadingMore?: boolean
  onLoadMore?: () => void | Promise<void>
}

type QuickChatRow =
  | { type: "loadMore"; key: "load-more" }
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
  hasMore = false,
  loadingMore = false,
  onLoadMore,
}: QuickChatMessagesProps) {
  const { t } = useTranslation()

  const rows = useMemo<QuickChatRow[]>(() => {
    const next: QuickChatRow[] = []
    if (hasMore && onLoadMore) {
      next.push({ type: "loadMore", key: "load-more" })
    }
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
  }, [messages, onNavigateToSession, sessionId, hasMore, onLoadMore])

  const getRowKey = useCallback((row: QuickChatRow) => row.key, [])
  const estimateSize = useCallback(
    (index: number) => {
      const row = rows[index]
      if (!row) return 72
      if (row.type === "loadMore") return 32
      if (row.type === "viewFullChat") return 28
      if (row.msg.role === "event") return 28
      if (row.msg.role === "user") return 58
      return 72
    },
    [rows],
  )

  const lastMsg = messages[messages.length - 1]
  const latestUserTurnKey = getLatestUserTurnKey(messages)
  const followKey = `${rows.length}:${lastMsg?.role ?? ""}:${lastMsg?.content.length ?? 0}:${lastMsg?.toolCalls?.length ?? 0}`
  const canAnchorRow = useCallback(
    (row: QuickChatRow) => row.type === "message",
    [],
  )
  const {
    scrollRef,
    virtualizer,
    virtualItems,
    totalSize,
    isAutoFollowPaused,
    hasUnseenOutput,
    resumeAutoFollow,
  } = useVirtualFeed({
    rows,
    getRowKey,
    estimateSize,
    overscan: 6,
    gap: 12,
    paddingStart: 12,
    paddingEnd: 12,
    followOutput: loading,
    followKey,
    forceFollowKey: latestUserTurnKey,
    resetKey: sessionId ?? "quick-chat",
    canAnchorRow,
    onStartReached: onLoadMore,
    canLoadMore: hasMore,
    loadingMore,
  })
  const showJumpToLatest = isAutoFollowPaused && (loading || hasUnseenOutput)

  if (messages.length === 0) {
    return null
  }

  const renderRow = (row: QuickChatRow) => {
    if (row.type === "loadMore") {
      return <LoadMoreRow loadingMore={loadingMore} onLoadMore={onLoadMore} />
    }

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
      return <div className="text-xs text-center text-muted-foreground py-1">{msg.content}</div>
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
    <div className="relative flex-1 min-h-0">
      <div ref={scrollRef} className="h-full overflow-y-auto px-4">
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

      {showJumpToLatest && (
        <div className="pointer-events-none absolute inset-x-0 bottom-3 z-20 flex justify-center px-4">
          <IconTip label={t("chat.scrollToBottom")}>
            <button
              type="button"
              onClick={() => resumeAutoFollow("smooth")}
              className="pointer-events-auto inline-flex h-8 w-8 cursor-pointer items-center justify-center rounded-full border border-border/70 bg-background/95 text-foreground shadow-lg shadow-black/10 backdrop-blur transition-colors hover:bg-muted"
              aria-label={t("chat.scrollToBottom")}
            >
              <ArrowDown className="h-4 w-4" />
            </button>
          </IconTip>
        </div>
      )}
    </div>
  )
}

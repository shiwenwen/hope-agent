import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { ArrowDown, ExternalLink } from "lucide-react"
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso"
import type { Message } from "@/types/chat"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { IconTip } from "@/components/ui/tooltip"
import LoadMoreRow from "./LoadMoreRow"
import { getLatestUserTurnKey, getMessageRowKey } from "./chatScrollKeys"

interface QuickChatMessagesProps {
  messages: Message[]
  loading: boolean
  sessionId: string | null
  onNavigateToSession?: (sessionId: string) => void
  hasMore?: boolean
  loadingMore?: boolean
  onLoadMore?: () => void | Promise<void>
}

interface FeedItem {
  msg: Message
  originalIndex: number
}

const INITIAL_FIRST_ITEM_INDEX = 1_000_000

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
  const virtuosoRef = useRef<VirtuosoHandle>(null)

  const items = useMemo<FeedItem[]>(
    () => messages.map((msg, originalIndex) => ({ msg, originalIndex })),
    [messages],
  )

  // See MessageList for the tail-equal prepend detection rationale.
  const [view, setView] = useState<{ data: FeedItem[]; firstItemIndex: number }>({
    data: items,
    firstItemIndex: INITIAL_FIRST_ITEM_INDEX,
  })
  useLayoutEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- derived state pattern: virtuoso needs same-tick (data, firstItemIndex)
    setView((prev) => {
      if (prev.data === items) return prev
      const delta = items.length - prev.data.length
      const isPrepend =
        delta > 0 &&
        prev.data.length > 0 &&
        items[items.length - 1].msg === prev.data[prev.data.length - 1].msg
      return {
        data: items,
        firstItemIndex: isPrepend ? prev.firstItemIndex - delta : prev.firstItemIndex,
      }
    })
  }, [items])

  const isAtBottomRef = useRef(true)
  const [isAtBottom, setIsAtBottom] = useState(true)
  const [hasUnseenOutput, setHasUnseenOutput] = useState(false)
  const prevMsgLenRef = useRef(messages.length)

  useEffect(() => {
    if (!isAtBottomRef.current && messages.length > prevMsgLenRef.current) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- reactive flag: new messages arrived while user is scrolled away
      setHasUnseenOutput(true)
    }
    prevMsgLenRef.current = messages.length
  }, [messages.length])

  const handleAtBottomStateChange = useCallback((b: boolean) => {
    isAtBottomRef.current = b
    setIsAtBottom(b)
    if (b) setHasUnseenOutput(false)
  }, [])

  // See MessageList for the force-following flag rationale.
  const forceFollowingRef = useRef(false)
  const forceFollowResetRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const armForceFollowing = useCallback(() => {
    forceFollowingRef.current = true
    if (forceFollowResetRef.current) clearTimeout(forceFollowResetRef.current)
    forceFollowResetRef.current = setTimeout(() => {
      forceFollowingRef.current = false
    }, 600)
  }, [])

  const followOutput = useCallback(
    (atBottom: boolean) => {
      if (forceFollowingRef.current) return loading ? "smooth" : "auto"
      if (!atBottom) return false
      return loading ? "smooth" : "auto"
    },
    [loading],
  )

  const scrollToLastUser = useCallback(() => {
    let userIdx = -1
    for (let i = items.length - 1; i >= 0; i--) {
      if (items[i].msg.role === "user") {
        userIdx = i
        break
      }
    }
    if (userIdx < 0) return
    armForceFollowing()
    // scrollToIndex takes a data-relative index; firstItemIndex only affects
    // rendered item ids, not this API.
    virtuosoRef.current?.scrollToIndex({
      index: userIdx,
      align: "start",
      behavior: "smooth",
    })
  }, [items, armForceFollowing])

  const lastUserKey = useMemo(() => getLatestUserTurnKey(messages), [messages])
  const lastSeenUserKeyRef = useRef<string | null>(lastUserKey)
  useEffect(() => {
    if (!lastUserKey || lastUserKey === lastSeenUserKeyRef.current) return
    lastSeenUserKeyRef.current = lastUserKey
    scrollToLastUser()
  }, [lastUserKey, scrollToLastUser])

  useEffect(
    () => () => {
      if (forceFollowResetRef.current) clearTimeout(forceFollowResetRef.current)
    },
    [],
  )

  const handleStartReached = useCallback(() => {
    if (!hasMore || loadingMore || !onLoadMore) return
    void onLoadMore()
  }, [hasMore, loadingMore, onLoadMore])

  const handleJumpToLatest = useCallback(() => {
    armForceFollowing()
    virtuosoRef.current?.scrollToIndex({
      index: "LAST",
      align: "end",
      behavior: "smooth",
    })
  }, [armForceFollowing])

  const computeItemKey = useCallback(
    (_index: number, item: FeedItem) => getMessageRowKey(item.msg, item.originalIndex),
    [],
  )

  const itemContent = useCallback(
    (_absoluteIndex: number, item: FeedItem) => {
      const { msg, originalIndex } = item

      if (msg.role === "event" || msg.isPlanTrigger) {
        return (
          <div className="pb-3 text-xs text-center text-muted-foreground py-1">{msg.content}</div>
        )
      }

      if (msg.role === "user") {
        return (
          <div className="pb-3 flex justify-end">
            <div className="max-w-[80%] rounded-2xl rounded-br-md bg-primary text-primary-foreground px-3.5 py-2 text-sm">
              {msg.content}
            </div>
          </div>
        )
      }

      const isLastAssistant = originalIndex === messages.length - 1 && msg.role === "assistant"
      const isStreaming = isLastAssistant && loading

      return (
        <div className="pb-3 flex justify-start">
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
    },
    [loading, messages.length],
  )

  const Header = useCallback(
    () => (
      <div className="pt-3 flex flex-col gap-2">
        {hasMore && onLoadMore && (
          <LoadMoreRow loadingMore={loadingMore} onLoadMore={onLoadMore} />
        )}
        {sessionId && onNavigateToSession && (
          <button
            type="button"
            onClick={() => onNavigateToSession(sessionId)}
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors mx-auto"
          >
            <ExternalLink className="h-3 w-3" />
            {t("quickChat.viewFullChat")}
          </button>
        )}
      </div>
    ),
    [hasMore, loadingMore, onLoadMore, onNavigateToSession, sessionId, t],
  )

  const components = useMemo(() => ({ Header }), [Header])

  if (messages.length === 0) {
    return null
  }

  const showJumpToLatest = !isAtBottom && (loading || hasUnseenOutput)

  return (
    <div className="relative flex-1 min-h-0">
      <Virtuoso
        key={sessionId ?? "quick-chat"}
        ref={virtuosoRef}
        className="h-full px-4"
        data={view.data}
        firstItemIndex={view.firstItemIndex}
        initialTopMostItemIndex={view.data.length > 0 ? view.data.length - 1 : 0}
        computeItemKey={computeItemKey}
        itemContent={itemContent}
        components={components}
        startReached={handleStartReached}
        atBottomStateChange={handleAtBottomStateChange}
        followOutput={followOutput}
        defaultItemHeight={72}
        increaseViewportBy={{ top: 120, bottom: 240 }}
        atBottomThreshold={32}
      />

      {showJumpToLatest && (
        <div className="pointer-events-none absolute inset-x-0 bottom-3 z-20 flex justify-center px-4">
          <IconTip label={t("chat.scrollToBottom")}>
            <button
              type="button"
              onClick={handleJumpToLatest}
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

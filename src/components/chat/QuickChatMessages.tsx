import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { ArrowDown, ExternalLink } from "lucide-react"
import type { Message } from "@/types/chat"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { IconTip } from "@/components/ui/tooltip"
import LoadMoreRow from "./LoadMoreRow"
import {
  findMessageRowByKey,
  getLatestUserTurnKey,
  getMessageRowKey,
} from "./chatScrollKeys"

interface QuickChatMessagesProps {
  messages: Message[]
  loading: boolean
  sessionId: string | null
  onNavigateToSession?: (sessionId: string) => void
  hasMore?: boolean
  loadingMore?: boolean
  onLoadMore?: () => void | Promise<void>
}

const AT_BOTTOM_THRESHOLD_PX = 32
const LOAD_MORE_THRESHOLD_PX = 200
// Windowed view: see MessageList for rationale. QuickChat is a popup with
// shorter typical history but same shape — Load More can still accumulate
// thousands of rows over time.
const MAX_DOM_MESSAGES = 200
const UNLOAD_BATCH = 30

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
  const containerRef = useRef<HTMLDivElement | null>(null)
  const contentRef = useRef<HTMLDivElement | null>(null)
  const sessionKey = sessionId ?? "quick-chat"

  const [atBottom, setAtBottom] = useState(true)
  const atBottomRef = useRef(true)
  // Suspends auto-follow once the user gestures up; cleared on bottom-reach
  // or jump-to-latest. See MessageList for full rationale.
  const userScrollLockRef = useRef(false)

  // Windowed view: see MessageList for rationale.
  const [displayedStart, setDisplayedStart] = useState(0)
  const [displayedStartSession, setDisplayedStartSession] = useState(sessionKey)
  if (displayedStartSession !== sessionKey) {
    setDisplayedStartSession(sessionKey)
    setDisplayedStart(0)
  }
  const displayedStartRef = useRef(displayedStart)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  displayedStartRef.current = displayedStart
  const messagesLenRef = useRef(messages.length)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  messagesLenRef.current = messages.length

  // Render slice. `originalIndex` preserved for keys + data attributes.
  const items = useMemo(() => {
    const out: { msg: Message; originalIndex: number }[] = []
    const start = Math.min(displayedStart, Math.max(0, messages.length - 1))
    for (let i = start; i < messages.length; i++) {
      out.push({ msg: messages[i], originalIndex: i })
    }
    return out
  }, [messages, displayedStart])

  // Top-anchor fallback for window decrement / Load More prepend (same as
  // MessageList's pattern; see comment there for the keyframe rationale).
  const prevScrollHeightRef = useRef(0)
  const prevFirstItemMsgRef = useRef<Message | null>(items[0]?.msg ?? null)
  useLayoutEffect(() => {
    const el = containerRef.current
    if (!el) return
    const oldHeight = prevScrollHeightRef.current
    const newHeight = el.scrollHeight
    const oldFirst = prevFirstItemMsgRef.current
    const newFirst = items[0]?.msg ?? null
    if (
      newFirst &&
      oldFirst &&
      newFirst !== oldFirst &&
      newHeight > oldHeight &&
      oldHeight > 0 &&
      !atBottomRef.current
    ) {
      el.scrollTop += newHeight - oldHeight
    }
    prevScrollHeightRef.current = newHeight
    prevFirstItemMsgRef.current = newFirst
  }, [items])

  const lastSessionKeyRef = useRef<string | null>(null)
  useLayoutEffect(() => {
    if (lastSessionKeyRef.current !== sessionKey) {
      lastSessionKeyRef.current = sessionKey
      atBottomRef.current = true
      userScrollLockRef.current = false
    }
    const el = containerRef.current
    if (!el) return
    if (!atBottomRef.current || userScrollLockRef.current) return
    el.scrollTop = el.scrollHeight
  }, [messages, sessionKey])

  useEffect(() => {
    setAtBottom(true)
  }, [sessionKey])

  // ResizeObserver: re-pin to bottom whenever layout changes. Watch content
  // (grows from async renders) AND container itself (shrinks when sibling
  // input grows). See MessageList for full rationale.
  useEffect(() => {
    if (typeof ResizeObserver === "undefined") return
    const el = containerRef.current
    const content = contentRef.current
    if (!el || !content) return
    const ro = new ResizeObserver(() => {
      if (atBottomRef.current && !userScrollLockRef.current) {
        el.scrollTop = el.scrollHeight
      }
    })
    ro.observe(content)
    ro.observe(el)
    return () => ro.disconnect()
  }, [sessionKey])

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    let raf = 0
    const onScroll = () => {
      if (raf) return
      raf = requestAnimationFrame(() => {
        raf = 0
        const dist = el.scrollHeight - el.scrollTop - el.clientHeight
        const at = dist < AT_BOTTOM_THRESHOLD_PX
        if (at !== atBottomRef.current) {
          atBottomRef.current = at
          setAtBottom(at)
        }
        if (at) userScrollLockRef.current = false

        // Windowed view advance: cap DOM at MAX_DOM_MESSAGES. See MessageList.
        const totalLen = messagesLenRef.current
        const renderedCount = totalLen - displayedStartRef.current
        if (at && renderedCount > MAX_DOM_MESSAGES) {
          setDisplayedStart((prev) =>
            Math.min(Math.max(0, totalLen - 1), prev + UNLOAD_BATCH),
          )
        }

        // Near top: restore local first, then fall through to remote.
        if (el.scrollTop < LOAD_MORE_THRESHOLD_PX) {
          if (displayedStartRef.current > 0) {
            setDisplayedStart((prev) => Math.max(0, prev - UNLOAD_BATCH))
          } else if (hasMore && !loadingMore && onLoadMore) {
            void onLoadMore()
          }
        }
      })
    }
    const arrowKeys = new Set([
      "ArrowUp",
      "ArrowDown",
      "PageUp",
      "PageDown",
      "Home",
      "End",
    ])
    const lockOnIntent = () => {
      // See MessageList for rationale: scrolling down from bottom is a
      // no-op so no `scroll` event ever clears the lock — auto-follow
      // would freeze and the jump button wouldn't show.
      if (atBottomRef.current) return
      userScrollLockRef.current = true
    }
    const onKey = (e: KeyboardEvent) => {
      if (arrowKeys.has(e.key)) lockOnIntent()
    }
    el.addEventListener("scroll", onScroll, { passive: true })
    el.addEventListener("wheel", lockOnIntent, { passive: true })
    el.addEventListener("touchmove", lockOnIntent, { passive: true })
    el.addEventListener("keydown", onKey)
    return () => {
      el.removeEventListener("scroll", onScroll)
      el.removeEventListener("wheel", lockOnIntent)
      el.removeEventListener("touchmove", lockOnIntent)
      el.removeEventListener("keydown", onKey)
      if (raf) cancelAnimationFrame(raf)
    }
    // `sessionKey` belongs in deps: the outer `<div key={sessionKey}>` remounts
    // the scroll container on session swap, otherwise listeners stay bound to
    // the detached DOM.
  }, [sessionKey, hasMore, loadingMore, onLoadMore])

  const lastUserKey = useMemo(() => getLatestUserTurnKey(messages), [messages])
  const lastSeenUserKeyRef = useRef<string | null>(lastUserKey)
  const messagesRef = useRef(messages)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot for stable callbacks
  messagesRef.current = messages
  useEffect(() => {
    if (!lastUserKey || lastUserKey === lastSeenUserKeyRef.current) return
    lastSeenUserKeyRef.current = lastUserKey

    const msgs = messagesRef.current
    let userIdx = -1
    for (let i = msgs.length - 1; i >= 0; i--) {
      if (msgs[i].role === "user") {
        userIdx = i
        break
      }
    }
    if (userIdx < 0) return

    const el = containerRef.current
    if (!el) return
    userScrollLockRef.current = false
    atBottomRef.current = true
    setAtBottom(true)
    const target = findMessageRowByKey(el, getMessageRowKey(msgs[userIdx], userIdx))
    if (target) {
      target.scrollIntoView({ block: "start", behavior: "smooth" })
    } else {
      el.scrollTop = el.scrollHeight
    }
  }, [lastUserKey])

  const handleJumpToLatest = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    userScrollLockRef.current = false
    atBottomRef.current = true
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" })
  }, [])

  if (messages.length === 0) {
    return null
  }

  const showJumpToLatest = !atBottom
  // LoadMoreRow only when the window is fully expanded (older local rows
  // already restored), otherwise scrolling near top would short-circuit
  // local restore and trigger redundant remote fetch.
  const canShowLoadMore = hasMore && displayedStart === 0
  const showHeader = canShowLoadMore || (sessionId && onNavigateToSession)

  return (
    <div className="relative flex-1 min-h-0">
      <div
        ref={containerRef}
        key={sessionKey}
        className="h-full overflow-y-auto overflow-x-hidden px-4"
      >
        <div ref={contentRef}>
        {showHeader && (
          <div className="pt-3 flex flex-col gap-2">
            {canShowLoadMore && onLoadMore && (
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
        )}

        {items.map(({ msg, originalIndex }) => {
          const rowKey = getMessageRowKey(msg, originalIndex)
          const isEvent = msg.role === "event" || msg.isPlanTrigger
          const isUser = msg.role === "user"
          const isStreaming =
            !isEvent &&
            !isUser &&
            originalIndex === messages.length - 1 &&
            msg.role === "assistant" &&
            loading

          let rowClass: string
          if (isEvent) {
            rowClass = "pb-3 text-xs text-center text-muted-foreground py-1"
          } else if (isUser) {
            rowClass = "pb-3 grid grid-cols-1 justify-items-end"
          } else {
            rowClass = "pb-3 grid grid-cols-1 justify-items-start"
          }

          return (
            <div
              key={rowKey}
              data-message-key={rowKey}
              data-message-id={!isEvent && msg.dbId != null ? msg.dbId : undefined}
              className={rowClass}
            >
              {isEvent ? (
                msg.content
              ) : isUser ? (
                <div className="max-w-[80%] rounded-2xl rounded-br-md bg-primary text-primary-foreground px-3.5 py-2 text-sm break-words">
                  {msg.content}
                </div>
              ) : (
                <div className="max-w-[85%] rounded-2xl rounded-bl-md bg-muted px-3.5 py-2 text-sm break-words">
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
              )}
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

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { ArrowDown, Ghost } from "lucide-react"
import { cn } from "@/lib/utils"
import { isCenteredSystemMessage } from "./chatUtils"
import MessageBubble from "./MessageBubble"
import MessageContextMenu from "./MessageContextMenu"
import LoadMoreRow from "./LoadMoreRow"
import AskUserQuestionBlock from "./ask-user/AskUserQuestionBlock"
import PlanCardBlock from "./plan-mode/PlanCardBlock"
import {
  findMessageRowByKey,
  getLatestUserTurnKey,
  getMessageRowKey,
} from "./chatScrollKeys"
import type { AskUserQuestionGroup } from "./ask-user/AskUserQuestionBlock"
import type { PlanCardData } from "./plan-mode/PlanCardBlock"
import type { Message, AgentSummaryForSidebar } from "@/types/chat"
import type { PlanModeState } from "./plan-mode/usePlanMode"

interface MessageListProps {
  messages: Message[]
  loading: boolean
  agents: AgentSummaryForSidebar[]
  hasMore: boolean
  loadingMore: boolean
  onLoadMore: () => void | Promise<void>
  sessionId?: string | null
  incognito?: boolean
  pendingScrollTarget?: number | null
  onScrollTargetHandled?: () => void
  pendingQuestionGroup?: AskUserQuestionGroup | null
  onQuestionSubmitted?: () => void
  planCardData?: PlanCardData | null
  planState?: PlanModeState
  onOpenPlanPanel?: () => void
  onApprovePlan?: () => void
  onExitPlan?: () => void
  planSubagentRunning?: boolean
  onSwitchModel?: (providerId: string, modelId: string) => void
  onViewSystemPrompt?: () => void
  onSwitchSession?: (sessionId: string) => void
  onOpenDiff?: (
    metadata:
      | import("@/types/chat").FileChangeMetadata
      | import("@/types/chat").FileChangesMetadata,
  ) => void
}

const AT_BOTTOM_THRESHOLD_PX = 48
const LOAD_MORE_THRESHOLD_PX = 200
// Windowed view: cap simultaneously-rendered messages so a long-running
// session that's been Load-More'd many times doesn't accumulate thousands of
// markdown / shiki / katex subtrees in DOM. `messages` itself is not trimmed
// — only the render slice. See `displayedStart`.
const MAX_DOM_MESSAGES = 200
const UNLOAD_BATCH = 30

export default function MessageList({
  messages,
  loading,
  agents,
  hasMore,
  loadingMore,
  onLoadMore,
  sessionId,
  incognito = false,
  pendingScrollTarget,
  onScrollTargetHandled,
  pendingQuestionGroup,
  onQuestionSubmitted,
  planCardData,
  planState,
  onOpenPlanPanel,
  onApprovePlan,
  onExitPlan,
  planSubagentRunning,
  onSwitchModel,
  onViewSystemPrompt,
  onSwitchSession,
  onOpenDiff,
}: MessageListProps) {
  const { t } = useTranslation()
  const containerRef = useRef<HTMLDivElement | null>(null)
  const contentRef = useRef<HTMLDivElement | null>(null)
  const sessionKey = sessionId ?? "draft-session"

  const [hoveredMsgIndex, setHoveredMsgIndex] = useState<number | null>(null)
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null)
  const [highlightMessageId, setHighlightMessageId] = useState<number | null>(null)
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const highlightTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [contextMenu, setContextMenu] = useState<{
    x: number
    y: number
    index: number
  } | null>(null)

  // Single source of truth: are we at (or following) the bottom?
  // Default true so the first paint after mount/session swap aligns to bottom.
  const [atBottom, setAtBottom] = useState(true)
  const atBottomRef = useRef(true)
  // User-intent flag: when true, auto-follow (useLayoutEffect + ResizeObserver)
  // is suspended even if streaming would otherwise pin to bottom. Set by user
  // gestures (wheel / touch / arrow keys) and cleared when the user reaches
  // bottom on their own or clicks jump-to-latest.
  const userScrollLockRef = useRef(false)

  // Windowed view start: only `messages.slice(displayedStart)` is rendered.
  // Advances when at bottom + DOM bloated (drops top); decrements when user
  // scrolls near top + local older messages exist (restores them); falls
  // through to remote `onLoadMore` only when the window is fully expanded.
  // Reset on session swap via the prop-derived state pattern below.
  const [displayedStart, setDisplayedStart] = useState(0)
  const [displayedStartSession, setDisplayedStartSession] = useState(sessionKey)
  const [displayedStartMessagesLength, setDisplayedStartMessagesLength] = useState(
    messages.length,
  )
  if (displayedStartSession !== sessionKey) {
    setDisplayedStartSession(sessionKey)
    setDisplayedStartMessagesLength(messages.length)
    setDisplayedStart(0)
  } else if (displayedStartMessagesLength !== messages.length) {
    const prevLength = displayedStartMessagesLength
    setDisplayedStartMessagesLength(messages.length)
    if (
      displayedStart !== 0 &&
      (messages.length < prevLength || displayedStart >= messages.length)
    ) {
      setDisplayedStart(0)
    }
  }
  // Refs mirror state/props for the scroll listener which is bound in an
  // effect with deps {sessionKey, hasMore, loadingMore, onLoadMore} — keeping
  // these out of deps avoids re-binding on every token / window step.
  const displayedStartRef = useRef(displayedStart)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  displayedStartRef.current = displayedStart
  const messagesRef = useRef(messages)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  messagesRef.current = messages

  // Filter isMeta but preserve originalIndex for MessageBubble props. Slice
  // starts at `displayedStart` so older messages outside the window aren't
  // mounted at all.
  const items = useMemo(() => {
    const out: { msg: Message; originalIndex: number }[] = []
    const start = Math.min(displayedStart, Math.max(0, messages.length - 1))
    for (let i = start; i < messages.length; i++) {
      const msg = messages[i]
      if (!msg.isMeta) out.push({ msg, originalIndex: i })
    }
    return out
  }, [messages, displayedStart])

  // Baseline for entrance animation: only messages appended *after* this
  // session was opened animate in. The initial set renders statically — no
  // distracting cascade when entering an existing conversation. Render-time
  // prop-derived state per React docs: rebase on session swap.
  const [animationBaseline, setAnimationBaseline] = useState(messages.length)
  const [animationBaselineSession, setAnimationBaselineSession] = useState(sessionKey)
  if (animationBaselineSession !== sessionKey) {
    setAnimationBaselineSession(sessionKey)
    setAnimationBaseline(messages.length)
  }

  // Top-anchor fallback: when `items[0]` reference changes (Load More
  // prepended remote rows OR window decremented to restore local rows) and
  // scrollHeight grows while the user is not at bottom, manually compensate
  // `scrollTop` by the height delta. `overflow-anchor: auto` covers this in
  // most browsers but Safari is occasionally imprecise; this is the
  // belt-and-suspenders. Skipped at advance (window dropped top, scrollHeight
  // shrinks instead of grows) and at streaming append (items[0] unchanged).
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

  // Follow bottom: any messages change while we're tracking bottom keeps the
  // viewport pinned. Session swap re-arms atBottomRef synchronously here so
  // the same effect run scrolls to the new session's bottom — running this in
  // a separate useEffect would leave the first paint of the new session
  // tracking the *previous* session's atBottomRef value.
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

  // Sync state to ref on session swap (state lags ref by one effect tick,
  // only affects jump-to-latest button paint).
  useEffect(() => {
    setAtBottom(true)
  }, [sessionKey])

  // ResizeObserver: re-pin to bottom whenever the layout changes while we're
  // tracking bottom. Two targets:
  //   - contentRef: content total height grows from async-rendered subtrees
  //     (markdown, shiki, katex, mermaid, images).
  //   - containerRef: scroll container height shrinks/grows when siblings
  //     (memory toast, ChatInput textarea expanding) take/return space —
  //     without this, sibling-resize hides the bottom of the conversation
  //     because the browser doesn't auto-adjust scrollTop.
  // Re-attach on sessionKey change because outer `<div key={sessionKey}>`
  // remounts both refs to fresh DOM nodes.
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

  // Scroll listener: track atBottom + trigger load-more near top.
  // The user-intent listeners (wheel/touch/keyboard) below set
  // userScrollLockRef synchronously, before the streaming useLayoutEffect
  // could pin the viewport back to bottom — without this lock, scroll-pin
  // races with user gestures and the user can never actually scroll up.
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
        // Reaching bottom (by hand or via auto-follow) clears the user lock so
        // streaming follows again.
        if (at) userScrollLockRef.current = false

        // Windowed view advance: at bottom + DOM exceeds cap → drop top.
        // `overflow-anchor: auto` keeps the user's bottom-aligned position
        // stable when the top messages disappear.
        const totalLen = messagesRef.current.length
        const renderedCount = totalLen - displayedStartRef.current
        if (at && renderedCount > MAX_DOM_MESSAGES) {
          setDisplayedStart((prev) =>
            Math.min(Math.max(0, totalLen - 1), prev + UNLOAD_BATCH),
          )
        }

        // Near top: restore local older messages first; fall through to remote
        // onLoadMore only when the window is fully expanded.
        if (el.scrollTop < LOAD_MORE_THRESHOLD_PX) {
          if (displayedStartRef.current > 0) {
            setDisplayedStart((prev) => Math.max(0, prev - UNLOAD_BATCH))
          } else if (hasMore && !loadingMore) {
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
      // Skip locking when already at bottom: scrolling down from bottom is
      // a no-op (scrollTop pinned at max), so no `scroll` event fires to
      // ever clear the lock — auto-follow then stays suspended forever and
      // the jump-to-latest button never appears (atBottom state is still
      // true). When the user actually drags up, the scroll handler flips
      // atBottomRef false on the next tick and subsequent gestures lock.
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
    // `sessionKey` is part of the deps because the outer `<div key={sessionKey}>`
    // remounts the scroll container on session swap — without re-running this
    // effect, the listeners would stay bound to the old (detached) DOM node.
  }, [sessionKey, hasMore, loadingMore, onLoadMore])

  // forceFollow on lastUserKey change (user sent a new message): scroll the
  // user bubble into view and re-arm follow-bottom so the assistant stream tails.
  const lastUserKey = useMemo(() => getLatestUserTurnKey(messages), [messages])
  const lastSeenUserKeyRef = useRef<string | null>(lastUserKey)
  const lastSeenUserSessionRef = useRef(sessionKey)
  useEffect(() => {
    if (lastSeenUserSessionRef.current !== sessionKey) {
      lastSeenUserSessionRef.current = sessionKey
      lastSeenUserKeyRef.current = lastUserKey
      return
    }
    if (!lastUserKey || lastUserKey === lastSeenUserKeyRef.current) return
    lastSeenUserKeyRef.current = lastUserKey

    const msgs = messagesRef.current
    let userIdx = -1
    for (let i = msgs.length - 1; i >= 0; i--) {
      const m = msgs[i]
      if (m.role === "user" && !m.isMeta) {
        userIdx = i
        break
      }
    }
    if (userIdx < 0) return

    const el = containerRef.current
    if (!el) return
    // User just sent a message — they want to see the response stream in.
    // Clear any prior scroll-lock from earlier history reading.
    userScrollLockRef.current = false
    atBottomRef.current = true
    setAtBottom(true)
    const target = findMessageRowByKey(el, getMessageRowKey(msgs[userIdx], userIdx))
    if (target) {
      target.scrollIntoView({ block: "start", behavior: "smooth" })
    } else {
      el.scrollTop = el.scrollHeight
    }
  }, [lastUserKey, sessionKey])

  // Search-result jump: scroll target dbId into view + 2s highlight pulse.
  // If the target is outside the windowed slice (`displayedStart > targetIdx`),
  // expand the window first and let the effect re-run on next render.
  const handledScrollTargetRef = useRef<number | null>(null)
  useEffect(() => {
    if (pendingScrollTarget == null) {
      handledScrollTargetRef.current = null
      return
    }
    if (handledScrollTargetRef.current === pendingScrollTarget) return

    const targetIdx = messagesRef.current.findIndex(
      (m) => m.dbId === pendingScrollTarget,
    )
    if (targetIdx >= 0 && targetIdx < displayedStart) {
      setDisplayedStart(0)
      return
    }

    const el = containerRef.current
    if (!el) return
    const target = el.querySelector<HTMLElement>(
      `[data-message-id="${pendingScrollTarget}"]`,
    )
    if (!target) return

    handledScrollTargetRef.current = pendingScrollTarget
    target.scrollIntoView({ block: "center" })
    setHighlightMessageId(pendingScrollTarget)
    if (highlightTimerRef.current) clearTimeout(highlightTimerRef.current)
    highlightTimerRef.current = setTimeout(() => setHighlightMessageId(null), 2000)
    onScrollTargetHandled?.()
  }, [pendingScrollTarget, onScrollTargetHandled, displayedStart])

  useEffect(
    () => () => {
      if (highlightTimerRef.current) clearTimeout(highlightTimerRef.current)
      if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
    },
    [],
  )

  useEffect(() => {
    if (!contextMenu) return
    const close = () => setContextMenu(null)
    document.addEventListener("mousedown", close)
    document.addEventListener("scroll", close, true)
    return () => {
      document.removeEventListener("mousedown", close)
      document.removeEventListener("scroll", close, true)
    }
  }, [contextMenu])

  const handleJumpToLatest = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    // Clear the user-intent lock so auto-follow resumes once we land at
    // bottom. atBottomRef already true here lets ResizeObserver tail any
    // height changes during the smooth scroll. Don't touch atBottom state —
    // let scroll listener flip it when the scroll actually reaches bottom,
    // otherwise the button blinks.
    userScrollLockRef.current = false
    atBottomRef.current = true
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" })
  }, [])

  const handleContextMenu = useCallback((e: React.MouseEvent, index: number) => {
    const msg = messagesRef.current[index]
    if (msg.role !== "assistant" || !msg.content) return
    e.preventDefault()
    setContextMenu({ x: e.clientX, y: e.clientY, index })
  }, [])

  const handleCopyMessage = useCallback((content: string, index: number) => {
    navigator.clipboard
      .writeText(content)
      .then(() => {
        if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
        setCopiedIndex(index)
        copiedTimerRef.current = setTimeout(() => setCopiedIndex(null), 1500)
      })
      .catch(() => {})
  }, [])

  const planCardVisible = Boolean(
    planCardData && planState && planState !== "off" && planState !== "planning",
  )
  const showEmpty = items.length === 0
  const hasFooterContent =
    pendingQuestionGroup || planCardVisible || planSubagentRunning || showEmpty
  // Show whenever user is scrolled away from bottom — independent of loading
  // state. Lets the user always have a one-click way back to latest.
  const showJumpToLatest = !atBottom && items.length > 0

  return (
    <div className="relative flex-1 min-h-0 min-w-0 overflow-hidden">
      <div
        ref={containerRef}
        key={sessionKey}
        // `overflow-anchor: none` opts out of the browser's default scroll-
        // anchoring. Otherwise the browser tries to keep visible elements at
        // their viewport position when content above grows (e.g. Load More
        // prepend), and the `useLayoutEffect` top-anchor below tries to do
        // the same — the result is double-compensation, which the user reads
        // as "the scroll keeps moving by itself after the load finished".
        className="h-full overflow-y-auto overflow-x-hidden px-4 [overflow-anchor:none]"
      >
        <div ref={contentRef}>
        {hasMore && displayedStart === 0 && (
          <div className="pt-6">
            <LoadMoreRow loadingMore={loadingMore} onLoadMore={onLoadMore} />
          </div>
        )}

        {items.map((item) => {
          const { msg, originalIndex } = item
          const rowKey = getMessageRowKey(msg, originalIndex)
          const isLast = originalIndex === messages.length - 1
          // Only the last bubble cares about the `loading` prop (drives
          // streaming-bubble class, dots placeholder, MarkdownRenderer
          // streaming hint). Pass false to all others so global loading
          // flips don't re-render the entire list — that's the source of
          // the post-stream "flicker" (markdown / shiki / katex subtree
          // rebuilds when each bubble's loading prop changes).
          const bubbleLoading = isLast ? loading : false
          return (
            <div
              key={rowKey}
              data-message-key={rowKey}
              data-message-id={msg.dbId ?? undefined}
              className={cn(
                "grid w-full min-w-0 grid-cols-1 rounded-lg pb-4 transition-colors",
                msg.dbId === highlightMessageId && "message-hit-pulse",
                isCenteredSystemMessage(msg)
                  ? "justify-items-center"
                  : msg.role === "user" && !msg.fromAgentId
                    ? "justify-items-end"
                    : "justify-items-start",
                isLast &&
                  originalIndex >= animationBaseline &&
                  "animate-fade-slide-in",
              )}
            >
              <MessageBubble
                msg={msg}
                index={originalIndex}
                isLast={isLast}
                loading={bubbleLoading}
                agents={agents}
                isHovered={hoveredMsgIndex === originalIndex}
                onHover={setHoveredMsgIndex}
                onContextMenu={handleContextMenu}
                isCopied={copiedIndex === originalIndex}
                onCopy={handleCopyMessage}
                sessionId={sessionId}
                onOpenPlanPanel={onOpenPlanPanel}
                onSwitchSession={onSwitchSession}
                onSwitchModel={onSwitchModel}
                onViewSystemPrompt={onViewSystemPrompt}
                onOpenDiff={onOpenDiff}
              />
            </div>
          )
        })}

        {hasFooterContent && (
          <div className="flex flex-col gap-4 pt-2 pb-6">
            {pendingQuestionGroup && (
              <div className="w-full">
                <AskUserQuestionBlock
                  key={pendingQuestionGroup.requestId}
                  group={pendingQuestionGroup}
                  onSubmitted={onQuestionSubmitted}
                />
              </div>
            )}
            {planCardVisible && planCardData && (
              <div className="flex justify-start">
                <div className="max-w-[85%] w-full">
                  <PlanCardBlock
                    data={planCardData}
                    planState={planState ?? "off"}
                    onOpenPanel={onOpenPlanPanel}
                    onApprove={onApprovePlan}
                    onExit={onExitPlan}
                  />
                </div>
              </div>
            )}
            {planSubagentRunning && (
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-blue-500/5 border border-blue-500/20 text-sm text-blue-600 dark:text-blue-400 animate-in fade-in slide-in-from-bottom-2 duration-300">
                <span className="animate-spin h-3.5 w-3.5 border-2 border-current border-t-transparent rounded-full shrink-0" />
                <span>{t("planMode.planningInProgress")}</span>
              </div>
            )}
            {showEmpty && (
              <div className="flex min-h-[50vh] items-center justify-center animate-in fade-in-0 duration-300">
                {incognito ? (
                  <div className="max-w-[360px] px-4 text-center text-muted-foreground">
                    <Ghost className="mx-auto mb-3 h-6 w-6" />
                    <div className="text-sm font-semibold text-foreground/70">
                      {t("chat.incognitoEmptyTitle")}
                    </div>
                    <p className="mt-2 text-sm leading-relaxed">{t("chat.incognitoEmptyBody")}</p>
                  </div>
                ) : (
                  <p className="text-muted-foreground text-sm">{t("chat.howCanIHelp")}</p>
                )}
              </div>
            )}
          </div>
        )}
        </div>
      </div>

      {showJumpToLatest && (
        <div className="pointer-events-none absolute inset-x-0 bottom-4 z-20 flex justify-center px-4">
          <button
            type="button"
            onClick={handleJumpToLatest}
            className="pointer-events-auto inline-flex h-9 w-9 cursor-pointer items-center justify-center rounded-full border border-border/70 bg-background/95 text-foreground shadow-lg shadow-black/10 backdrop-blur transition-colors hover:bg-muted"
            aria-label={t("chat.scrollToBottom")}
          >
            <ArrowDown className="h-4 w-4" />
          </button>
        </div>
      )}

      {contextMenu && (
        <MessageContextMenu
          contextMenu={contextMenu}
          onCopy={(index) => {
            const msg = messages[index]
            if (msg?.content) handleCopyMessage(msg.content, index)
          }}
          onClose={() => setContextMenu(null)}
        />
      )}
    </div>
  )
}

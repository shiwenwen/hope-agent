import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { ArrowDown, Ghost } from "lucide-react"
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso"
import { cn } from "@/lib/utils"
import { isCenteredSystemMessage } from "./chatUtils"
import MessageBubble from "./MessageBubble"
import MessageContextMenu from "./MessageContextMenu"
import LoadMoreRow from "./LoadMoreRow"
import AskUserQuestionBlock from "./ask-user/AskUserQuestionBlock"
import PlanCardBlock from "./plan-mode/PlanCardBlock"
import { getLatestUserTurnKey, getMessageRowKey } from "./chatScrollKeys"
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
  /**
   * Database id of a message to scroll into view (set when jumping from a
   * history search result). Cleared via `onScrollTargetHandled` once applied.
   */
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
  /** Jump to another session (e.g. a sub-agent's child session). */
  onSwitchSession?: (sessionId: string) => void
  /** Open the right-side diff panel for a file change payload. */
  onOpenDiff?: (
    metadata:
      | import("@/types/chat").FileChangeMetadata
      | import("@/types/chat").FileChangesMetadata,
  ) => void
}

interface FeedItem {
  msg: Message
  originalIndex: number
}

const INITIAL_FIRST_ITEM_INDEX = 1_000_000

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
  const virtuosoRef = useRef<VirtuosoHandle>(null)

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

  // Filter isMeta but preserve originalIndex (MessageBubble expects the original
  // messages-array index for hover/copy/context-menu wiring).
  const items = useMemo<FeedItem[]>(() => {
    const out: FeedItem[] = []
    messages.forEach((msg, originalIndex) => {
      if (!msg.isMeta) out.push({ msg, originalIndex })
    })
    return out
  }, [messages])

  // Tail-equal compare distinguishes prepend (handleLoadMore) from append (new
  // streaming message). On prepend we shift firstItemIndex down by the delta so
  // virtuoso keeps the visible item at the same absolute position — no jitter,
  // no manual scrollTop anchoring.
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

  // ref + state pair: ref defends against atBottomStateChange's debounced
  // staleness (used by the unseen-output check below); state drives the
  // jump-to-latest button render.
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

  // Force-following flag: overrides followOutput's atBottom check for ~600ms
  // after a programmatic scroll (forceFollowKey or jump-to-latest), so streaming
  // tokens that arrive right after still get auto-scrolled even when the
  // initial state was "not at bottom".
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

  const handledScrollTargetRef = useRef<number | null>(null)
  useEffect(() => {
    if (pendingScrollTarget == null) {
      handledScrollTargetRef.current = null
      return
    }
    if (handledScrollTargetRef.current === pendingScrollTarget) return
    const idx = items.findIndex((it) => it.msg.dbId === pendingScrollTarget)
    if (idx < 0) return
    const target = pendingScrollTarget
    handledScrollTargetRef.current = target
    virtuosoRef.current?.scrollToIndex({
      index: idx,
      align: "center",
    })
    // eslint-disable-next-line react-hooks/set-state-in-effect -- one-shot reaction to a parent-driven request token
    setHighlightMessageId(target)
    if (highlightTimerRef.current) clearTimeout(highlightTimerRef.current)
    highlightTimerRef.current = setTimeout(() => setHighlightMessageId(null), 2000)
    onScrollTargetHandled?.()
  }, [pendingScrollTarget, onScrollTargetHandled, items])

  useEffect(
    () => () => {
      if (highlightTimerRef.current) clearTimeout(highlightTimerRef.current)
      if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
      if (forceFollowResetRef.current) clearTimeout(forceFollowResetRef.current)
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

  const handleStartReached = useCallback(() => {
    if (!hasMore || loadingMore) return
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

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, index: number) => {
      const msg = messages[index]
      if (msg.role !== "assistant" || !msg.content) return
      e.preventDefault()
      setContextMenu({ x: e.clientX, y: e.clientY, index })
    },
    [messages],
  )

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

  const computeItemKey = useCallback(
    (_index: number, item: FeedItem) => getMessageRowKey(item.msg, item.originalIndex),
    [],
  )

  const itemContent = useCallback(
    (_absoluteIndex: number, item: FeedItem) => {
      const { msg, originalIndex } = item
      return (
        <div
          data-message-id={msg.dbId ?? undefined}
          className={cn(
            "flex rounded-lg pb-4 transition-colors",
            msg.dbId === highlightMessageId && "message-hit-pulse",
            isCenteredSystemMessage(msg)
              ? "justify-center"
              : msg.role === "user" && !msg.fromAgentId
                ? "justify-end"
                : "justify-start",
            originalIndex === messages.length - 1 && "animate-fade-slide-in",
          )}
        >
          <MessageBubble
            msg={msg}
            index={originalIndex}
            isLast={originalIndex === messages.length - 1}
            loading={loading}
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
    },
    [
      agents,
      copiedIndex,
      handleContextMenu,
      handleCopyMessage,
      highlightMessageId,
      hoveredMsgIndex,
      loading,
      messages.length,
      onOpenDiff,
      onOpenPlanPanel,
      onSwitchModel,
      onSwitchSession,
      onViewSystemPrompt,
      sessionId,
    ],
  )

  const Header = useCallback(
    () =>
      hasMore ? (
        <div className="pt-6">
          <LoadMoreRow loadingMore={loadingMore} onLoadMore={onLoadMore} />
        </div>
      ) : null,
    [hasMore, loadingMore, onLoadMore],
  )

  const planCardVisible = Boolean(
    planCardData && planState && planState !== "off" && planState !== "planning",
  )

  const Footer = useCallback(
    () =>
      pendingQuestionGroup || planCardVisible || planSubagentRunning ? (
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
        </div>
      ) : null,
    [
      onApprovePlan,
      onExitPlan,
      onOpenPlanPanel,
      onQuestionSubmitted,
      pendingQuestionGroup,
      planCardData,
      planCardVisible,
      planState,
      planSubagentRunning,
      t,
    ],
  )

  const components = useMemo(() => ({ Header, Footer }), [Header, Footer])

  const showJumpToLatest = !isAtBottom && (loading || hasUnseenOutput)
  const showEmpty =
    messages.length === 0 && !pendingQuestionGroup && !planCardVisible && !planSubagentRunning

  return (
    <div className="relative flex-1 min-h-0">
      {showEmpty ? (
        <div className="flex h-full items-center justify-center animate-in fade-in-0 duration-300">
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
      ) : (
        <Virtuoso
          key={sessionId ?? "draft-session"}
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
          defaultItemHeight={120}
          increaseViewportBy={{ top: 200, bottom: 400 }}
          atBottomThreshold={48}
        />
      )}

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

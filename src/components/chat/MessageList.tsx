import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { useVirtualFeed } from "@/components/common/useVirtualFeed"
import MessageBubble from "./MessageBubble"
import MessageContextMenu from "./MessageContextMenu"
import LoadMoreRow from "./LoadMoreRow"
import AskUserQuestionBlock from "./ask-user/AskUserQuestionBlock"
import PlanCardBlock from "./plan-mode/PlanCardBlock"
import type { AskUserQuestionGroup } from "./ask-user/AskUserQuestionBlock"
import type { PlanCardData } from "./plan-mode/PlanCardBlock"
import type { Message, AgentSummaryForSidebar } from "@/types/chat"
import type { PlanModeState, PlanStep } from "./plan-mode/usePlanMode"

interface MessageListProps {
  messages: Message[]
  loading: boolean
  agents: AgentSummaryForSidebar[]
  hasMore: boolean
  loadingMore: boolean
  onLoadMore: () => void | Promise<void>
  // Plan mode
  sessionId?: string | null
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
  planSteps?: PlanStep[]
  onOpenPlanPanel?: () => void
  onApprovePlan?: () => void
  onExitPlan?: () => void
  onPausePlan?: () => void
  onResumePlan?: () => void
  planSubagentRunning?: boolean
  onSwitchModel?: (providerId: string, modelId: string) => void
  onViewSystemPrompt?: () => void
  /** Jump to another session (e.g. a sub-agent's child session). */
  onSwitchSession?: (sessionId: string) => void
}

type ChatRow =
  | { type: "loadMore"; key: "load-more" }
  | { type: "empty"; key: "empty" }
  | { type: "message"; key: string; msg: Message; originalIndex: number }
  | { type: "askUser"; key: string; group: AskUserQuestionGroup }
  | { type: "planCard"; key: string; data: PlanCardData }
  | { type: "planRunning"; key: "plan-running" }

function getMessageRowKey(msg: Message, originalIndex: number): string {
  if (typeof msg.dbId === "number") return `message:${msg.dbId}`
  return `message:${msg.role}:${msg.timestamp ?? "pending"}:${originalIndex}`
}

export default function MessageList({
  messages,
  loading,
  agents,
  hasMore,
  loadingMore,
  onLoadMore,
  sessionId,
  pendingScrollTarget,
  onScrollTargetHandled,
  pendingQuestionGroup,
  onQuestionSubmitted,
  planCardData,
  planState,
  planSteps,
  onOpenPlanPanel,
  onApprovePlan,
  onExitPlan,
  onPausePlan,
  onResumePlan,
  planSubagentRunning,
  onSwitchModel,
  onViewSystemPrompt,
  onSwitchSession,
}: MessageListProps) {
  const { t } = useTranslation()
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

  const rows = useMemo<ChatRow[]>(() => {
    const next: ChatRow[] = []
    if (hasMore) next.push({ type: "loadMore", key: "load-more" })
    if (messages.length === 0) next.push({ type: "empty", key: "empty" })

    messages.forEach((msg, originalIndex) => {
      if (msg.isMeta) return
      next.push({
        type: "message",
        key: getMessageRowKey(msg, originalIndex),
        msg,
        originalIndex,
      })
    })

    if (pendingQuestionGroup) {
      next.push({
        type: "askUser",
        key: `ask-user:${pendingQuestionGroup.requestId}`,
        group: pendingQuestionGroup,
      })
    }

    if (planCardData && planState && planState !== "off" && planState !== "planning") {
      next.push({
        type: "planCard",
        key: `plan-card:${planCardData.sessionId}`,
        data: planCardData,
      })
    }

    if (planSubagentRunning) {
      next.push({ type: "planRunning", key: "plan-running" })
    }

    return next
  }, [hasMore, messages, pendingQuestionGroup, planCardData, planState, planSubagentRunning])

  const getRowKey = useCallback((row: ChatRow) => row.key, [])
  const canAnchorRow = useCallback((row: ChatRow) => row.type === "message", [])
  const estimateSize = useCallback(
    (index: number) => {
      const row = rows[index]
      if (!row) return 120
      if (row.type === "loadMore") return 40
      if (row.type === "empty") return 240
      if (row.type === "planRunning") return 52
      if (row.type === "askUser" || row.type === "planCard") return 180
      if (row.msg.role === "user") return 76
      if (row.msg.role === "event" || row.msg.isSubagentResult || row.msg.isCronTrigger) return 48
      return 120
    },
    [rows],
  )

  const lastMsg = messages[messages.length - 1]
  const followKey = `${messages.length}:${lastMsg?.role ?? ""}:${lastMsg?.content.length ?? 0}:${lastMsg?.contentBlocks?.length ?? 0}`
  const { scrollRef, virtualizer, virtualItems, totalSize } = useVirtualFeed({
    rows,
    getRowKey,
    estimateSize,
    overscan: 8,
    gap: 16,
    paddingStart: 24,
    paddingEnd: 24,
    followOutput: loading,
    followKey,
    resetKey: sessionId ?? "draft-session",
    canAnchorRow,
    onStartReached: onLoadMore,
    canLoadMore: hasMore,
    loadingMore,
    startThreshold: 50,
  })

  // Close context menu on outside click or scroll
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

  useEffect(() => {
    if (pendingScrollTarget === null || pendingScrollTarget === undefined) return
    const targetIndex = rows.findIndex(
      (row) => row.type === "message" && row.msg.dbId === pendingScrollTarget,
    )
    if (targetIndex < 0) return

    const target = pendingScrollTarget
    virtualizer.scrollToIndex(targetIndex, { align: "center" })
    const frame = requestAnimationFrame(() => {
      setHighlightMessageId(target)
      if (highlightTimerRef.current) clearTimeout(highlightTimerRef.current)
      highlightTimerRef.current = setTimeout(() => setHighlightMessageId(null), 2000)
    })
    onScrollTargetHandled?.()

    return () => {
      cancelAnimationFrame(frame)
      if (highlightTimerRef.current) {
        clearTimeout(highlightTimerRef.current)
        highlightTimerRef.current = null
      }
    }
  }, [onScrollTargetHandled, pendingScrollTarget, rows, virtualizer])

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

  const renderRow = (row: ChatRow) => {
    switch (row.type) {
      case "loadMore":
        return <LoadMoreRow loadingMore={loadingMore} onLoadMore={onLoadMore} />
      case "empty":
        return (
          <div className="flex min-h-[50vh] items-center justify-center animate-in fade-in-0 duration-300">
            <p className="text-muted-foreground text-sm">{t("chat.howCanIHelp")}</p>
          </div>
        )
      case "message": {
        const { msg, originalIndex } = row
        return (
          <div
            data-message-id={msg.dbId ?? undefined}
            className={cn(
              "flex rounded-lg transition-colors",
              msg.dbId === highlightMessageId && "message-hit-pulse",
              msg.role === "event" || msg.isSubagentResult || msg.isCronTrigger
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
            />
          </div>
        )
      }
      case "askUser":
        return (
          <div className="w-full">
            <AskUserQuestionBlock
              key={row.group.requestId}
              group={row.group}
              onSubmitted={onQuestionSubmitted}
            />
          </div>
        )
      case "planCard":
        return (
          <div className="flex justify-start">
            <div className="max-w-[85%] w-full">
              <PlanCardBlock
                data={{
                  ...row.data,
                  steps: planSteps || row.data.steps,
                }}
                planState={planState}
                onOpenPanel={onOpenPlanPanel}
                onApprove={onApprovePlan}
                onExit={onExitPlan}
                onPause={onPausePlan}
                onResume={onResumePlan}
              />
            </div>
          </div>
        )
      case "planRunning":
        return (
          <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-blue-500/5 border border-blue-500/20 text-sm text-blue-600 dark:text-blue-400 animate-in fade-in slide-in-from-bottom-2 duration-300">
            <span className="animate-spin h-3.5 w-3.5 border-2 border-current border-t-transparent rounded-full shrink-0" />
            <span>{t("planMode.planningInProgress")}</span>
          </div>
        )
    }
  }

  return (
    <div ref={scrollRef} className="flex-1 overflow-y-auto px-4">
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

      {/* Right-click context menu for assistant messages */}
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

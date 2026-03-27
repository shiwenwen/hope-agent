import { useState, useRef, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import MessageBubble from "./MessageBubble"
import MessageContextMenu from "./MessageContextMenu"
import PlanQuestionBlock from "./plan-mode/PlanQuestionBlock"
import PlanCardBlock from "./plan-mode/PlanCardBlock"
import type { PlanQuestionGroup } from "./plan-mode/PlanQuestionBlock"
import type { PlanCardData } from "./plan-mode/PlanCardBlock"
import type { Message, AgentSummaryForSidebar } from "@/types/chat"
import type { PlanModeState, PlanStep } from "./plan-mode/usePlanMode"

interface MessageListProps {
  messages: Message[]
  loading: boolean
  agents: AgentSummaryForSidebar[]
  hasMore: boolean
  loadingMore: boolean
  onLoadMore: () => void
  scrollContainerRef: React.RefObject<HTMLDivElement | null>
  bottomRef: React.RefObject<HTMLDivElement | null>
  // Plan mode
  pendingQuestionGroup?: PlanQuestionGroup | null
  onQuestionSubmitted?: () => void
  planCardData?: PlanCardData | null
  planState?: PlanModeState
  planSteps?: PlanStep[]
  onOpenPlanPanel?: () => void
  onApprovePlan?: () => void
  onExitPlan?: () => void
  onPausePlan?: () => void
  onResumePlan?: () => void
}

export default function MessageList({
  messages,
  loading,
  agents,
  hasMore,
  loadingMore,
  onLoadMore,
  scrollContainerRef,
  bottomRef,
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
}: MessageListProps) {
  const { t } = useTranslation()
  const [hoveredMsgIndex, setHoveredMsgIndex] = useState<number | null>(null)
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null)
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [contextMenu, setContextMenu] = useState<{
    x: number
    y: number
    index: number
  } | null>(null)

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

  function handleContextMenu(e: React.MouseEvent, index: number) {
    const msg = messages[index]
    if (msg.role !== "assistant" || !msg.content) return
    e.preventDefault()
    setContextMenu({ x: e.clientX, y: e.clientY, index })
  }

  function handleCopyMessage(content: string, index: number) {
    navigator.clipboard
      .writeText(content)
      .then(() => {
        if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
        setCopiedIndex(index)
        copiedTimerRef.current = setTimeout(() => setCopiedIndex(null), 1500)
      })
      .catch(() => {})
  }


  return (
    <div ref={scrollContainerRef} className="flex-1 overflow-y-auto px-4 py-6 space-y-4">
      {/* Load more indicator */}
      {hasMore && (
        <div className="flex justify-center py-2">
          {loadingMore ? (
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <div className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-muted-foreground border-t-transparent" />
              {t("chat.loadingMore")}
            </div>
          ) : (
            <button
              onClick={onLoadMore}
              className="text-xs text-muted-foreground hover:text-foreground transition-colors"
            >
              {t("chat.loadMore")}
            </button>
          )}
        </div>
      )}
      {messages.length === 0 && (
        <div className="flex items-center justify-center h-full animate-in fade-in-0 duration-300">
          <p className="text-muted-foreground text-sm">{t("chat.howCanIHelp")}</p>
        </div>
      )}
      {messages.map((msg, i) => (
        <div
          key={i}
          className={cn(
            "flex",
            msg.role === "event" || msg.isSubagentResult
              ? "justify-center"
              : msg.role === "user" && !msg.fromAgentId
                ? "justify-end"
                : "justify-start",
            i === messages.length - 1 && "animate-fade-slide-in",
          )}
        >
          <MessageBubble
            msg={msg}
            index={i}
            isLast={i === messages.length - 1}
            loading={loading}
            agents={agents}
            hoveredMsgIndex={hoveredMsgIndex}
            onHover={setHoveredMsgIndex}
            onContextMenu={handleContextMenu}
            copiedIndex={copiedIndex}
            onCopy={handleCopyMessage}
          />
        </div>
      ))}
      {/* Plan Question Block (interactive Q&A during planning) */}
      {pendingQuestionGroup && (
        <div className="flex justify-start">
          <div className="max-w-[85%] w-full">
            <PlanQuestionBlock
              group={pendingQuestionGroup}
              onSubmitted={onQuestionSubmitted}
            />
          </div>
        </div>
      )}

      {/* Plan Card Block (plan summary after submit_plan) */}
      {planCardData && planState && planState !== "off" && planState !== "planning" && (
        <div className="flex justify-start">
          <div className="max-w-[85%] w-full">
            <PlanCardBlock
              data={{
                ...planCardData,
                steps: planSteps || planCardData.steps,
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
      )}

      <div ref={bottomRef} />

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

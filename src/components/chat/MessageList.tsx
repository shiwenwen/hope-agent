import { useState, useRef, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import MessageBubble from "./MessageBubble"
import MessageContextMenu from "./MessageContextMenu"
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
  onLoadMore: () => void
  scrollContainerRef: React.RefObject<HTMLDivElement | null>
  bottomRef: React.RefObject<HTMLDivElement | null>
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

  // Scroll a specific message into view + highlight it (used when jumping
  // from a history search result). Runs whenever the pending target or the
  // message list changes — retries on the next tick until the DOM node is
  // present, then clears the pending target.
  useEffect(() => {
    if (pendingScrollTarget === null || pendingScrollTarget === undefined) return
    if (messages.length === 0) return

    const target = pendingScrollTarget
    const container = scrollContainerRef.current
    if (!container) return

    const el = container.querySelector<HTMLElement>(
      `[data-message-id="${target}"]`,
    )
    if (!el) return

    el.scrollIntoView({ behavior: "smooth", block: "center" })
    el.classList.add("message-hit-pulse")
    const timer = setTimeout(() => {
      el.classList.remove("message-hit-pulse")
    }, 2000)

    onScrollTargetHandled?.()
    return () => clearTimeout(timer)
  }, [pendingScrollTarget, messages, scrollContainerRef, onScrollTargetHandled])

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
          key={msg.dbId ?? `${msg.role}-${msg.timestamp ?? i}`}
          data-message-id={msg.dbId ?? undefined}
          className={cn(
            "flex rounded-lg transition-colors",
            msg.role === "event" || msg.isSubagentResult || msg.isCronTrigger
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
            sessionId={sessionId}
            onOpenPlanPanel={onOpenPlanPanel}
            onSwitchModel={onSwitchModel}
            onViewSystemPrompt={onViewSystemPrompt}
          />
        </div>
      ))}
      {/* Ask-user Question Block (interactive Q&A) */}
      {pendingQuestionGroup && (
        <div className="flex justify-start">
          <div className="max-w-[85%] w-full">
            <AskUserQuestionBlock
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

      {/* Plan sub-agent running indicator */}
      {planSubagentRunning && (
        <div className="flex items-center gap-2 mx-4 mb-2 px-3 py-2 rounded-lg bg-blue-500/5 border border-blue-500/20 text-sm text-blue-600 dark:text-blue-400 animate-in fade-in slide-in-from-bottom-2 duration-300">
          <span className="animate-spin h-3.5 w-3.5 border-2 border-current border-t-transparent rounded-full shrink-0" />
          <span>{t("planMode.planningInProgress")}</span>
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

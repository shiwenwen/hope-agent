/**
 * PlanDetachedWindow — root component for the independent plan Tauri window.
 * Rendered when `?window=plan` is in the URL (see main.tsx).
 * Receives sessionId via URL search param.
 */
import { useEffect, useCallback, useMemo, useRef, useState } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { useTranslation } from "react-i18next"
import { initLanguageFromConfig } from "@/i18n/i18n"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { usePlanMode } from "@/components/chat/plan-mode/usePlanMode"
import { groupStepsByPhase } from "@/components/chat/plan-mode/planParser"
import { PlanStepItem } from "@/components/chat/plan-mode/PlanStepItem"
import { CommentPopover } from "@/components/chat/plan-mode/CommentPopover"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import {
  ClipboardList,
  Play,
  Loader2,
  CheckCircle,
  MessageSquareQuote,
  Pause,
  X,
} from "lucide-react"

export default function PlanDetachedWindow() {
  const { t } = useTranslation()

  // Get sessionId from URL
  const sessionId = new URLSearchParams(window.location.search).get("sessionId")

  // Init language
  useEffect(() => {
    initLanguageFromConfig()
  }, [])

  const planMode = usePlanMode(sessionId)

  const {
    planState,
    planSteps,
    planContent,
    progress,
    completedCount,
    setPlanState,
    exitPlanMode,
    approvePlan,
    pauseExecution,
    resumeExecution,
  } = planMode

  const handleClose = useCallback(() => {
    getCurrentWindow().close()
  }, [])

  const contentRef = useRef<HTMLDivElement>(null)
  const [commentPopover, setCommentPopover] = useState<{
    position: { top: number; left: number }
    selectedText: string
  } | null>(null)

  const groupedPhases = useMemo(() => groupStepsByPhase(planSteps), [planSteps])

  const allDone =
    planSteps.length > 0 &&
    planSteps.every(
      (s) => s.status === "completed" || s.status === "skipped" || s.status === "failed",
    )

  const showProgressBar =
    planState === "executing" || planState === "paused" || planState === "completed" || allDone
  const showMarkdown = planContent && (planState === "review" || planState === "planning")
  const showStepList =
    planState === "executing" || planState === "paused" || planState === "completed"
  const canComment = (planState === "review" || planState === "planning") && !!sessionId

  const iconColor =
    planState === "completed"
      ? "text-green-500"
      : planState === "executing"
        ? "text-blue-500"
        : planState === "paused"
          ? "text-yellow-500"
          : planState === "review"
            ? "text-purple-500"
            : "text-blue-500"

  const highlightSelection = useCallback((range: Range) => {
    try {
      const mark = document.createElement("mark")
      mark.className = "bg-blue-200/50 dark:bg-blue-500/30 rounded-sm plan-comment-highlight"
      range.surroundContents(mark)
    } catch {
      const treeWalker = document.createTreeWalker(
        range.commonAncestorContainer,
        NodeFilter.SHOW_TEXT,
      )
      const textNodes: Text[] = []
      while (treeWalker.nextNode()) {
        const node = treeWalker.currentNode as Text
        if (range.intersectsNode(node)) textNodes.push(node)
      }
      for (const node of textNodes) {
        const mark = document.createElement("mark")
        mark.className = "bg-blue-200/50 dark:bg-blue-500/30 rounded-sm plan-comment-highlight"
        node.parentNode?.insertBefore(mark, node)
        mark.appendChild(node)
      }
    }
  }, [])

  const clearHighlight = useCallback(() => {
    if (!contentRef.current) return
    const marks = contentRef.current.querySelectorAll("mark.plan-comment-highlight")
    marks.forEach((mark) => {
      const parent = mark.parentNode
      if (parent) {
        while (mark.firstChild) parent.insertBefore(mark.firstChild, mark)
        parent.removeChild(mark)
      }
    })
  }, [])

  const handleMouseUp = useCallback(() => {
    if (!canComment || !contentRef.current) return
    const selection = window.getSelection()
    if (!selection || selection.isCollapsed || !selection.toString().trim()) return

    const selectedText = selection.toString().trim()
    const range = selection.getRangeAt(0)
    if (!contentRef.current.contains(range.commonAncestorContainer)) return

    const rect = range.getBoundingClientRect()
    const containerRect = contentRef.current.getBoundingClientRect()
    const top = rect.bottom - containerRect.top + contentRef.current.scrollTop + 4
    let left = rect.left - containerRect.left
    left = Math.max(0, Math.min(left, contentRef.current.clientWidth - 280))

    clearHighlight()
    highlightSelection(range.cloneRange())
    selection.removeAllRanges()
    setCommentPopover({ position: { top, left }, selectedText })
  }, [canComment, clearHighlight, highlightSelection])

  useEffect(() => {
    const handleMouseDown = () => {
      if (!commentPopover) return
      clearHighlight()
      setCommentPopover(null)
    }
    document.addEventListener("mousedown", handleMouseDown)
    return () => document.removeEventListener("mousedown", handleMouseDown)
  }, [commentPopover, clearHighlight])

  const handleCommentSubmit = useCallback(
    async (comment: string) => {
      if (!commentPopover || !sessionId) return
      const feedback = [
        `<plan-inline-comment>`,
        `The user selected the following section from the current plan and requests a revision:`,
        ``,
        `<selected-text>`,
        commentPopover.selectedText,
        `</selected-text>`,
        ``,
        `<revision-request>`,
        comment,
        `</revision-request>`,
        ``,
        `Please revise the plan to address this feedback. Modify the quoted section while keeping the rest of the plan intact, then resubmit the updated plan using the submit_plan tool.`,
        `</plan-inline-comment>`,
      ].join("\n")
      clearHighlight()
      setCommentPopover(null)
      window.getSelection()?.removeAllRanges()
      setPlanState("planning")
      try {
        await getTransport().call("set_plan_mode", { sessionId, state: "planning" })
        await getTransport().startChat(
          {
            message: feedback,
            attachments: [],
            sessionId,
            planMode: "planning",
            displayText: comment,
          },
          () => {},
        )
      } catch (e) {
        logger.error("plan", "PlanDetachedWindow::comment", "Failed to submit plan comment", e)
      }
    },
    [clearHighlight, commentPopover, sessionId, setPlanState],
  )

  return (
    <TooltipProvider>
      <div className="flex flex-col h-screen bg-background text-foreground">
        {/* Title bar - draggable */}
        <div
          className="flex items-center gap-2 px-3 py-2 pt-8 border-b border-border bg-secondary/30 shrink-0"
          data-tauri-drag-region
        >
          <ClipboardList className={cn("h-4 w-4", iconColor)} />
          <span className="text-sm font-medium truncate flex-1">{t("planMode.panelTitle")}</span>
          <IconTip label={t("common.close")}>
            <button
              className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
              onClick={handleClose}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </div>

        {/* Progress bar */}
        {showProgressBar && planSteps.length > 0 && (
          <div className="px-3 py-2 border-b border-border/50">
            <div className="flex items-center justify-between text-xs text-muted-foreground mb-1">
              <span>
                {completedCount}/{planSteps.length} {t("planMode.stepsCompleted")}
              </span>
              <span>{progress}%</span>
            </div>
            <div className="h-1.5 bg-secondary rounded-full overflow-hidden">
              <div
                className={cn(
                  "h-full rounded-full transition-all duration-500 ease-out",
                  planState === "completed" || allDone
                    ? "bg-green-500"
                    : planState === "paused"
                      ? "bg-yellow-500"
                      : "bg-blue-500",
                )}
                style={{ width: `${progress}%` }}
              />
            </div>
          </div>
        )}

        {/* Paused banner */}
        {planState === "paused" && (
          <div className="px-3 py-2 bg-yellow-500/10 border-b border-yellow-500/20 text-sm text-yellow-600 flex items-center gap-2">
            <Pause className="h-3.5 w-3.5" />
            {t("planMode.pausedBanner")}
          </div>
        )}

        {canComment && showMarkdown && (
          <div className="px-3 py-1.5 bg-blue-500/5 border-b border-blue-500/10 text-[11px] text-muted-foreground flex items-center gap-1.5">
            <MessageSquareQuote className="h-3 w-3 shrink-0 text-blue-500/60" />
            {t("planMode.comment.hint")}
          </div>
        )}

        {/* Main content area */}
        <div
          className="flex-1 overflow-y-auto relative"
          ref={contentRef}
          onMouseUp={canComment ? handleMouseUp : undefined}
        >
          {showMarkdown && (
            <div className={cn("px-3 py-3", canComment && "select-text cursor-text")}>
              <div className="text-sm leading-relaxed">
                <MarkdownRenderer content={planContent} />
              </div>
            </div>
          )}

          {planState === "planning" && !planContent && (
            <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
              <ClipboardList className="h-8 w-8 mb-3 opacity-30" />
              <span className="text-sm">{t("planMode.planning")}</span>
            </div>
          )}

          {showStepList && (
            <div className="px-3 py-2 space-y-1">
              {groupedPhases.map((phase) => (
                <div key={phase.name} className="mb-3">
                  <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-1.5 px-1">
                    {phase.name}
                  </div>
                  {phase.steps.map((step) => (
                    <PlanStepItem key={step.index} step={step} detailed />
                  ))}
                </div>
              ))}
              {planSteps.length === 0 && (
                <div className="text-sm text-muted-foreground text-center py-8">
                  {t("planMode.noSteps")}
                </div>
              )}
            </div>
          )}

          {commentPopover && (
            <CommentPopover
              position={commentPopover.position}
              selectedText={commentPopover.selectedText}
              onSubmit={handleCommentSubmit}
              onClose={() => {
                clearHighlight()
                setCommentPopover(null)
                window.getSelection()?.removeAllRanges()
              }}
            />
          )}
        </div>

        {/* Action bar */}
        <div className="px-3 py-3 border-t border-border bg-secondary/20 shrink-0 space-y-2">
          {planState === "planning" && (
            <Button variant="ghost" className="w-full" onClick={exitPlanMode}>
              {t("planMode.exitWithout")}
            </Button>
          )}

          {planState === "review" && (
            <>
              <Button
                className="w-full bg-blue-600 hover:bg-blue-700 text-white"
                onClick={approvePlan}
              >
                <Play className="h-4 w-4 mr-2" />
                {t("planMode.approveAndExecute")}
              </Button>
              <Button variant="ghost" className="w-full" onClick={exitPlanMode}>
                {t("planMode.exitWithout")}
              </Button>
            </>
          )}

          {planState === "executing" && !allDone && (
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2 text-sm text-blue-600">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>{t("planMode.executing")}</span>
              </div>
              <Button size="sm" variant="outline" onClick={pauseExecution} className="gap-1.5">
                <Pause className="h-3.5 w-3.5" />
                {t("planMode.pause")}
              </Button>
            </div>
          )}

          {planState === "paused" && (
            <>
              <Button
                className="w-full bg-yellow-600 hover:bg-yellow-700 text-white"
                onClick={resumeExecution}
              >
                <Play className="h-4 w-4 mr-2" />
                {t("planMode.resume")}
              </Button>
              <Button variant="ghost" className="w-full" onClick={exitPlanMode}>
                {t("planMode.exitWithout")}
              </Button>
            </>
          )}

          {(planState === "completed" || allDone) && (
            <>
              <div className="flex items-center gap-2 text-sm text-green-600">
                <CheckCircle className="h-4 w-4" />
                <span>{t("planMode.completed")}</span>
              </div>
              <Button variant="ghost" className="w-full" onClick={exitPlanMode}>
                {t("planMode.exitWithout")}
              </Button>
            </>
          )}
        </div>
      </div>
    </TooltipProvider>
  )
}

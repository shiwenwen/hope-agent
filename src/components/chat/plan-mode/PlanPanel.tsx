import { useMemo, useEffect, useState, useCallback, useRef } from "react"
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"
import { invoke } from "@tauri-apps/api/core"
import { logger } from "@/lib/logger"
import {
  ClipboardList,
  X,
  Play,
  Loader2,
  CheckCircle,
  Pause,
  History,
  RotateCcw,
  Send,
  MessageSquareQuote,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { IconTip } from "@/components/ui/tooltip"
import { useTranslation } from "react-i18next"
import { groupStepsByPhase } from "./planParser"
import { PlanStepItem } from "./PlanStepItem"
import type { PlanModeState, PlanStep } from "./usePlanMode"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"

interface PlanPanelProps {
  planState: PlanModeState
  planSteps: PlanStep[]
  planContent: string
  progress: number
  completedCount: number
  sessionId: string | null
  onApprove: () => void
  onExit: () => void
  onClose: () => void
  onPause?: () => void
  onResume?: () => void
  onRequestChanges?: (feedback: string) => void
}

/** Floating comment popover shown when user selects text in the plan */
function CommentPopover({
  position,
  selectedText,
  onSubmit,
  onClose,
}: {
  position: { top: number; left: number }
  selectedText: string
  onSubmit: (comment: string) => void
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [comment, setComment] = useState("")
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  useEffect(() => {
    // Auto-focus textarea on mount
    setTimeout(() => textareaRef.current?.focus(), 50)
  }, [])

  const handleSubmit = useCallback(() => {
    if (!comment.trim()) return
    onSubmit(comment.trim())
    setComment("")
  }, [comment, onSubmit])

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault()
      handleSubmit()
    }
    if (e.key === "Escape") {
      e.preventDefault()
      onClose()
    }
  }, [handleSubmit, onClose])

  return (
    <div
      className="absolute z-50 w-[280px] rounded-lg border border-border bg-popover shadow-lg animate-in fade-in zoom-in-95 duration-150"
      style={{ top: position.top, left: position.left }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <div className="px-3 py-2 border-b border-border/50 bg-secondary/30 rounded-t-lg">
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <MessageSquareQuote className="h-3 w-3 shrink-0" />
          <span className="truncate italic">&ldquo;{selectedText.length > 60 ? selectedText.slice(0, 60) + "…" : selectedText}&rdquo;</span>
        </div>
      </div>
      <div className="p-2 space-y-2">
        <Textarea
          ref={textareaRef}
          value={comment}
          onChange={(e) => setComment(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t("planMode.comment.placeholder")}
          className="text-sm min-h-[48px] max-h-[120px] resize-none border-border/50"
          rows={2}
        />
        <div className="flex items-center justify-between">
          <span className="text-[10px] text-muted-foreground">
            {t("planMode.comment.shortcut")}
          </span>
          <div className="flex gap-1.5">
            <Button size="sm" variant="ghost" className="h-7 px-2 text-xs" onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              className="h-7 px-2.5 text-xs gap-1"
              disabled={!comment.trim()}
              onClick={handleSubmit}
            >
              <Send className="h-3 w-3" />
              {t("planMode.comment.submit")}
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}

export function PlanPanel({
  planState,
  planSteps,
  planContent,
  progress,
  completedCount,
  sessionId,
  onApprove,
  onExit,
  onClose,
  onPause,
  onResume,
  onRequestChanges,
}: PlanPanelProps) {
  const { t } = useTranslation()
  const [showVersions, setShowVersions] = useState(false)
  const [versions, setVersions] = useState<{ version: number; filePath: string; modifiedAt: string; isCurrent: boolean }[]>([])
  const [loadingVersions, setLoadingVersions] = useState(false)
  const [hasCheckpoint, setHasCheckpoint] = useState(false)
  const [rollingBack, setRollingBack] = useState(false)

  // Comment popover state
  const [commentPopover, setCommentPopover] = useState<{
    position: { top: number; left: number }
    selectedText: string
  } | null>(null)
  const contentRef = useRef<HTMLDivElement>(null)

  // Adjust window min size
  useEffect(() => {
    const win = getCurrentWindow()
    win.setMinSize(new LogicalSize(1240, 480))
    return () => {
      win.setMinSize(new LogicalSize(840, 480))
    }
  }, [])

  const handleLoadVersions = useCallback(async () => {
    if (!sessionId) return
    setLoadingVersions(true)
    try {
      const v = await invoke<{ version: number; filePath: string; modifiedAt: string; isCurrent: boolean }[]>(
        "get_plan_versions", { sessionId }
      )
      setVersions(v)
      setShowVersions(true)
    } catch (e) {
      logger.error("plan", "PlanPanel::loadVersions", "Failed to load plan versions", e)
    } finally {
      setLoadingVersions(false)
    }
  }, [sessionId])

  // Check for git checkpoint availability
  useEffect(() => {
    if (!sessionId) return
    if (planState === "executing" || planState === "paused" || planState === "completed") {
      invoke<string | null>("get_plan_checkpoint", { sessionId })
        .then((ref) => setHasCheckpoint(!!ref))
        .catch(() => setHasCheckpoint(false))
    } else {
      setHasCheckpoint(false)
    }
  }, [sessionId, planState])

  const handleRollback = useCallback(async () => {
    if (!sessionId) return
    setRollingBack(true)
    try {
      const msg = await invoke<string>("plan_rollback", { sessionId })
      logger.info("plan", "PlanPanel::rollback", "Rollback result", msg)
      setHasCheckpoint(false)
    } catch (e) {
      logger.error("plan", "PlanPanel::rollback", "Failed to rollback", e)
    } finally {
      setRollingBack(false)
    }
  }, [sessionId])

  const handleRestoreVersion = useCallback(async (filePath: string) => {
    if (!sessionId) return
    try {
      await invoke("restore_plan_version", { sessionId, filePath })
      setShowVersions(false)
    } catch (e) {
      logger.error("plan", "PlanPanel::restoreVersion", "Failed to restore plan version", e)
    }
  }, [sessionId])

  // Handle text selection for inline commenting
  const handleMouseUp = useCallback(() => {
    if (!contentRef.current) return
    // Only allow commenting in review/planning states
    if (planState !== "review" && planState !== "planning") return

    const selection = window.getSelection()
    if (!selection || selection.isCollapsed || !selection.toString().trim()) {
      return
    }

    const selectedText = selection.toString().trim()
    if (!selectedText) return

    // Check if selection is within the content area
    const range = selection.getRangeAt(0)
    if (!contentRef.current.contains(range.commonAncestorContainer)) return

    // Calculate position relative to the content container
    const rect = range.getBoundingClientRect()
    const containerRect = contentRef.current.getBoundingClientRect()

    // Position the popover below the selection, clamped within container bounds
    const top = rect.bottom - containerRect.top + contentRef.current.scrollTop + 4
    let left = rect.left - containerRect.left
    // Clamp to prevent overflow (popover is 280px wide)
    left = Math.max(0, Math.min(left, contentRef.current.clientWidth - 280))

    setCommentPopover({ position: { top, left }, selectedText })
  }, [planState])

  // Close comment popover when clicking outside or selection changes
  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      // Don't close if clicking inside the popover (handled by stopPropagation there)
      if (commentPopover) {
        setCommentPopover(null)
      }
    }
    // Use mousedown on document to dismiss
    document.addEventListener("mousedown", handleMouseDown)
    return () => document.removeEventListener("mousedown", handleMouseDown)
  }, [commentPopover])

  // Submit comment: format as quoted selection + comment and send to model
  const handleCommentSubmit = useCallback((comment: string) => {
    if (!commentPopover || !onRequestChanges) return
    const feedback = `> ${commentPopover.selectedText}\n\n${comment}`
    onRequestChanges(feedback)
    setCommentPopover(null)
    window.getSelection()?.removeAllRanges()
  }, [commentPopover, onRequestChanges])

  const groupedPhases = useMemo(
    () => groupStepsByPhase(planSteps),
    [planSteps]
  )

  const allDone =
    planSteps.length > 0 &&
    planSteps.every(
      (s) =>
        s.status === "completed" ||
        s.status === "skipped" ||
        s.status === "failed"
    )

  const showProgressBar = planState === "executing" || planState === "paused" || planState === "completed" || allDone
  // Show markdown content in review and planning states (read-only)
  const showMarkdown = planContent && (planState === "review" || planState === "planning")
  // Show step list in executing/paused/completed states
  const showStepList = planState === "executing" || planState === "paused" || planState === "completed"

  // Title bar icon color based on state
  const iconColor = planState === "completed" ? "text-green-500"
    : planState === "executing" ? "text-blue-500"
    : planState === "paused" ? "text-yellow-500"
    : planState === "review" ? "text-purple-500"
    : "text-blue-500"

  // Whether inline commenting is enabled
  const canComment = (planState === "review" || planState === "planning") && !!onRequestChanges

  return (
    <div className="flex flex-col border-l border-border w-[400px] shrink-0 max-w-[40vw] bg-background animate-in slide-in-from-right-2 duration-200">
      {/* Title bar */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border bg-secondary/30 shrink-0">
        <ClipboardList className={cn("h-4 w-4", iconColor)} />
        <span className="text-sm font-medium truncate flex-1">{t("planMode.panelTitle")}</span>
        <div className="flex items-center gap-0.5">
          {/* Version history button */}
          {(planState === "review" || planState === "planning") && (
            <IconTip label={t("planMode.versions")}>
              <button
                className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
                onClick={handleLoadVersions}
                disabled={loadingVersions}
              >
                {loadingVersions ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <History className="h-3.5 w-3.5" />}
              </button>
            </IconTip>
          )}
          <IconTip label={t("common.close")}>
            <button
              className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
              onClick={onClose}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </div>
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
                planState === "completed" || allDone ? "bg-green-500"
                  : planState === "paused" ? "bg-yellow-500"
                  : "bg-blue-500"
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

      {/* Comment hint banner */}
      {canComment && showMarkdown && (
        <div className="px-3 py-1.5 bg-blue-500/5 border-b border-blue-500/10 text-[11px] text-muted-foreground flex items-center gap-1.5">
          <MessageSquareQuote className="h-3 w-3 shrink-0 text-blue-500/60" />
          {t("planMode.comment.hint")}
        </div>
      )}

      {/* Version history overlay */}
      {showVersions && (
        <div className="px-3 py-2 border-b border-border/50 bg-secondary/30">
          <div className="flex items-center justify-between mb-2">
            <span className="text-xs font-medium text-muted-foreground">{t("planMode.versionHistory")}</span>
            <button
              className="p-0.5 rounded hover:bg-secondary text-muted-foreground"
              onClick={() => setShowVersions(false)}
            >
              <X className="h-3 w-3" />
            </button>
          </div>
          <div className="space-y-1 max-h-[200px] overflow-y-auto">
            {versions.map((v) => (
              <div
                key={v.version}
                className={cn(
                  "flex items-center gap-2 px-2 py-1.5 rounded text-xs",
                  v.isCurrent ? "bg-blue-500/10 text-blue-600" : "hover:bg-secondary/60"
                )}
              >
                <span className="font-medium">v{v.version}</span>
                <span className="text-muted-foreground flex-1 truncate">
                  {v.modifiedAt ? new Date(v.modifiedAt).toLocaleString() : ""}
                </span>
                {v.isCurrent ? (
                  <span className="text-[10px] px-1.5 py-0.5 rounded bg-blue-500/20 text-blue-600">{t("planMode.currentVersion")}</span>
                ) : (
                  <button
                    className="p-0.5 rounded hover:bg-secondary text-muted-foreground hover:text-foreground"
                    onClick={() => handleRestoreVersion(v.filePath)}
                  >
                    <RotateCcw className="h-3 w-3" />
                  </button>
                )}
              </div>
            ))}
            {versions.length === 0 && (
              <div className="text-xs text-muted-foreground text-center py-3">{t("planMode.noVersions")}</div>
            )}
          </div>
        </div>
      )}

      {/* Main content area */}
      <div className="flex-1 overflow-y-auto relative" ref={contentRef} onMouseUp={canComment ? handleMouseUp : undefined}>
        {/* Read-only markdown content (planning + review states) */}
        {showMarkdown && (
          <div className={cn("px-3 py-3", canComment && "select-text cursor-text")}>
            <div className="prose prose-sm dark:prose-invert max-w-none">
              <MarkdownRenderer content={planContent} />
            </div>
          </div>
        )}

        {/* No content placeholder for planning state */}
        {planState === "planning" && !planContent && (
          <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
            <ClipboardList className="h-8 w-8 mb-3 opacity-30" />
            <span className="text-sm">{t("planMode.planning")}</span>
          </div>
        )}

        {/* Executing / Paused / Completed: step list with progress */}
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

        {/* Comment popover (positioned absolutely within content area) */}
        {commentPopover && (
          <CommentPopover
            position={commentPopover.position}
            selectedText={commentPopover.selectedText}
            onSubmit={handleCommentSubmit}
            onClose={() => {
              setCommentPopover(null)
              window.getSelection()?.removeAllRanges()
            }}
          />
        )}
      </div>

      {/* Action bar */}
      <div className="px-3 py-3 border-t border-border bg-secondary/20 shrink-0 space-y-2">
        {/* Planning: exit only */}
        {planState === "planning" && (
          <Button variant="ghost" className="w-full" onClick={onExit}>
            {t("planMode.exitWithout")}
          </Button>
        )}

        {/* Review: approve or exit */}
        {planState === "review" && (
          <>
            <Button
              className="w-full bg-blue-600 hover:bg-blue-700 text-white"
              onClick={onApprove}
            >
              <Play className="h-4 w-4 mr-2" />
              {t("planMode.approveAndExecute")}
            </Button>
            <Button variant="ghost" className="w-full" onClick={onExit}>
              {t("planMode.exitWithout")}
            </Button>
          </>
        )}

        {/* Executing: show status + pause button */}
        {planState === "executing" && !allDone && (
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 text-sm text-blue-600">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span>{t("planMode.executing")}</span>
            </div>
            {onPause && (
              <Button size="sm" variant="outline" onClick={onPause} className="gap-1.5">
                <Pause className="h-3.5 w-3.5" />
                {t("planMode.pause")}
              </Button>
            )}
          </div>
        )}

        {/* Paused: resume, rollback, or exit */}
        {planState === "paused" && (
          <>
            {onResume && (
              <Button
                className="w-full bg-yellow-600 hover:bg-yellow-700 text-white"
                onClick={onResume}
              >
                <Play className="h-4 w-4 mr-2" />
                {t("planMode.resume")}
              </Button>
            )}
            {hasCheckpoint && (
              <Button
                variant="outline"
                className="w-full text-destructive border-destructive/30 hover:bg-destructive/10"
                onClick={handleRollback}
                disabled={rollingBack}
              >
                {rollingBack ? <Loader2 className="h-4 w-4 mr-2 animate-spin" /> : <RotateCcw className="h-4 w-4 mr-2" />}
                {t("planMode.rollback")}
              </Button>
            )}
            <Button variant="ghost" className="w-full" onClick={onExit}>
              {t("planMode.exitWithout")}
            </Button>
          </>
        )}

        {/* Completed */}
        {(planState === "completed" || allDone) && (
          <>
            <div className="flex items-center gap-2 text-sm text-green-600">
              <CheckCircle className="h-4 w-4" />
              <span>{t("planMode.completed")}</span>
            </div>
            {hasCheckpoint && planSteps.some((s) => s.status === "failed") && (
              <Button
                variant="outline"
                className="w-full text-destructive border-destructive/30 hover:bg-destructive/10"
                onClick={handleRollback}
                disabled={rollingBack}
              >
                {rollingBack ? <Loader2 className="h-4 w-4 mr-2 animate-spin" /> : <RotateCcw className="h-4 w-4 mr-2" />}
                {t("planMode.rollback")}
              </Button>
            )}
            <Button variant="ghost" className="w-full" onClick={onExit}>
              {t("planMode.exitWithout")}
            </Button>
          </>
        )}
      </div>
    </div>
  )
}

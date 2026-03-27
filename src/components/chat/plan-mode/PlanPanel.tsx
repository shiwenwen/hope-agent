import { useMemo, useEffect, useState, useCallback } from "react"
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"
import { invoke } from "@tauri-apps/api/core"
import {
  ClipboardList,
  X,
  Play,
  Loader2,
  CheckCircle,
  Save,
  FileText,
  Pause,
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
  onPlanContentChange: (content: string) => void
  onApprove: () => void
  onExit: () => void
  onClose: () => void
  onPause?: () => void
  onResume?: () => void
}

export function PlanPanel({
  planState,
  planSteps,
  planContent,
  progress,
  completedCount,
  sessionId,
  onPlanContentChange,
  onApprove,
  onExit,
  onClose,
  onPause,
  onResume,
}: PlanPanelProps) {
  const { t } = useTranslation()
  const [editContent, setEditContent] = useState(planContent)
  const [dirty, setDirty] = useState(false)
  const [saving, setSaving] = useState(false)

  // Sync external planContent → local editContent (when LLM updates the plan)
  useEffect(() => {
    if (!dirty) {
      setEditContent(planContent)
    }
  }, [planContent, dirty])

  // Adjust window min size
  useEffect(() => {
    const win = getCurrentWindow()
    win.setMinSize(new LogicalSize(1240, 480))
    return () => {
      win.setMinSize(new LogicalSize(840, 480))
    }
  }, [])

  const handleEditChange = useCallback((value: string) => {
    setEditContent(value)
    setDirty(true)
  }, [])

  const handleSave = useCallback(async () => {
    if (!sessionId || !dirty) return
    setSaving(true)
    try {
      await invoke("save_plan_content", { sessionId, content: editContent })
      onPlanContentChange(editContent)
      setDirty(false)
    } catch (e) {
      console.error("Failed to save plan:", e)
    } finally {
      setSaving(false)
    }
  }, [sessionId, editContent, dirty, onPlanContentChange])

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

  const isEditable = planState === "planning"
  const showProgressBar = planState === "executing" || planState === "paused" || planState === "completed" || allDone
  const showStepList = planState !== "planning"

  // Title bar icon color based on state
  const iconColor = planState === "completed" ? "text-green-500"
    : planState === "executing" ? "text-blue-500"
    : planState === "paused" ? "text-yellow-500"
    : planState === "review" ? "text-purple-500"
    : "text-blue-500"

  return (
    <div className="flex flex-col border-l border-border w-[400px] shrink-0 max-w-[40vw] bg-background animate-in slide-in-from-right-2 duration-200">
      {/* Title bar */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border bg-secondary/30 shrink-0">
        <ClipboardList className={cn("h-4 w-4", iconColor)} />
        <span className="text-sm font-medium truncate flex-1">{t("planMode.panelTitle")}</span>
        <div className="flex items-center gap-0.5">
          {/* Save button (only in editing mode when dirty) */}
          {isEditable && dirty && (
            <IconTip label={t("common.save")}>
              <button
                className="p-1 rounded hover:bg-secondary transition-colors text-blue-500 hover:text-blue-600"
                onClick={handleSave}
                disabled={saving}
              >
                {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />}
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

      {/* Main content area */}
      <div className="flex-1 overflow-y-auto">
        {/* Planning mode: show editable textarea + step preview */}
        {isEditable && (
          <div className="flex flex-col h-full">
            <div className="flex-1 p-3">
              <Textarea
                value={editContent}
                onChange={(e) => handleEditChange(e.target.value)}
                placeholder={t("planMode.editorPlaceholder")}
                className="h-full min-h-[200px] resize-none border-border/50 bg-secondary/20 text-sm font-mono"
              />
            </div>
            {planSteps.length > 0 && (
              <div className="border-t border-border/50 px-3 py-2">
                <div className="flex items-center gap-1.5 mb-2">
                  <FileText className="h-3.5 w-3.5 text-muted-foreground" />
                  <span className="text-xs text-muted-foreground font-medium">
                    {planSteps.length} {t("planMode.stepsDetected")}
                  </span>
                </div>
                <div className="space-y-0.5 max-h-[200px] overflow-y-auto">
                  {planSteps.map((step) => (
                    <PlanStepItem key={step.index} step={step} />
                  ))}
                </div>
              </div>
            )}
          </div>
        )}

        {/* Review mode: read-only markdown */}
        {planState === "review" && planContent && (
          <div className="px-3 py-3">
            <div className="prose prose-sm dark:prose-invert max-w-none">
              <MarkdownRenderer content={planContent} />
            </div>
          </div>
        )}

        {/* Executing / Paused / Completed: step list with progress */}
        {showStepList && planState !== "review" && (
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

        {/* Paused: resume or exit */}
        {planState === "paused" && (
          <>
            <Button
              className="w-full bg-yellow-600 hover:bg-yellow-700 text-white"
              onClick={onResume}
            >
              <Play className="h-4 w-4 mr-2" />
              {t("planMode.resume")}
            </Button>
            <Button variant="ghost" className="w-full" onClick={onExit}>
              {t("planMode.exitWithout")}
            </Button>
          </>
        )}

        {/* Completed */}
        {(planState === "completed" || allDone) && (
          <div className="flex items-center gap-2 text-sm text-green-600">
            <CheckCircle className="h-4 w-4" />
            <span>{t("planMode.completed")}</span>
          </div>
        )}
      </div>
    </div>
  )
}

import { useMemo, useEffect } from "react"
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"
import {
  ClipboardList,
  X,
  Play,
  Loader2,
  CheckCircle,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { useTranslation } from "react-i18next"
import { groupStepsByPhase } from "./planParser"
import { PlanStepItem } from "./PlanStepItem"
import type { PlanModeState, PlanStep } from "./usePlanMode"

interface PlanPanelProps {
  planState: PlanModeState
  planSteps: PlanStep[]
  planContent: string
  progress: number
  completedCount: number
  onApprove: () => void
  onKeepPlanning?: () => void
  onExit: () => void
  onClose: () => void
}

export function PlanPanel({
  planState,
  planSteps,
  progress,
  completedCount,
  onApprove,
  onKeepPlanning,
  onExit,
  onClose,
}: PlanPanelProps) {
  const { t } = useTranslation()

  // Adjust window min size when panel is mounted/unmounted
  useEffect(() => {
    const win = getCurrentWindow()
    win.setMinSize(new LogicalSize(1240, 480))
    return () => {
      win.setMinSize(new LogicalSize(840, 480))
    }
  }, [])

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

  const planTitle = groupedPhases.length > 0 ? groupedPhases[0].name : "Plan"

  return (
    <div className="flex flex-col border-l border-border w-[400px] shrink-0 max-w-[40vw] bg-background animate-in slide-in-from-right-2 duration-200">
      {/* Title bar */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border bg-secondary/30 shrink-0">
        <ClipboardList className="h-4 w-4 text-blue-500" />
        <span className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
          Plan
        </span>
        <span className="text-sm font-medium truncate flex-1">{planTitle}</span>
        <div className="flex items-center gap-0.5">
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
      {planSteps.length > 0 && (
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
                allDone ? "bg-green-500" : "bg-blue-500"
              )}
              style={{ width: `${progress}%` }}
            />
          </div>
        </div>
      )}

      {/* Step list (scrollable) */}
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
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
        {planSteps.length === 0 && planState === "planning" && (
          <div className="text-sm text-muted-foreground text-center py-8">
            {t("planMode.placeholder")}
          </div>
        )}
      </div>

      {/* Action bar */}
      <div className="px-3 py-3 border-t border-border bg-secondary/20 shrink-0 space-y-2">
        {planState === "planning" && planSteps.length > 0 && (
          <>
            <Button
              className="w-full bg-blue-600 hover:bg-blue-700 text-white"
              onClick={onApprove}
            >
              <Play className="h-4 w-4 mr-2" />
              {t("planMode.approveAndExecute")}
            </Button>
            <div className="flex gap-2">
              {onKeepPlanning && (
                <Button variant="outline" className="flex-1" onClick={onKeepPlanning}>
                  {t("planMode.keepPlanning")}
                </Button>
              )}
              <Button variant="ghost" className="flex-1" onClick={onExit}>
                {t("planMode.exitWithout")}
              </Button>
            </div>
          </>
        )}
        {planState === "planning" && planSteps.length === 0 && (
          <Button variant="ghost" className="w-full" onClick={onExit}>
            {t("planMode.exitWithout")}
          </Button>
        )}
        {planState === "executing" && !allDone && (
          <div className="flex items-center gap-2 text-sm text-blue-600">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("planMode.executing")}</span>
          </div>
        )}
        {allDone && planState !== "off" && (
          <div className="flex items-center gap-2 text-sm text-green-600">
            <CheckCircle className="h-4 w-4" />
            <span>{t("planMode.completed")}</span>
          </div>
        )}
      </div>
    </div>
  )
}

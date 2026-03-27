import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import {
  ClipboardList,
  ChevronRight,
  Play,
  X,
  CheckCircle,
  Loader2,
  Pause,
} from "lucide-react"
import type { ParsedPlanStep } from "./planParser"
import { groupStepsByPhase } from "./planParser"

export interface PlanCardData {
  title: string
  steps: ParsedPlanStep[]
  sessionId: string
}

interface PlanCardBlockProps {
  data: PlanCardData
  planState: "off" | "planning" | "review" | "executing" | "paused" | "completed"
  onOpenPanel?: () => void
  onApprove?: () => void
  onExit?: () => void
  onPause?: () => void
  onResume?: () => void
}

export default function PlanCardBlock({
  data,
  planState,
  onOpenPanel,
  onApprove,
  onExit,
  onPause,
  onResume,
}: PlanCardBlockProps) {
  const { t } = useTranslation()

  const phases = useMemo(() => groupStepsByPhase(data.steps), [data.steps])

  const completedCount = useMemo(
    () => data.steps.filter(s => s.status === "completed" || s.status === "skipped" || s.status === "failed").length,
    [data.steps],
  )

  const progress = useMemo(
    () => data.steps.length > 0 ? Math.round((completedCount / data.steps.length) * 100) : 0,
    [completedCount, data.steps.length],
  )

  const borderColor = planState === "completed"
    ? "border-green-500/20"
    : planState === "executing"
    ? "border-blue-500/20"
    : planState === "paused"
    ? "border-yellow-500/20"
    : "border-purple-500/20"

  const bgColor = planState === "completed"
    ? "bg-green-500/5"
    : planState === "executing"
    ? "bg-blue-500/5"
    : planState === "paused"
    ? "bg-yellow-500/5"
    : "bg-purple-500/5"

  const iconColor = planState === "completed"
    ? "text-green-600"
    : planState === "executing"
    ? "text-blue-600"
    : planState === "paused"
    ? "text-yellow-600"
    : "text-purple-600"

  return (
    <div className={cn("my-2 rounded-lg border p-4 space-y-3", borderColor, bgColor)}>
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <ClipboardList className={cn("h-4 w-4", iconColor)} />
          <span className="text-sm font-medium">{data.title}</span>
        </div>
        <button
          onClick={onOpenPanel}
          className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
        >
          {t("planMode.openPanel")}
          <ChevronRight className="h-3 w-3" />
        </button>
      </div>

      {/* Summary */}
      <div className="text-xs text-muted-foreground">
        {phases.length} {t("planMode.card.phases")} · {data.steps.length} {t("planMode.card.steps")}
      </div>

      {/* Progress bar (when executing/paused/completed) */}
      {(planState === "executing" || planState === "paused" || planState === "completed") && (
        <div className="space-y-1">
          <div className="flex items-center justify-between text-xs">
            <span className="text-muted-foreground">
              {completedCount}/{data.steps.length} {t("planMode.stepsCompleted")}
            </span>
            <span className={cn("font-medium", iconColor)}>{progress}%</span>
          </div>
          <div className="h-1.5 rounded-full bg-secondary overflow-hidden">
            <div
              className={cn(
                "h-full rounded-full transition-all duration-500 ease-out",
                planState === "completed" ? "bg-green-500" : planState === "paused" ? "bg-yellow-500" : "bg-blue-500"
              )}
              style={{ width: `${progress}%` }}
            />
          </div>
        </div>
      )}

      {/* Phase list */}
      <div className="space-y-1">
        {phases.map((phase, i) => {
          const phaseCompleted = phase.steps.filter(
            s => s.status === "completed" || s.status === "skipped" || s.status === "failed"
          ).length
          const phaseTotal = phase.steps.length
          return (
            <div key={i} className="flex items-center gap-2 text-xs text-muted-foreground">
              <ChevronRight className="h-3 w-3 shrink-0" />
              <span className="truncate">{phase.name}</span>
              {(planState === "executing" || planState === "paused" || planState === "completed") && (
                <span className="shrink-0 ml-auto">
                  {phaseCompleted}/{phaseTotal}
                </span>
              )}
              {planState !== "executing" && planState !== "paused" && planState !== "completed" && (
                <span className="shrink-0 ml-auto">({phaseTotal})</span>
              )}
            </div>
          )
        })}
      </div>

      {/* Action buttons */}
      <div className="flex items-center gap-2 pt-1">
        {planState === "review" && (
          <>
            <Button size="sm" onClick={onApprove} className="gap-1.5">
              <Play className="h-3.5 w-3.5" />
              {t("planMode.approveAndExecute")}
            </Button>
            <Button size="sm" variant="ghost" onClick={onExit} className="gap-1.5 text-muted-foreground">
              <X className="h-3.5 w-3.5" />
              {t("planMode.exitWithout")}
            </Button>
          </>
        )}
        {planState === "executing" && (
          <Button size="sm" variant="outline" onClick={onPause} className="gap-1.5">
            <Pause className="h-3.5 w-3.5" />
            {t("planMode.pause")}
          </Button>
        )}
        {planState === "paused" && (
          <>
            <Button size="sm" onClick={onResume} className="gap-1.5">
              <Play className="h-3.5 w-3.5" />
              {t("planMode.resume")}
            </Button>
            <Button size="sm" variant="ghost" onClick={onExit} className="gap-1.5 text-muted-foreground">
              <X className="h-3.5 w-3.5" />
              {t("planMode.exitWithout")}
            </Button>
          </>
        )}
        {planState === "completed" && (
          <div className="flex items-center gap-1.5 text-sm text-green-600">
            <CheckCircle className="h-3.5 w-3.5" />
            <span>{t("planMode.completed")}</span>
          </div>
        )}
        {planState === "planning" && (
          <div className="flex items-center gap-1.5 text-xs text-blue-600">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            <span>{t("planMode.planning")}</span>
          </div>
        )}
      </div>
    </div>
  )
}

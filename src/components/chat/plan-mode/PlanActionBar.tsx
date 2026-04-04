import {
  Play,
  Loader2,
  CheckCircle,
  Pause,
  RotateCcw,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { useTranslation } from "react-i18next"
import type { PlanModeState, PlanStep } from "./usePlanMode"

interface PlanActionBarProps {
  planState: PlanModeState
  planSteps: PlanStep[]
  allDone: boolean
  hasCheckpoint: boolean
  rollingBack: boolean
  onApprove: () => void
  onExit: () => void
  onPause?: () => void
  onResume?: () => void
  onRollback: () => void
}

export function PlanActionBar({
  planState,
  planSteps,
  allDone,
  hasCheckpoint,
  rollingBack,
  onApprove,
  onExit,
  onPause,
  onResume,
  onRollback,
}: PlanActionBarProps) {
  const { t } = useTranslation()

  return (
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
              onClick={onRollback}
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
              onClick={onRollback}
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
  )
}

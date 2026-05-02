import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { ClipboardList, ChevronRight, Play, X, CheckCircle, Loader2 } from "lucide-react"
import type { PlanModeState } from "./usePlanMode"

export interface PlanCardData {
  title: string
}

interface PlanCardBlockProps {
  data: PlanCardData
  planState: PlanModeState
  onOpenPanel?: () => void
  onApprove?: () => void
  onExit?: () => void
}

export default function PlanCardBlock({
  data,
  planState,
  onOpenPanel,
  onApprove,
  onExit,
}: PlanCardBlockProps) {
  const { t } = useTranslation()

  const borderColor =
    planState === "completed"
      ? "border-green-500/20"
      : planState === "executing"
        ? "border-blue-500/20"
        : "border-purple-500/20"

  const bgColor =
    planState === "completed"
      ? "bg-green-500/5"
      : planState === "executing"
        ? "bg-blue-500/5"
        : "bg-purple-500/5"

  const iconColor =
    planState === "completed"
      ? "text-green-600"
      : planState === "executing"
        ? "text-blue-600"
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

      {/* Action buttons */}
      <div className="flex items-center gap-2 pt-1">
        {planState === "review" && (
          <>
            <Button size="sm" onClick={onApprove} className="gap-1.5">
              <Play className="h-3.5 w-3.5" />
              {t("planMode.approveAndExecute")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={onExit}
              className="gap-1.5 text-muted-foreground"
            >
              <X className="h-3.5 w-3.5" />
              {t("planMode.exitWithout")}
            </Button>
          </>
        )}
        {planState === "executing" && (
          <div className="flex items-center gap-1.5 text-sm text-blue-600">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            <span>{t("planMode.executing")}</span>
          </div>
        )}
        {planState === "completed" && (
          <div className="flex items-center gap-1.5 text-sm text-green-600">
            <CheckCircle className="h-3.5 w-3.5" />
            <span>{t("planMode.completed")}</span>
          </div>
        )}
      </div>
    </div>
  )
}

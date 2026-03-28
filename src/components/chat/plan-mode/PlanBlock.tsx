import { useState, useEffect, useCallback } from "react"
import { ChevronRight, ClipboardList, PanelRightOpen } from "lucide-react"
import { invoke } from "@tauri-apps/api/core"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { useTranslation } from "react-i18next"
import { detectPlanContent, type ParsedPlanStep } from "./planParser"
import { PlanStepItem } from "./PlanStepItem"
import type { PlanModeState, PlanStep } from "./usePlanMode"

interface PlanBlockProps {
  content: string
  sessionId: string | null
  planState: PlanModeState
  /** Live steps from usePlanMode (updated via events) */
  liveSteps?: PlanStep[]
  onOpenPanel?: () => void
  onApprove?: () => void
  onExit?: () => void
}

export function PlanBlock({
  content,
  sessionId,
  planState,
  liveSteps,
  onOpenPanel,
  onApprove,
  onExit,
}: PlanBlockProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(true)
  const [parsedSteps, setParsedSteps] = useState<ParsedPlanStep[]>([])
  const [planTitle, setPlanTitle] = useState<string>("")

  useEffect(() => {
    const { steps, title } = detectPlanContent(content)
    setParsedSteps(steps)
    setPlanTitle(title || t("planMode.plan"))
  }, [content, t])

  // Save plan content to backend when detected
  useEffect(() => {
    if (sessionId && parsedSteps.length > 0 && planState === "planning") {
      invoke("save_plan_content", { sessionId, content }).catch(() => {})
    }
  }, [sessionId, parsedSteps.length, planState, content])

  // Use live steps if available (during execution), otherwise parsed steps
  const displaySteps = liveSteps && liveSteps.length > 0 ? liveSteps : parsedSteps
  const completedCount = displaySteps.filter(
    (s) => s.status === "completed" || s.status === "skipped" || s.status === "failed"
  ).length

  const handleToggle = useCallback(() => setExpanded((p) => !p), [])

  if (parsedSteps.length === 0) return null

  return (
    <div className="my-3 border border-blue-500/20 rounded-xl overflow-hidden bg-blue-500/5">
      {/* Header */}
      <button
        onClick={handleToggle}
        className="flex items-center gap-2 w-full px-4 py-3 hover:bg-blue-500/5 transition-colors"
      >
        <ChevronRight
          className={cn(
            "h-4 w-4 text-muted-foreground transition-transform duration-200",
            expanded && "rotate-90"
          )}
        />
        <ClipboardList className="h-4 w-4 text-blue-500" />
        <span className="text-sm font-medium flex-1 text-left">{planTitle}</span>
        <span className="text-xs text-muted-foreground">
          {completedCount}/{displaySteps.length}
        </span>
        {onOpenPanel && (
          <IconTip label={t("planMode.openPanel")}>
            <PanelRightOpen
              className="h-3.5 w-3.5 text-muted-foreground hover:text-foreground"
              onClick={(e) => {
                e.stopPropagation()
                onOpenPanel()
              }}
            />
          </IconTip>
        )}
      </button>

      {/* Expandable content */}
      <div
        className={cn(
          "overflow-hidden transition-all duration-300 ease-in-out",
          expanded ? "max-h-[2000px] opacity-100" : "max-h-0 opacity-0"
        )}
      >
        <div className="px-4 pb-3 space-y-1">
          {displaySteps.map((step) => (
            <PlanStepItem key={step.index} step={step} />
          ))}
        </div>

        {/* Action bar (only in review state, after plan is submitted) */}
        {planState === "review" && (
          <div className="flex items-center gap-2 px-4 py-2 border-t border-blue-500/10 bg-blue-500/5">
            <Button
              size="sm"
              onClick={(e) => {
                e.stopPropagation()
                onApprove?.()
              }}
              className="bg-blue-600 hover:bg-blue-700 text-white"
            >
              {t("planMode.approveAndExecute")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={(e) => {
                e.stopPropagation()
                onExit?.()
              }}
            >
              {t("planMode.exitWithout")}
            </Button>
          </div>
        )}
      </div>
    </div>
  )
}

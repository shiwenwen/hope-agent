import { Circle, Loader2, CheckCircle, XCircle, MinusCircle } from "lucide-react"
import { cn } from "@/lib/utils"
import { formatDuration, type ParsedPlanStep } from "./planParser"
import type { PlanStep } from "./usePlanMode"

type StepLike = ParsedPlanStep | PlanStep

interface PlanStepItemProps {
  step: StepLike
  detailed?: boolean
}

export function PlanStepItem({ step, detailed }: PlanStepItemProps) {
  return (
    <div
      className={cn(
        "flex items-start gap-2 py-1 px-2 rounded-lg text-sm transition-colors",
        step.status === "in_progress" && "bg-blue-500/10",
        step.status === "completed" && "opacity-70"
      )}
    >
      {/* Status icon */}
      {step.status === "pending" && (
        <Circle className="h-4 w-4 text-muted-foreground mt-0.5 shrink-0" />
      )}
      {step.status === "in_progress" && (
        <Loader2 className="h-4 w-4 text-blue-500 animate-spin mt-0.5 shrink-0" />
      )}
      {step.status === "completed" && (
        <CheckCircle className="h-4 w-4 text-green-500 mt-0.5 shrink-0" />
      )}
      {step.status === "failed" && (
        <XCircle className="h-4 w-4 text-red-500 mt-0.5 shrink-0" />
      )}
      {step.status === "skipped" && (
        <MinusCircle className="h-4 w-4 text-gray-400 mt-0.5 shrink-0" />
      )}

      {/* Step title */}
      <div className="flex-1 min-w-0">
        <span
          className={cn(
            step.status === "completed" && "line-through text-muted-foreground"
          )}
        >
          {step.title}
        </span>
        {detailed && "description" in step && step.description && (
          <p className="text-xs text-muted-foreground mt-0.5">{step.description}</p>
        )}
      </div>

      {/* Duration */}
      {step.durationMs != null && step.durationMs > 0 && (
        <span className="text-xs text-muted-foreground shrink-0">
          {formatDuration(step.durationMs)}
        </span>
      )}
    </div>
  )
}

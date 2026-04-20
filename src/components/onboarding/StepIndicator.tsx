import { Check, Minus } from "lucide-react"

import { cn } from "@/lib/utils"

import { ONBOARDING_STEPS, type OnboardingStepKey } from "./types"

interface StepIndicatorProps {
  current: number
  skipped: Set<OnboardingStepKey>
  /** Override the step list — e.g. the short 2-step remote-mode flow. */
  steps?: OnboardingStepKey[]
}

/**
 * Compact horizontal progress strip shown above the wizard card. Each
 * step is a pill: current (solid), past-completed (muted with check),
 * past-skipped (muted with minus), future (outlined).
 */
export function StepIndicator({ current, skipped, steps }: StepIndicatorProps) {
  const list = steps ?? ONBOARDING_STEPS
  return (
    <ol className="flex items-center justify-center gap-1.5 px-4 pt-4">
      {list.map((key, idx) => {
        const state: "past-done" | "past-skipped" | "current" | "future" =
          idx < current
            ? skipped.has(key)
              ? "past-skipped"
              : "past-done"
            : idx === current
              ? "current"
              : "future"
        return (
          <li
            key={key}
            aria-current={state === "current" ? "step" : undefined}
            className={cn(
              "flex items-center gap-1 h-6 min-w-6 px-2 rounded-full text-[11px] font-medium transition-colors",
              state === "current" && "bg-primary text-primary-foreground",
              state === "past-done" && "bg-muted text-muted-foreground",
              state === "past-skipped" && "bg-muted text-muted-foreground/60",
              state === "future" && "bg-transparent border border-border text-muted-foreground",
            )}
          >
            {state === "past-done" && <Check className="h-3 w-3" strokeWidth={3} />}
            {state === "past-skipped" && <Minus className="h-3 w-3" strokeWidth={3} />}
            <span>{idx + 1}</span>
          </li>
        )
      })}
    </ol>
  )
}

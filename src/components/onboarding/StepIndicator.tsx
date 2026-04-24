import { cn } from "@/lib/utils"

import { ONBOARDING_STEPS, type OnboardingStepKey } from "./types"

interface StepIndicatorProps {
  current: number
  skipped: Set<OnboardingStepKey>
  /** Override the step list — e.g. the short 2-step remote-mode flow. */
  steps?: OnboardingStepKey[]
}

/**
 * Single-rail onboarding indicator modeled after the editorial concept:
 * one quiet line, a larger active number above it, a glowing active dot
 * on the rail, and completed steps reduced to plain dots.
 */
export function StepIndicator({ current, skipped, steps }: StepIndicatorProps) {
  const list = steps ?? ONBOARDING_STEPS
  const total = list.length
  const widthClass =
    list.length <= 3 ? "max-w-sm" : list.length <= 6 ? "max-w-xl" : "max-w-4xl"

  return (
    <ol
      className={cn(
        "relative mx-auto flex w-full items-start px-10 pb-4 pt-5",
        widthClass,
      )}
    >
      {list.map((key, idx) => {
        const isPast = idx < current
        const isCurrent = idx === current
        const isFuture = idx > current
        const isSkippedPast = isPast && skipped.has(key)
        const label = String(idx + 1).padStart(2, "0")

        return (
          <li
            key={key}
            aria-current={isCurrent ? "step" : undefined}
            className="relative flex min-w-0 flex-1 flex-col items-center"
          >
            <div className="flex h-11 items-end justify-center">
              <div className="flex items-end gap-1.5">
                <span
                  className={cn(
                    "tabular-nums font-medium tracking-[0.08em] transition-all duration-200",
                    isCurrent &&
                      [
                        "text-[2.2rem] leading-none text-primary",
                        "dark:text-white",
                      ],
                    isPast &&
                      "text-[1rem] leading-none text-foreground/52 dark:text-white/50",
                    isFuture &&
                      "text-[1.05rem] leading-none text-foreground/70 dark:text-white/70",
                    isSkippedPast &&
                      "text-muted-foreground/40 dark:text-white/32",
                  )}
                >
                  {label}
                </span>
                {isCurrent && (
                  <span className="mb-0.5 text-sm font-medium tabular-nums text-muted-foreground/75 dark:text-white/38">
                    /{total.toString().padStart(2, "0")}
                  </span>
                )}
              </div>
            </div>

            <div className="relative mt-3 h-4 w-full">
              {idx > 0 && (
                <span
                  aria-hidden="true"
                  className="absolute left-0 right-1/2 top-1/2 h-px -translate-y-1/2 bg-border/65 dark:bg-white/14"
                />
              )}
              {idx < list.length - 1 && (
                <span
                  aria-hidden="true"
                  className="absolute left-1/2 right-0 top-1/2 h-px -translate-y-1/2 bg-border/65 dark:bg-white/14"
                />
              )}

              {isPast && (
                <span
                  aria-hidden="true"
                  className={cn(
                    "absolute left-1/2 top-1/2 h-1 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full bg-foreground/50 dark:bg-white/52",
                    isSkippedPast && "bg-muted-foreground/45 dark:bg-white/28",
                  )}
                />
              )}

              {isCurrent && (
                <span
                  aria-hidden="true"
                  className="pointer-events-none absolute left-1/2 top-1/2 flex -translate-x-1/2 -translate-y-1/2 items-center justify-center"
                >
                  <span
                    className={cn(
                      "absolute h-10 w-10 rounded-full blur-xl",
                      "bg-[radial-gradient(circle,rgba(56,189,248,0.22)_0%,rgba(56,189,248,0.08)_45%,transparent_75%)]",
                      "dark:bg-[radial-gradient(circle,rgba(186,230,253,0.28)_0%,rgba(186,230,253,0.10)_45%,transparent_78%)]",
                    )}
                  />
                  <span
                    className={cn(
                      "absolute h-px w-20 blur-[2px]",
                      "bg-[linear-gradient(to_right,transparent,rgba(56,189,248,0.45)_35%,rgba(56,189,248,0.9)_50%,rgba(56,189,248,0.45)_65%,transparent)]",
                      "dark:bg-[linear-gradient(to_right,transparent,rgba(186,230,253,0.5)_35%,rgba(186,230,253,0.95)_50%,rgba(186,230,253,0.5)_65%,transparent)]",
                    )}
                  />
                  <span
                    className={cn(
                      "absolute h-4 w-4 rounded-full blur-[6px]",
                      "bg-[radial-gradient(circle,rgba(56,189,248,0.6)_0%,rgba(56,189,248,0.22)_45%,transparent_78%)]",
                      "dark:bg-[radial-gradient(circle,rgba(186,230,253,0.7)_0%,rgba(186,230,253,0.25)_45%,transparent_78%)]",
                    )}
                  />
                  <span
                    className={cn(
                      "relative h-1 w-1 rounded-full",
                      "bg-sky-200 shadow-[0_0_8px_rgba(125,211,252,0.95)]",
                      "dark:bg-sky-100 dark:shadow-[0_0_8px_rgba(186,230,253,0.95)]",
                    )}
                  />
                </span>
              )}

            </div>
          </li>
        )
      })}
    </ol>
  )
}

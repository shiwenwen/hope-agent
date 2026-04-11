import { Fragment, type ReactNode } from "react"
import { Circle, Loader2, CheckCircle, XCircle, MinusCircle } from "lucide-react"
import { Streamdown } from "streamdown"
import { code } from "@streamdown/code"
import { cjk } from "@streamdown/cjk"
import "streamdown/styles.css"
import { cn } from "@/lib/utils"
import { formatDuration, type ParsedPlanStep } from "./planParser"
import type { PlanStep } from "./usePlanMode"

type StepLike = ParsedPlanStep | PlanStep

interface PlanStepItemProps {
  step: StepLike
  detailed?: boolean
}

// ── Inline markdown via Streamdown ────────────────────────────────
// Streamdown has no native `inline` mode; by default it wraps content
// in `<div class="space-y-4 ..."><p>…</p></div>`. For single-line step
// titles we unwrap both layers:
//   1. `className="contents"` on Streamdown hides the outer div from
//      layout (`display: contents`), so its block-ness / margin classes
//      don't affect flow.
//   2. `components={{ p: Fragment }}` removes the `<p>` wrapper, so
//      inline children (<strong>, <em>, <code>, text) flow inline.
// Only `code` + `cjk` plugins are loaded — same minimal bundle that
// AskUserQuestionBlock uses per AGENTS.md.
const inlinePlugins = { code, cjk }
const inlineComponents = {
  p: ({ children }: { children?: ReactNode }) => <Fragment>{children}</Fragment>,
}

function InlineMarkdown({ text }: { text: string }) {
  return (
    <Streamdown
      className="contents"
      plugins={inlinePlugins}
      components={inlineComponents}
    >
      {text}
    </Streamdown>
  )
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
          <InlineMarkdown text={step.title} />
        </span>
        {detailed && "description" in step && step.description && (
          <p className="text-xs text-muted-foreground mt-0.5">
            <InlineMarkdown text={step.description} />
          </p>
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

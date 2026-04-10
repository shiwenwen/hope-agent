import type { ReactNode } from "react"
import { Circle, Loader2, CheckCircle, XCircle, MinusCircle } from "lucide-react"
import { cn } from "@/lib/utils"
import { formatDuration, type ParsedPlanStep } from "./planParser"
import type { PlanStep } from "./usePlanMode"

type StepLike = ParsedPlanStep | PlanStep

interface PlanStepItemProps {
  step: StepLike
  detailed?: boolean
}

/**
 * Lightweight inline markdown renderer for plan step titles/descriptions.
 *
 * Supports `**bold**`, `*italic*`, `` `code` ``, `~~strike~~` and leaves
 * everything else as plain text. Block-level markdown is intentionally not
 * supported since step titles are single-line.
 */
function renderInlineMarkdown(text: string): ReactNode[] {
  const nodes: ReactNode[] = []
  let buffer = ""
  let i = 0
  let key = 0

  const flushBuffer = () => {
    if (buffer) {
      nodes.push(buffer)
      buffer = ""
    }
  }

  while (i < text.length) {
    const ch = text[i]

    // Backslash escape (\* \` etc.)
    if (ch === "\\" && i + 1 < text.length) {
      buffer += text[i + 1]
      i += 2
      continue
    }

    // **bold**
    if (ch === "*" && text[i + 1] === "*") {
      const end = text.indexOf("**", i + 2)
      if (end > i + 2) {
        flushBuffer()
        nodes.push(
          <strong key={key++} className="font-semibold">
            {renderInlineMarkdown(text.slice(i + 2, end))}
          </strong>
        )
        i = end + 2
        continue
      }
    }

    // *italic* (require non-space after opening and non-space before closing)
    if (ch === "*" && text[i + 1] && text[i + 1] !== "*" && text[i + 1] !== " ") {
      const end = text.indexOf("*", i + 1)
      if (end > i + 1 && text[end - 1] !== " ") {
        flushBuffer()
        nodes.push(
          <em key={key++}>{renderInlineMarkdown(text.slice(i + 1, end))}</em>
        )
        i = end + 1
        continue
      }
    }

    // `inline code`
    if (ch === "`") {
      const end = text.indexOf("`", i + 1)
      if (end > i + 1) {
        flushBuffer()
        nodes.push(
          <code
            key={key++}
            className="px-1 py-0.5 rounded bg-muted text-[0.85em] font-mono"
          >
            {text.slice(i + 1, end)}
          </code>
        )
        i = end + 1
        continue
      }
    }

    // ~~strikethrough~~
    if (ch === "~" && text[i + 1] === "~") {
      const end = text.indexOf("~~", i + 2)
      if (end > i + 2) {
        flushBuffer()
        nodes.push(
          <s key={key++}>{renderInlineMarkdown(text.slice(i + 2, end))}</s>
        )
        i = end + 2
        continue
      }
    }

    buffer += ch
    i++
  }

  flushBuffer()
  return nodes
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
          {renderInlineMarkdown(step.title)}
        </span>
        {detailed && "description" in step && step.description && (
          <p className="text-xs text-muted-foreground mt-0.5">
            {renderInlineMarkdown(step.description)}
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

import { useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { AlertCircle, CheckCircle2, ChevronRight } from "lucide-react"
import { cn } from "@/lib/utils"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { formatDuration } from "../chatUtils"
import { MediaHoistContext } from "./mediaHoistContext"
import ToolMediaPreview from "./ToolMediaPreview"
import type { ToolCall } from "@/types/chat"

interface ProcessedBlockGroupProps {
  children: ReactNode
  failedCount?: number
  /** Total processing time across the folded steps (tools + thinking), ms. */
  totalElapsedMs?: number
  /** Steps that produced renderable media — hoisted below the group so their
   *  output (generated images, sent attachments, …) stays visible while the
   *  steps are collapsed. Their inline previews are suppressed via context. */
  mediaTools?: ToolCall[]
}

export default function ProcessedBlockGroup({
  children,
  failedCount = 0,
  totalElapsedMs,
  mediaTools,
}: ProcessedBlockGroupProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const elapsedText = totalElapsedMs != null && totalElapsedMs > 0 ? formatDuration(totalElapsedMs) : null

  return (
    <div className="my-1 text-xs animate-in fade-in-0 duration-200 motion-reduce:animate-none">
      <button
        type="button"
        aria-expanded={expanded}
        className="flex items-center gap-1.5 w-full pl-0 pr-1 py-1 text-left hover:bg-secondary/60 rounded-md transition-colors"
        onClick={() => setExpanded((v) => !v)}
      >
        <ChevronRight
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-muted-foreground/50 transition-transform duration-200",
            expanded && "rotate-90",
          )}
        />
        <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground/75" />
        <span className="font-medium text-muted-foreground/75">
          {t("executionStatus.processed.completed")}
        </span>
        {elapsedText && (
          <span className="shrink-0 font-medium text-muted-foreground/75 tabular-nums">
            {elapsedText}
          </span>
        )}
        {failedCount > 0 && (
          <span className="shrink-0 rounded-full bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-500">
            <span className="inline-flex items-center gap-0.5">
              <AlertCircle className="h-3 w-3" />
              {t("executionStatus.tool.group.failedCount", { count: failedCount })}
            </span>
          </span>
        )}
      </button>
      {/* Collapsed: suppress the steps' inline media and hoist it below so the
          output stays visible while folded. Expanded: show each step's media
          inline next to the step that produced it (no suppression, no hoist). */}
      <MediaHoistContext.Provider value={!expanded}>
        <AnimatedCollapse open={expanded}>
          <div className="ml-3 border-l border-border/40 pl-2 animate-in fade-in-0 slide-in-from-top-1 duration-150">
            {children}
          </div>
        </AnimatedCollapse>
      </MediaHoistContext.Provider>
      {!expanded &&
        mediaTools?.map((tool) => (
          <ToolMediaPreview key={tool.callId} tool={tool} className="ml-1" />
        ))}
    </div>
  )
}

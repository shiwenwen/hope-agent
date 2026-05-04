import { useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { AlertCircle, CheckCircle2, ChevronRight } from "lucide-react"
import { cn } from "@/lib/utils"

interface ProcessedBlockGroupProps {
  children: ReactNode
  failedCount?: number
}

export default function ProcessedBlockGroup({
  children,
  failedCount = 0,
}: ProcessedBlockGroupProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)

  return (
    <div className="my-1 text-xs">
      <button
        type="button"
        aria-expanded={expanded}
        className="flex items-center gap-1.5 w-full pl-0 pr-1 py-1 text-left hover:bg-secondary/60 rounded-md transition-colors"
        onClick={() => setExpanded((v) => !v)}
      >
        <ChevronRight
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-muted-foreground/60 transition-transform duration-200",
            expanded && "rotate-90",
          )}
        />
        <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="font-medium text-muted-foreground">
          {t("executionStatus.processed.completed")}
        </span>
        {failedCount > 0 && (
          <span className="shrink-0 rounded-full bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-500">
            <span className="inline-flex items-center gap-0.5">
              <AlertCircle className="h-3 w-3" />
              {t("executionStatus.tool.group.failedCount", { count: failedCount })}
            </span>
          </span>
        )}
      </button>
      {expanded && (
        <div className="ml-3 border-l border-border/40 pl-2 animate-in fade-in-0 slide-in-from-top-1 duration-150">
          {children}
        </div>
      )}
    </div>
  )
}

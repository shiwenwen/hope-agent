import { useState } from "react"
import { ChevronRight, ListChecks } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import {
  getTaskDisplayLabel,
  type TaskProgressSnapshot,
} from "./taskProgress"
import { TASK_STATUS_ICON } from "./taskStatusIcon"

interface TaskProgressPanelProps {
  snapshot: TaskProgressSnapshot
  className?: string
  defaultExpanded?: boolean
  variant?: "card" | "embedded"
}

export default function TaskProgressPanel({
  snapshot,
  className,
  defaultExpanded = true,
  variant = "card",
}: TaskProgressPanelProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(defaultExpanded)
  const fallbackTaskLabel = String(t("settings.browser.untitledTab", { defaultValue: "Untitled" }))
  const taskLabel = String(t("chat.tasks"))
  const progressLabel = String(
    t("chat.taskProgress", {
      completed: snapshot.completed,
      total: snapshot.total,
    }),
  )

  return (
    <div
      className={cn(
        "overflow-hidden animate-in fade-in-0 slide-in-from-bottom-1 duration-200",
        variant === "embedded"
          ? "rounded-t-2xl border-b border-border/70 bg-secondary/20"
          : "rounded-2xl border border-border/80 bg-card/95 shadow-sm",
        className,
      )}
    >
      <button
        type="button"
        aria-expanded={expanded}
        aria-label={`${taskLabel} ${progressLabel}`}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-secondary/45"
        onClick={() => setExpanded((value) => !value)}
      >
        <ListChecks className="h-4 w-4 shrink-0 text-blue-500" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
          {taskLabel}
          <span className="px-1.5 font-normal text-muted-foreground">·</span>
          <span className="font-normal text-muted-foreground">{progressLabel}</span>
        </span>
        <ChevronRight
          className={cn(
            "h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200",
            expanded && "rotate-90",
          )}
        />
      </button>

      {expanded && (
        <div className="border-t border-border/60 px-3 py-2">
          <ol className="max-h-[30vh] space-y-1 overflow-y-auto pr-1">
            {snapshot.tasks.map((task, index) => {
              const { Icon, cls } = TASK_STATUS_ICON[task.status] ?? TASK_STATUS_ICON.pending
              const label = getTaskDisplayLabel(task, fallbackTaskLabel)
              return (
                <li
                  key={task.id}
                  className={cn(
                    "flex min-h-7 items-start gap-2 rounded-md px-2 py-1 text-sm",
                    task.status === "in_progress" && "bg-blue-500/10",
                    task.status === "completed" && "opacity-75",
                  )}
                >
                  <Icon className={cn("mt-0.5 h-3.5 w-3.5 shrink-0", cls)} />
                  <span className="w-5 shrink-0 text-right tabular-nums text-muted-foreground">
                    {index + 1}.
                  </span>
                  <span
                    className={cn(
                      "min-w-0 flex-1 break-words leading-5",
                      task.status === "completed" && "text-muted-foreground line-through",
                    )}
                  >
                    {label}
                  </span>
                </li>
              )
            })}
          </ol>
        </div>
      )}
    </div>
  )
}

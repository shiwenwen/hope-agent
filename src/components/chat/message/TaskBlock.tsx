import { useMemo, useState } from "react"
import { ChevronRight, ListChecks } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"
import {
  createCurrentTaskProgressSnapshot,
  getTaskDisplayLabel,
  getTaskProgressSummaryText,
  parseTaskToolResult,
} from "@/components/chat/tasks/taskProgress"
import { TASK_STATUS_ICON } from "@/components/chat/tasks/taskStatusIcon"

interface TaskBlockProps {
  tool: ToolCall
}

export default function TaskBlock({ tool }: TaskBlockProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(true)

  const rawTasks = useMemo(() => parseTaskToolResult(tool.result), [tool.result])
  const snapshot = useMemo(() => createCurrentTaskProgressSnapshot(rawTasks), [rawTasks])
  const tasks = snapshot.tasks
  const summaryText = useMemo(() => getTaskProgressSummaryText(snapshot, t), [snapshot, t])

  if (tasks.length === 0) {
    return (
      <div className="my-1.5 flex items-center gap-1.5 rounded-lg border border-border bg-secondary/40 px-2.5 py-1.5 text-xs text-muted-foreground">
        <ListChecks className="h-3.5 w-3.5 shrink-0" />
        <span>{summaryText}</span>
      </div>
    )
  }

  const fallbackTaskLabel = String(t("settings.browser.untitledTab", { defaultValue: "Untitled" }))

  return (
    <div className="my-1.5 rounded-lg border border-border bg-secondary/40 text-xs">
      <button
        className="flex w-full items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-secondary/70"
        onClick={() => setExpanded(!expanded)}
      >
        <ChevronRight
          className={cn(
            "h-3 w-3 shrink-0 text-muted-foreground transition-transform duration-200",
            expanded && "rotate-90",
          )}
        />
        <ListChecks className="h-3.5 w-3.5 shrink-0 text-blue-500" />
        <span className="font-medium text-foreground">{summaryText}</span>
      </button>

      {expanded && (
        <ul className="space-y-0.5 px-2 pb-2">
          {tasks.map((tk) => {
            const { Icon, cls } = TASK_STATUS_ICON[tk.status] ?? TASK_STATUS_ICON.pending
            const label = getTaskDisplayLabel(tk, fallbackTaskLabel)
            return (
              <li
                key={tk.id}
                className={cn(
                  "flex items-start gap-2 rounded px-1.5 py-1",
                  tk.status === "in_progress" && "bg-blue-500/10",
                  tk.status === "completed" && "opacity-70",
                )}
              >
                <Icon className={cn("mt-0.5 h-3.5 w-3.5 shrink-0", cls)} />
                <span
                  className={cn(
                    "min-w-0 flex-1 break-words",
                    tk.status === "completed" && "text-muted-foreground line-through",
                  )}
                >
                  {label}
                </span>
                <span className="shrink-0 text-[10px] text-muted-foreground">#{tk.id}</span>
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}

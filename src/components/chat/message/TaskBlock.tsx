import { useMemo, useState } from "react"
import { ChevronRight, Circle, CheckCircle, Loader2, ListChecks } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import type { Task, TaskStatus, ToolCall } from "@/types/chat"

interface TaskBlockProps {
  tool: ToolCall
}

const STATUS_ICON: Record<TaskStatus, { Icon: typeof Circle; cls: string }> = {
  pending: { Icon: Circle, cls: "text-muted-foreground" },
  in_progress: { Icon: Loader2, cls: "animate-spin text-blue-500" },
  completed: { Icon: CheckCircle, cls: "text-green-500" },
}

export default function TaskBlock({ tool }: TaskBlockProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(true)

  const tasks = useMemo<Task[]>(() => {
    if (!tool.result) return []
    try {
      const parsed = JSON.parse(tool.result)
      return Array.isArray(parsed) ? (parsed as Task[]) : []
    } catch {
      return []
    }
  }, [tool.result])

  const summary = useMemo(() => {
    const total = tasks.length
    const completed = tasks.filter((tk) => tk.status === "completed").length
    return { total, completed }
  }, [tasks])

  if (tasks.length === 0) {
    return (
      <div className="my-1.5 flex items-center gap-1.5 rounded-lg border border-border bg-secondary/40 px-2.5 py-1.5 text-xs text-muted-foreground">
        <ListChecks className="h-3.5 w-3.5 shrink-0" />
        <span>{t("chat.taskNoneYet", "No tasks yet")}</span>
      </div>
    )
  }

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
        <span className="font-medium">{t("chat.tasks", "Tasks")}</span>
        <span className="text-muted-foreground">
          {t("chat.taskProgress", "{{completed}}/{{total}} completed", {
            completed: summary.completed,
            total: summary.total,
          })}
        </span>
      </button>

      {expanded && (
        <ul className="space-y-0.5 px-2 pb-2">
          {tasks.map((tk) => {
            const { Icon, cls } = STATUS_ICON[tk.status] ?? STATUS_ICON.pending
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
                  {tk.content}
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

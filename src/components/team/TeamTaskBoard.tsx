import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import type { TeamTask, TeamMember, KanbanColumn } from "./teamTypes"
import { KANBAN_COLUMNS } from "./teamTypes"
import { TeamTaskCard } from "./TeamTaskCard"

interface TeamTaskBoardProps {
  tasks: TeamTask[]
  members: TeamMember[]
}

const COLUMN_LABELS: Record<KanbanColumn, string> = {
  todo: "To Do",
  doing: "Doing",
  review: "Review",
  done: "Done",
}

const COLUMN_COLORS: Record<KanbanColumn, string> = {
  todo: "bg-gray-200 dark:bg-gray-700",
  doing: "bg-blue-200 dark:bg-blue-900",
  review: "bg-yellow-200 dark:bg-yellow-900",
  done: "bg-green-200 dark:bg-green-900",
}

export function TeamTaskBoard({ tasks, members }: TeamTaskBoardProps) {
  const { t } = useTranslation()

  const grouped = (() => {
    const map: Record<string, TeamTask[]> = {}
    for (const col of KANBAN_COLUMNS) {
      map[col] = []
    }
    for (const task of tasks) {
      const col = task.columnName as KanbanColumn
      if (map[col]) {
        map[col].push(task)
      } else {
        map["todo"].push(task)
      }
    }
    return map
  })()

  return (
    <div className="grid grid-cols-4 gap-2">
      {KANBAN_COLUMNS.map((col) => {
        const colTasks = grouped[col]
        return (
          <div key={col} className="flex flex-col gap-2">
            {/* Column header */}
            <div className="flex items-center gap-2 rounded-md px-2 py-1.5">
              <span className={cn("h-2 w-2 rounded-full", COLUMN_COLORS[col])} />
              <span className="text-xs font-medium text-foreground">
                {t(`team.column.${col}`, COLUMN_LABELS[col])}
              </span>
              <span className="ml-auto rounded-full bg-muted px-1.5 py-0.5 text-[10px] tabular-nums text-muted-foreground">
                {colTasks.length}
              </span>
            </div>

            {/* Task list */}
            <div className="flex flex-col gap-1.5 rounded-lg bg-muted/30 p-1.5 min-h-[80px]">
              {colTasks.length === 0 ? (
                <div className="flex items-center justify-center py-4 text-[11px] text-muted-foreground/50">
                  {t("team.noTasks", "No tasks")}
                </div>
              ) : (
                colTasks.map((task) => <TeamTaskCard key={task.id} task={task} members={members} />)
              )}
            </div>
          </div>
        )
      })}
    </div>
  )
}

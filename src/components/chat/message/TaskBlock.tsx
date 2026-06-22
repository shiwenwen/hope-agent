import { useMemo, useState } from "react"
import { AlertCircle, ChevronRight, CirclePause, ListChecks } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import type { ChatTurnStatus, ToolCall } from "@/types/chat"
import {
  createCurrentTaskProgressSnapshot,
  getTaskDisplayLabel,
  getTaskProgressSummaryText,
  type TaskProgressSnapshot,
  parseTaskToolResult,
} from "@/components/chat/tasks/taskProgress"
import { TASK_STATUS_ICON } from "@/components/chat/tasks/taskStatusIcon"

type TaskExecutionState = "idle" | "running" | "cancelling" | "interrupted" | "failed"

interface TaskBlockProps {
  tool: ToolCall
  executionState?: ChatTurnStatus | TaskExecutionState | null
}

function normalizeExecutionState(
  state: ChatTurnStatus | TaskExecutionState | null | undefined,
): TaskExecutionState {
  switch (state) {
    case "running":
    case "cancelling":
    case "interrupted":
    case "failed":
      return state
    default:
      return "idle"
  }
}

function getTaskBlockSummaryText(
  snapshot: TaskProgressSnapshot,
  t: Parameters<typeof getTaskProgressSummaryText>[1],
  executionState: TaskExecutionState,
): string {
  if (snapshot.inProgress) {
    const key =
      executionState === "running"
        ? "chat.taskProgressRunning"
        : executionState === "cancelling"
          ? "chat.taskProgressCancelling"
          : executionState === "failed"
            ? "chat.taskProgressFailed"
            : "chat.taskProgressWaiting"
    return String(t(key, {
      completed: snapshot.completed,
      total: snapshot.total,
      remaining: snapshot.remaining,
    }))
  }

  return getTaskProgressSummaryText(snapshot, t)
}

function TaskBlockList({
  tasks,
  summaryText,
  taskExecutionState,
  fallbackTaskLabel,
  expansionKey,
  defaultExpanded,
}: {
  tasks: TaskProgressSnapshot["tasks"]
  summaryText: string
  taskExecutionState: TaskExecutionState
  fallbackTaskLabel: string
  expansionKey: string
  defaultExpanded: boolean
}) {
  const [expandedByKey, setExpandedByKey] = useState<Record<string, boolean>>({})
  const expanded = expandedByKey[expansionKey] ?? defaultExpanded

  return (
    <div className="my-1.5 rounded-lg border border-border bg-secondary/40 text-xs">
      <button
        aria-expanded={expanded}
        className="flex w-full items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-secondary/70"
        onClick={() =>
          setExpandedByKey((prev) => ({
            ...prev,
            [expansionKey]: !expanded,
          }))
        }
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

      <AnimatedCollapse open={expanded}>
        <ul className="space-y-0.5 px-2 pb-2">
          {tasks.map((tk) => {
            const baseIcon = TASK_STATUS_ICON[tk.status] ?? TASK_STATUS_ICON.pending
            const isPausedInProgress = tk.status === "in_progress" && taskExecutionState !== "running"
            const Icon =
              tk.status === "in_progress" && taskExecutionState === "failed"
                ? AlertCircle
                : isPausedInProgress
                  ? CirclePause
                  : baseIcon.Icon
            const cls =
              tk.status === "in_progress" && taskExecutionState === "failed"
                ? "text-destructive"
                : isPausedInProgress
                  ? "text-muted-foreground"
                  : baseIcon.cls
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
              </li>
            )
          })}
        </ul>
      </AnimatedCollapse>
    </div>
  )
}

export default function TaskBlock({ tool, executionState }: TaskBlockProps) {
  const { t } = useTranslation()

  const rawTasks = useMemo(() => parseTaskToolResult(tool.result), [tool.result])
  const snapshot = useMemo(() => createCurrentTaskProgressSnapshot(rawTasks), [rawTasks])
  const tasks = snapshot.tasks
  const allTasksCompleted = tasks.length > 0 && snapshot.completed === snapshot.total
  const taskExecutionState = normalizeExecutionState(executionState)
  const summaryText = useMemo(
    () => getTaskBlockSummaryText(snapshot, t, taskExecutionState),
    [snapshot, t, taskExecutionState],
  )

  if (tasks.length === 0) {
    return (
      <div className="my-1.5 flex items-center gap-1.5 rounded-lg border border-border bg-secondary/40 px-2.5 py-1.5 text-xs text-muted-foreground">
        <ListChecks className="h-3.5 w-3.5 shrink-0" />
        <span>{summaryText}</span>
      </div>
    )
  }

  const fallbackTaskLabel = String(t("settings.browser.untitledTab", { defaultValue: "Untitled" }))
  const expansionKey = allTasksCompleted ? "completed" : "active"

  return (
    <TaskBlockList
      tasks={tasks}
      summaryText={summaryText}
      taskExecutionState={taskExecutionState}
      fallbackTaskLabel={fallbackTaskLabel}
      expansionKey={expansionKey}
      defaultExpanded={!allTasksCompleted}
    />
  )
}

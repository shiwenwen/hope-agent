import { useState } from "react"
import { AlertCircle, ChevronRight, CirclePause, ListChecks, PanelRight, Trash2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { IconTip } from "@/components/ui/tooltip"
import type { Task, TaskStatus } from "@/types/chat"
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
  executionState?: "idle" | "running" | "cancelling" | "interrupted" | "failed"
  onOpenWorkspace?: () => void
  workspaceOpen?: boolean
}

// Cycle: pending → in_progress → completed → pending. Mirrors how a user
// would manually walk a task through its lifecycle if the model dropped it.
const NEXT_STATUS: Record<TaskStatus, TaskStatus> = {
  pending: "in_progress",
  in_progress: "completed",
  completed: "pending",
}

export default function TaskProgressPanel({
  snapshot,
  className,
  defaultExpanded = true,
  variant = "card",
  executionState = "idle",
  onOpenWorkspace,
  workspaceOpen = false,
}: TaskProgressPanelProps) {
  const { t } = useTranslation()
  const allTasksCompleted = snapshot.total > 0 && snapshot.completed === snapshot.total
  const expansionKey = `${allTasksCompleted ? "completed" : "active"}:${
    workspaceOpen ? "workspace" : "chat"
  }`
  const defaultPanelExpanded = defaultExpanded && !workspaceOpen && !allTasksCompleted
  const [expandedByKey, setExpandedByKey] = useState<Record<string, boolean>>({})
  const expanded = expandedByKey[expansionKey] ?? defaultPanelExpanded
  // Per-row in-flight set so a slow RPC on task A doesn't disable the
  // controls on tasks B/C/D — matters most on HTTP transport latency.
  const [busyIds, setBusyIds] = useState<Set<number>>(() => new Set())

  async function withBusy(id: number, op: () => Promise<unknown>, label: string) {
    if (busyIds.has(id)) return
    setBusyIds((prev) => new Set(prev).add(id))
    try {
      await op()
    } catch (err) {
      logger.warn("chat", "TaskProgressPanel::withBusy", `${label} failed`, err)
    } finally {
      setBusyIds((prev) => {
        const next = new Set(prev)
        next.delete(id)
        return next
      })
    }
  }

  const cycleStatus = (task: Task) =>
    withBusy(
      task.id,
      () =>
        getTransport().call("update_task_status", {
          id: task.id,
          status: NEXT_STATUS[task.status],
        }),
      "update_task_status",
    )

  const removeTask = (task: Task) =>
    withBusy(task.id, () => getTransport().call("delete_task", { id: task.id }), "delete_task")

  const fallbackTaskLabel = String(t("settings.browser.untitledTab", { defaultValue: "Untitled" }))
  const taskLabel = String(t("chat.tasks"))
  const progressKey =
    snapshot.inProgress && executionState === "running"
      ? "chat.taskProgressRunning"
      : snapshot.inProgress && executionState === "cancelling"
        ? "chat.taskProgressCancelling"
        : snapshot.inProgress && executionState === "failed"
          ? "chat.taskProgressFailed"
          : snapshot.inProgress
            ? "chat.taskProgressWaiting"
            : "chat.taskProgress"
  const progressLabel = String(
    t(progressKey, {
      completed: snapshot.completed,
      total: snapshot.total,
    }),
  )

  return (
    <div
      className={cn(
        "overflow-hidden animate-in fade-in-0 slide-in-from-bottom-1 duration-200",
        variant === "embedded"
          ? "rounded-t-2xl border-b border-border/70 bg-white dark:bg-card"
          : "rounded-2xl border border-border/80 bg-surface-floating shadow-sm",
        className,
      )}
    >
      <div className="flex w-full items-center gap-1 px-3 py-2 transition-colors hover:bg-secondary/45">
        <button
          type="button"
          aria-expanded={expanded}
          aria-label={`${taskLabel} ${progressLabel}`}
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
          onClick={() =>
            setExpandedByKey((prev) => ({
              ...prev,
              [expansionKey]: !expanded,
            }))
          }
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
        {onOpenWorkspace && (
          <IconTip label={String(t("workspace.openPanel", { defaultValue: "打开工作台" }))}>
            <button
              type="button"
              aria-label={String(t("workspace.openPanel", { defaultValue: "打开工作台" }))}
              className={cn(
                "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring/50",
                workspaceOpen && "bg-secondary text-foreground",
              )}
              onClick={onOpenWorkspace}
            >
              <PanelRight className="h-4 w-4" />
            </button>
          </IconTip>
        )}
      </div>

      <AnimatedCollapse open={expanded}>
        <div className="border-t border-border/60 px-3 py-2">
          <ol className="max-h-[30vh] space-y-1 overflow-y-auto pr-1">
            {snapshot.tasks.map((task, index) => {
              const baseIcon = TASK_STATUS_ICON[task.status] ?? TASK_STATUS_ICON.pending
              const isPausedInProgress = task.status === "in_progress" && executionState !== "running"
              const Icon =
                task.status === "in_progress" && executionState === "failed"
                  ? AlertCircle
                  : isPausedInProgress
                    ? CirclePause
                    : baseIcon.Icon
              const cls =
                task.status === "in_progress" && executionState === "failed"
                  ? "text-destructive"
                  : isPausedInProgress
                    ? "text-muted-foreground"
                    : baseIcon.cls
              const label = getTaskDisplayLabel(task, fallbackTaskLabel)
              const cycleTip = String(
                t(`chat.taskActions.cycleTo.${NEXT_STATUS[task.status]}`),
              )
              const deleteTip = String(t("chat.taskActions.delete"))
              const busy = busyIds.has(task.id)
              return (
                <li
                  key={task.id}
                  className={cn(
                    "group/task flex min-h-7 items-start gap-2 rounded-md px-2 py-1 text-sm transition-[background-color,opacity] duration-150",
                    task.status === "in_progress" && "bg-blue-500/10",
                    task.status === "completed" && "opacity-75",
                  )}
                >
                  <IconTip label={cycleTip} side="right">
                    <button
                      type="button"
                      aria-label={cycleTip}
                      disabled={busy}
                      onClick={() => cycleStatus(task)}
                      className={cn(
                        "mt-0.5 inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-sm transition-colors hover:bg-secondary disabled:opacity-50",
                      )}
                    >
                      <Icon className={cn("h-3.5 w-3.5", cls)} />
                    </button>
                  </IconTip>
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
                  <IconTip label={deleteTip} side="left">
                    <button
                      type="button"
                      aria-label={deleteTip}
                      disabled={busy}
                      onClick={() => removeTask(task)}
                      className={cn(
                        "mt-0.5 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground opacity-0 transition-opacity hover:bg-destructive/15 hover:text-destructive group-hover/task:opacity-100 focus-visible:opacity-100 disabled:opacity-50",
                      )}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </IconTip>
                </li>
              )
            })}
          </ol>
        </div>
      </AnimatedCollapse>
    </div>
  )
}

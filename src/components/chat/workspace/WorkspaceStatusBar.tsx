import { useTranslation } from "react-i18next"
import { ListChecks, PanelRight } from "lucide-react"
import { cn } from "@/lib/utils"
import { shouldShowTaskProgressPanel, type TaskProgressSnapshot } from "@/components/chat/tasks/taskProgress"
import type { WorkspaceTaskExecutionState } from "./taskExecutionState"

interface WorkspaceStatusBarProps {
  snapshot: TaskProgressSnapshot | null | undefined
  executionState?: WorkspaceTaskExecutionState
  /** 打开 / 激活右侧工作台面板。 */
  onOpen: () => void
}

/**
 * 输入框上方的极简状态条 —— 仅在有未完成任务时显示「任务 · 运行中 N/M」实时进度,
 * 点击打开工作台面板查看详情。无任务则不渲染:空闲时工作台改从标题栏的「工作台」开关
 * 打开(不再在输入框上方常驻一个入口)。复用现有 `chat.taskProgress*` 文案。
 */
export default function WorkspaceStatusBar({
  snapshot,
  executionState = "idle",
  onOpen,
}: WorkspaceStatusBarProps) {
  const { t } = useTranslation()
  const taskBar = shouldShowTaskProgressPanel(snapshot) ? snapshot : null
  if (!taskBar) return null

  const progressKey =
    taskBar.inProgress && executionState === "running"
      ? "chat.taskProgressRunning"
      : taskBar.inProgress && executionState === "cancelling"
        ? "chat.taskProgressCancelling"
        : taskBar.inProgress && executionState === "failed"
          ? "chat.taskProgressFailed"
          : taskBar.inProgress
            ? "chat.taskProgressWaiting"
            : "chat.taskProgress"

  return (
    <button
      type="button"
      onClick={onOpen}
      aria-label={t("workspace.openPanel", "打开工作台")}
      className="flex w-full items-center gap-2 rounded-t-2xl border-b border-border/70 bg-white px-3 py-1.5 text-left transition-colors hover:bg-secondary/45 dark:bg-card"
    >
      <ListChecks
        className={cn(
          "h-3.5 w-3.5 shrink-0",
          executionState === "failed" ? "text-destructive" : "text-blue-500",
        )}
      />
      <span className="min-w-0 flex-1 truncate text-xs">
        <span className="font-medium text-foreground">{t("chat.tasks")}</span>
        <span className="px-1.5 text-muted-foreground">·</span>
        <span className="text-muted-foreground">
          {String(t(progressKey, { completed: taskBar.completed, total: taskBar.total }))}
        </span>
      </span>
      <PanelRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/70" />
    </button>
  )
}

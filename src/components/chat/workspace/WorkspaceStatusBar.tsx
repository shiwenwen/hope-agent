import { useTranslation } from "react-i18next"
import { LayoutDashboard, ListChecks, PanelRight } from "lucide-react"
import { cn } from "@/lib/utils"
import { shouldShowTaskProgressPanel, type TaskProgressSnapshot } from "@/components/chat/tasks/taskProgress"
import type { WorkspaceTaskExecutionState } from "./taskExecutionState"

interface WorkspaceStatusBarProps {
  snapshot: TaskProgressSnapshot | null | undefined
  executionState?: WorkspaceTaskExecutionState
  /** 工作台有无任何内容(任务 / 文件 / 来源)——无未完成任务时,只要有内容仍显示入口,
   *  保证用户关闭面板后还能重新打开。 */
  hasContent?: boolean
  /** 打开 / 激活右侧工作台面板。 */
  onOpen: () => void
}

/**
 * 输入框上方的极简状态条 —— 替代原先内联的 TaskProgressPanel。有未完成任务时显示
 * 「任务 · 运行中 N/M」,否则只要工作台有内容(文件 / 来源)就显示一个通用「工作台」
 * 入口;两者都无则不渲染。点击打开工作台面板。复用现有 `chat.taskProgress*` 文案。
 */
export default function WorkspaceStatusBar({
  snapshot,
  executionState = "idle",
  hasContent = false,
  onOpen,
}: WorkspaceStatusBarProps) {
  const { t } = useTranslation()
  const taskBar = shouldShowTaskProgressPanel(snapshot) ? snapshot : null
  if (!taskBar && !hasContent) return null

  const progressKey =
    taskBar?.inProgress && executionState === "running"
      ? "chat.taskProgressRunning"
      : taskBar?.inProgress && executionState === "cancelling"
        ? "chat.taskProgressCancelling"
        : taskBar?.inProgress && executionState === "failed"
          ? "chat.taskProgressFailed"
          : taskBar?.inProgress
            ? "chat.taskProgressWaiting"
            : "chat.taskProgress"

  return (
    <button
      type="button"
      onClick={onOpen}
      aria-label={t("workspace.openPanel", "打开工作台")}
      className="flex w-full items-center gap-2 rounded-t-2xl border-b border-border/70 bg-white px-3 py-1.5 text-left transition-colors hover:bg-secondary/45 dark:bg-card"
    >
      {taskBar ? (
        <>
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
        </>
      ) : (
        <>
          <LayoutDashboard className="h-3.5 w-3.5 shrink-0 text-blue-500" />
          <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground">
            {t("workspace.panelTitle", "工作台")}
          </span>
        </>
      )}
      <PanelRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/70" />
    </button>
  )
}

import { useTranslation } from "react-i18next"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import type { CronPreflightReport } from "./CronJobForm.types"

interface Props {
  report: CronPreflightReport | null
  busy?: boolean
  confirmLabel: string
  onClose: () => void
  onConfirm: () => void
  onRetry?: () => void
}

export default function CronPreflightDialog({
  report,
  busy = false,
  confirmLabel,
  onClose,
  onConfirm,
  onRetry,
}: Props) {
  const { t } = useTranslation()
  if (!report) return null
  const blockers = report.issues.filter((issue) => issue.severity === "blocker")
  const execution = report.execution
  const environment = [execution.effectivePermissionMode, execution.effectiveSandboxMode]
    .filter(Boolean)
    .join(" · ")
  const model = execution.primaryModel
    ? `${execution.primaryModel.providerId}/${execution.primaryModel.modelId}`
    : null
  const summary = [
    execution.resolvedAgentId,
    execution.projectName && `${t("cron.project")}: ${execution.projectName}`,
    model && `${t("workspace.environment.model")}: ${model}`,
    execution.workspaceMode,
    execution.baseRef && `${t("workspace.git.prBase")}: ${execution.baseRef}`,
    execution.workspaceDirtyFiles != null &&
      t("workspace.worktree.changed", { count: execution.workspaceDirtyFiles }),
    environment,
  ].filter(Boolean)
  const retry = blockers.length > 0 && onRetry
  const issueText = (code: string) => {
    if (code === "remote_workspace_writes_disabled") return t("fileEditor.remoteWritesTitle")
    if (code === "session_unavailable") return t("pet.navigation.unavailable")
    const key =
      code.startsWith("workspace_") || code.startsWith("remote_workspace_")
        ? "workspace.environment.worktree"
        : code.startsWith("project_")
          ? "cron.project"
          : code.startsWith("delivery_")
            ? "cron.deliveryTargets"
            : code.startsWith("sandbox_")
              ? "cron.sandboxMode"
              : code.startsWith("agent_")
                ? "cron.agent"
                : code.startsWith("model_")
                  ? "workspace.environment.model"
                  : code.includes("schedule") || code === "no_future_run"
                    ? "cron.schedule"
                    : code.startsWith("task_")
                      ? "cron.tasks"
                      : "cron.runNow"
    return t("cron.preflightIssue", { code: `${t(key)} · ${code.replaceAll("_", " ")}` })
  }
  return (
    <AlertDialog open onOpenChange={(open) => !open && !busy && onClose()}>
      <AlertDialogContent className="max-w-lg">
        <AlertDialogHeader>
          <AlertDialogTitle>{t("cron.preflightTitle")}</AlertDialogTitle>
          <AlertDialogDescription>{t("cron.preflightDescription")}</AlertDialogDescription>
        </AlertDialogHeader>
        <div className="max-h-[50vh] space-y-2 overflow-y-auto text-xs">
          {summary.length > 0 && (
            <p className="rounded-md bg-muted/30 px-3 py-2">{summary.join(" · ")}</p>
          )}
          {report.nextRuns.length > 0 && (
            <p className="rounded-md bg-muted/30 px-3 py-2">
              {t("cron.nextRun")}:{" "}
              {report.nextRuns.map((run) => new Date(run).toLocaleString()).join(" · ")}
            </p>
          )}
          {report.issues.map((issue) => (
            <p
              key={issue.code}
              className={
                issue.severity === "blocker"
                  ? "rounded-md bg-destructive/10 px-3 py-2 text-destructive"
                  : "rounded-md bg-amber-500/10 px-3 py-2 text-amber-700 dark:text-amber-300"
              }
            >
              {issueText(issue.code)}
            </p>
          ))}
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            disabled={busy || (!retry && !report.canProceed)}
            onClick={(event) => {
              event.preventDefault()
              retry ? retry() : onConfirm()
            }}
          >
            {retry ? t("common.retry") : confirmLabel}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

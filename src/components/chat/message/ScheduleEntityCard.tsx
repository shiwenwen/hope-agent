import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlarmClock, CopyPlus, ExternalLink, RotateCw } from "lucide-react"
import { toast } from "sonner"

import { requestCronTaskDraft, requestCronTaskFocus } from "@/components/cron/cronNavigation"
import { formatSchedule, statusColor, statusLabel } from "@/components/cron/cronHelpers"
import CronPreflightDialog from "@/components/cron/CronPreflightDialog"
import type {
  CronJob,
  CronJobSnapshot,
  CronPreflightReport,
  CronRunLog,
  CronRunNowResult,
  CronWorkspaceMode,
} from "@/components/cron/CronJobForm.types"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import type { ScheduleEntityMetadata } from "@/types/chat"

/** Run-log statuses that leave the occurrence unfinished and worth retrying. */
const FAILED_RUN_STATUSES = new Set(["error", "timeout"])

function formatDateTime(value: string | null | undefined): string | null {
  if (!value) return null
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date)
}

function workspaceLabel(mode: CronWorkspaceMode, t: (key: string) => string): string {
  if (mode === "persistent") return t("cron.workspaceModePersistent")
  if (mode === "fresh") return t("chat.projectRuntime.worktree")
  return t("cron.project")
}

export default function ScheduleEntityCard({
  metadata,
  className,
}: {
  metadata: ScheduleEntityMetadata
  className?: string
}) {
  const { t } = useTranslation()
  // `undefined` = still loading; `null` = the ledger has no row at all.
  const [snapshot, setSnapshot] = useState<CronJobSnapshot | null | undefined>(undefined)
  const [lastRun, setLastRun] = useState<CronRunLog | null>(null)
  const [runPreflight, setRunPreflight] = useState<CronPreflightReport | null>(null)
  const [runNowBusy, setRunNowBusy] = useState(false)

  const refresh = useCallback(async () => {
    // Deleting a task only stops future occurrences, so this card resolves the
    // retained row (tombstone included) instead of the live-only lookup.
    const current = await getTransport().call<CronJobSnapshot | null>("cron_get_job_snapshot", {
      id: metadata.entityId,
    })
    setSnapshot(current ?? null)
    if (!current || current.deleted) {
      setLastRun(null)
      return
    }
    const logs = await getTransport()
      .call<CronRunLog[]>("cron_get_run_logs", { jobId: metadata.entityId, limit: 1 })
      .catch(() => [])
    setLastRun(logs?.[0] ?? null)
  }, [metadata.entityId])

  useEffect(() => {
    void refresh().catch(() => undefined)
    return getTransport().listen("cron:run_completed", (payload) => {
      const event = payload as { job_id?: unknown; jobId?: unknown }
      if (event.job_id === metadata.entityId || event.jobId === metadata.entityId) {
        void refresh().catch(() => undefined)
      }
    })
  }, [metadata.entityId, refresh])

  const job: CronJob | null = snapshot?.job ?? null
  const missing = snapshot === null
  const deleted = missing || !!snapshot?.deleted
  const schedule = job?.schedule ?? metadata.schedule ?? null
  const state = job?.status ?? metadata.state ?? "active"
  const running = Boolean(job?.runningAt)
  const nextRunAt = job?.nextRunAt ?? metadata.nextRunAt ?? null
  const workspaceMode = job?.workspacePolicy.mode ?? metadata.workspaceMode ?? "project"
  const workspaceBaseRef = job?.workspacePolicy.baseRef ?? metadata.workspaceBaseRef ?? null
  const scheduleText = schedule ? formatSchedule(schedule, (key) => t(key)) : null
  const nextRunText = formatDateTime(nextRunAt)
  const lastRunFailed = !deleted && !running && FAILED_RUN_STATUSES.has(lastRun?.status ?? "")
  const stateText = missing
    ? t("cron.jobNotFound")
    : deleted
      ? t("common.deleted")
      : running
        ? t("cron.runStatusRunning")
        : statusLabel(state, (key) => t(key))

  async function handleRunAgain() {
    if (!job) return
    setRunNowBusy(true)
    try {
      const report = await getTransport().call<CronPreflightReport>("cron_preflight", {
        request: { operation: "runNow", jobId: job.id, expectedRevision: job.revision },
      })
      setRunPreflight(report)
    } catch (error) {
      logger.error("cron", "ScheduleEntityCard::preflightRunNow", "Run-now preflight failed", error)
      toast.error(t("cron.preflightTitle"), { description: String(error) })
    } finally {
      setRunNowBusy(false)
    }
  }

  async function confirmRunAgain() {
    if (!job || !runPreflight?.canProceed) return
    setRunNowBusy(true)
    try {
      const result = await getTransport().call<CronRunNowResult>("cron_run_now", {
        id: job.id,
        expectedRevision: job.revision,
      })
      if (result.status === "rejected") {
        setRunPreflight(result.report)
        return
      }
      setRunPreflight(null)
      await refresh()
    } catch (error) {
      logger.error("cron", "ScheduleEntityCard::runNow", "Run-now start failed", error)
      toast.error(t("cron.preflightTitle"), { description: String(error) })
    } finally {
      setRunNowBusy(false)
    }
  }

  return (
    <div
      className={cn(
        "my-2 flex min-w-0 items-center gap-3 rounded-xl border border-border/60 bg-secondary/20 px-3 py-3",
        className,
      )}
    >
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-background text-muted-foreground shadow-sm">
        <AlarmClock className="h-5 w-5" />
      </div>
      <div className="min-w-0 flex-1">
        <div
          className={cn(
            "truncate text-sm font-medium text-foreground",
            deleted && "text-muted-foreground line-through",
          )}
        >
          {job?.name ?? metadata.title ?? t("cron.title")}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
          <span className="inline-flex items-center gap-1.5">
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                deleted ? "bg-muted-foreground" : running ? "bg-blue-500" : statusColor(state),
              )}
            />
            {stateText}
          </span>
          {scheduleText && <span>{scheduleText}</span>}
          {!deleted && nextRunText && schedule?.type !== "at" && (
            <span>
              {t("cron.nextRun")}: {nextRunText}
            </span>
          )}
          {lastRunFailed && <span className="text-red-500">{t("cron.runStatusError")}</span>}
          <span className="rounded bg-secondary px-1.5 py-0.5 text-foreground/80">
            {workspaceLabel(workspaceMode, (key) => t(key))}
            {workspaceBaseRef ? ` · ${workspaceBaseRef}` : ""}
          </span>
        </div>
        {deleted && !missing && (
          <p className="mt-1 text-[11px] leading-4 text-muted-foreground">
            {t("cron.deletedTaskHint")}
          </p>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        {lastRunFailed && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={runNowBusy}
            onClick={() => void handleRunAgain()}
          >
            <RotateCw className="mr-1.5 h-3.5 w-3.5" />
            {t("cron.runAgain")}
          </Button>
        )}
        {deleted ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={!job}
            onClick={() => job && requestCronTaskDraft(job)}
          >
            <CopyPlus className="mr-1.5 h-3.5 w-3.5" />
            {t("cron.copyAsNewTask")}
          </Button>
        ) : (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={snapshot === undefined}
            onClick={() => requestCronTaskFocus(metadata.entityId)}
          >
            {t("fileActions.open")}
            <ExternalLink className="ml-1.5 h-3.5 w-3.5" />
          </Button>
        )}
      </div>
      <CronPreflightDialog
        report={runPreflight}
        busy={runNowBusy}
        confirmLabel={t("cron.runNow")}
        onClose={() => setRunPreflight(null)}
        onConfirm={() => void confirmRunAgain()}
        onRetry={() => void handleRunAgain()}
      />
    </div>
  )
}

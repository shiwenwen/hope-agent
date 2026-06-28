import { useCallback, useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { ArrowUpRight, Cpu, Layers, Terminal, X, XCircle, type LucideIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import {
  type BackgroundJobSnapshot,
  type BackgroundJobStatus,
  backgroundJobLabel,
  isBackgroundJobCancellable,
} from "@/types/background-jobs"
import {
  type LocalModelJobSnapshot,
  localModelJobPercent,
  phaseTranslationKey,
} from "@/types/local-model-jobs"
import { useLocalModelJobsMirror } from "./useLocalModelJobsMirror"
import { BackgroundJobKindIcon, BackgroundJobStatusChip } from "./jobDisplay"
import { resolveBackgroundSubagentSessionId } from "./subagentSession"

function SectionHeader({
  icon: Icon,
  title,
  count,
}: {
  icon: LucideIcon
  title: string
  count: number
}) {
  return (
    <div className="flex items-center gap-2 px-1 pb-1.5 pt-0.5">
      <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <span className="text-xs font-medium text-foreground/80">{title}</span>
      <span className="text-xs font-normal tabular-nums text-muted-foreground">{count}</span>
    </div>
  )
}

function EmptyHint({ children }: { children: ReactNode }) {
  return (
    <div className="px-2 py-4 text-center text-xs text-muted-foreground/70">{children}</div>
  )
}

function BackgroundJobRow({
  job,
  onCancel,
  onViewSubagentSession,
  viewing,
}: {
  job: BackgroundJobSnapshot
  onCancel: (jobId: string) => void
  onViewSubagentSession?: (job: BackgroundJobSnapshot) => void
  viewing?: boolean
}) {
  const { t } = useTranslation()
  const label = backgroundJobLabel(job, t)
  const showGroupProgress =
    job.kind === "group" && job.childCount != null && job.childCount > 0
  const groupDone = job.childrenTerminal ?? 0
  const groupTotal = job.childCount ?? 0
  const groupPct = groupTotal > 0 ? Math.round((groupDone / groupTotal) * 100) : 0
  const cancellable = isBackgroundJobCancellable(job)
  const canViewSubagentSession =
    job.kind === "subagent" && !!job.subagentRunId && !!onViewSubagentSession

  return (
    <div className="flex flex-col gap-1 rounded-md border border-border/50 bg-secondary/30 px-2.5 py-1.5">
      <div className="flex items-center gap-2">
        <BackgroundJobKindIcon
          kind={job.kind}
          className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
        />
        <IconTip label={label}>
          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/90">
            {label}
          </span>
        </IconTip>
        <BackgroundJobStatusChip status={job.status} />
        {canViewSubagentSession && (
          <IconTip label={t("subagent.viewChildSession", "查看子会话")}>
            <button
              type="button"
              onClick={() => onViewSubagentSession(job)}
              disabled={viewing}
              className="rounded p-0.5 text-muted-foreground/60 transition-colors hover:bg-secondary hover:text-foreground disabled:cursor-wait disabled:opacity-50"
              aria-label={t("subagent.viewChildSession", "查看子会话")}
            >
              <ArrowUpRight className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        )}
        {cancellable && (
          <IconTip label={t("common.cancel", "取消")}>
            <button
              type="button"
              onClick={() => onCancel(job.jobId)}
              className="rounded p-0.5 text-muted-foreground/60 transition-colors hover:bg-secondary hover:text-red-500"
              aria-label={t("common.cancel", "取消")}
            >
              <XCircle className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        )}
      </div>
      {showGroupProgress && (
        <div className="flex items-center gap-2">
          <div className="h-1 flex-1 overflow-hidden rounded-full bg-secondary">
            <div
              className="h-full rounded-full bg-blue-500 transition-all duration-300"
              style={{ width: `${groupPct}%` }}
            />
          </div>
          <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
            {groupDone}/{groupTotal}
          </span>
        </div>
      )}
      {job.error && job.status !== "cancelled" && (
        <div className="truncate text-[10px] text-destructive/80" title={job.error}>
          {job.error}
        </div>
      )}
    </div>
  )
}

function LocalModelJobRow({ job }: { job: LocalModelJobSnapshot }) {
  const { t } = useTranslation()
  const pct = localModelJobPercent(job)
  const phaseKey = phaseTranslationKey(job.phase)
  const phaseLabel = phaseKey ? t(phaseKey, job.phase) : job.phase
  return (
    <div className="flex flex-col gap-1 rounded-md border border-border/50 bg-secondary/20 px-2.5 py-1.5">
      <div className="flex items-center gap-2">
        <Cpu className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <IconTip label={job.displayName || job.modelId}>
          <span className="min-w-0 flex-1 truncate text-[11px] text-foreground/85">
            {job.displayName || job.modelId}
          </span>
        </IconTip>
        <span className="shrink-0 text-[10px] text-muted-foreground/80">{phaseLabel}</span>
      </div>
      {pct != null && (
        <div className="flex items-center gap-2">
          <div className="h-1 flex-1 overflow-hidden rounded-full bg-secondary">
            <div
              className="h-full rounded-full bg-sky-500 transition-all duration-300"
              style={{ width: `${Math.round(pct)}%` }}
            />
          </div>
          <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
            {Math.round(pct)}%
          </span>
        </div>
      )}
    </div>
  )
}

/**
 * R4 background-jobs panel: a session's background jobs (cancellable) plus a
 * read-only mirror of the global local-model jobs (downloads / installs /
 * reembeds), so "什么在后台跑" lives in one place. Mounted as a sibling exclusive
 * right panel in {@link ChatScreen}.
 */
export default function BackgroundJobsPanel({
  jobs,
  onClose,
  onViewSubagentSession,
}: {
  jobs: BackgroundJobSnapshot[]
  onClose: () => void
  onViewSubagentSession?: (sessionId: string) => void
}) {
  const { t } = useTranslation()
  // The global model-job mirror only subscribes while this panel is mounted.
  const localModelJobs = useLocalModelJobsMirror()
  const [pendingCancel, setPendingCancel] = useState<Set<string>>(new Set())
  const [pendingViewRunIds, setPendingViewRunIds] = useState<Set<string>>(new Set())

  const clearPending = useCallback((jobId: string) => {
    setPendingCancel((prev) => {
      if (!prev.has(jobId)) return prev
      const next = new Set(prev)
      next.delete(jobId)
      return next
    })
  }, [])

  const handleCancel = useCallback(
    (jobId: string) => {
      setPendingCancel((prev) => new Set(prev).add(jobId))
      getTransport()
        .call<{ accepted?: boolean }>("cancel_runtime_task", { kind: "async_job", id: jobId })
        .then((res) => {
          // Backend declined (job already gone / not found / DB unavailable): no
          // `job:*` event will arrive to reconcile, so revert the optimistic
          // override and re-expose the cancel button instead of getting stuck.
          if (res?.accepted === false) clearPending(jobId)
        })
        .catch(() => clearPending(jobId))
    },
    [clearPending],
  )

  const handleViewSubagentSession = useCallback(
    (job: BackgroundJobSnapshot) => {
      const runId = job.subagentRunId
      if (!runId || !onViewSubagentSession) return
      setPendingViewRunIds((prev) => new Set(prev).add(runId))
      resolveBackgroundSubagentSessionId(runId)
        .then((childSessionId) => {
          if (childSessionId) onViewSubagentSession(childSessionId)
        })
        .catch(() => {})
        .finally(() => {
          setPendingViewRunIds((prev) => {
            if (!prev.has(runId)) return prev
            const next = new Set(prev)
            next.delete(runId)
            return next
          })
        })
    },
    [onViewSubagentSession],
  )

  // Optimistically show "cancelling" for any cancellable job whose cancel is in
  // flight (covers running / queued / awaiting_approval — the full cancellable
  // set). The override is gated on `isBackgroundJobCancellable`, so once a refetch
  // flips the job terminal a lingering id is inert (no stale "cancelling" pin) —
  // and `accepted === false` already self-heals via `clearPending`.
  const visibleJobs = jobs.map((j) =>
    pendingCancel.has(j.jobId) && isBackgroundJobCancellable(j)
      ? ({ ...j, status: "cancelling" as BackgroundJobStatus })
      : j,
  )

  return (
    <div className="flex h-full min-h-0 w-full flex-col overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2">
        <Layers className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="truncate text-sm font-medium">
          {t("backgroundJobs.panelTitle", "后台任务")}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="ml-auto h-7 w-7 shrink-0"
          onClick={onClose}
          aria-label={t("common.close", "关闭")}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <div className="flex-1 space-y-3 overflow-auto p-2">
        <div>
          <SectionHeader
            icon={Terminal}
            title={t("backgroundJobs.sectionSession", "本会话")}
            count={visibleJobs.length}
          />
          {visibleJobs.length > 0 ? (
            <div className="space-y-1">
              {visibleJobs.map((job) => (
                <BackgroundJobRow
                  key={job.jobId}
                  job={job}
                  onCancel={handleCancel}
                  onViewSubagentSession={handleViewSubagentSession}
                  viewing={!!job.subagentRunId && pendingViewRunIds.has(job.subagentRunId)}
                />
              ))}
            </div>
          ) : (
            <EmptyHint>{t("backgroundJobs.empty", "暂无后台任务")}</EmptyHint>
          )}
        </div>

        {localModelJobs.length > 0 && (
          <div>
            <SectionHeader
              icon={Cpu}
              title={t("backgroundJobs.sectionLocalModel", "本地模型")}
              count={localModelJobs.length}
            />
            <div className="space-y-1">
              {localModelJobs.map((job) => (
                <LocalModelJobRow key={job.jobId} job={job} />
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

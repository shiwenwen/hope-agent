import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Activity,
  AlertCircle,
  CheckCircle2,
  Loader2,
  RotateCcw,
  StopCircle,
  Trash2,
  XCircle,
} from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { Progress } from "@/components/ui/progress"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { cn } from "@/lib/utils"
import { useKnowledgeReembedJobs } from "@/hooks/useKnowledgeReembedJobs"
import { kbLabel, type KnowledgeBaseMeta } from "@/types/knowledge"
import {
  isLocalModelJobActive,
  localModelJobPercent,
  type LocalModelJobSnapshot,
  type LocalModelJobStatus,
} from "@/types/local-model-jobs"
import {
  knowledgeJobsActionOperation,
  knowledgeJobsErrorDetail,
  knowledgeJobsErrorToast,
} from "./knowledgeJobsFeedback"

// The backend stamps model_id with this sentinel for an embedding-disabled,
// FTS-only reindex (see knowledge/reembed.rs); a vector re-embed carries the
// embedding config id instead. Orthogonal to scope (single-KB vs full).
const FTS_MODEL_ID = "knowledge-reindex"

const IMPORT_TOOL_NAME = "knowledge_source_import"

/** Friendly relative time ("just now" / "2 min ago") in the active language. */
function formatRelative(unixSec: number, lang: string): string {
  const diff = Math.round(unixSec - Date.now() / 1000) // < 0 = in the past
  const abs = Math.abs(diff)
  const rtf = new Intl.RelativeTimeFormat(lang, { numeric: "auto" })
  if (abs < 60) return rtf.format(diff, "second")
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute")
  if (abs < 86400) return rtf.format(Math.round(diff / 3600), "hour")
  return rtf.format(Math.round(diff / 86400), "day")
}

function statusTone(status: LocalModelJobStatus | ImportJobStatus): string {
  switch (status) {
    case "completed":
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
    case "failed":
    case "interrupted":
    case "timed_out":
      return "border-destructive/30 bg-destructive/10 text-destructive"
    case "cancelled":
    case "paused":
      return "border-muted-foreground/25 bg-muted text-muted-foreground"
    case "cancelling":
      return "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300"
    default:
      return "border-primary/25 bg-primary/10 text-primary"
  }
}

// ── Knowledge source import tracking (async_jobs `job:*`) ─────────────────
//
// Distinct EventBus namespace + payload shape from `local_model_job:*`
// (snake_case, no seed-on-mount query available since `JobManager` only
// exposes session-scoped listing and these jobs have `session_id: null`).
// Purely event-driven: a batch import already running before this component
// mounts only appears once its next per-item `job:progress` tick lands
// (which happens for every item of an active import, so in practice within
// moments) — acceptable since the persistent "N success / M failed" summary
// in the Sources panel and the desktop completion notification don't depend
// on this button having seen the job.

type ImportJobStatus = "running" | "completed" | "failed" | "cancelled" | "timed_out"

interface ImportActivityJob {
  jobId: string
  status: ImportJobStatus
  current: number
  total: number
  importedCount?: number
  duplicateCount?: number
  failedCount?: number
  createdAt: number
  completedAt?: number
}

function nowSecs(): number {
  return Math.floor(Date.now() / 1000)
}

function useKnowledgeImportJobs() {
  const [jobs, setJobs] = useState<Record<string, ImportActivityJob>>({})

  useEffect(() => {
    const onCreated = (raw: unknown) => {
      const ev = parsePayload<{ job_id?: string; tool?: string }>(raw)
      if (!ev?.job_id || ev.tool !== IMPORT_TOOL_NAME) return
      const jobId = ev.job_id
      setJobs((prev) =>
        prev[jobId]
          ? prev
          : { ...prev, [jobId]: { jobId, status: "running", current: 0, total: 0, createdAt: nowSecs() } },
      )
    }
    const onProgress = (raw: unknown) => {
      const ev = parsePayload<{ job_id?: string; current?: number; total?: number }>(raw)
      if (!ev?.job_id) return
      const jobId = ev.job_id
      setJobs((prev) => {
        const existing = prev[jobId]
        if (!existing) return prev
        return { ...prev, [jobId]: { ...existing, current: ev.current ?? existing.current, total: ev.total ?? existing.total } }
      })
    }
    const onCompleted = (raw: unknown) => {
      const ev = parsePayload<{
        job_id?: string
        tool?: string
        status?: string
        imported_count?: number
        duplicate_count?: number
        failed_count?: number
      }>(raw)
      if (!ev?.job_id || ev.tool !== IMPORT_TOOL_NAME) return
      const jobId = ev.job_id
      setJobs((prev) => {
        const existing = prev[jobId]
        return {
          ...prev,
          [jobId]: {
            jobId,
            status: (ev.status as ImportJobStatus) ?? "completed",
            current: existing?.current ?? 0,
            total: existing?.total ?? 0,
            importedCount: ev.imported_count,
            duplicateCount: ev.duplicate_count,
            failedCount: ev.failed_count,
            createdAt: existing?.createdAt ?? nowSecs(),
            completedAt: nowSecs(),
          },
        }
      })
    }
    const un1 = getTransport().listen("job:created", onCreated)
    const un2 = getTransport().listen("job:progress", onProgress)
    const un3 = getTransport().listen("job:completed", onCompleted)
    return () => {
      un1()
      un2()
      un3()
    }
  }, [])

  return useMemo(() => Object.values(jobs).sort((a, b) => b.createdAt - a.createdAt), [jobs])
}

function isImportJobActive(job: ImportActivityJob): boolean {
  return job.status === "running"
}

/**
 * Top-right "knowledge space activity" entry: a bell-style icon that pulses
 * while anything is running and opens a floating panel listing every
 * reindex/re-embed job (`local_model_job:*`, covers settings-page rebuild,
 * per-space Reindex, and binding a new external space — all the same
 * `knowledge_reembed` job kind) together with every source-import batch
 * (`job:*`), merged into one time-sorted list. `kbs` is used to render the
 * target space's name instead of a raw id for single-KB-scoped jobs.
 *
 * Self-contained: seeded from `local_model_job_list` for the reembed half
 * (survives navigation / refresh / app restart), kept live via both event
 * streams.
 */
export default function KnowledgeActivityButton({ kbs }: { kbs: KnowledgeBaseMeta[] }) {
  const { t } = useTranslation()
  const {
    jobs: reembedJobs,
    activeCount: reembedActiveCount,
    dismiss: dismissReembedJob,
    loadError: reembedLoadError,
  } = useKnowledgeReembedJobs()
  const importJobs = useKnowledgeImportJobs()
  const [open, setOpen] = useState(false)
  const [actioning, setActioning] = useState<Record<string, boolean>>({})
  const rootRef = useRef<HTMLDivElement>(null)

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false)
    }
    document.addEventListener("mousedown", onDown)
    document.addEventListener("keydown", onKey)
    return () => {
      document.removeEventListener("mousedown", onDown)
      document.removeEventListener("keydown", onKey)
    }
  }, [open])

  const importActiveCount = useMemo(() => importJobs.filter(isImportJobActive).length, [importJobs])
  const activeCount = reembedActiveCount + importActiveCount
  const isEmpty = reembedJobs.length === 0 && importJobs.length === 0
  const loadError = useMemo(
    () =>
      reembedLoadError ? knowledgeJobsErrorToast("loadJobs", t, reembedLoadError) : null,
    [reembedLoadError, t],
  )

  const runAction = useCallback(
    async (jobId: string, action: "cancel" | "retry" | "clear") => {
      setActioning((prev) => ({ ...prev, [jobId]: true }))
      try {
        if (action === "cancel") {
          await getTransport().call<LocalModelJobSnapshot>("local_model_job_cancel", { jobId })
        } else if (action === "retry") {
          await getTransport().call<LocalModelJobSnapshot>("local_model_job_retry", { jobId })
        } else {
          await getTransport().call("local_model_job_clear", { jobId })
          // `local_model_job_clear` deletes the DB row but emits no event, so
          // the hook's local state needs an explicit nudge or the cleared row
          // keeps showing until the next full reseed.
          dismissReembedJob(jobId)
        }
      } catch (e) {
        const failure = knowledgeJobsErrorToast(knowledgeJobsActionOperation(action), t, e)
        toast.error(
          failure.title,
          failure.description ? { description: failure.description } : undefined,
        )
      } finally {
        setActioning((prev) => {
          const next = { ...prev }
          delete next[jobId]
          return next
        })
      }
    },
    [dismissReembedJob, t],
  )

  return (
    <div ref={rootRef} className="relative">
      <IconTip label={t("knowledge.jobs.tooltip", "Knowledge activity")} side="bottom">
        <Button
          variant="ghost"
          size="icon"
          className="relative h-8 w-8"
          onClick={() => setOpen((v) => !v)}
        >
          <Activity className="h-4 w-4" />
          {activeCount > 0 && (
            <span className="absolute right-1 top-1 flex h-2 w-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-primary/70" />
              <span className="relative inline-flex h-2 w-2 rounded-full bg-primary" />
            </span>
          )}
        </Button>
      </IconTip>

      <FloatingMenu
        open={open}
        positionClassName="top-full right-0 mt-1.5"
        originClassName="origin-top-right"
        className="ha-menu-from-top w-[340px] overflow-hidden p-0"
        onEscapeKeyDown={() => setOpen(false)}
      >
          <div className="flex items-center justify-between border-b border-border-soft/60 px-3 py-2">
            <span className="text-xs font-medium">{t("knowledge.jobs.title", "Knowledge activity")}</span>
            {activeCount > 0 && (
              <span className="flex items-center gap-1 text-[11px] text-primary">
                <Loader2 className="h-3 w-3 animate-spin" />
                {activeCount}
              </span>
            )}
          </div>
          <div className="max-h-[360px] overflow-y-auto p-2">
            {loadError ? (
              <div className="mb-2 rounded-md border border-destructive/25 bg-destructive/10 p-2 text-[11px] text-destructive">
                <div className="flex items-center gap-1.5 font-medium">
                  <AlertCircle className="h-3.5 w-3.5" />
                  {loadError.title}
                </div>
                {loadError.description ? (
                  <div className="mt-1 leading-relaxed">{loadError.description}</div>
                ) : null}
              </div>
            ) : null}
            {isEmpty ? (
              <div className="flex flex-col items-center justify-center gap-1.5 px-4 py-8 text-center">
                <Activity className="h-6 w-6 text-muted-foreground/70" />
                <span className="text-xs font-medium">
                  {t("knowledge.jobs.empty", "No activity yet.")}
                </span>
                <span className="text-[11px] leading-relaxed text-muted-foreground">
                  {t(
                    "knowledge.jobs.emptyHint",
                    "Bind or reindex a space, or import files, and progress shows up here.",
                  )}
                </span>
              </div>
            ) : (
              <div className="space-y-2">
                {reembedJobs.map((job) => (
                  <ReembedJobRow
                    key={job.jobId}
                    job={job}
                    kbs={kbs}
                    busy={!!actioning[job.jobId]}
                    onAction={runAction}
                  />
                ))}
                {importJobs.map((job) => (
                  <ImportJobRow key={job.jobId} job={job} />
                ))}
              </div>
            )}
          </div>
      </FloatingMenu>
    </div>
  )
}

function ReembedJobRow({
  job,
  kbs,
  busy,
  onAction,
}: {
  job: LocalModelJobSnapshot
  kbs: KnowledgeBaseMeta[]
  busy: boolean
  onAction: (jobId: string, action: "cancel" | "retry" | "clear") => void
}) {
  const { t, i18n } = useTranslation()
  const active = isLocalModelJobActive(job)
  const isFts = job.modelId === FTS_MODEL_ID
  const singleKbId = job.targetKbIds?.length === 1 ? job.targetKbIds[0] : null
  const singleKb = singleKbId ? kbs.find((kb) => kb.id === singleKbId) : null
  // File-granular scope (bind / per-KB reindex): bytesCompleted/bytesTotal
  // carry file counts. KB-granular scope (rare "rebuild everything"): they
  // carry KB counts, as before.
  const done = Number(job.bytesCompleted ?? 0)
  const total = Number(job.bytesTotal ?? 0)
  const percent = localModelJobPercent(job)
  const result = (job.resultJson ?? null) as
    | { reindexed?: number; failedFiles?: number; kbCount?: number }
    | null

  const title = isFts
    ? t("knowledge.jobs.kindReindex", "Rebuild index")
    : t("knowledge.jobs.kindReembed", "Re-embed vectors")
  const subject = singleKbId
    ? t("knowledge.jobs.space", {
        name: singleKb ? kbLabel(singleKb.emoji, singleKb.name) : singleKbId,
        defaultValue: "Space: {{name}}",
      })
    : isFts
      ? t("knowledge.jobs.fts", "Full-text index")
      : t("knowledge.jobs.model", { name: job.displayName, defaultValue: "Model: {{name}}" })
  const progressLabel = singleKbId
    ? t("knowledge.jobs.progressFiles", { done, total, defaultValue: "{{done}} / {{total}} files" })
    : t("settings.knowledgeEmbedding.reembed.progress", { done, total })
  const when = job.completedAt ?? job.createdAt

  return (
    <div className="rounded-md border border-border bg-card p-2.5">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <StatusIcon status={job.status} />
          <span className="truncate text-xs font-medium" title={title}>
            {title}
          </span>
        </div>
        <span
          className={cn(
            "shrink-0 rounded border px-1.5 py-0.5 text-[10px] font-medium",
            statusTone(job.status),
          )}
        >
          {t(`localModelJobs.status.${job.status}`)}
        </span>
      </div>

      <div className="mt-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <span className="truncate" title={subject}>
          {subject}
        </span>
        <span className="shrink-0">·</span>
        <span className="shrink-0">{formatRelative(when, i18n.language)}</span>
      </div>

      {active && (
        <div className="mt-2">
          <Progress value={percent} indeterminate={percent == null} className="h-1.5" />
          {total > 0 && <div className="mt-1 text-[11px] text-muted-foreground">{progressLabel}</div>}
        </div>
      )}

      {job.status === "completed" && (
        <div className="mt-1.5 text-[11px] text-muted-foreground">
          {(result?.failedFiles ?? 0) > 0
            ? t("knowledge.jobs.resultPartial", {
                notes: result?.reindexed ?? 0,
                failed: result?.failedFiles ?? 0,
                defaultValue: "Reindexed {{notes}} notes · {{failed}} skipped",
              })
            : t("knowledge.jobs.resultOk", {
                notes: result?.reindexed ?? 0,
                defaultValue: "Reindexed {{notes}} notes",
              })}
        </div>
      )}

      {job.error && (
        <p className="mt-1.5 break-words text-[11px] text-destructive">
          {knowledgeJobsErrorDetail(job.error)}
        </p>
      )}

      <div className="mt-2 flex justify-end gap-1.5">
        {active && (
          <Button
            variant="outline"
            size="sm"
            className="h-6 px-2 text-[11px]"
            disabled={busy}
            onClick={() => onAction(job.jobId, "cancel")}
          >
            {busy ? (
              <Loader2 className="mr-1 h-3 w-3 animate-spin" />
            ) : (
              <XCircle className="mr-1 h-3 w-3" />
            )}
            {t("localModelJobs.actions.cancel", "Cancel")}
          </Button>
        )}
        {!active && job.status !== "completed" && job.status !== "cancelled" && (
          <Button
            variant="outline"
            size="sm"
            className="h-6 px-2 text-[11px]"
            disabled={busy}
            onClick={() => onAction(job.jobId, "retry")}
          >
            <RotateCcw className="mr-1 h-3 w-3" />
            {t("localModelJobs.actions.retry", "Retry")}
          </Button>
        )}
        {!active && (
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-2 text-[11px]"
            disabled={busy}
            onClick={() => onAction(job.jobId, "clear")}
          >
            <Trash2 className="mr-1 h-3 w-3" />
            {t("localModelJobs.actions.clear", "Clear")}
          </Button>
        )}
      </div>
    </div>
  )
}

function ImportJobRow({ job }: { job: ImportActivityJob }) {
  const { t, i18n } = useTranslation()
  const active = isImportJobActive(job)
  const percent = job.total > 0 ? Math.min(100, (job.current / job.total) * 100) : null
  const when = job.completedAt ?? job.createdAt

  return (
    <div className="rounded-md border border-border bg-card p-2.5">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <StatusIcon status={job.status} />
          <span className="truncate text-xs font-medium">
            {t("knowledge.jobs.kindImport", "Import sources")}
          </span>
        </div>
        <span className={cn("shrink-0 rounded border px-1.5 py-0.5 text-[10px] font-medium", statusTone(job.status))}>
          {t(`knowledge.jobs.importStatus.${job.status}`, job.status)}
        </span>
      </div>

      <div className="mt-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <span className="shrink-0">{formatRelative(when, i18n.language)}</span>
      </div>

      {active && (
        <div className="mt-2">
          <Progress value={percent} indeterminate={percent == null} className="h-1.5" />
          {job.total > 0 && (
            <div className="mt-1 text-[11px] text-muted-foreground">
              {t("knowledge.jobs.progressItems", {
                done: job.current,
                total: job.total,
                defaultValue: "{{done}} / {{total}} sources",
              })}
            </div>
          )}
        </div>
      )}

      {!active && job.importedCount != null && (
        <div className="mt-1.5 text-[11px] text-muted-foreground">
          {(job.failedCount ?? 0) > 0
            ? t("knowledge.jobs.importResultPartial", {
                imported: job.importedCount,
                failed: job.failedCount,
                defaultValue: "Imported {{imported}} · {{failed}} failed",
              })
            : t("knowledge.jobs.importResultOk", {
                imported: job.importedCount,
                defaultValue: "Imported {{imported}} sources",
              })}
        </div>
      )}
    </div>
  )
}

function StatusIcon({ status }: { status: LocalModelJobStatus | ImportJobStatus }) {
  switch (status) {
    case "running":
    case "cancelling":
      return <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-primary" />
    case "completed":
      return <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
    case "failed":
    case "interrupted":
    case "timed_out":
      return <AlertCircle className="h-3.5 w-3.5 shrink-0 text-destructive" />
    case "cancelled":
    case "paused":
      return <StopCircle className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
    default:
      return <StopCircle className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
  }
}

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
import { Progress } from "@/components/ui/progress"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import {
  isLocalModelJobActive,
  localModelJobPercent,
  LOCAL_MODEL_JOB_EVENTS,
  type LocalModelJobSnapshot,
  type LocalModelJobStatus,
} from "@/types/local-model-jobs"

// This floating panel is scoped to knowledge rebuild jobs only — the per-space
// reindex / re-embed jobs spawned via `start_knowledge_reembed_job` (toolbar 🔄,
// space context-menu "Reindex", and embedding model switches). Folder/note
// reindex run synchronously (no job) so they never appear here.
const KNOWLEDGE_JOB_KIND = "knowledge_reembed"

// The backend stamps model_id with this sentinel for an embedding-disabled,
// FTS-only reindex (see knowledge/reembed.rs); a vector re-embed carries the
// embedding config id instead. We branch the localized title/subtitle on it
// rather than on the English-only `displayName`.
const FTS_MODEL_ID = "knowledge-reindex"

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

function statusTone(status: LocalModelJobStatus): string {
  switch (status) {
    case "completed":
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
    case "failed":
    case "interrupted":
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

/**
 * Top-right "rebuild tasks" entry for the Knowledge view: a bell-style icon that
 * pulses while a rebuild runs and opens a floating panel listing every knowledge
 * reindex/re-embed job with live progress + per-job cancel / retry / clear.
 *
 * Self-contained: owns its own job list (seeded from `local_model_job_list`,
 * kept live via the LocalModelJobs event stream, filtered to knowledge jobs) so
 * it survives navigation / refresh / app restart the same way the toolbar 🔄
 * progress does.
 */
export default function KnowledgeJobsButton() {
  const { t } = useTranslation()
  const [jobs, setJobs] = useState<LocalModelJobSnapshot[]>([])
  const [open, setOpen] = useState(false)
  const [actioning, setActioning] = useState<Record<string, boolean>>({})
  const rootRef = useRef<HTMLDivElement>(null)

  const upsert = useCallback((job: LocalModelJobSnapshot) => {
    if (job.kind !== KNOWLEDGE_JOB_KIND) return
    setJobs((prev) => {
      const idx = prev.findIndex((j) => j.jobId === job.jobId)
      if (idx === -1) return [job, ...prev]
      const next = [...prev]
      next[idx] = job
      return next
    })
  }, [])

  const refresh = useCallback(async () => {
    try {
      const list = await getTransport().call<LocalModelJobSnapshot[]>("local_model_job_list")
      setJobs(list.filter((j) => j.kind === KNOWLEDGE_JOB_KIND))
    } catch (e) {
      logger.warn("knowledge", "KnowledgeJobsButton::refresh", "Failed to load jobs", e)
    }
  }, [])

  // Initial load + live updates. setState here runs inside async/event callbacks,
  // never synchronously in the effect body (no cascading render).
  useEffect(() => {
    void refresh()
    const onSnap = (raw: unknown) => {
      const job = parsePayload<LocalModelJobSnapshot>(raw)
      if (!job) return
      upsert(job)
    }
    const un1 = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.created, onSnap)
    const un2 = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.updated, onSnap)
    const un3 = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.completed, onSnap)
    return () => {
      un1()
      un2()
      un3()
    }
  }, [refresh, upsert])

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

  const sorted = useMemo(() => [...jobs].sort((a, b) => b.createdAt - a.createdAt), [jobs])
  const activeCount = useMemo(() => sorted.filter(isLocalModelJobActive).length, [sorted])

  const runAction = useCallback(
    async (jobId: string, action: "cancel" | "retry" | "clear") => {
      setActioning((prev) => ({ ...prev, [jobId]: true }))
      try {
        if (action === "cancel") {
          upsert(
            await getTransport().call<LocalModelJobSnapshot>("local_model_job_cancel", { jobId }),
          )
        } else if (action === "retry") {
          upsert(
            await getTransport().call<LocalModelJobSnapshot>("local_model_job_retry", { jobId }),
          )
        } else {
          await getTransport().call("local_model_job_clear", { jobId })
          setJobs((prev) => prev.filter((j) => j.jobId !== jobId))
        }
      } catch (e) {
        toast.error(String(e))
      } finally {
        setActioning((prev) => {
          const next = { ...prev }
          delete next[jobId]
          return next
        })
      }
    },
    [upsert],
  )

  return (
    <div ref={rootRef} className="relative">
      <IconTip label={t("knowledge.jobs.tooltip", "Rebuild tasks")} side="bottom">
        <Button
          variant="ghost"
          size="icon"
          className="relative h-8 w-8"
          onClick={() => {
            // Re-seed on open so a fresh open is never stale (events keep an open
            // panel live, but the list could have drifted while it was closed).
            if (!open) void refresh()
            setOpen((v) => !v)
          }}
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

      {open && (
        <div className="absolute right-0 top-full z-50 mt-1 w-[340px] rounded-lg border border-border bg-popover shadow-lg">
          <div className="flex items-center justify-between border-b border-border-soft/60 px-3 py-2">
            <span className="text-xs font-medium">{t("knowledge.jobs.title", "Rebuild tasks")}</span>
            {activeCount > 0 && (
              <span className="flex items-center gap-1 text-[11px] text-primary">
                <Loader2 className="h-3 w-3 animate-spin" />
                {activeCount}
              </span>
            )}
          </div>
          <div className="max-h-[360px] overflow-y-auto p-2">
            {sorted.length === 0 ? (
              <div className="flex flex-col items-center justify-center gap-1.5 px-4 py-8 text-center">
                <Activity className="h-6 w-6 text-muted-foreground/70" />
                <span className="text-xs font-medium">
                  {t("knowledge.jobs.empty", "No rebuild tasks yet.")}
                </span>
                <span className="text-[11px] leading-relaxed text-muted-foreground">
                  {t(
                    "knowledge.jobs.emptyHint",
                    "Reindex a space, or switch the embedding model, and progress shows up here.",
                  )}
                </span>
              </div>
            ) : (
              <div className="space-y-2">
                {sorted.map((job) => (
                  <JobRow
                    key={job.jobId}
                    job={job}
                    busy={!!actioning[job.jobId]}
                    onAction={runAction}
                  />
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function JobRow({
  job,
  busy,
  onAction,
}: {
  job: LocalModelJobSnapshot
  busy: boolean
  onAction: (jobId: string, action: "cancel" | "retry" | "clear") => void
}) {
  const { t, i18n } = useTranslation()
  const active = isLocalModelJobActive(job)
  const isFts = job.modelId === FTS_MODEL_ID
  // done/total KB counts ride on bytesCompleted/bytesTotal (the job is KB-granular).
  const done = Number(job.bytesCompleted ?? 0)
  const total = Number(job.bytesTotal ?? 0)
  const percent = localModelJobPercent(job)
  const result = (job.resultJson ?? null) as { reindexed?: number; kbCount?: number } | null

  // Localized title by task type — the backend `displayName` is English-only.
  const title = isFts
    ? t("knowledge.jobs.kindReindex", "Rebuild index")
    : t("knowledge.jobs.kindReembed", "Re-embed vectors")
  // Secondary descriptor: a "full-text" tag for FTS rebuilds (no embedding model
  // involved), or the embedding model in use for a vector re-embed.
  const subject = isFts
    ? t("knowledge.jobs.fts", "Full-text index")
    : t("knowledge.jobs.model", { name: job.displayName, defaultValue: "Model: {{name}}" })
  // Terminal → when it finished; active → when it started.
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
          {total > 0 && (
            <div className="mt-1 text-[11px] text-muted-foreground">
              {t("settings.knowledgeEmbedding.reembed.progress", { done, total })}
            </div>
          )}
        </div>
      )}

      {job.status === "completed" && (
        <div className="mt-1.5 text-[11px] text-muted-foreground">
          {t("knowledge.jobs.result", {
            notes: result?.reindexed ?? 0,
            kbs: result?.kbCount ?? total,
            defaultValue: "Rebuilt {{notes}} notes · {{kbs}} spaces",
          })}
        </div>
      )}

      {job.error && <p className="mt-1.5 break-words text-[11px] text-destructive">{job.error}</p>}

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

function StatusIcon({ status }: { status: LocalModelJobStatus }) {
  switch (status) {
    case "running":
    case "cancelling":
      return <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-primary" />
    case "completed":
      return <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
    case "failed":
    case "interrupted":
      return <AlertCircle className="h-3.5 w-3.5 shrink-0 text-destructive" />
    case "cancelled":
    case "paused":
      return <StopCircle className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
  }
}
